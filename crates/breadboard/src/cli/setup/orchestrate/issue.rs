//! An issue of wt:orchestrate (orchestrator.mjs): read from GitLab with the
//! worktree it gets, its entry in the record, the brief its agent reads, its
//! worktree made by `wt:new --origin` in a child and its claude started.

use crate::cli::lifecycle::handover;
use crate::cli::lifecycle::layout::nvim::ide_flag;
use crate::cli::setup::conversations::session_file;
use crate::cli::setup::herdr;
use crate::cli::setup::proc::{self, quote};
use crate::cli::setup::repo;
use breadboard_core::time::now_ms;
use breadboard_core::{record, runs};
use serde_json::{Map, Value};
use std::fs;
use std::io::Read;
use std::path::Path;

/// Its first prompt: one line without quotes.
pub const BRIEF_PROMPT: &str = "Read .claude/brief.md and work on the issue it describes.";

/// An issue as wt:orchestrate starts it: the lifecycle group's reading,
/// one for a fresh start and a handover (its feature, branch, why not).
pub type Item = handover::Issue;

/// The leading number of an argument (`12` of `12-x`).
pub fn number_of(arg: &str) -> String {
    arg.chars().take_while(char::is_ascii_digit).collect()
}

/// An issue (its number, or `<n>-<feature>`) as GitLab has it, or why it can't be read.
pub fn read_issue(arg: &str) -> Result<Item, String> {
    handover::read_issue(&repo::lifecycle(), arg).map_err(|failed| failed.0)
}

pub use handover::{record_entry, start_entry};

/// What an orchestrated agent is told (verbatim, agents read it).
pub struct Brief<'a> {
    pub item: &'a Item,
    pub name: &'a str,
    pub peer: Option<&'a str>,
    pub extra: &'a str,
    pub before: Option<String>,
}

/// The text is the lifecycle group's (handover.rs `brief_of`), one for both:
/// a fresh agent's and a handed-over one's must not drift.
pub fn brief_of(brief: &Brief) -> String {
    let main = repo::main_dir();
    handover::brief_of(
        brief.item,
        brief.name,
        &record::orchestrator_label(&main, brief.name),
        brief.peer,
        brief.extra,
        brief.before.as_deref(),
        &main,
    )
}

/// `wt:new <feature> --origin <name>` in a child, its output shown, its stderr read for `✗ …`.
pub fn make_worktree(item: &Item, name: &str, is_shared: bool) -> handover::Made {
    proc::flush_out();
    handover::make_worktree(&repo::lifecycle(), item, name, is_shared)
}

/// A random conversation id (UUID v4).
pub fn new_session_id() -> String {
    let mut bytes = [0u8; 16];
    let is_read = fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok();
    if !is_read {
        let seed = now_ms() as u128 ^ (u128::from(std::process::id()) << 64);
        bytes = seed.to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// What starting an agent did.
pub struct Agent {
    pub is_started: bool,
    pub command: String,
    pub id: String,
}

/// The claude of an orchestrated worktree: its brief written, started in the
/// idle `<feature>` tab of its space (bound before it starts), else the command to run by hand.
pub fn start_agent(dir: &Path, brief: &str) -> Agent {
    let feature = repo::feature_of(dir);
    let _ = record::write_brief(dir, brief);
    let id = new_session_id();
    let main = repo::main_dir();
    if runs::is_headless(&main) {
        return start_headless(&main, dir, &feature, id);
    }
    let command = format!(
        "cd {} && claude --session-id {id} --name {feature}{} {}",
        quote(&dir.to_string_lossy()),
        ide_flag(dir),
        quote(BRIEF_PROMPT)
    );
    let workspace = if herdr::is_in_herdr() {
        herdr::space_of(dir)
    } else {
        None
    };
    let pane = workspace.and_then(|workspace| herdr::idle_pane_of(&workspace, &feature, &herdr::tabs_of(&workspace)));
    let is_started = pane.is_some_and(|pane| herdr::herdr_did(&["pane", "run", &pane, &command]));
    if is_started {
        let _ = fs::write(session_file(dir), format!("{id}\n"));
    }
    Agent {
        is_started,
        command,
        id,
    }
}

/// A headless agent (#359): bound first, then its brief queued as the first
/// run of a new session (`--session-id`, no transcript yet); no tab.
fn start_headless(main: &Path, dir: &Path, feature: &str, id: String) -> Agent {
    let is_bound = fs::write(session_file(dir), format!("{id}\n")).is_ok();
    let event = runs::Event {
        session: id.clone(),
        dir: dir.to_path_buf(),
        name: feature.to_string(),
        line: BRIEF_PROMPT.to_string(),
        at: now_ms(),
    };
    let is_started = is_bound && runs::enqueue(main, event).is_ok();
    Agent {
        is_started,
        command: format!(
            "{} runs tell {} {}",
            quote(&crate::cli::lifecycle::proc::exe()),
            quote(&dir.to_string_lossy()),
            quote(BRIEF_PROMPT)
        ),
        id,
    }
}

/// A start that failed once its worktree was made: the worktree goes again
/// (wt:remove in a child), a queued issue back to the queue (handover.rs
/// `undo_start`, one for both).
pub fn undo_start(name: &str, item: &Item, known: Option<&Value>, why: &str, claim: Option<&super::setups::Claim>) {
    proc::flush_out();
    handover::undo_start(&repo::lifecycle(), name, item, known, why, || {
        if let Some(claim) = claim {
            claim.release();
        }
    });
}

/// The record's entry of `number` now.
pub fn known_entry(name: &str, number: i64) -> Option<Value> {
    breadboard_core::json::read(&record::record_file(&repo::main_dir(), name))?
        .get("issues")?
        .as_array()?
        .iter()
        .find(|issue| breadboard_core::json::int_of(issue, "number") == Some(number))
        .cloned()
}

/// `fields` as an object for the record writes.
pub fn object(fields: Value) -> Map<String, Value> {
    fields.as_object().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_start_names_an_issue_as_a_handover_does() {
        // The critic's title: Node's NFKD gives this branch, the handover too.
        let title = "Cæsar\u{2019}s re\u{301}sume\u{301} of the plugin";
        assert_eq!(format!("230-{}", handover::slug_of(title)), "230-c-sar-s-resume-plugin");
    }

    #[test]
    fn makes_a_uuid() {
        let id = new_session_id();
        assert!(breadboard_core::record::is_session_id(&id), "{id}");
        assert_eq!(&id[14..15], "4");
    }
}
