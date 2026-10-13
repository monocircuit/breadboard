//! The main checkout follows its default branch by itself (#348): `breadboard
//! serve` looks every few minutes and right after a merge it saw (`wake`),
//! and fast-forwards the branch when it safely can. Only then: on the branch,
//! no changes to tracked files, `--ff-only`. Anything else is skipped with
//! its reason, never stashed, reset or forced. After a move the checkout's
//! install runs when its lockfile changed and breadboard's binary is built
//! again when its sources did (the overviews and the daemon follow it by
//! themselves); new migrations are only named, the worktrees run them. How
//! and whether: `[main_checkout]` of breadboard.toml (`Settings`).
//!
//! What it did lives in `<git common dir>/breadboard/develop.json`, shared
//! by every daemon and overview of the clone: when it last looked, the last
//! update, why the last look skipped, and the steps after it. The overviews
//! show it in their band (`band`).

use crate::git::{self, git};
use crate::model::develop::merges_in;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// A fetch that takes longer is given up (a network that hangs, a key that asks).
const FETCH_WAIT: Duration = Duration::from_secs(60);
/// A look's lock older than this belongs to a look that died.
const LOOK_STALE: Duration = Duration::from_secs(5 * 60);

/// What breadboard's binary is built from (the launcher's `source_now`).
pub const BREADBOARD_SOURCES: [&str; 4] = [
    "packages/breadboard/crates/",
    "packages/breadboard/hooks/",
    "packages/breadboard/Cargo.toml",
    "packages/breadboard/Cargo.lock",
];
/// The launcher of the main checkout's binary.
pub const LAUNCHER: &str = "packages/breadboard/herdr/breadboard.sh";

/// What the repo's breadboard.toml says of it (`[main_checkout]`, the branch, the migrations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub is_on: bool,
    /// How often it looks when nothing woke it.
    pub every: Duration,
    /// The branch it follows: `develop`.
    pub branch: String,
    /// What runs after an update that changed a file of `install_when`.
    pub install: Option<String>,
    pub install_when: Vec<String>,
    /// The migrations' journal in the checkout, and what runs them in a stack.
    pub journal: Option<String>,
    pub migrate: Option<String>,
}

impl Settings {
    /// The settings of the repo whose main checkout is `main`.
    pub fn of(main: &Path) -> Settings {
        let loaded = crate::config::of_main(main);
        let config = &loaded.config.main_checkout;
        let migrations = loaded
            .config
            .stack
            .as_ref()
            .and_then(|stack| Some((stack, stack.migrations.as_ref()?)));
        Settings {
            is_on: config.follow,
            every: Duration::from_secs(config.every_minutes.saturating_mul(60)),
            branch: loaded.branch.clone(),
            install: config.install.clone(),
            install_when: config.install_when.clone(),
            journal: migrations.map(|(stack, migrations)| {
                let dir = stack.dir.trim_start_matches("./").trim_end_matches('/');
                if dir.is_empty() || dir == "." {
                    migrations.journal.clone()
                } else {
                    format!("{dir}/{}", migrations.journal)
                }
            }),
            migrate: migrations.map(|(_, migrations)| migrations.run.clone()),
        }
    }

    /// The step `install` is shown as: its first two words, `pnpm install`.
    fn install_name(&self) -> Option<String> {
        let install = self.install.as_deref()?;
        Some(install.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
    }
}

/// What the file says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// The branch it follows, as the band names it.
    pub branch: String,
    /// When it last looked (ms), whatever came of it.
    pub looked_at: i64,
    /// The last update that moved develop.
    pub updated: Option<Update>,
    /// Why the last look didn't update; None: it was up to date or moved.
    pub skipped: Option<Skip>,
    /// The steps after the last update, in order.
    pub steps: Vec<Step>,
}

/// An update that moved develop.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Update {
    pub at: i64,
    pub from: String,
    pub to: String,
    pub commits: u64,
    /// The MRs merged in it, newest first.
    pub mrs: Vec<i64>,
    /// It brought migrations: what runs them in each stack (`pnpm pg:migrate`).
    pub migrate: Option<String>,
}

/// Why a look left develop as it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skip {
    pub at: i64,
    /// `local changes in a.rs, b.rs`
    pub why: String,
    /// The commits it would have brought (0: none known).
    pub behind: u64,
}

/// A step after an update: `pnpm install`, the binary's build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// `pnpm install`, `breadboard`
    pub what: String,
    pub state: StepState,
    /// Its output, for when it failed.
    pub log: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Running,
    Done,
    Failed,
}

/// What a look came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look {
    UpToDate,
    /// It moved: the update and the files it changed.
    Moved(Update, Vec<String>),
    Skipped(Skip),
}

/// The folder of the clone's shared files (the launcher's store): `<git common dir>/breadboard`.
pub fn store_of(main: &Path) -> Option<PathBuf> {
    git::git_dirs(main).map(|(_, common)| common.join("breadboard"))
}

pub fn state_file(store: &Path) -> PathBuf {
    store.join("develop.json")
}

fn wake_file(store: &Path) -> PathBuf {
    store.join("develop.wake")
}

/// The state, empty when there is none (never looked, or a file it can't read).
pub fn read(store: &Path) -> State {
    std::fs::read_to_string(state_file(store))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// `change` on the state, under its lock (two daemons of the clone, each
/// with a step running, never undo the other's write).
pub fn change(store: &Path, change: impl FnOnce(&mut State)) -> std::io::Result<()> {
    if !has_store(store) {
        return Ok(());
    }
    let file = state_file(store);
    crate::lock::with_dir_lock(&file, Duration::from_secs(10), || {
        let mut state = read(store);
        change(&mut state);
        let text = serde_json::to_string_pretty(&state).unwrap_or_default();
        let new = file.with_extension("json.new");
        std::fs::write(&new, format!("{text}\n")).and_then(|()| std::fs::rename(&new, &file))
    })?
}

/// Something merged (the overview's own merge, the branch moved on GitLab):
/// the next look comes now, not after its while.
pub fn wake(main: &Path) {
    if let Some(store) = store_of(main)
        && has_store(&store)
    {
        let _ = std::fs::write(wake_file(&store), crate::time::now_ms().to_string());
    }
}

/// The store there, made when its git directory is: never a git directory
/// made again under a checkout being removed (a gauntlet world's end).
fn has_store(store: &Path) -> bool {
    store.is_dir() || std::fs::create_dir(store).is_ok()
}

/// Whether a look is due at `now`: `every` since the last, or woken since.
pub fn is_due(store: &Path, now: i64, every: Duration) -> bool {
    let state = read(store);
    let every = i64::try_from(every.as_millis()).unwrap_or(i64::MAX);
    if now - state.looked_at >= every {
        return true;
    }
    let woken = std::fs::metadata(wake_file(store))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |at| i64::try_from(at.as_millis()).unwrap_or(0));
    woken >= state.looked_at
}

/// One look at the main checkout `main` when one is due, under the look's
/// lock (one daemon of the clone looks, another finds it done; due is asked
/// again under it): the state written, what came of it. None: not due, or
/// another looks right now.
pub fn look_if_due(main: &Path, store: &Path, settings: &Settings) -> Option<Look> {
    // Asked every few seconds: nothing is written before a look is due.
    if !is_due(store, crate::time::now_ms(), settings.every) || !has_store(store) {
        return None;
    }
    let lock = store.join("develop.look");
    if std::fs::create_dir(crate::lock::lock_of(&lock)).is_err() {
        // Another daemon looks; one whose look died long ago is taken over next time.
        if crate::lock::age_of(&crate::lock::lock_of(&lock)).is_some_and(|age| age > LOOK_STALE) {
            let _ = std::fs::remove_dir_all(crate::lock::lock_of(&lock));
        }
        return None;
    }
    let now = crate::time::now_ms();
    let look = is_due(store, now, settings.every).then(|| {
        let look = look(main, now, settings);
        let _ = change(store, |state| {
            state.branch = settings.branch.clone();
            record(state, &look, now);
        });
        look
    });
    let _ = std::fs::remove_dir_all(crate::lock::lock_of(&lock));
    look
}

/// What a look says in the state.
pub fn record(state: &mut State, look: &Look, now: i64) {
    state.looked_at = now;
    match look {
        Look::UpToDate => state.skipped = None,
        Look::Moved(update, _) => {
            state.skipped = None;
            state.updated = Some(update.clone());
            state.steps.clear();
        }
        Look::Skipped(skip) => state.skipped = Some(skip.clone()),
    }
}

/// One look: fetch, then fast-forward when it safely can.
pub fn look(main: &Path, now: i64, settings: &Settings) -> Look {
    let skip = |why: String, behind: u64| Look::Skipped(Skip { at: now, why, behind });
    let branch = git(main, &["branch", "--show-current"]).out().to_string();
    if branch != settings.branch {
        let on = if branch.is_empty() {
            "a detached HEAD".into()
        } else {
            branch
        };
        return skip(format!("the main checkout is on {on}"), 0);
    }
    if let Err(why) = fetch(main, &settings.branch) {
        return skip(format!("git fetch failed: {why}"), 0);
    }
    let upstream = format!("origin/{}", settings.branch);
    let behind = git::count(main, &format!("HEAD..{upstream}")).unwrap_or(0);
    if behind == 0 {
        return Look::UpToDate;
    }
    let ahead = git::count(main, &format!("{upstream}..HEAD")).unwrap_or(0);
    if ahead > 0 {
        let commits = if ahead == 1 { "commit" } else { "commits" };
        return skip(format!("diverged, {ahead} {commits} of its own"), behind);
    }
    let changed: Vec<String> = git(main, &["status", "--porcelain", "--untracked-files=no"])
        .lines()
        .map(git::path_of_status)
        .collect();
    if !changed.is_empty() {
        return skip(format!("local changes in {}", names(&changed)), behind);
    }
    let from = git(main, &["rev-parse", "HEAD"]).out().to_string();
    let merged = git(main, &["merge", "--ff-only", "--quiet", &upstream]);
    if !merged.is_ok {
        let why = merged.stderr.lines().find(|line| !line.trim().is_empty()).unwrap_or("");
        return skip(format!("git merge --ff-only failed: {}", why.trim()), behind);
    }
    let to = git(main, &["rev-parse", "HEAD"]).out().to_string();
    let range = format!("{from}..{to}");
    let files: Vec<String> = git(main, &["diff", "--name-only", &range])
        .lines()
        .map(String::from)
        .collect();
    let (merges, _) = merges_in(main, &range);
    let update = Update {
        at: now,
        commits: git::count(main, &range).unwrap_or(behind),
        mrs: merges.iter().filter_map(|merge| merge.mr).collect(),
        migrate: settings
            .journal
            .as_ref()
            .filter(|journal| files.contains(journal))
            .and(settings.migrate.clone()),
        from,
        to,
    };
    Look::Moved(update, files)
}

/// `a.rs, b.rs and 3 more`
fn names(files: &[String]) -> String {
    match files {
        [one] => one.clone(),
        [one, two] => format!("{one}, {two}"),
        [one, two, rest @ ..] => format!("{one}, {two} and {} more", rest.len()),
        [] => String::new(),
    }
}

/// `git fetch origin <branch>`, never asking (no terminal, no prompt), given
/// up after FETCH_WAIT (the daemon has no terminal to ask on); Err: git's first line, or that it hung.
fn fetch(main: &Path, branch: &str) -> Result<(), String> {
    let mut child = Command::new("git")
        .args(["fetch", "--quiet", "origin", branch])
        .current_dir(main)
        .env("GIT_TERMINAL_PROMPT", "0")
        // ssh's passphrase or a GUI's askpass would wait for no one: it fails instead.
        .env("SSH_ASKPASS_REQUIRE", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => {
                let mut said = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut said);
                }
                let line = said.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
                return Err(line.trim_start_matches("fatal: ").to_string());
            }
            Ok(None) if started.elapsed() < FETCH_WAIT => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer in {} s", FETCH_WAIT.as_secs()));
            }
        }
    }
}

/// The steps an update's files call for, in order: the install when a file
/// of `install_when` changed, breadboard's build when its sources did (and
/// the checkout has breadboard's launcher).
pub fn steps_for(main: &Path, files: &[String], settings: &Settings) -> Vec<String> {
    let mut steps = vec![];
    if let Some(install) = settings.install_name()
        && files.iter().any(|file| settings.install_when.contains(file))
    {
        steps.push(install);
    }
    if main.join(LAUNCHER).is_file()
        && files
            .iter()
            .any(|file| BREADBOARD_SOURCES.iter().any(|source| file.starts_with(source)))
    {
        steps.push(BUILD.into());
    }
    steps
}

/// The step that builds breadboard's binary.
pub const BUILD: &str = "breadboard";

/// The command of a step in the main checkout: the build through the
/// launcher, the install through the shell (it is a command line).
pub fn command_of(main: &Path, step: &str, settings: &Settings) -> Command {
    let mut command = if step == BUILD {
        let mut command = Command::new("sh");
        command.arg(main.join(LAUNCHER)).arg("--built");
        command
    } else {
        let mut command = Command::new("sh");
        command.arg("-c").arg(settings.install.as_deref().unwrap_or("true"));
        command
    };
    command.current_dir(main).stdin(Stdio::null());
    command
}

/// Runs the steps `files` call for, one after the other, each in the state
/// while it runs and with how it ended, its output in `<store>/develop-<step>.log`.
pub fn run_steps(main: &Path, store: &Path, files: &[String], settings: &Settings) {
    for what in steps_for(main, files, settings) {
        let what = what.as_str();
        let log = store.join(format!("develop-{}.log", what.replace(' ', "-")));
        let _ = change(store, |state| {
            state.steps.retain(|step| step.what != what);
            state.steps.push(Step {
                what: what.into(),
                state: StepState::Running,
                log: Some(log.clone()),
            });
        });
        let is_ok = std::fs::File::create(&log)
            .and_then(|out| {
                let err = out.try_clone()?;
                command_of(main, what, settings).stdout(out).stderr(err).status()
            })
            .is_ok_and(|status| status.success());
        let _ = change(store, |state| {
            if let Some(step) = state.steps.iter_mut().find(|step| step.what == what) {
                step.state = if is_ok { StepState::Done } else { StepState::Failed };
            }
        });
    }
}

/// What the band says of it: `develop updated 17:42 · +4 commits (!331,
/// !332…)`, or why it wasn't (`true`: a warning). None before the first look.
pub fn band(state: &State) -> Option<(String, bool)> {
    let branch = if state.branch.is_empty() {
        "develop"
    } else {
        &state.branch
    };
    if let Some(skip) = &state.skipped {
        let behind = if skip.behind > 0 {
            format!(" (↓{})", skip.behind)
        } else {
            String::new()
        };
        return Some((format!("{branch}: not updated{behind}, {}", skip.why), true));
    }
    let Some(update) = &state.updated else {
        return (state.looked_at > 0)
            .then(|| (format!("{branch} checked {}", crate::time::hm(state.looked_at)), false));
    };
    let commits = if update.commits == 1 { "commit" } else { "commits" };
    let mut text = format!(
        "{branch} updated {} · +{} {commits}",
        crate::time::hm(update.at),
        update.commits
    );
    if !update.mrs.is_empty() {
        let shown: Vec<String> = update.mrs.iter().take(2).map(|mr| format!("!{mr}")).collect();
        let more = if update.mrs.len() > 2 { "…" } else { "" };
        text.push_str(&format!(" ({}{more})", shown.join(", ")));
    }
    let mut is_warning = false;
    for step in &state.steps {
        match step.state {
            StepState::Running if step.what == BUILD => text.push_str(" · breadboard builds…"),
            StepState::Running => text.push_str(&format!(" · {}…", step.what)),
            StepState::Done => {}
            StepState::Failed => {
                is_warning = true;
                text.push_str(&format!(" · {} failed", step.what));
            }
        }
    }
    if let Some(migrate) = &update.migrate {
        text.push_str(&format!(" · new migrations, {migrate} in each stack"));
    }
    Some((text, is_warning))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, run};
    use crate::lock::scratch::Scratch;

    const BRANCH: &str = "develop";

    /// The platform's: its lockfile installs, vita's journal names migrations.
    fn settings() -> Settings {
        Settings {
            is_on: true,
            every: Duration::from_secs(120),
            branch: BRANCH.into(),
            install: Some("pnpm install --frozen-lockfile".into()),
            install_when: vec!["pnpm-lock.yaml".into()],
            journal: Some("products/vita/web/src/server/db/migrations/meta/_journal.json".into()),
            migrate: Some("pnpm pg:migrate".into()),
        }
    }

    /// A second clone that pushes to the same origin: the colleague whose merges come in.
    fn colleague(scratch: &Scratch) -> PathBuf {
        let dir = scratch.join("colleague");
        run(
            scratch.path(),
            &[
                "clone",
                "--quiet",
                "--branch",
                BRANCH,
                scratch.join("origin.git").to_str().unwrap(),
                dir.to_str().unwrap(),
            ],
        );
        dir
    }

    fn push(dir: &Path, file: &str, text: &str, subject: &str) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "--quiet", "-m", subject]);
        run(dir, &["push", "--quiet", "origin", BRANCH]);
    }

    /// A merge of a branch into develop as GitLab writes it, pushed.
    fn merge_mr(dir: &Path, iid: i64, file: &str) {
        let branch = format!("feature/{iid}-x");
        run(dir, &["checkout", "--quiet", "-b", &branch]);
        std::fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
        std::fs::write(dir.join(file), format!("{iid}\n")).unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "--quiet", "-m", "feat: x"]);
        run(dir, &["checkout", "--quiet", BRANCH]);
        let message = format!(
            "Merge branch '{branch}' into '{BRANCH}'\n\nfeat: x\n\nSee merge request monocircuit/platform!{iid}"
        );
        run(dir, &["merge", "--quiet", "--no-ff", "-m", &message, &branch]);
        run(dir, &["push", "--quiet", "origin", BRANCH]);
    }

    fn head(dir: &Path) -> String {
        run(dir, &["rev-parse", "HEAD"])
    }

    #[test]
    fn fast_forwards_a_clean_develop_and_says_what_came_in() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let other = colleague(&scratch);
        assert_eq!(look(&main, 1, &settings()), Look::UpToDate);
        merge_mr(&other, 331, "a.md");
        merge_mr(&other, 332, "pnpm-lock.yaml");
        push(&other, "packages/breadboard/crates/x.rs", "x\n", "fix: direct");
        let before = head(&main);
        let Look::Moved(update, files) = look(&main, 7, &settings()) else {
            panic!("didn't move")
        };
        assert_eq!(head(&main), head(&other));
        assert_eq!(
            (update.from.as_str(), update.to.as_str()),
            (before.as_str(), head(&other).as_str())
        );
        assert_eq!(update.commits, 5);
        assert_eq!(update.mrs, vec![332, 331]);
        assert_eq!(update.migrate, None);
        // No launcher in this checkout: nothing to build with, only the install.
        assert_eq!(steps_for(&main, &files, &settings()), vec!["pnpm install"]);
        let launcher = main.join(LAUNCHER);
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, "exit 0\n").unwrap();
        assert_eq!(steps_for(&main, &files, &settings()), vec!["pnpm install", BUILD]);
        // A repo without an install: none.
        let bare = Settings {
            install: None,
            install_when: vec![],
            ..settings()
        };
        assert_eq!(steps_for(&main, &files, &bare), vec![BUILD]);
        assert_eq!(look(&main, 8, &settings()), Look::UpToDate);
    }

    #[test]
    fn never_touches_a_dirty_or_diverged_or_other_checkout() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let other = colleague(&scratch);
        push(&other, "a.md", "a\n", "feat: a");
        // Local changes: skipped, the change and HEAD as they were.
        std::fs::write(main.join("README.md"), "mine\n").unwrap();
        let before = head(&main);
        let Look::Skipped(skip) = look(&main, 1, &settings()) else {
            panic!("moved a dirty checkout")
        };
        assert_eq!((skip.why.as_str(), skip.behind), ("local changes in README.md", 1));
        assert_eq!(head(&main), before);
        assert_eq!(std::fs::read_to_string(main.join("README.md")).unwrap(), "mine\n");
        run(&main, &["checkout", "--quiet", "README.md"]);
        // An untracked file isn't a change: it moves.
        std::fs::write(main.join("notes.txt"), "x\n").unwrap();
        assert!(matches!(look(&main, 2, &settings()), Look::Moved(..)));
        // A commit of its own: diverged, skipped.
        push(&other, "b.md", "b\n", "feat: b");
        std::fs::write(main.join("c.md"), "c\n").unwrap();
        run(&main, &["add", "c.md"]);
        run(&main, &["commit", "--quiet", "-m", "local"]);
        let local = head(&main);
        let Look::Skipped(skip) = look(&main, 3, &settings()) else {
            panic!("moved a diverged checkout")
        };
        assert_eq!(skip.why, "diverged, 1 commit of its own");
        assert_eq!(head(&main), local);
        // Another branch: skipped.
        run(&main, &["checkout", "--quiet", "-b", "feature/9-x"]);
        let Look::Skipped(skip) = look(&main, 4, &settings()) else {
            panic!("moved another branch")
        };
        assert_eq!(skip.why, "the main checkout is on feature/9-x");
    }

    #[test]
    fn says_a_fetch_that_fails() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        run(
            &main,
            &[
                "remote",
                "set-url",
                "origin",
                scratch.join("gone.git").to_str().unwrap(),
            ],
        );
        let Look::Skipped(skip) = look(&main, 1, &settings()) else {
            panic!()
        };
        assert!(skip.why.starts_with("git fetch failed: "), "{}", skip.why);
    }

    #[test]
    fn looks_every_while_or_when_woken_and_one_at_a_time() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let store = store_of(&main).unwrap();
        let other = colleague(&scratch);
        // Never looked: due.
        assert_eq!(look_if_due(&main, &store, &settings()), Some(Look::UpToDate));
        // Just looked: not due, until a merge wakes it.
        assert_eq!(look_if_due(&main, &store, &settings()), None);
        merge_mr(&other, 5, "a.md");
        std::thread::sleep(Duration::from_millis(20));
        wake(&main);
        assert!(matches!(look_if_due(&main, &store, &settings()), Some(Look::Moved(..))));
        let state = read(&store);
        assert_eq!(state.updated.as_ref().map(|update| update.mrs.clone()), Some(vec![5]));
        // Another look under way: this one leaves it to that one.
        wake(&main);
        std::fs::create_dir(crate::lock::lock_of(&store.join("develop.look"))).unwrap();
        assert_eq!(look_if_due(&main, &store, &settings()), None);
    }

    #[test]
    fn runs_the_steps_and_keeps_how_they_ended() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let store = store_of(&main).unwrap();
        // A launcher that fails: its step says so, and its log has why.
        let launcher = main.join(LAUNCHER);
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, "echo cargo broke >&2; exit 1\n").unwrap();
        run_steps(&main, &store, &["packages/breadboard/Cargo.lock".into()], &settings());
        let state = read(&store);
        assert_eq!(state.steps.len(), 1);
        assert_eq!(
            (state.steps[0].what.as_str(), state.steps[0].state),
            (BUILD, StepState::Failed)
        );
        let log = std::fs::read_to_string(state.steps[0].log.as_ref().unwrap()).unwrap();
        assert_eq!(log, "cargo broke\n");
        std::fs::write(&launcher, "exit 0\n").unwrap();
        run_steps(&main, &store, &["packages/breadboard/Cargo.lock".into()], &settings());
        assert_eq!(read(&store).steps[0].state, StepState::Done);
    }

    #[test]
    fn writes_nothing_until_due_and_never_makes_a_removed_git_directory_again() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let store = store_of(&main).unwrap();
        assert_eq!(look_if_due(&main, &store, &settings()), Some(Look::UpToDate));
        // Not due: no lock made, nothing touched.
        let before = std::fs::read_dir(&store).unwrap().count();
        assert_eq!(look_if_due(&main, &store, &settings()), None);
        assert_eq!(std::fs::read_dir(&store).unwrap().count(), before);
        // The checkout being removed (a gauntlet world's end): its .git stays gone.
        std::fs::remove_dir_all(main.join(".git")).unwrap();
        assert_eq!(look_if_due(&main, &store, &settings()), None);
        wake(&main);
        let _ = change(&store, |state| state.looked_at = 1);
        assert!(!main.join(".git").exists());
    }

    #[test]
    fn names_new_migrations_and_runs_the_repo_s_install() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let store = store_of(&main).unwrap();
        let other = colleague(&scratch);
        let journal = settings().journal.unwrap();
        merge_mr(&other, 12, &journal);
        let Look::Moved(update, files) = look(&main, 1, &settings()) else {
            panic!("didn't move")
        };
        assert_eq!(update.migrate.as_deref(), Some("pnpm pg:migrate"));
        let mut state = State {
            branch: "main".into(),
            ..State::default()
        };
        record(&mut state, &Look::Moved(update, files), 1);
        let (text, _) = band(&state).unwrap();
        assert!(text.starts_with("main updated "), "{text}");
        assert!(
            text.ends_with(" (!12) · new migrations, pnpm pg:migrate in each stack"),
            "{text}"
        );
        // The install is the repo's command line, run in the main checkout.
        let install = Settings {
            install: Some("echo installed > installed.txt".into()),
            install_when: vec!["deps.lock".into()],
            ..settings()
        };
        run_steps(&main, &store, &["deps.lock".into()], &install);
        assert_eq!(
            std::fs::read_to_string(main.join("installed.txt")).unwrap(),
            "installed\n"
        );
        assert_eq!(read(&store).steps[0].what, "echo installed");
    }

    #[test]
    fn says_in_the_band_when_and_what_or_why_not() {
        let mut state = State::default();
        assert_eq!(band(&state), None);
        let update = Update {
            at: 0,
            commits: 4,
            mrs: vec![332, 331, 330],
            ..Update::default()
        };
        record(&mut state, &Look::Moved(update, vec![]), 0);
        let (text, is_warning) = band(&state).unwrap();
        assert!(
            text.starts_with("develop updated ") && text.ends_with(" · +4 commits (!332, !331…)"),
            "{text}"
        );
        assert!(!is_warning);
        state.steps.push(Step {
            what: BUILD.into(),
            state: StepState::Running,
            log: None,
        });
        assert!(band(&state).unwrap().0.ends_with(" · breadboard builds…"));
        state.steps[0].state = StepState::Failed;
        assert_eq!(band(&state).unwrap(), (text.clone() + " · breadboard failed", true));
        let skip = Skip {
            at: 1,
            why: "local changes in README.md".into(),
            behind: 2,
        };
        record(&mut state, &Look::Skipped(skip), 1);
        assert_eq!(
            band(&state).unwrap(),
            ("develop: not updated (↓2), local changes in README.md".into(), true)
        );
        // Up to date again: the last update shows, its steps kept.
        record(&mut state, &Look::UpToDate, 2);
        assert!(band(&state).unwrap().0.starts_with("develop updated "));
    }
}
