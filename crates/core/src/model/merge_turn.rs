//! Whether a merge is offered for an open MR now, and if not, what it waits
//! for (#344): the one rule NEXT, ACTIONS, the MR row, the worktree's state
//! word and herdr's sidebar take it from. Before it, the state word said
//! `waits for you: merge` from a green pipeline alone while ACTIONS offered
//! nothing, because the MR came after another in the merge order.

use super::Tree;
use super::merge_order::After;
pub use super::mr_blockers::MARK_READY;
use super::mr_blockers::{Reason, is_draft_reason, row_blockers_of};
use crate::gitlab::{IssueBox, Job, Mr};
use crate::json::{self, ints_of, str_of};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

/// The turn of an MR from the reasons of its row: a reason on another MR
/// (`who` is `!N`) waits for it, the draft is the merge's own step, any
/// other reason is its own lack and goes first.
pub fn turn_of(reasons: &[Reason]) -> MergeTurn {
    let mut waits: Vec<i64> = Vec::new();
    let mut lacks = false;
    for reason in reasons {
        if is_draft_reason(&reason.text) {
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
/// ready), else the agent's, once it is its turn (#280). Without
/// `needs_pipeline` (#443) wt:check at its pushed head stands for its pipeline.
#[allow(clippy::too_many_arguments)]
pub fn merge_verdict_of(
    mr: &Mr,
    boxes: &[IssueBox],
    behind: u64,
    after: &[After],
    jobs: &[Job],
    tree: Option<&Tree>,
    done: &HashSet<i64>,
    needs_pipeline: bool,
) -> (Vec<Reason>, MergeTurn) {
    let mut reasons = row_blockers_of(mr, boxes, behind, after, jobs, tree, done, needs_pipeline);
    let turn = turn_of(&reasons);
    for reason in reasons.iter_mut().filter(|reason| reason.text == MARK_READY) {
        reason.who = if turn == MergeTurn::Offered { "you" } else { "agent" }.into();
    }
    (reasons, turn)
}

/// A turn as the orchestration's overview hands it to herdr's sidebar, whose
/// daemon knows neither the merge order nor the boxes (#344): in the
/// worktree's `.claude/merge-turn.json`, for the commit GitLab's pipeline of
/// its MR ran on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    /// The commit of the MR's pipeline when the turn was found.
    pub sha: String,
    pub turn: MergeTurn,
}

pub fn turn_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("merge-turn.json")
}

impl TurnRecord {
    pub fn to_value(&self) -> Value {
        let (turn, waits) = match &self.turn {
            MergeTurn::Offered => ("offered", Vec::new()),
            MergeTurn::Waits(iids) => ("waits", iids.clone()),
            MergeTurn::Lacks => ("lacks", Vec::new()),
        };
        json!({ "sha": self.sha, "turn": turn, "waits": waits })
    }

    pub fn from_value(value: &Value) -> Option<TurnRecord> {
        let turn = match str_of(value, "turn")? {
            "offered" => MergeTurn::Offered,
            "waits" => MergeTurn::Waits(ints_of(value, "waits")),
            "lacks" => MergeTurn::Lacks,
            _ => return None,
        };
        Some(TurnRecord {
            sha: str_of(value, "sha")?.to_string(),
            turn,
        })
    }

    /// The turn as it holds now: None once the MR's pipeline runs on another
    /// commit (`sha`), or when every MR it waited for is gone from `open`
    /// (merged meanwhile: its overview finds the next turn).
    pub fn turn_now(&self, sha: Option<&str>, open: &HashSet<i64>) -> Option<MergeTurn> {
        if sha != Some(self.sha.as_str()) {
            return None;
        }
        match &self.turn {
            MergeTurn::Waits(iids) => {
                let left: Vec<i64> = iids.iter().copied().filter(|iid| open.contains(iid)).collect();
                (!left.is_empty()).then_some(MergeTurn::Waits(left))
            }
            turn => Some(turn.clone()),
        }
    }
}

/// The turn the overview left in the worktree `dir`.
pub fn turn_record_of(dir: &Path) -> Option<TurnRecord> {
    TurnRecord::from_value(&json::read(&turn_file(dir))?)
}

/// Writes `record` into `dir` unless it is there already: the sidebar reads
/// it on each tick, a write only when the turn changed.
pub fn write_turn_record(dir: &Path, record: &TurnRecord) -> std::io::Result<bool> {
    if turn_record_of(dir).as_ref() == Some(record) {
        return Ok(false);
    }
    json::write_pretty(&turn_file(dir), &record.to_value())?;
    Ok(true)
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
    fn the_sidebar_takes_a_record_only_for_its_commit_and_while_its_waits_are_open() {
        let record = TurnRecord {
            sha: "a".into(),
            turn: MergeTurn::Waits(vec![333, 330]),
        };
        assert_eq!(TurnRecord::from_value(&record.to_value()), Some(record.clone()));
        let open: HashSet<i64> = [333].into();
        assert_eq!(record.turn_now(Some("a"), &open), Some(MergeTurn::Waits(vec![333])));
        assert_eq!(record.turn_now(Some("b"), &open), None, "pushed since");
        assert_eq!(record.turn_now(Some("a"), &HashSet::new()), None, "all merged since");
        let offered = TurnRecord {
            sha: "a".into(),
            turn: MergeTurn::Offered,
        };
        assert_eq!(offered.turn_now(Some("a"), &open), Some(MergeTurn::Offered));
        let dir = std::env::temp_dir().join(format!("merge-turn-{}", std::process::id()));
        assert_eq!(write_turn_record(&dir, &record).ok(), Some(true));
        assert_eq!(
            write_turn_record(&dir, &record).ok(),
            Some(false),
            "unchanged: no write"
        );
        assert_eq!(turn_record_of(&dir), Some(record));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn without_its_own_pipeline_wt_check_offers_the_merge() {
        // #443: a draft into develop, its bar full, every push's pipeline skipped.
        use crate::check::CheckState;
        use crate::gitlab::Pipeline;
        let mr = Mr {
            iid: 443,
            state: "opened".into(),
            is_draft: true,
            sha: Some("abc".into()),
            merge_status: Some("draft_status".into()),
            pipeline: Some(Pipeline {
                status: "skipped".into(),
                sha: Some("abc".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let boxes = [IssueBox {
            heading: Some("Work".into()),
            text: "Tests".into(),
            is_ticked: true,
            line: 0,
        }];
        let tree = |sha: &str| Tree {
            checked: CheckState::Tested {
                sha: sha.into(),
                packages: Vec::new(),
                no_test: None,
            },
            ..Default::default()
        };
        let verdict = |tree: &Tree, needs_pipeline: bool| {
            merge_verdict_of(&mr, &boxes, 0, &[], &[], Some(tree), &HashSet::new(), needs_pipeline).1
        };
        assert_eq!(verdict(&tree("abc"), false), MergeTurn::Offered);
        assert_eq!(verdict(&tree("old"), false), MergeTurn::Lacks, "checked another commit");
        assert_eq!(
            verdict(&tree("abc"), true),
            MergeTurn::Lacks,
            "the switch on: its pipeline"
        );
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
