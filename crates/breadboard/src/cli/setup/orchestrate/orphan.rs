//! A worktree whose agent is gone (#455): its agent moved on (a hotfix's
//! `wt:new` took it along, a `--to` elsewhere) and bound itself nowhere here.
//! `wt:orchestrate <name> <issue>` used to refuse it (`… exists already`),
//! a `--to` to a full agent is refused (#445): it gets a fresh agent in
//! place, no new worktree.

use super::issue::Item;
use crate::cli::lifecycle::remove::mrs_of;
use crate::cli::setup::conversations::{bound_session, left_in, running_claudes};
use crate::cli::setup::proc::step;
use crate::cli::setup::repo;
use breadboard_core::fresh::HANDOFF_NEXT;
use breadboard_core::json::str_of;
use breadboard_core::record;
use std::path::Path;
use std::time::Duration;

/// How long the read of its MRs may take; unanswered, it is no orphan.
const MRS_LIMIT: Duration = Duration::from_secs(15);

/// Whether `item`'s worktree takes a fresh agent in place: there and the
/// orchestration `name`'s, its issue open, no conversation bound to it and
/// no claude standing in it, its branch's MR not merged (GitLab answered:
/// a merged one's worktree only waits for its removal).
pub fn is_orphan(name: &str, item: &Item) -> bool {
    let dir = &item.dir;
    // read_issue's word for an open issue whose worktree is there.
    item.why.as_deref().is_some_and(|why| why.ends_with("exists already"))
        && record::origin_of(dir).is_some_and(|origin| origin.name == name)
        && bound_session(dir).is_none()
        && left_in(dir, &running_claudes(), None).is_empty()
        && mrs_of(&repo::lifecycle(), &item.branch, MRS_LIMIT)
            .is_some_and(|mrs| !mrs.iter().any(|mr| str_of(mr, "state") == Some("merged")))
}

/// What the brief tells the agent that takes it over.
pub fn taken_over_text(bb: &str) -> String {
    format!(
        "An agent worked on this issue here before you and is gone: its commits (and its MR, if it opened one) are on the branch. `{bb} status` says where it stands; go on from there, don't start over."
    )
}

/// The brief's `## Handoff` about the note its gone agent wrote in it at its
/// last run (#445's `handoff-next.md`), None without one.
pub fn left_handoff(dir: &Path) -> Option<String> {
    dir.join(HANDOFF_NEXT).exists().then(|| {
        step(&format!("its gone agent's handoff: {HANDOFF_NEXT}"));
        format!(
            "`{HANDOFF_NEXT}` is the handoff the agent before you wrote here at its last run. Read it before you go on: what it learned about these files. It is a note from before: where it and the code or the issue differ, the code and the issue win."
        )
    })
}
