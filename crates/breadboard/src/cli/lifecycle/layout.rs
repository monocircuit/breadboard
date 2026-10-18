//! `wt:herdr [--no-focus]`: the layout of herdr built from the worktrees
//! that exist (worktree/layout.mjs). A space per purpose:
//!
//! ```text
//! <main space>          root · overview (live, --herdr) · changelog
//! ├─ <product>           root · overview (every orchestration of it) · orchestrator · changelog · changes
//! ├─ <feature>           root · overview (this worktree) · <feature> (its agent) · nvim
//! └─ ⇶ <title>           root · overview · orchestrator · changelog · changes
//!
//! With `[products.*]` in breadboard.toml (#405) an orchestration works in
//! its product's space, and the main space keeps `root` and `overview`: the
//! changelog is the products'. An orchestration of no product keeps `⇶ <title>`.
//! ```
//!
//! Idempotent: also the way back after a herdr restart (the plugin runs it
//! with `--no-focus` at startup) or tabs closed by hand. And `release_space`,
//! what goes with a worktree that wt:remove took.

pub mod nvim;
pub mod orchestrations;
pub mod resume;
pub mod spaces;
pub mod stale;
pub mod tabs;

use super::args::{Args, Takes};
use super::claude::share_conversations;
use super::herdr::{self, call, s};
use super::plugin;
use super::proc::{Done, NOTICE, done, fail, say_wait, step};
use super::repo::Repo;
use super::stack_procs;
use breadboard_core::git::Worktree;
use breadboard_core::record::SPACE;
use orchestrations::{mark_origins, orchestrator_space, prune_orchestrators, restore_chat, sort_spaces};
use serde_json::Value;
use spaces::{busy_reason, is_busy, main_space};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::fs;
use std::time::{Duration, Instant, SystemTime};
use tabs::{CHANGELOG, OVERVIEW, ROOT};

const TAKES: Takes = Takes {
    switches: &["--no-focus"],
    valued: &[],
    lists: &[],
    words: 0,
    about: "Lays out herdr: the main space and a space per worktree, its tabs in order, the agents resumed (the way back after a herdr restart). --no-focus leaves the focus where it is.",
    usage: "usage: pnpm wt:herdr [--no-focus]",
};

pub fn command(args: &Args) -> Done {
    if args.check(&TAKES)? {
        return Ok(());
    }
    if !herdr::is_in() {
        return fail("this runs inside herdr (HERDR_ENV is not set)");
    }
    let repo = Repo::here()?;
    // Said first: the layout takes seconds, today's was silent until its end;
    // a wait for herdr shows under the step it belongs to.
    let count = repo.trees().len();
    step(&format!(
        "herdr: the layout of {SPACE} and its {count} worktree{}",
        if count == 1 { "" } else { "s" }
    ));
    plugin::heal(&repo.main);
    let Some((main, report)) = exclusive(|| layout(&repo, true)) else {
        return fail(if herdr::is_silent() {
            format!(
                "herdr did not answer in {} s: nothing was laid out; {run} wt:herdr again once it answers",
                herdr::LIMIT.as_secs(),
                run = breadboard_core::config::run_word()
            )
        } else {
            "herdr did not answer, is its server running? (herdr status)".into()
        });
    };
    for line in report {
        println!("  {line}");
    }
    if !args.has("--no-focus") {
        call(&["workspace", "focus", &main]);
    }
    done(&format!("{SPACE} and a space per worktree"));
    Ok(())
}

thread_local! {
    /// How deep this thread is in a locked change of the layout.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// A lock left by a killed process counts as gone after this long.
const STALE: Duration = Duration::from_secs(60);

/// `run` as one change of the layout at a time, across every process of the
/// user (lock.mjs `exclusive`): the directory `$TMPDIR/platform-herdr-layout-
/// <uid>.lock`, shared with today's Node. Re-entrant; outside herdr no lock.
/// Inside, herdr's reads are remembered (herdr::remembering).
pub fn exclusive<T>(run: impl FnOnce() -> T) -> T {
    if DEPTH.get() > 0 || !herdr::is_in() {
        return run();
    }
    let lock = breadboard_core::lock::layout_lock();
    let started = Instant::now();
    let mut is_said = false;
    while fs::create_dir(&lock).is_err() {
        // Another process lays herdr out (a wt:new, the overview): said once it takes a while.
        if !is_said && started.elapsed() >= NOTICE {
            say_wait(&format!(
                "  waiting for another change of herdr's layout to finish, at most {} s ({})",
                STALE.as_secs(),
                lock.display()
            ));
            is_said = true;
        }
        let age = fs::metadata(&lock)
            .and_then(|meta| meta.modified())
            .map(|at| SystemTime::now().duration_since(at).unwrap_or_default())
            .unwrap_or_default();
        if age > STALE || started.elapsed() > STALE {
            let _ = fs::remove_dir_all(&lock);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _held = Held::new(lock);
    // Under the lock no other layout changes herdr: its reads are remembered
    // until a write, and forgotten when the lock goes.
    herdr::remembering(run)
}

/// The layout lock this thread holds: let go when it is dropped, a panic in
/// the layout too (else every wt:new, overview and wt:herdr waited up to
/// [`STALE`] for a lock nobody holds).
struct Held(std::path::PathBuf);

impl Held {
    fn new(lock: std::path::PathBuf) -> Held {
        DEPTH.set(DEPTH.get() + 1);
        Held(lock)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        DEPTH.set(DEPTH.get() - 1);
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The whole layout from `git worktree list`: the main space, a space per
/// orchestration and per worktree, the background processes of every stack
/// in step with it (`stack_procs`, #407);
/// a tab of a worktree left in the main space moves into its space, one
/// whose directory is gone closes. `(main space, a line per change)`; None
/// when herdr does not answer.
pub fn layout(repo: &Repo, should_resume: bool) -> Option<(String, Vec<String>)> {
    let main = main_space(repo)?;
    let mut report = Vec::new();
    let trees = repo.trees();
    report.extend(prune_spaces(repo));
    let dirs: Vec<_> = trees.iter().map(|tree| tree.dir.clone()).collect();
    let merged = share_conversations(&repo.main, &dirs);
    if !merged.is_empty() {
        report.push(format!(
            "conversations of {} moved to the project of {}",
            merged.join(", "),
            repo.base()
        ));
    }

    let tabs = herdr::tabs_of(&main);
    let panes = herdr::panes_of(&main);
    let current = herdr::current_pane();
    for tab in &tabs {
        let label = s(tab, "label");
        if [OVERVIEW, ROOT, CHANGELOG].contains(&label) {
            continue;
        }
        let own: Vec<&Value> = panes
            .iter()
            .filter(|pane| s(pane, "tab_id") == s(tab, "tab_id"))
            .collect();
        let tree = trees.iter().find(|tree| {
            let feature = repo.feature_of(&tree.dir);
            label == feature || label.starts_with(&format!("{feature} :"))
        });
        // A worktree has no space of its own any more (#409): its tab stays where it is.
        if tree.is_some() {
            continue;
        }
        let is_gone = own.iter().all(|pane| s(pane, "cwd").ends_with(" (deleted)"));
        if is_gone && !is_busy(tab, &panes, current.as_ref()) && call(&["tab", "close", s(tab, "tab_id")]).is_some() {
            report.push(format!("{label} closed, its directory is gone"));
        }
    }

    // The spaces of the orchestrations and the products (#405); after a restart the orchestrator's conversation comes back (#287).
    let mut orchestrations: Vec<String> = Vec::new();
    for name in herdr::spaces()
        .iter()
        .flat_map(|space| repo.orchestrations_of(s(space, "label")))
    {
        if !orchestrations.contains(&name) {
            orchestrations.push(name);
        }
    }
    for name in orchestrations {
        orchestrator_space(repo, &name);
        // Its epic stack's processes (#351, #407).
        let changed = stack_procs::sync(repo, &repo.checkout_of(&name));
        if !changed.is_empty() {
            report.push(format!("{}: {}", repo.space_label_of(&name), changed.join(" ")));
        }
        if let Some(resumed) = should_resume.then(|| restore_chat(repo, &name)).flatten() {
            report.push(format!("{}: {resumed}", repo.space_label_of(&name)));
        }
    }

    // No worktree has a space of its own (#409): those of before close once nothing runs in them.
    report.extend(retire_worktree_spaces(repo, &trees));
    for tree in &trees {
        // Claude Code's IDE plugin for the nvim of the detail pane, cloned here: a click clones nothing.
        nvim::ide_for(&tree.dir, &repo.main);
        let changed = stack_procs::sync(repo, &tree.dir);
        if !changed.is_empty() {
            report.push(format!("{}: {}", repo.feature_of(&tree.dir), changed.join(" ")));
        }
    }
    let changed = stack_procs::sync(repo, &repo.main);
    if !changed.is_empty() {
        report.push(format!("{SPACE}: {}", changed.join(" ")));
    }
    // Last, when every space is there: herdr forgets the marks on a restart.
    mark_origins(repo);
    report.extend(sort_spaces(repo));
    Some((main, report))
}

/// The spaces worktrees had before #409 close: one whose every pane idles
/// in its shell or runs nvim or a log viewer goes as a whole; one where
/// anything else runs (a claude, a dev server, this command) is kept, a
/// line saying why, until it ends. What closed and what stays.
pub fn retire_worktree_spaces(repo: &Repo, trees: &[Worktree]) -> Vec<String> {
    let features: BTreeSet<String> = trees
        .iter()
        .filter(|tree| {
            tree.dir != repo.main
                && !tree
                    .dir
                    .starts_with(breadboard_core::git::orchestrations_dir(&repo.main))
        })
        .map(|tree| repo.feature_of(&tree.dir))
        .collect();
    let current = herdr::current_pane();
    let mut said = Vec::new();
    for space in herdr::spaces() {
        let label = s(&space, "label");
        if !features.contains(label) || label == SPACE.get() || repo.is_product(label) {
            continue;
        }
        let workspace = s(&space, "workspace_id");
        let tabs = herdr::tabs_of(workspace);
        let panes = herdr::panes_of(workspace);
        let busy: Vec<String> = panes
            .iter()
            .filter_map(|pane| {
                let why = why_kept(pane, current.as_ref())?;
                let tab = tabs
                    .iter()
                    .find(|tab| s(tab, "tab_id") == s(pane, "tab_id"))
                    .map_or("a tab", |tab| s(tab, "label"));
                Some(format!("{tab}: {why}"))
            })
            .collect();
        if busy.is_empty() {
            if call(&["workspace", "close", workspace]).is_some() {
                said.push(format!("space {label} closed: a worktree has no space any more"));
            }
        } else {
            said.push(format!("space {label} kept until it ends ({})", busy.join("; ")));
        }
    }
    said
}

/// Why a pane of a worktree's old space keeps it: an agent, this command,
/// anything but an idle shell, nvim or a log viewer. None when it may close.
fn why_kept(pane: &Value, current: Option<&Value>) -> Option<String> {
    if current.is_some_and(|current| s(current, "pane_id") == s(pane, "pane_id")) {
        return Some("it runs this command".into());
    }
    if tabs::is_agents(pane) {
        return Some("an agent runs there".into());
    }
    let (is_idle, commands) = herdr::foreground_of(s(pane, "pane_id"));
    if is_idle {
        return None;
    }
    match commands.iter().find(|command| !is_viewer(command)) {
        None if !commands.is_empty() => None,
        None => Some("something runs there".into()),
        Some(command) => Some(format!(
            "{} runs there",
            command.split_whitespace().next().unwrap_or(command)
        )),
    }
}

/// Whether a command only shows something: nvim, a pager, a log being followed.
pub fn is_viewer(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let program = words
        .next()
        .map(|program| program.rsplit('/').next().unwrap_or(program))
        .unwrap_or("");
    match program {
        "nvim" | "vim" | "vi" | "less" | "more" | "tail" | "sh" | "bash" | "zsh" | "fish" => true,
        "docker" => command.split_whitespace().any(|word| word == "logs"),
        _ => false,
    }
}

/// The `overview` tabs that hold only a shell run their overview again
/// (#388): after a herdr server or machine restart herdr brings the tabs
/// back as bare shells, and the startup's layout may come late or not at
/// all. Under the layout's lock, so a pane is judged and typed into by one
/// process; only a shell that idles in its one pane gets the command, never
/// a pane that runs something. What it did and why not, a line each (#401):
/// a tab whose overview runs (or a space without one) says nothing.
pub fn heal_overviews(repo: &Repo) -> Vec<String> {
    exclusive(|| {
        let trees = repo.trees();
        let mut said = Vec::new();
        for space in herdr::spaces() {
            let label = s(&space, "label");
            let Some(command) = overview_command_of(repo, &trees, label) else {
                continue;
            };
            let workspace = s(&space, "workspace_id");
            let tabs = herdr::tabs_of(workspace);
            match tabs::idle_pane_or_why(workspace, OVERVIEW, &tabs) {
                Ok(idle) if herdr::did(&["pane", "run", &idle, &command]) => {
                    said.push(format!("the overview runs again in {label} (typed into {idle})"));
                }
                Ok(idle) => said.push(format!("{label}: herdr took no `pane run` into {idle}")),
                Err(tabs::NotIdle::Runs(_) | tabs::NotIdle::NoTab) => {}
                Err(why) => said.push(format!("{label}: its overview tab left, {why}")),
            }
        }
        said
    })
}

/// What the `overview` tab of the space `label` runs; None for a space with none.
pub fn overview_command_of(repo: &Repo, trees: &[Worktree], label: &str) -> Option<String> {
    let view = if label == SPACE.get() {
        tabs::View::Herdr
    } else if repo.is_product(label) {
        tabs::View::Product(label)
    } else if let Some(name) = repo.orchestration_of(label) {
        return Some(tabs::overview_command(tabs::View::Orchestration(&name)));
    } else {
        let tree = trees.iter().find(|tree| {
            repo.feature_of(&tree.dir) == label && !breadboard_core::record::is_orchestrated(&repo.main, &tree.dir)
        })?;
        tabs::View::Worktree(&tree.dir)
    };
    Some(tabs::overview_command(view))
}

/// The line of a release: `closed the space x` | `closed 3 tabs of x` and
/// `kept a tab (an agent works there)` | `kept a tab of x (logs: it has
/// several panes)`: the feature named once, a kept tab by its label only
/// where that says more than the feature.
pub fn release_line(feature: &str, closed: usize, kept: &[String]) -> Option<String> {
    if closed == 0 && kept.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if closed > 0 {
        parts.push(if kept.is_empty() {
            format!("closed the space {feature}")
        } else {
            format!("closed {closed} tab{} of {feature}", if closed == 1 { "" } else { "s" })
        });
    }
    if !kept.is_empty() {
        let count = if kept.len() == 1 {
            "a tab".to_string()
        } else {
            format!("{} tabs", kept.len())
        };
        let of = if closed == 0 {
            format!(" of {feature}")
        } else {
            String::new()
        };
        parts.push(format!("kept {count}{of} ({})", kept.join("; ")));
    }
    Some(parts.join(", "))
}

/// After wt:remove: the space of the worktree goes, its directory is gone.
/// A tab with an agent at work, several panes or this very command stays;
/// `is_whole` (`--close-space`, the ✕ of the overview): all but this
/// command's tab goes. None outside herdr or when nothing was there.
pub fn release_space(feature: &str, is_whole: bool) -> Option<String> {
    if !herdr::is_in() {
        return None;
    }
    let current = herdr::current_pane();
    let mut closed = 0;
    let mut kept: Vec<String> = Vec::new();
    for space in herdr::spaces() {
        let workspace = s(&space, "workspace_id");
        let is_own = s(&space, "label") == feature;
        let tabs: Vec<Value> = herdr::tabs_of(workspace)
            .into_iter()
            .filter(|tab| {
                let label = s(tab, "label");
                is_own || label == feature || label.starts_with(&format!("{feature} :"))
            })
            .collect();
        if tabs.is_empty() {
            continue;
        }
        let panes = herdr::panes_of(workspace);
        let is_current = |tab: &Value| {
            current
                .as_ref()
                .is_some_and(|current| s(current, "tab_id") == s(tab, "tab_id"))
        };
        let mut busy = Vec::new();
        for tab in &tabs {
            let why = if is_whole {
                is_current(tab).then_some("it runs this command")
            } else {
                busy_reason(tab, &panes, current.as_ref())
            };
            if let Some(why) = why {
                let label = s(tab, "label");
                let label = label.strip_prefix(&format!("{feature} : ")).unwrap_or(label);
                kept.push(if label == feature {
                    why.to_string()
                } else {
                    format!("{label}: {why}")
                });
                busy.push(s(tab, "tab_id").to_string());
            }
        }
        // Nothing to keep in its own space: the space goes as a whole.
        if is_own && busy.is_empty() && call(&["workspace", "close", workspace]).is_some() {
            closed += tabs.len();
            continue;
        }
        for tab in &tabs {
            if !busy.iter().any(|id| id == s(tab, "tab_id")) && call(&["tab", "close", s(tab, "tab_id")]).is_some() {
                closed += 1;
            }
        }
    }
    release_line(feature, closed, &kept)
}

/// The spaces of worktrees that are gone close, whoever removed them: a
/// space whose label names no worktree and whose every pane stands in that
/// worktree's deleted directory; and an orchestration's whose last worktree
/// is gone. What closed.
pub fn prune_spaces(repo: &Repo) -> Vec<String> {
    if !herdr::is_in() {
        return Vec::new();
    }
    let live: BTreeSet<String> = repo
        .worktrees()
        .iter()
        .map(|tree| repo.space_label(&tree.dir))
        .collect();
    let mut closed = Vec::new();
    for space in herdr::spaces() {
        let label = s(&space, "label");
        if live.contains(label) {
            continue;
        }
        let dir = repo.dir_of(label);
        if dir.exists() {
            continue;
        }
        let dir = super::proc::text(&dir);
        let panes = herdr::panes_of(s(&space, "workspace_id"));
        let is_orphan = !panes.is_empty()
            && panes.iter().all(|pane| {
                let cwd = s(pane, "cwd");
                !cwd.is_empty()
                    && (cwd == dir || cwd.starts_with(&format!("{dir}/")) || cwd.starts_with(&format!("{dir} ")))
            });
        if is_orphan && let Some(released) = release_space(label, false) {
            closed.push(released);
        }
    }
    closed.extend(prune_orchestrators(repo));
    closed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_space_closes_under_nvim_a_pager_or_logs_only() {
        assert!(is_viewer("nvim --listen /r/p-1/.claude/nvim.sock"));
        assert!(is_viewer("/usr/bin/tail -n 200 -F dev.log"));
        assert!(is_viewer(
            "docker compose --env-file .env.local logs -f --tail 200 postgres"
        ));
        assert!(!is_viewer("docker compose up"));
        assert!(!is_viewer("claude --resume x"));
        assert!(!is_viewer("node scripts/dev.mjs"));
    }

    #[test]
    fn a_panic_in_a_layout_lets_the_lock_go() {
        let lock = std::env::temp_dir().join(format!("lifecycle-layout-lock-{}", std::process::id()));
        fs::create_dir_all(&lock).unwrap();
        let held = lock.clone();
        let panicked = std::panic::catch_unwind(move || {
            let _held = Held::new(held);
            assert_eq!(DEPTH.get(), 1);
            panic!("a layout fails");
        });
        assert!(panicked.is_err());
        assert!(!lock.exists());
        assert_eq!(DEPTH.get(), 0);
    }

    #[test]
    fn says_what_a_release_closed_and_kept() {
        assert_eq!(release_line("3-x", 0, &[]), None);
        assert_eq!(release_line("3-x", 4, &[]).as_deref(), Some("closed the space 3-x"));
        let kept = vec!["an agent works there".to_string()];
        assert_eq!(
            release_line("3-x", 8, &kept).as_deref(),
            Some("closed 8 tabs of 3-x, kept a tab (an agent works there)")
        );
        assert_eq!(
            release_line("3-x", 0, &kept).as_deref(),
            Some("kept a tab of 3-x (an agent works there)")
        );
        let two = vec![
            "a: it has several panes".to_string(),
            "b: it runs this command".to_string(),
        ];
        assert_eq!(
            release_line("3-x", 1, &two).as_deref(),
            Some("closed 1 tab of 3-x, kept 2 tabs (a: it has several panes; b: it runs this command)")
        );
    }

    #[test]
    fn the_lock_is_reentrant() {
        assert_eq!(exclusive(|| exclusive(|| 4)), 4);
    }
}
