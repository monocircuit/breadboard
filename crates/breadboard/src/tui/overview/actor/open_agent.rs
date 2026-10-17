//! A click on an agent (#361): a headless task agent is a session id, not a
//! tab, so the click opens `claude --resume <id>` interactively in the tab
//! `<feature>` of its worktree's space, once no run of it is in flight. While
//! that claude runs, the daemon holds the agent's queue (`runs`, #358); its
//! lines go headless again once it ends. A claude that runs the conversation
//! already is only put in front.

use super::herdr::{self, quote};
use breadboard_core::conversations::{bound_session, interactive_resumes, running_claudes};
use breadboard_core::git::feature_of;
use breadboard_core::runs::{self, State, Table};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a click waits for a run of the agent to end before it opens it.
const RUN_WAIT: Duration = Duration::from_secs(15 * 60);

/// What a click on an agent does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// A claude runs the conversation: its tab in front.
    Focus,
    /// A headless run is in flight: wait for it to end, then open.
    Wait,
    /// Nothing runs it: `claude --resume` in a tab.
    Open,
}

/// The step for an agent whose conversation a claude on this machine runs
/// (`is_running`) and whose run the status table says `run`.
pub fn step_of(is_running: bool, run: Option<State>) -> Step {
    if is_running {
        // A headless run is a claude too: it is no tab to put in front.
        if run == Some(State::Running) {
            return Step::Wait;
        }
        return Step::Focus;
    }
    if run == Some(State::Running) {
        Step::Wait
    } else {
        Step::Open
    }
}

/// The shell line that opens the conversation `id` in `dir` under `name`.
pub fn open_command(dir: &Path, id: &str, name: &str) -> String {
    format!(
        "cd {} && claude --resume {id} --name {}",
        quote(&dir.to_string_lossy()),
        quote(name)
    )
}

fn run_of(main: &Path, id: &str) -> Option<State> {
    let table: Table = runs::read_or_default(&runs::status_file(main));
    table.runs.get(id).map(|run| run.state)
}

/// Whether a claude runs the conversation: its session file, or an
/// interactive resume of it still at its folder's trust question (no file yet).
fn is_running(id: &str) -> bool {
    running_claudes().iter().any(|claude| claude.id == id) || interactive_resumes().contains(id)
}

/// Opens the agent of the worktree `dir` (on the actor's thread: a wait may
/// take minutes). What it did, for the log; Err why not.
pub fn open(main: &Path, dir: &Path) -> Result<String, String> {
    let id = bound_session(dir).ok_or("its worktree binds no conversation")?;
    let feature = feature_of(main, dir);
    let started = Instant::now();
    loop {
        match step_of(is_running(&id), run_of(main, &id)) {
            Step::Focus => {
                let tab = herdr::agent_tab_of_session(&id);
                herdr::focus_tab(main, dir, tab.as_deref());
                return Ok(format!("{feature}'s claude runs already: its tab in front"));
            }
            Step::Wait if started.elapsed() < RUN_WAIT => std::thread::sleep(Duration::from_secs(2)),
            Step::Wait => return Err("its run did not end within 15 min".into()),
            Step::Open => break,
        }
    }
    // Its space may be closed (#386): the click opens it again.
    let space = herdr::space_of(main, dir)
        .or_else(|| {
            let repo = crate::cli::lifecycle::repo::Repo {
                main: main.to_path_buf(),
            };
            crate::cli::lifecycle::layout::open_space(&repo, dir)
        })
        .ok_or("herdr has no space of its worktree and made none")?;
    let tabs = breadboard_core::herdr::tabs_of(&space);
    let command = open_command(dir, &id, &feature);
    if !herdr::ensure_tab(&space, &feature, dir, Some(&command), &tabs) {
        return Err(format!("its tab {feature} is busy: something else runs in it"));
    }
    let tab = breadboard_core::herdr::tabs_of(&space)
        .into_iter()
        .find(|tab| tab.label == feature)
        .map(|tab| tab.id);
    herdr::focus_tab(main, dir, tab.as_deref());
    Ok(format!(
        "{feature} opened in its tab: its lines wait until that claude ends"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_an_agent_only_between_its_runs() {
        assert_eq!(step_of(false, None), Step::Open);
        assert_eq!(step_of(false, Some(State::Idle)), Step::Open);
        assert_eq!(step_of(false, Some(State::Queued)), Step::Open);
        assert_eq!(step_of(false, Some(State::Asks)), Step::Open);
        assert_eq!(step_of(false, Some(State::Running)), Step::Wait);
        // Its run is the claude that runs it: no tab to put in front.
        assert_eq!(step_of(true, Some(State::Running)), Step::Wait);
        assert_eq!(step_of(true, Some(State::Queued)), Step::Focus);
        assert_eq!(step_of(true, None), Step::Focus);
    }

    #[test]
    fn resumes_it_where_its_worktree_is() {
        assert_eq!(
            open_command(Path::new("/r/p-7-x"), "0123", "7-x"),
            "cd /r/p-7-x && claude --resume 0123 --name 7-x"
        );
    }
}
