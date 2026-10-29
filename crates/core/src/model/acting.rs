//! What the actor of an orchestration keeps between its reads, as a file
//! (#498): the daemon acts for every orchestration of its repos by itself
//! (`breadboard serve`, a headless overview each), and what it did and has
//! under way is written next to the record, `<name>.acting`, after
//! each change. An overview that doesn't act draws it from there: the log
//! under the band, the removals, starts and merges under way, the lines
//! that didn't get in. Its own clicks stay its own and win.

use super::Paint;
use super::auto_actions::Removal;
use super::workers::StartRun;
use crate::record::records_dir;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A thing done, for the log: when, how it went, what — in words, and
/// what exactly was run or typed, the way lazygit's command log puts the
/// command under its action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Did {
    pub at: i64,
    /// Green done, red failed, yellow under way or waiting.
    pub paint: Paint,
    pub text: String,
    /// The command run or the line typed, word for word.
    pub detail: Option<String>,
    /// One act's entry: its end replaces its start, never a second entry.
    pub key: Option<String>,
    /// How often it was done: an act done again, at any later tick, counts here.
    pub times: u32,
    /// When it was done the last time (ms): `at` stays its first.
    pub last: i64,
}

/// Where a merge stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergePhase {
    Armed,
    Running,
    Merged,
    Failed,
}

/// A merge as the file keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merging {
    pub phase: MergePhase,
    pub error: Option<String>,
}

/// What the actor of one orchestration keeps (core's `ViewState`'s writing
/// side and the actor's log).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Acting {
    /// The process that acts: a file of one that is gone is nobody's.
    pub pid: i64,
    /// The log, newest first.
    pub did: Vec<Did>,
    pub removals: BTreeMap<PathBuf, Removal>,
    /// The starts and handovers, by issue.
    pub starts: BTreeMap<i64, StartRun>,
    /// A failed pipeline's line that didn't get in: why, by its key.
    pub pipeline_tries: BTreeMap<String, String>,
    /// The worktrees whose ended wait didn't get in yet.
    pub undelivered_waits: BTreeSet<PathBuf>,
    /// The merges, by iid.
    pub merges: BTreeMap<i64, Merging>,
}

/// `.claude/orchestrations/<name>.acting` (JSON): not `.json`, or it would read as a record.
pub fn acting_file(main: &Path, name: &str) -> PathBuf {
    records_dir(main).join(format!("{name}.acting"))
}

/// What the actor of `name` keeps, while the process that wrote it lives.
pub fn read(main: &Path, name: &str) -> Option<Acting> {
    let text = std::fs::read_to_string(acting_file(main, name)).ok()?;
    let acting: Acting = serde_json::from_str(&text).ok()?;
    crate::lock::is_alive(acting.pid).then_some(acting)
}

/// Written whole, when it changed since `last` (the text written then).
pub fn write(main: &Path, name: &str, acting: &Acting, last: &mut Option<String>) {
    let Ok(text) = serde_json::to_string(acting) else {
        return;
    };
    if last.as_deref() == Some(text.as_str()) {
        return;
    }
    if crate::json::write_whole(&acting_file(main, name), &format!("{text}\n")).is_ok() {
        *last = Some(text);
    }
}

/// Two logs in one, newest first: `own` (an overview's clicks) and what the
/// actor of the orchestration did, `kept` entries at most.
pub fn merged_log(own: &[Did], acting: &[Did], kept: usize) -> Vec<Did> {
    let mut all: Vec<Did> = own.iter().chain(acting).cloned().collect();
    all.sort_by_key(|did| std::cmp::Reverse(did.last.max(did.at)));
    all.truncate(kept);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn did(text: &str, at: i64) -> Did {
        Did {
            at,
            paint: Paint::Green,
            text: text.into(),
            detail: None,
            key: None,
            times: 1,
            last: at,
        }
    }

    #[test]
    fn keeps_what_the_actor_did_while_it_lives() {
        let main = std::env::temp_dir().join(format!("bb-acting-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&main);
        std::fs::create_dir_all(records_dir(&main)).unwrap();
        let mut acting = Acting {
            pid: i64::from(std::process::id()),
            did: vec![did("removed x", 2)],
            ..Acting::default()
        };
        acting.removals.insert("/w/x".into(), Removal::default());
        acting.starts.insert(7, StartRun::default());
        acting.merges.insert(
            12,
            Merging {
                phase: MergePhase::Running,
                error: None,
            },
        );
        let mut last = None;
        write(&main, "b", &acting, &mut last);
        assert!(last.is_some());
        assert_eq!(read(&main, "b"), Some(acting.clone()));
        // Not a record: the orchestrations stay what they were.
        assert!(crate::record::recorded(&main).is_empty());
        // Of a process that is gone: nobody's.
        acting.pid = 0;
        let mut last = None;
        write(&main, "b", &acting, &mut last);
        assert_eq!(read(&main, "b"), None);
        let _ = std::fs::remove_dir_all(&main);
    }

    #[test]
    fn puts_two_logs_in_one_newest_first() {
        let own = [did("clicked", 5)];
        let acting = [did("told x", 9), did("removed y", 1)];
        let texts: Vec<String> = merged_log(&own, &acting, 2).into_iter().map(|did| did.text).collect();
        assert_eq!(texts, ["told x", "clicked"]);
    }
}
