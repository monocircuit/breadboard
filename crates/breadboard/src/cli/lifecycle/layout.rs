//! `wt:herdr [--no-focus]`: the layout of herdr built from the worktrees
//! that exist (worktree/layout.mjs). A space per purpose:
//!
//! ```text
//! Monocircuit/Platform   root · overview (live, --herdr) · changelog · stack tabs
//! ├─ <feature>           root · overview (this worktree) · <feature> (its agent) · nvim · stack tabs
//! └─ ⇶ <title>           root · overview · orchestrator · changelog · changes
//! ```
//!
//! Idempotent: also the way back after a herdr restart (the plugin runs it
//! with `--no-focus` at startup) or tabs closed by hand. And `release_space`,
//! what goes with a worktree that wt:remove took.

pub mod nvim;
pub mod orchestrations;
pub mod resume;
pub mod spaces;
pub mod tabs;

use super::args::{Args, Takes};
use super::claude::share_conversations;
use super::herdr::{self, call, s};
use super::plugin;
use super::proc::{Done, NOTICE, done, fail, say_wait, step};
use super::repo::{Repo, VITA, compose_args, env_file_of, offset_of};
use breadboard_core::record::SPACE;
use orchestrations::{mark_origins, orchestrator_space, prune_orchestrators, restore_chat, sort_spaces};
use serde_json::Value;
use spaces::{busy_reason, is_busy, main_space, worktree_space};
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};
use tabs::{CHANGELOG, OVERVIEW, Opened, ROOT, Want, is_grouped, open_tab};

const TAKES: Takes = Takes {
    switches: &["--no-focus"],
    valued: &[],
    lists: &[],
    words: 0,
    about: "Lays out herdr: Monocircuit/Platform and a space per worktree, its tabs in order, the agents resumed (the way back after a herdr restart). --no-focus leaves the focus where it is.",
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
                "herdr did not answer in {} s: nothing was laid out; pnpm wt:herdr again once it answers",
                herdr::LIMIT.as_secs()
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
/// orchestration and per worktree, the stack tabs of every stack that runs;
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
        if let Some(tree) = tree {
            let feature = repo.feature_of(&tree.dir);
            if let Some(workspace) = worktree_space(repo, &tree.dir, false, false) {
                for pane in &own {
                    let moved = call(&[
                        "pane",
                        "move",
                        s(pane, "pane_id"),
                        "--workspace",
                        &workspace,
                        "--new-tab",
                    ]);
                    if let Some(created) =
                        moved.and_then(|moved| herdr::id_at(&moved, &["move_result", "created_tab", "tab_id"]))
                    {
                        call(&["tab", "rename", &created, &feature]);
                    }
                }
            }
            report.push(format!("{label} moved into the space {feature}"));
            continue;
        }
        let is_gone = own.iter().all(|pane| s(pane, "cwd").ends_with(" (deleted)"));
        if is_gone && !is_busy(tab, &panes, current.as_ref()) && call(&["tab", "close", s(tab, "tab_id")]).is_some() {
            report.push(format!("{label} closed, its directory is gone"));
        }
    }

    // The spaces of the orchestrations; after a restart the orchestrator's conversation comes back (#287).
    let mut orchestrations: Vec<String> = Vec::new();
    for name in herdr::spaces()
        .iter()
        .filter_map(|space| repo.orchestration_of(s(space, "label")))
    {
        if !orchestrations.contains(&name) {
            orchestrations.push(name);
        }
    }
    for name in orchestrations {
        let workspace = orchestrator_space(repo, &name);
        // Its epic stack's tabs (#351), in the orchestration's space.
        let changed = sync_stack_tabs(repo, &repo.checkout_of(&name), workspace.as_deref());
        if !changed.is_empty() {
            report.push(format!("{}: {}", repo.orchestrator_label(&name), changed.join(" ")));
        }
        if let Some(resumed) = should_resume.then(|| restore_chat(repo, &name)).flatten() {
            report.push(format!("{}: {resumed}", repo.orchestrator_label(&name)));
        }
    }

    let known: HashMap<String, Value> = herdr::spaces()
        .into_iter()
        .map(|space| (s(&space, "label").to_string(), space))
        .collect();
    for tree in &trees {
        let feature = repo.feature_of(&tree.dir);
        let before = known.get(&feature);
        let workspace = worktree_space(repo, &tree.dir, true, should_resume);
        match (before, &workspace) {
            (None, Some(_)) => report.push(format!("space {feature} opened")),
            // Today's said nothing: a space may be there now, without its tabs.
            (None, None) => report.push(format!(
                "space {feature} not set up: herdr gave no id back for it (worktree open, workspace create)"
            )),
            _ => {}
        }
        let was_linked = before
            .and_then(|space| space.get("worktree")?.get("is_linked_worktree")?.as_bool())
            .unwrap_or(false);
        if before.is_some() && !was_linked && workspace.as_deref().is_some_and(is_grouped) {
            report.push(format!("space {feature} moved into its group"));
        }
        let changed = sync_stack_tabs(repo, &tree.dir, workspace.as_deref());
        if !changed.is_empty() {
            report.push(format!("{feature}: {}", changed.join(" ")));
        }
    }
    let changed = sync_stack_tabs(repo, &repo.main, Some(&main));
    if !changed.is_empty() {
        report.push(format!("{SPACE}: {}", changed.join(" ")));
    }
    // Last, when every space is there: herdr forgets the marks on a restart.
    mark_origins(repo);
    report.extend(sort_spaces(repo));
    Some((main, report))
}

fn is_port_free(port: i64) -> bool {
    u16::try_from(port).is_ok_and(|port| TcpListener::bind(("127.0.0.1", port)).is_ok())
}

/// The tab of a checkout's app (`pnpm dev`) and drizzle studio's.
const DEV: &str = "dev";
const STUDIO: &str = "studio";
const STUDIO_PORT: i64 = 4983;

/// While the stack of `dir` runs, its space shows it: a tab per container
/// following its logs, `dev` and `studio` while their port is free; when it
/// is down those tabs close, unless busy. A worktree on the shared stack has
/// none. What changed: `+postgres`, `-dev`.
pub fn sync_stack_tabs(repo: &Repo, dir: &Path, workspace: Option<&str>) -> Vec<String> {
    let Some(workspace) = workspace.filter(|_| env_file_of(dir).exists()) else {
        return Vec::new();
    };
    let is_main = dir == repo.main;
    let offset = if is_main { 0 } else { offset_of(dir) };
    if !is_main && offset == 0 {
        return Vec::new();
    }
    let vita = dir.join(VITA);
    let compose = |args: &[&str]| super::proc::output("docker", &compose_args(args), &vita, None);
    // A task stack has no Postgres of its own (POSTGRES_SCALE=0, #351): no tab follows one.
    let has_postgres = super::repo::read_env(&env_file_of(dir)).is_none_or(|env| {
        !env.iter()
            .any(|(key, value)| key == "POSTGRES_SCALE" && value.trim() == "0")
    });
    let services: Vec<String> = compose(&["config", "--services"])
        .lines()
        .filter(|service| has_postgres || *service != "postgres")
        .map(String::from)
        .collect();
    let is_up = !compose(&["ps", "--status", "running", "-q"]).out().is_empty();
    let tabs = herdr::tabs_of(workspace);
    let mut changed = Vec::new();
    if is_up {
        let logs = "docker compose --env-file .env.local -f docker-compose.local.yml logs -f --tail 200";
        let mut wanted: Vec<(String, String)> = services
            .iter()
            .map(|service| (service.clone(), format!("{logs} {service}")))
            .collect();
        if is_port_free(3000 + offset) {
            wanted.push((DEV.into(), "pnpm dev".into()));
        }
        if is_port_free(STUDIO_PORT + offset) {
            wanted.push((STUDIO.into(), format!("pnpm pg:studio --port {}", STUDIO_PORT + offset)));
        }
        let mut empty = Vec::new();
        for (label, command) in wanted {
            let want = Want {
                label: &label,
                cwd: &vita,
                command: Some(command),
            };
            match open_tab(workspace, &want, &tabs) {
                Opened::Started => changed.push(format!("+{label}")),
                Opened::Empty => empty.push(label),
                Opened::Kept => {}
            }
        }
        // A tab herdr gave no pane runs nothing: said, never counted as running (log 2026-10-02 00:45).
        if !empty.is_empty() {
            let dot = if changed.is_empty() { "" } else { "· " };
            changed.push(format!(
                "{dot}{} opened empty: herdr gave no pane, nothing runs there",
                empty.join(" ")
            ));
        }
        return changed;
    }
    let ours: BTreeSet<&str> = services.iter().map(String::as_str).chain([DEV, STUDIO]).collect();
    // The panes and the one running this are read only when there is a tab to close (a read per pane).
    if !tabs.iter().any(|tab| ours.contains(s(tab, "label"))) {
        return changed;
    }
    let panes = herdr::panes_of(workspace);
    let current = herdr::current_pane();
    for tab in &tabs {
        if !ours.contains(s(tab, "label")) || is_busy(tab, &panes, current.as_ref()) {
            continue;
        }
        if call(&["tab", "close", s(tab, "tab_id")]).is_some() {
            changed.push(format!("-{}", s(tab, "label")));
        }
    }
    changed
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
