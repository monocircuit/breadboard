//! Whether a merge is offered for an open MR now, and if not, what it waits
//! for (#344): the one rule NEXT, ACTIONS, the MR row, the worktree's state
//! word and herdr's sidebar take it from. Before it, the state word said
//! `waits for you: merge` from a green pipeline alone while ACTIONS offered
//! nothing, because the MR came after another in the merge order.

use super::Tree;
use super::merge_order::After;
use super::mr_blockers::{Reason, row_blockers_of};
use crate::gitlab::{IssueBox, Job, Mr};
use std::collections::HashSet;

/// Where an open MR stands with its merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeTurn {
    /// Nothing but the draft holds it up: ACTIONS offers its merge (the
    /// agent's posted one, else the overview's own), two clicks mark it
    /// ready and merge it.
    Offered,
    /// Done itself, it waits for these MRs: the merge order's `after` and
    /// the agent's `wt:wait`. Nobody's step yet.
    Waits(Vec<i64>),
    /// It still lacks something of its own: pipeline, boxes, tries,
    /// conflicts, a change, unpushed commits. The row's reasons say what.
    Lacks,
}

/// The reason a draft gives: the merge action marks it ready.
pub const MARK_READY: &str = "draft: mark ready";

/// The turn of an MR from the reasons of its row: a reason on another MR
/// (`who` is `!N`) waits for it, the draft is the merge's own step, any
/// other reason is its own lack and goes first.
pub fn turn_of(reasons: &[Reason]) -> MergeTurn {
    let mut waits: Vec<i64> = Vec::new();
    let mut lacks = false;
    for reason in reasons {
        if reason.text == MARK_READY {
            continue;
        }
        match reason.who.strip_prefix('!').and_then(|iid| iid.parse::<i64>().ok()) {
            Some(iid) => {
                if !waits.contains(&iid) {
                    waits.push(iid);
                }
            }
            None => lacks = true,
        }
    }
    if lacks {
        MergeTurn::Lacks
    } else if waits.is_empty() {
        MergeTurn::Offered
    } else {
        MergeTurn::Waits(waits)
    }
}

/// `waits for !333, !330`.
pub fn waits_for_text(iids: &[i64]) -> String {
    let names: Vec<String> = iids.iter().map(|iid| format!("!{iid}")).collect();
    format!("waits for {}", names.join(", "))
}

/// The reasons of an open MR's row and its turn, the draft's owner set by
/// the turn: `you` only while its merge is offered (the two clicks mark it
/// ready), else the agent's, once it is its turn (#280).
pub fn merge_verdict_of(
    mr: &Mr,
    boxes: &[IssueBox],
    behind: u64,
    after: &[After],
    jobs: &[Job],
    tree: Option<&Tree>,
    done: &HashSet<i64>,
) -> (Vec<Reason>, MergeTurn) {
    let mut reasons = row_blockers_of(mr, boxes, behind, after, jobs, tree, done);
    let turn = turn_of(&reasons);
    for reason in reasons.iter_mut().filter(|reason| reason.text == MARK_READY) {
        reason.who = if turn == MergeTurn::Offered { "you" } else { "agent" }.into();
    }
    (reasons, turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(text: &str, who: &str) -> Reason {
        Reason {
            text: text.into(),
            who: who.into(),
            is_conflict: false,
        }
    }

    #[test]
    fn nothing_but_the_draft_offers_the_merge() {
        assert_eq!(turn_of(&[]), MergeTurn::Offered);
        assert_eq!(turn_of(&[reason(MARK_READY, "agent")]), MergeTurn::Offered);
    }

    #[test]
    fn a_done_mr_behind_another_waits_for_it() {
        let reasons = [
            reason(MARK_READY, "you"),
            reason("after !333 (shares package.json +3)", "!333"),
            reason("waits for !330", "!330"),
            reason("waits for !330", "!330"),
        ];
        assert_eq!(turn_of(&reasons), MergeTurn::Waits(vec![333, 330]));
        assert_eq!(waits_for_text(&[333, 330]), "waits for !333, !330");
    }

    #[test]
    fn its_own_lack_goes_before_a_wait() {
        let reasons = [
            reason("1 acceptance box open: works", "agent"),
            reason("after !333 (shares a.rs)", "!333"),
        ];
        assert_eq!(turn_of(&reasons), MergeTurn::Lacks);
        assert_eq!(turn_of(&[reason("try #4", "you")]), MergeTurn::Lacks);
        assert_eq!(turn_of(&[reason("pipeline running", "GitLab")]), MergeTurn::Lacks);
    }
}
