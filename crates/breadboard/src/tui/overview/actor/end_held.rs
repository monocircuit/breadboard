//! A claude open in the tab of a headless agent holds its lines (#361);
//! one herdr resumed by itself after a restart is no one's (#386). A click
//! on ACTIONS' `held` row (and `breadboard runs release <dir>`) ends it once
//! herdr says it is idle, so no answer is cut off; its lines then run
//! headless again. One herdr's restore started (a bare `claude --resume`,
//! no click's `--name`) the overview ends by itself once it idles.

use breadboard_core::conversations::{bound_session, interactive_pids_of};
use breadboard_core::git::feature_of;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long it waits for the claude to finish what it does.
const IDLE_WAIT: Duration = Duration::from_secs(15 * 60);

/// Whether herdr's state of the open claude lets it end: it does nothing
/// (`idle`, `done`), or herdr no longer shows it in a pane.
pub fn is_quiet(status: Option<&str>) -> bool {
    matches!(status, None | Some("idle" | "done"))
}

/// Ends the claude open on the agent of `dir` once it idles (on the actor's
/// thread: the wait may take minutes). What it did, for the log; Err why not.
pub fn end(main: &Path, dir: &Path) -> Result<String, String> {
    let id = bound_session(dir).ok_or("its worktree binds no conversation")?;
    let feature = feature_of(main, dir);
    let started = Instant::now();
    loop {
        let status = breadboard_core::herdr::all_panes()
            .into_iter()
            .find(|pane| pane.session.as_deref() == Some(id.as_str()))
            // A pane whose state herdr doesn't know isn't taken for idle.
            .map(|pane| pane.status.unwrap_or_else(|| "unknown".into()));
        if is_quiet(status.as_deref()) {
            break;
        }
        if started.elapsed() >= IDLE_WAIT {
            return Err(format!("{feature}'s open claude kept working for 15 min: it stays"));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let pids = interactive_pids_of(&id);
    if pids.is_empty() {
        return Ok(format!("{feature}: no open claude runs it any more"));
    }
    for pid in &pids {
        // SAFETY: a signal to a pid read from /proc a moment ago; SIGTERM lets claude save.
        unsafe { libc::kill(*pid as libc::pid_t, libc::SIGTERM) };
    }
    crate::daemon::client::wake_runs(main);
    Ok(format!("{feature}'s open claude ended: its lines run headless again"))
}

/// herdr's own resume of a headless agent (#386): `claude --resume <id>`
/// without a name in a pane its restore brought back, never a click's. Ended
/// as soon as herdr says it is idle, else left for the next read: a turn is
/// never cut off. What it did, None when nothing.
pub fn end_herdr_resume(main: &Path, dir: &Path, id: &str, pid: i64) -> Option<String> {
    let status = breadboard_core::herdr::all_panes()
        .into_iter()
        .find(|pane| pane.session.as_deref() == Some(id))
        .map(|pane| pane.status.unwrap_or_else(|| "unknown".into()))?;
    if !is_quiet(Some(&status)) {
        return None;
    }
    // SAFETY: a signal to a pid read from /proc a moment ago; SIGTERM lets claude save.
    (unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0).then(|| {
        crate::daemon::client::wake_runs(main);
        format!(
            "ended the claude herdr reopened for {} after a restart: its lines run headless again",
            feature_of(main, dir)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_it_only_while_it_does_nothing() {
        assert!(is_quiet(Some("idle")));
        assert!(is_quiet(Some("done")));
        assert!(is_quiet(None));
        assert!(!is_quiet(Some("working")));
        assert!(!is_quiet(Some("blocked")));
        assert!(!is_quiet(Some("")));
    }
}
