//! An issue of wt:orchestrate (orchestrator.mjs): read from GitLab with the
//! worktree it gets, its entry in the record, the brief its agent reads, its
//! worktree made by `wt:new --origin` in a child and its claude started.

use crate::cli::lifecycle::handover;
use crate::cli::setup::conversations::session_file;
use crate::cli::setup::proc::{self, quote};
use crate::cli::setup::repo;
use breadboard_core::time::now_ms;
use breadboard_core::{record, runs};
use serde_json::{Map, Value};
use std::fs;
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
    /// A fresh agent's `## Handoff`: the retired agent's note it reads (#381).
    pub handoff: Option<String>,
}

/// The handoff a fresh agent on `item` reads (#381): the retired agent of
/// `name` sharing the most files its description names, its kept note and
/// the brief's `## Handoff` about it. None without one.
pub fn handoff_for(name: &str, item: &handover::Issue) -> Option<(std::path::PathBuf, String)> {
    let main = repo::main_dir();
    let found = record::record_of(&main, name)?;
    let paths = breadboard_core::gitlab::paths_of(&item.description);
    let (worker, shared) = breadboard_core::model::workers::handoff_of(&found.retired(), &paths)?;
    let retired = worker.retired.as_ref()?;
    let kept = main.join(retired.handoff.as_ref()?);
    if !kept.exists() {
        return None;
    }
    Some((kept, handoff_text(worker, &shared)))
}

/// What the brief says about the handoff in `.claude/handoff.md`.
pub fn handoff_text(worker: &record::Worker, shared: &[String]) -> String {
    let name = worker
        .display_name
        .clone()
        .or_else(|| worker.name.clone())
        .unwrap_or_else(|| "an agent".into());
    let issues = if worker.issues.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = worker.issues.iter().map(|number| format!("#{number}")).collect();
        format!(" ({})", list.join(", "))
    };
    let context = worker
        .retired
        .as_ref()
        .and_then(|retired| retired.context)
        .map(|share| format!(" at {} % of its context", (share * 100.0).round() as i64))
        .unwrap_or_default();
    format!(
        "`.claude/handoff.md` is the handoff of {name}, who worked on {} before{issues} and retired{context}. Read it before you start: what it learned about those files. It is a note from before: where it and the code or the issue differ, the code and the issue win.",
        shared.join(", ")
    )
}

/// The handoff copied into the worktree `dir` as `.claude/handoff.md`; false when it couldn't be.
pub fn give_handoff(kept: &Path, dir: &Path) -> bool {
    let target = dir.join(crate::cli::lifecycle::retire::HANDOFF);
    target.parent().is_some_and(|parent| fs::create_dir_all(parent).is_ok()) && fs::copy(kept, &target).is_ok()
}

/// The text is the lifecycle group's (handover.rs `brief_of`), one for both:
/// a fresh agent's and a handed-over one's must not drift.
pub fn brief_of(brief: &Brief) -> String {
    let main = repo::main_dir();
    let mut text = handover::brief_of(
        brief.item,
        brief.name,
        &record::space_label_of(&main, brief.name),
        brief.peer,
        brief.extra,
        brief.before.as_deref(),
        &main,
    );
    if let Some(handoff) = &brief.handoff {
        text.push_str(&format!("\n## Handoff\n\n{handoff}\n"));
    }
    text
}

/// `wt:new <feature> --origin <name>` in a child, its output shown, its stderr read for `✗ …`.
pub fn make_worktree(item: &Item, name: &str, is_shared: bool) -> handover::Made {
    proc::flush_out();
    handover::make_worktree(&repo::lifecycle(), item, name, is_shared)
}

/// A random conversation id (UUID v4).
pub fn new_session_id() -> String {
    breadboard_core::conversations::new_session_id()
}

/// What starting an agent did.
pub struct Agent {
    pub is_started: bool,
    pub command: String,
    pub id: String,
}

/// The agent of an orchestrated worktree: its brief written, then started
/// headless (#361), else the command that queues it by hand.
pub fn start_agent(dir: &Path, brief: &str) -> Agent {
    let feature = repo::feature_of(dir);
    let _ = record::write_brief(dir, brief);
    start_headless(&repo::main_dir(), dir, &feature, new_session_id())
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
        allowed: Vec::new(),
    };
    let is_started = is_bound && runs::enqueue(main, event).is_ok();
    if is_started {
        crate::daemon::client::wake_runs(main);
    }
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
    use breadboard_core::record::{Retired, Worker};

    #[test]
    fn names_the_handoff_in_the_brief() {
        let worker = Worker {
            name: Some("340-x".into()),
            display_name: Some("Mira Albrecht".into()),
            issues: vec![340, 352],
            retired: Some(Retired {
                context: Some(0.468),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            handoff_text(&worker, &["a.rs".into(), "b.rs".into()]),
            "`.claude/handoff.md` is the handoff of Mira Albrecht, who worked on a.rs, b.rs before (#340, #352) and retired at 47 % of its context. Read it before you start: what it learned about those files. It is a note from before: where it and the code or the issue differ, the code and the issue win."
        );
    }

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
