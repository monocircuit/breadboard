//! The command table of a node (#498): what may be asked of the daemon for
//! the user's own agents, by `breadboard command` today and over lab web's
//! stream later (#503). Each command maps onto what a click or the CLI does
//! already: a start or a hand-over (`wt:orchestrate`), an answer, a merge
//! into the batch and the ship of a batch (the overview's actor, the
//! daemon's headless one), a pause or resume of the runs, a line told and
//! an agent stopped (`breadboard runs`), a worktree's stack stopped
//! (`stack:down`).
//!
//! Every command carries an id of its asker's: the same id twice is done
//! once, the second gets the first's result (`kept`, the newest KEPT of the
//! repo in `.claude/commands.json`).

use crate::json;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The version of the commands and results: a node and its asker of another refuse each other.
pub const PROTOCOL: u32 = 1;
/// How many results the repo keeps, the newest.
const KEPT: usize = 200;

/// One command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    /// Issue `number` of the orchestration started: by a fresh agent, or handed to the agent `to`.
    StartIssue {
        orchestration: String,
        number: i64,
        #[serde(default)]
        to: Option<String>,
    },
    /// Action `id` in the actions file of `dir` answered, as a click does.
    AnswerAction {
        orchestration: String,
        dir: PathBuf,
        id: i64,
        answer: String,
        #[serde(default)]
        note: Option<String>,
    },
    /// The MR `iid` merged into its batch, as the second click of its merge does.
    MergeIntoBatch {
        orchestration: String,
        iid: i64,
    },
    /// The batch of `product` shipped into develop (`wt:batch <product> --ship`).
    ShipBatch {
        orchestration: String,
        product: String,
    },
    /// No run starts until the resume; `now`: the runs in flight end too.
    PauseRuns {
        #[serde(default)]
        now: bool,
    },
    ResumeRuns,
    /// A line queued for the agent of the worktree `dir`.
    Tell {
        dir: PathBuf,
        line: String,
    },
    /// The agent of `dir` stopped: its run in flight ends, a claude of it held in a pane too.
    StopAgent {
        dir: PathBuf,
    },
    /// The stack of the worktree `dir` stopped, as `stack:down` does (#514); the shared one of the main checkout never.
    StopStack {
        dir: PathBuf,
    },
}

impl Command {
    /// The orchestration whose actor does it, None for what the CLI does alone.
    pub fn orchestration(&self) -> Option<&str> {
        match self {
            Command::StartIssue { orchestration, .. }
            | Command::AnswerAction { orchestration, .. }
            | Command::MergeIntoBatch { orchestration, .. }
            | Command::ShipBatch { orchestration, .. } => Some(orchestration),
            Command::PauseRuns { .. }
            | Command::ResumeRuns
            | Command::Tell { .. }
            | Command::StopAgent { .. }
            | Command::StopStack { .. } => None,
        }
    }
}

/// A command as it is asked: its id, the repo, the protocol it speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asked {
    pub id: String,
    pub protocol: u32,
    pub main: PathBuf,
    pub command: Command,
}

/// What came of a command: done (`is_ok`) or refused, in words. A started
/// act (a merge, a start) is done once it started; how it ends is in the
/// actor's log (`model::acting`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    pub id: String,
    pub is_ok: bool,
    pub text: String,
}

impl Done {
    pub fn of(id: &str, result: Result<String, String>) -> Done {
        let (is_ok, text) = match result {
            Ok(text) => (true, text),
            Err(text) => (false, text),
        };
        Done {
            id: id.to_string(),
            is_ok,
            text,
        }
    }
}

/// Where the repo keeps the results.
pub fn done_file(main: &Path) -> PathBuf {
    main.join(".claude").join("commands.json")
}

/// The result of the command `id`, when it was done before.
pub fn kept(main: &Path, id: &str) -> Option<Done> {
    read(main).into_iter().find(|done| done.id == id)
}

/// `done` kept for a command asked again, the newest KEPT.
pub fn keep(main: &Path, done: &Done) {
    let mut all = read(main);
    all.retain(|known| known.id != done.id);
    all.push(done.clone());
    let skip = all.len().saturating_sub(KEPT);
    let all: Vec<Done> = all.into_iter().skip(skip).collect();
    if let Ok(text) = serde_json::to_string_pretty(&all) {
        let _ = json::write_whole(&done_file(main), &format!("{text}\n"));
    }
}

fn read(main: &Path) -> Vec<Done> {
    std::fs::read_to_string(done_file(main))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_a_command_in_one_line_with_its_kind() {
        let asked = Asked {
            id: "c1".into(),
            protocol: PROTOCOL,
            main: "/repo".into(),
            command: Command::StartIssue {
                orchestration: "b".into(),
                number: 7,
                to: None,
            },
        };
        let line = serde_json::to_string(&asked).unwrap();
        assert!(line.contains(r#""kind":"start_issue""#), "{line}");
        assert_eq!(serde_json::from_str::<Asked>(&line).unwrap(), asked);
        let resume: Command = serde_json::from_str(r#"{"kind":"resume_runs"}"#).unwrap();
        assert_eq!(resume, Command::ResumeRuns);
        let pause: Command = serde_json::from_str(r#"{"kind":"pause_runs"}"#).unwrap();
        assert_eq!(pause, Command::PauseRuns { now: false });
        assert_eq!(resume.orchestration(), None);
        assert_eq!(asked.command.orchestration(), Some("b"));
    }

    #[test]
    fn keeps_the_newest_results_by_id() {
        let main = std::env::temp_dir().join(format!("bb-command-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&main);
        std::fs::create_dir_all(main.join(".claude")).unwrap();
        assert_eq!(kept(&main, "a"), None);
        keep(&main, &Done::of("a", Ok("done".into())));
        keep(&main, &Done::of("b", Err("refused".into())));
        assert_eq!(kept(&main, "a").map(|done| done.is_ok), Some(true));
        assert_eq!(kept(&main, "b").map(|done| done.text), Some("refused".into()));
        for n in 0..KEPT {
            keep(&main, &Done::of(&format!("n{n}"), Ok(String::new())));
        }
        assert_eq!(kept(&main, "a"), None, "the oldest went");
        assert!(kept(&main, &format!("n{}", KEPT - 1)).is_some());
        let _ = std::fs::remove_dir_all(&main);
    }
}
