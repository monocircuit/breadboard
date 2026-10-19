//! The unpushed commits of a repo's headless agents, checked and pushed by
//! breadboard (#450, core's `push`), run by the daemon while any overview
//! of that repo is open: every EVERY it looks at each orchestrated
//! worktree; one whose agent has no run in flight or queued (and ended its
//! last one IDLE_MS ago), no claude open in it, no wt:check or background
//! command running and nothing uncommitted gets `wt:check`, then `git push`
//! and a draft MR from the issue when it has none. One worktree at a time,
//! across the daemons of the clone (`.claude/runs/push.pid`), and only while
//! memory allows a run (`runs::room_for`). A red check is one line to its
//! agent; a second red, or a push git refused, a `look` action of the
//! worktree. Off with `git config breadboard.push off`.

use breadboard_core::actions::{NewAction, actions_file, add_action};
use breadboard_core::check::{CheckState, check_of};
use breadboard_core::conversations::{bound_session, running_claudes};
use breadboard_core::git::{self, git};
use breadboard_core::lock::PidFile;
use breadboard_core::push::{self, Look, Pushing, Stage, Step};
use breadboard_core::runs::{self, Queue, State, Table, read_or_default, status_file};
use breadboard_core::time::now_ms;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// How often it looks; `BREADBOARD_PUSH_EVERY_MS` another (the gauntlet's ticks).
pub const EVERY: Duration = Duration::from_secs(15);

/// The pushes of `main`, as long as the daemon runs.
pub async fn push(main: PathBuf) {
    let every = std::env::var("BREADBOARD_PUSH_EVERY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(EVERY, Duration::from_millis);
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if !main.is_dir() {
            return;
        }
        let main = main.clone();
        // A check runs here: the next look waits for it.
        let _ = tokio::task::spawn_blocking(move || look(&main)).await;
    }
}

/// Whether breadboard pushes in `main`: not with `git config breadboard.push off`.
fn is_on(main: &Path) -> bool {
    git(main, &["config", "--get", push::MODE_KEY]).out() != "off"
}

/// One look over the orchestrated worktrees of `main`.
fn look(main: &Path) {
    if !is_on(main) {
        return;
    }
    let file = runs::dir_of(main).join("push.pid");
    let _ = std::fs::create_dir_all(runs::dir_of(main));
    // Another daemon of the clone at it.
    let Some(_held) = PidFile::hold(&file) else {
        return;
    };
    let table: Table = read_or_default(&status_file(main));
    let queue: Queue = read_or_default(&runs::queue_file(main));
    let states = runs::states_of(&table, &queue);
    let claudes: Vec<PathBuf> = running_claudes().into_iter().map(|claude| claude.cwd).collect();
    for tree in git::worktrees(main).into_iter().skip(1) {
        let Some(branch) = tree.branch else { continue };
        if !runs::is_headless(main, &tree.dir) {
            continue;
        }
        let is_busy = is_busy(&tree.dir, &states, &claudes, now_ms());
        take(main, &tree.dir, &branch, is_busy);
    }
}

/// Whether anything works in `dir` now, or just did.
fn is_busy(dir: &Path, states: &HashMap<String, runs::Run>, claudes: &[PathBuf], now: i64) -> bool {
    let run = bound_session(dir).and_then(|session| states.get(&session).cloned());
    let is_running = run.is_some_and(|run| {
        matches!(run.state, State::Running | State::Queued)
            || (run.state == State::Idle && now - run.since < push::idle_ms())
    });
    is_running
        || claudes.iter().any(|cwd| cwd.starts_with(dir))
        || matches!(check_of(dir), CheckState::Checking { .. })
        || breadboard_core::wake::wake_of_dir(dir).is_some()
}

/// The step of the worktree `dir` on `branch`, done.
fn take(main: &Path, dir: &Path, branch: &str, is_busy: bool) {
    let state = git::push_state(dir, Some(branch));
    let record = push::read(dir);
    if state.unpushed == 0 && record.is_none() {
        return;
    }
    let Some(head) = git::head_of(dir) else { return };
    let look = Look {
        head: head.clone(),
        unpushed: state.unpushed,
        is_busy,
        is_dirty: state.unpushed > 0 && !git(dir, &["status", "--porcelain"]).out().is_empty(),
        is_tested: state.unpushed > 0 && matches!(check_of(dir), CheckState::Tested { .. }),
    };
    let commits = push::commits_text(state.unpushed, state.unpushed_own, state.merged_base.as_deref());
    let run = breadboard_core::config::run_word();
    let at = |stage: Stage, reds: u32, why: Option<String>| Pushing {
        head: head.clone(),
        stage,
        at: now_ms(),
        reds,
        why,
    };
    let reds = record.as_ref().map_or(0, |record| record.reds);
    match push::step_of(&look, record.as_ref()) {
        Step::Wait => {}
        Step::Clear => push::clear(dir),
        Step::TellDirty => {
            if tell(main, dir, &push::dirty_line(&commits, &run)) {
                let _ = push::write(dir, &at(Stage::Dirty, reds, None));
            }
        }
        Step::Push => send(main, dir, branch, &commits, &at(Stage::Checking, reds, None)),
        Step::Check => {
            // Memory for a run: else the check waits for the next look.
            if runs::room_for(runs::available(), 0) == 0 {
                return;
            }
            let _ = push::write(dir, &at(Stage::Checking, reds, None));
            let (is_green, output) = check(dir);
            if is_green {
                return send(main, dir, branch, &commits, &at(Stage::Checking, reds, None));
            }
            let failures = match check_of(dir) {
                CheckState::Failed { failures, .. } => failures,
                _ => Vec::new(),
            };
            let reds = push::reds_after(record.as_ref());
            let why = (!failures.is_empty()).then(|| breadboard_core::check::failures_text(&failures));
            if reds < 2 && tell(main, dir, &push::red_line(&commits, &head, &failures, &output, &run)) {
                let _ = push::write(dir, &at(Stage::Told, reds, why));
                return;
            }
            ask(dir, &push::second_red_text(branch, &head, &failures));
            let _ = push::write(dir, &at(Stage::Asked, reds, Some("wt:check failed again".into())));
        }
    }
}

/// `wt:check` in `dir`: whether it passed, what it printed.
fn check(dir: &Path) -> (bool, String) {
    let out = Command::new(crate::cli::lifecycle::proc::exe())
        .arg("wt:check")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(out) => (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        ),
        Err(error) => (false, error.to_string()),
    }
}

/// The push of `dir`'s checked HEAD, then its draft MR; refused, the user's.
fn send(main: &Path, dir: &Path, branch: &str, commits: &str, pushing: &Pushing) {
    let out = git(dir, &["push", "--quiet", "-u", "origin", branch]);
    if !out.is_ok {
        let why = format!("{}{}", out.stdout, out.stderr);
        ask(dir, &push::refused_text(branch, &why));
        let _ = push::write(
            dir,
            &Pushing {
                stage: Stage::Asked,
                why: Some("push refused".into()),
                ..pushing.clone()
            },
        );
        return;
    }
    push::clear(dir);
    open_mr(dir, branch);
    if let Some(origin) = breadboard_core::record::origin_of(dir) {
        let _ = breadboard_core::journal::append(
            main,
            &origin.name,
            &format!("{branch}: {commits} checked and pushed by breadboard, no agent run (#450)"),
            now_ms(),
        );
    }
}

/// The draft MR of `branch` from its issue, once (`.claude/pushing-mr`):
/// `wt:mr` with `Closes #<n>` when its agent wrote no description yet; it
/// leaves an open MR as it is.
fn open_mr(dir: &Path, branch: &str) {
    let marker = dir.join(".claude").join("pushing-mr");
    if marker.exists() {
        return;
    }
    let mut command = Command::new(crate::cli::lifecycle::proc::exe());
    command
        .arg("wt:mr")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !dir.join(".claude").join("mr-description.md").is_file() {
        let Some(number) = git::issue_of_branch(branch) else {
            return;
        };
        let file = dir.join(".claude").join("pushing-mr.md");
        let text = format!(
            "Closes #{number}\n\nOpened by breadboard with the push of its first commits (#450); its agent writes the description ({} wt:mr --update).\n",
            breadboard_core::config::run_word()
        );
        if std::fs::write(&file, text).is_err() {
            return;
        }
        command.arg("--description-file").arg(&file);
    }
    if command.status().is_ok_and(|status| status.success()) {
        let _ = std::fs::write(&marker, "");
    }
}

/// One line for the headless agent of `dir`: false when none is bound there.
fn tell(main: &Path, dir: &Path, line: &str) -> bool {
    runs::enqueue_line(main, dir, line).is_ok()
}

/// A `look` action of the worktree `dir`: the user's from here.
fn ask(dir: &Path, text: &str) {
    let _ = add_action(
        &actions_file(dir),
        NewAction {
            kind: "look".into(),
            text: text.into(),
            ..Default::default()
        },
    );
}
