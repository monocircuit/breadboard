//! The merge order of an orchestration, computed on each read from what the
//! overview knows anyway (#213, merge-order.mjs). `wt:plan` is the record and
//! the tie-breaker; the facts go before it:
//! - dependencies: an issue that names another as a prerequisite comes after it;
//! - file overlap: of two MRs touching the same code files, the one further
//!   along goes first. It only ranks them (a soft reason, #392): the other
//!   keeps its green pipeline and merges right after while GitLab says it
//!   merges cleanly; a real conflict is what has it merge develop in.
//!   Overlaps in `.md` files are left out altogether;
//! - readiness: a ready, green MR moves up past the ones ahead that are all
//!   blocked (draft, failed pipeline, conflicts) and share no files with it.

use super::merge::blockers_of;
use crate::actions::Action;
use crate::gitlab::{IssueBox, Mr};
use std::collections::{BTreeSet, HashSet};

/// How far along an open MR is, lower is further: 0 ready and green, 1 ready
/// with its pipeline still to come, 2 blocked (failed pipeline, conflicts), 3 a draft.
pub fn progress_of(mr: &Mr) -> u8 {
    if mr.is_draft {
        return 3;
    }
    let status = mr.pipeline_status();
    if status == Some("failed") || mr.has_conflicts {
        return 2;
    }
    if status == Some("success") { 0 } else { 1 }
}

/// An issue of the orchestration with an MR, as the merge order takes it.
#[derive(Debug, Clone)]
pub struct MergeEntry<'a> {
    pub number: i64,
    pub mr: &'a Mr,
    /// The issue numbers it names as prerequisites.
    pub needs: &'a [i64],
    /// What its work touches.
    pub files: &'a BTreeSet<String>,
    /// Its issue's boxes.
    pub boxes: &'a [IssueBox],
    /// How many commits of develop its branch lacks.
    pub behind: u64,
    /// Its worktree's actions, open and delivered: its open tries hold it up.
    pub actions: &'a [Action],
    pub answered: &'a [Action],
}

/// An MR it comes directly after: `why` the edge's phrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct After {
    pub number: i64,
    pub iid: i64,
    pub why: String,
    /// Whether the MR must merge first (a prerequisite it needs); else the
    /// edge only ranks them (shared files) and the MR is offered in parallel.
    pub is_hard: bool,
}

/// What holds an open MR up, each in a few words; none when it is ready to
/// merge: the merge's blockers named with the draft, how far behind its target a
/// conflicting branch is, and the MRs it comes after with why.
pub fn hold_up_of(entry: &MergeEntry, after: &[After]) -> Vec<String> {
    let blockers = blockers_of(Some(entry.mr), entry.boxes, true, entry.actions, entry.answered)
        .into_iter()
        .map(|blocker| {
            if blocker == format!("conflicts with {}", entry.mr.target()) && entry.behind > 0 {
                return format!("{blocker} ({} behind)", entry.behind);
            }
            // Of the open boxes the first by name, the rest counted: one line.
            let is_boxes = blocker.split_once(' ').is_some_and(|(count, rest)| {
                count.parse::<u64>().is_ok() && (rest.starts_with("work box") || rest.starts_with("acceptance box"))
            });
            let parts: Vec<&str> = blocker.split(", ").collect();
            if is_boxes && parts.len() > 1 {
                format!("{} +{}", parts[0], parts.len() - 1)
            } else {
                blocker
            }
        });
    let mut hold_up: Vec<String> = Vec::new();
    if entry.mr.is_draft {
        hold_up.push("draft".into());
    }
    hold_up.extend(blockers);
    let hard: Vec<&After> = after.iter().filter(|pred| pred.is_hard).collect();
    if !hard.is_empty() {
        let list: Vec<String> = hard
            .iter()
            .map(|pred| format!("!{} ({})", pred.iid, pred.why))
            .collect();
        hold_up.push(format!("after {}", list.join(", ")));
    }
    hold_up
}

/// How many shared files a reason names before `+n`.
const NAMED_FILES: usize = 1;

/// `render-orchestration.mjs +2`: the base names of the first few files.
fn files_text(files: &[&String]) -> String {
    let named: Vec<&str> = files
        .iter()
        .take(NAMED_FILES)
        .map(|file| file.rsplit('/').next().unwrap_or(file))
        .collect();
    let more = if files.len() > NAMED_FILES {
        format!(" +{}", files.len() - NAMED_FILES)
    } else {
        String::new()
    };
    format!("{}{more}", named.join(", "))
}

/// An open MR in the computed order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItem {
    pub number: i64,
    /// Why it stands there, one short line.
    pub reason: String,
    /// The MRs it comes directly after.
    pub after: Vec<After>,
    /// What keeps it from merging.
    pub hold_up: Vec<String>,
}

/// The merge order (merge-order.mjs `mergeOrderOf`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeOrder {
    /// The open MRs in the computed order.
    pub open: Vec<OpenItem>,
    /// The merged ones in the order they were merged, then the closed.
    pub merged: Vec<i64>,
    /// Every issue in the order the view ranks them by: the open, what else the plan lists, the merged.
    pub numbers: Vec<i64>,
    /// The order to write into the plan: merged, open, the rest.
    pub planned: Vec<i64>,
    /// Whether the open ones leave the plan's order.
    pub is_off_plan: bool,
}

#[derive(Debug, Clone)]
struct Pred {
    number: i64,
    reason: String,
    why: String,
    is_hard: bool,
}

/// Whether a path is documentation: files of it never create an order (#392).
fn is_doc(file: &str) -> bool {
    file.ends_with(".md")
}

pub fn merge_order_of(entries: &[MergeEntry], plan: &[i64]) -> MergeOrder {
    let place_in = |number: i64| plan.iter().position(|planned| *planned == number).unwrap_or(usize::MAX);
    let by_plan = |a: &&MergeEntry, b: &&MergeEntry| {
        place_in(a.number)
            .cmp(&place_in(b.number))
            .then(a.number.cmp(&b.number))
    };
    let mut base: Vec<&MergeEntry> = entries.iter().filter(|entry| entry.mr.state == "opened").collect();
    base.sort_by(by_plan);
    let mut merged: Vec<&MergeEntry> = entries.iter().filter(|entry| entry.mr.state == "merged").collect();
    merged.sort_by(|a, b| {
        a.mr.merged_at
            .as_deref()
            .unwrap_or("")
            .cmp(b.mr.merged_at.as_deref().unwrap_or(""))
            .then(a.number.cmp(&b.number))
    });
    let mut closed: Vec<&MergeEntry> = entries
        .iter()
        .filter(|entry| entry.mr.state != "opened" && entry.mr.state != "merged")
        .collect();
    closed.sort_by(by_plan);

    // number → its predecessors, in the order of `base`.
    let mut before: Vec<(i64, Vec<Pred>)> = base.iter().map(|entry| (entry.number, Vec::new())).collect();
    let at = |before: &[(i64, Vec<Pred>)], number: i64| before.iter().position(|(known, _)| *known == number);
    fn reaches(before: &[(i64, Vec<Pred>)], from: i64, to: i64, seen: &mut HashSet<i64>) -> bool {
        if from == to {
            return true;
        }
        if !seen.insert(from) {
            return false;
        }
        let next: Vec<i64> = before
            .iter()
            .filter(|(_, preds)| preds.iter().any(|pred| pred.number == from))
            .map(|(number, _)| *number)
            .collect();
        next.into_iter().any(|next| reaches(before, next, to, seen))
    }
    for entry in &base {
        for need in entry.needs {
            if *need != entry.number
                && at(&before, *need).is_some()
                && let Some(own) = at(&before, entry.number)
            {
                before[own].1.push(Pred {
                    number: *need,
                    reason: format!("after #{need} (needs it)"),
                    why: format!("needs #{need}"),
                    is_hard: true,
                });
            }
        }
    }
    for (i, a) in base.iter().enumerate() {
        for b in &base[i + 1..] {
            let mut shared: Vec<&String> = a
                .files
                .iter()
                .filter(|file| !is_doc(file) && b.files.contains(*file))
                .collect();
            if shared.is_empty() {
                continue;
            }
            shared.sort();
            // Equally far along, the plan decides.
            let (first, second) = if progress_of(b.mr) < progress_of(a.mr) {
                (b, a)
            } else {
                (a, b)
            };
            if !reaches(&before, second.number, first.number, &mut HashSet::new())
                && let Some(own) = at(&before, second.number)
            {
                let text = files_text(&shared);
                before[own].1.push(Pred {
                    number: first.number,
                    reason: format!("goes after !{}: shares {text}", first.mr.iid),
                    why: format!("shares {text}"),
                    is_hard: false,
                });
            }
        }
    }

    let preds_of = |number: i64| -> &Vec<Pred> { &before[at(&before, number).unwrap_or(0)].1 };
    let mut open: Vec<OpenItem> = Vec::new();
    let mut placed: HashSet<i64> = HashSet::new();
    let mut remaining: Vec<&MergeEntry> = base.clone();
    while !remaining.is_empty() {
        let available: Vec<&MergeEntry> = remaining
            .iter()
            .copied()
            .filter(|entry| preds_of(entry.number).iter().all(|pred| placed.contains(&pred.number)))
            .collect();
        let mut reason = None;
        let pick = if available.is_empty() {
            reason = Some("as planned: its prerequisites go in a circle".to_string());
            remaining[0]
        } else {
            let ready_at = available.iter().position(|entry| progress_of(entry.mr) == 0);
            match ready_at {
                Some(ready_at)
                    if ready_at > 0 && available[..ready_at].iter().all(|entry| progress_of(entry.mr) >= 2) =>
                {
                    let blocked: Vec<String> = available[..ready_at]
                        .iter()
                        .map(|entry| format!("!{}", entry.mr.iid))
                        .collect();
                    reason = Some(format!(
                        "ready while {} {} blocked",
                        blocked.join(", "),
                        if ready_at > 1 { "are" } else { "is" }
                    ));
                    available[ready_at]
                }
                _ => available[0],
            }
        };
        let preds = preds_of(pick.number);
        let reason = reason.unwrap_or_else(|| {
            // The predecessor placed last is the one it waited for.
            let last = open
                .iter()
                .rev()
                .find(|item| preds.iter().any(|pred| pred.number == item.number));
            match last {
                Some(last) => preds
                    .iter()
                    .find(|pred| pred.number == last.number)
                    .map(|pred| pred.reason.clone())
                    .unwrap_or_default(),
                None if place_in(pick.number) == usize::MAX => "not in the plan: by number".into(),
                None => "as planned".into(),
            }
        });
        // The ones it comes directly after: not those another of them comes after.
        let after: Vec<After> = preds
            .iter()
            .enumerate()
            .filter(|(i, pred)| {
                preds.iter().position(|other| other.number == pred.number) == Some(*i)
                    && !preds.iter().any(|other| {
                        other.number != pred.number && reaches(&before, pred.number, other.number, &mut HashSet::new())
                    })
            })
            .map(|(_, pred)| After {
                number: pred.number,
                iid: base
                    .iter()
                    .find(|entry| entry.number == pred.number)
                    .map(|entry| entry.mr.iid)
                    .unwrap_or(0),
                why: pred.why.clone(),
                is_hard: pred.is_hard,
            })
            .collect();
        open.push(OpenItem {
            number: pick.number,
            reason,
            hold_up: hold_up_of(pick, &after),
            after,
        });
        placed.insert(pick.number);
        remaining.retain(|entry| !std::ptr::eq(*entry, pick));
    }

    let open_numbers: Vec<i64> = open.iter().map(|item| item.number).collect();
    let listed: HashSet<i64> = entries.iter().map(|entry| entry.number).collect();
    let plan_rest: Vec<i64> = plan.iter().copied().filter(|number| !listed.contains(number)).collect();
    let done: Vec<i64> = merged.iter().chain(closed.iter()).map(|entry| entry.number).collect();
    let plan_open: Vec<i64> = plan
        .iter()
        .copied()
        .filter(|number| at(&before, *number).is_some())
        .collect();
    MergeOrder {
        is_off_plan: !open_numbers.is_empty() && open_numbers != plan_open,
        numbers: open_numbers
            .iter()
            .chain(plan_rest.iter())
            .chain(done.iter())
            .copied()
            .collect(),
        planned: merged
            .iter()
            .map(|entry| entry.number)
            .chain(open_numbers.iter().copied())
            .chain(plan_rest.iter().copied())
            .collect(),
        merged: done,
        open,
    }
}

/// A row of the graph of the open MRs: an MR's (`number`, its `●` in its lane,
/// `below` the lanes on its further lines) or a line that joins or forks lanes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub number: Option<i64>,
    pub graph: String,
    pub below: String,
}

/// The open MRs as a graph like `git log --graph`, top to bottom in the merge
/// order: MRs that don't depend on each other stand in lanes side by side, one
/// that comes after others below them, their lanes joined into its own. A
/// lane is two characters wide.
pub fn graph_of(open: &[OpenItem]) -> Vec<GraphRow> {
    let mut successors: Vec<(i64, Vec<i64>)> = open.iter().map(|item| (item.number, Vec::new())).collect();
    for item in open {
        for pred in &item.after {
            if let Some((_, next)) = successors.iter_mut().find(|(number, _)| *number == pred.number) {
                next.push(item.number);
            }
        }
    }
    let mut lanes: Vec<Option<i64>> = Vec::new();
    let mut rows = Vec::new();
    let bar = |lane: &Option<i64>| if lane.is_none() { "  " } else { "│ " };
    let tie = |lanes: &[Option<i64>], cols: &[usize], corner: &str, middle: &str| {
        let (first, last) = (cols[0], cols[cols.len() - 1]);
        let mut graph = String::new();
        for (col, lane) in lanes.iter().enumerate() {
            let is_inside = col > first && col < last;
            if col == first {
                graph.push_str("├─");
            } else if col == last {
                graph.push_str(&format!("{corner} "));
            } else if cols.contains(&col) {
                graph.push_str(&format!("{middle}─"));
            } else if is_inside {
                graph.push_str(if lane.is_none() { "──" } else { "┼─" });
            } else {
                graph.push_str(bar(lane));
            }
        }
        GraphRow {
            number: None,
            graph: graph.trim_end().to_string(),
            below: String::new(),
        }
    };
    for item in open {
        let mut cols: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| **lane == Some(item.number))
            .map(|(col, _)| col)
            .collect();
        if cols.is_empty() {
            let col = lanes.iter().position(Option::is_none).unwrap_or(lanes.len());
            if col == lanes.len() {
                lanes.push(None);
            }
            lanes[col] = Some(item.number);
            cols = vec![col];
        }
        // Lanes that lead to it join into the leftmost.
        if cols.len() > 1 {
            rows.push(tie(&lanes, &cols, "┘", "┴"));
            for col in &cols[1..] {
                lanes[*col] = None;
            }
        }
        let col = cols[0];
        let graph: String = lanes
            .iter()
            .enumerate()
            .map(|(at, lane)| if at == col { "● " } else { bar(lane) })
            .collect::<String>();
        let next = successors
            .iter()
            .find(|(number, _)| *number == item.number)
            .map(|(_, next)| next.clone())
            .unwrap_or_default();
        lanes[col] = next.first().copied();
        rows.push(GraphRow {
            number: Some(item.number),
            graph: graph.trim_end().to_string(),
            below: lanes.iter().map(bar).collect::<String>().trim_end().to_string(),
        });
        // More MRs come after it: a lane forks off for each of the others.
        if next.len() > 1 {
            let mut forks = vec![col];
            for number in &next[1..] {
                let free = lanes
                    .iter()
                    .enumerate()
                    .position(|(at, lane)| lane.is_none() && at > col);
                let at = free.unwrap_or(lanes.len());
                if at == lanes.len() {
                    lanes.push(None);
                }
                lanes[at] = Some(*number);
                forks.push(at);
            }
            forks.sort();
            rows.push(tie(&lanes, &forks, "┐", "┬"));
        }
        while lanes.last() == Some(&None) {
            lanes.pop();
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::Pipeline;

    struct Fixture {
        number: i64,
        mr: Mr,
        needs: Vec<i64>,
        files: BTreeSet<String>,
        boxes: Vec<IssueBox>,
        behind: u64,
        actions: Vec<Action>,
    }

    fn mr_of(number: i64, status: &str) -> Mr {
        Mr {
            iid: number + 100,
            state: "opened".into(),
            pipeline: Some(Pipeline {
                status: status.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn fixture(number: i64, mr: Mr) -> Fixture {
        Fixture {
            number,
            mr,
            needs: vec![],
            files: BTreeSet::new(),
            boxes: vec![],
            behind: 0,
            actions: vec![],
        }
    }
    fn running(number: i64) -> Fixture {
        fixture(number, mr_of(number, "running"))
    }
    fn green(number: i64) -> Fixture {
        fixture(number, mr_of(number, "success"))
    }
    fn failed(number: i64) -> Fixture {
        fixture(number, mr_of(number, "failed"))
    }
    fn draft(number: i64) -> Fixture {
        fixture(
            number,
            Mr {
                is_draft: true,
                ..mr_of(number, "running")
            },
        )
    }
    fn needs(mut fixture: Fixture, needs: &[i64]) -> Fixture {
        fixture.needs = needs.to_vec();
        fixture
    }
    fn files(mut fixture: Fixture, files: &[&str]) -> Fixture {
        fixture.files = files.iter().map(|file| file.to_string()).collect();
        fixture
    }
    fn state(mut fixture: Fixture, state: &str, merged_at: Option<&str>) -> Fixture {
        fixture.mr.state = state.into();
        fixture.mr.merged_at = merged_at.map(String::from);
        fixture
    }
    fn entries(fixtures: &[Fixture]) -> Vec<MergeEntry<'_>> {
        fixtures
            .iter()
            .map(|f| MergeEntry {
                number: f.number,
                mr: &f.mr,
                needs: &f.needs,
                files: &f.files,
                boxes: &f.boxes,
                behind: f.behind,
                actions: &f.actions,
                answered: &[],
            })
            .collect()
    }
    fn order(fixtures: &[Fixture], plan: &[i64]) -> MergeOrder {
        merge_order_of(&entries(fixtures), plan)
    }
    fn numbers(order: &MergeOrder) -> Vec<i64> {
        order.open.iter().map(|item| item.number).collect()
    }

    #[test]
    fn ranks_ready_and_green_before_coming_blocked_and_draft() {
        assert_eq!(progress_of(&green(1).mr), 0);
        assert_eq!(progress_of(&running(1).mr), 1);
        assert_eq!(progress_of(&failed(1).mr), 2);
        assert_eq!(
            progress_of(&Mr {
                has_conflicts: true,
                ..green(1).mr
            }),
            2
        );
        assert_eq!(
            progress_of(&Mr {
                is_draft: true,
                ..green(1).mr
            }),
            3
        );
    }

    #[test]
    fn keeps_the_plan_when_no_fact_speaks_against_it() {
        let found = order(&[running(1), running(2), running(3)], &[3, 1, 2]);
        assert_eq!(numbers(&found), [3, 1, 2]);
        assert_eq!(found.open[0].reason, "as planned");
        assert!(!found.is_off_plan);
    }

    #[test]
    fn orders_by_number_without_a_plan() {
        let found = order(&[running(2), running(1)], &[]);
        assert_eq!(numbers(&found), [1, 2]);
        assert_eq!(found.open[0].reason, "not in the plan: by number");
        assert!(found.is_off_plan);
    }

    #[test]
    fn puts_an_issue_after_the_one_it_needs() {
        let found = order(&[needs(running(1), &[2]), running(2)], &[1, 2]);
        assert_eq!(numbers(&found), [2, 1]);
        assert_eq!(found.open[1].reason, "after #2 (needs it)");
        assert!(found.is_off_plan);
    }

    #[test]
    fn ignores_a_need_that_is_merged_or_not_in_the_orchestration() {
        let found = order(&[needs(running(1), &[2, 9]), state(running(2), "merged", None)], &[1]);
        assert_eq!(numbers(&found), [1]);
    }

    #[test]
    fn falls_back_to_the_plan_when_the_needs_go_in_a_circle() {
        let found = order(&[needs(running(1), &[2]), needs(running(2), &[1])], &[1, 2]);
        assert_eq!(numbers(&found), [1, 2]);
    }

    #[test]
    fn merges_the_one_further_along_first_when_two_share_files() {
        let shared = ["packages/breadboard/src/overview/render-orchestration.mjs"];
        let found = order(&[files(draft(1), &shared), files(green(2), &shared)], &[1, 2]);
        assert_eq!(numbers(&found), [2, 1]);
        assert_eq!(found.open[1].reason, "goes after !102: shares render-orchestration.mjs");
    }

    #[test]
    fn a_shared_code_file_ranks_but_holds_nothing_up() {
        let found = order(&[files(draft(1), &["a.mjs"]), files(green(2), &["a.mjs"])], &[1, 2]);
        assert_eq!(numbers(&found), [2, 1]);
        let second = &found.open[1];
        assert!(second.after.iter().all(|pred| !pred.is_hard), "{:?}", second.after);
        assert!(
            second.hold_up.iter().all(|reason| !reason.starts_with("after")),
            "{:?}",
            second.hold_up
        );
        let found = order(&[files(green(1), &["a.mjs"]), files(green(2), &["a.mjs"])], &[1, 2]);
        assert!(
            found.open.iter().all(|item| item.hold_up.is_empty()),
            "{:?}",
            found.open
        );
    }

    #[test]
    fn files_that_are_docs_create_no_order_at_all() {
        let docs = [
            "packages/breadboard/README.md",
            ".claude/skills/parallel-work/orchestrate.md",
        ];
        let found = order(&[files(running(1), &docs), files(running(2), &docs)], &[1, 2]);
        // No reason: the plan stands, nobody comes after anybody.
        assert_eq!(numbers(&found), [1, 2]);
        assert!(found.open.iter().all(|item| item.after.is_empty()), "{:?}", found.open);
        assert_eq!(found.open[1].reason, "as planned");
    }

    #[test]
    fn a_need_is_a_hard_wait() {
        let found = order(&[needs(green(1), &[2]), green(2)], &[2, 1]);
        assert!(found.open[1].after.iter().all(|pred| pred.is_hard));
        assert_eq!(found.open[1].hold_up, ["after !102 (needs #2)"]);
    }

    #[test]
    fn keeps_the_plan_between_two_equally_far_along() {
        let found = order(
            &[
                files(running(1), &["a.mjs"]),
                files(running(2), &["a.mjs", "b.mjs", "c.mjs"]),
            ],
            &[2, 1],
        );
        assert_eq!(numbers(&found), [2, 1]);
        assert_eq!(found.open[1].reason, "goes after !102: shares a.mjs");
    }

    #[test]
    fn lets_a_need_win_over_file_overlap() {
        let found = order(
            &[needs(files(green(1), &["a.mjs"]), &[2]), files(draft(2), &["a.mjs"])],
            &[1, 2],
        );
        assert_eq!(numbers(&found), [2, 1]);
    }

    #[test]
    fn moves_a_ready_mr_up_past_blocked_ones() {
        let found = order(&[failed(1), draft(2), green(3), running(4)], &[1, 2, 3, 4]);
        assert_eq!(numbers(&found), [3, 1, 2, 4]);
        assert_eq!(found.open[0].reason, "ready while !101, !102 are blocked");
        assert!(found.is_off_plan);
        let found = order(&[running(1), failed(2), green(3)], &[1, 2, 3]);
        assert_eq!(numbers(&found), [1, 3, 2]);
        assert_eq!(found.open[0].reason, "as planned");
        let found = order(&[failed(1), needs(green(2), &[1])], &[1, 2]);
        assert_eq!(numbers(&found), [1, 2]);
    }

    #[test]
    fn lists_the_merged_in_the_order_they_were_merged_the_closed_after() {
        let found = order(
            &[
                state(running(1), "merged", Some("2026-09-29T10:00:00Z")),
                state(running(2), "merged", Some("2026-09-28T10:00:00Z")),
                state(running(3), "closed", None),
                running(4),
            ],
            &[1, 2, 4, 5],
        );
        assert_eq!(found.merged, [2, 1, 3]);
        assert_eq!(found.numbers, [4, 5, 2, 1, 3]);
        assert_eq!(found.planned, [2, 1, 4, 5]);
        assert!(!found.is_off_plan);
    }

    fn work(text: &str, is_ticked: bool) -> IssueBox {
        IssueBox {
            heading: Some("Work".into()),
            text: text.into(),
            is_ticked,
            line: 0,
        }
    }

    #[test]
    fn holds_up_nothing_when_ready_and_names_everything_else() {
        let mut done = green(1);
        done.boxes = vec![work("a", true)];
        assert!(hold_up_of(&entries(std::slice::from_ref(&done))[0], &[]).is_empty());
        let mut blocked = fixture(
            1,
            Mr {
                is_draft: true,
                has_conflicts: true,
                ..mr_of(1, "failed")
            },
        );
        blocked.boxes = vec![
            work("UP NEXT row with a click that writes", false),
            work("b", false),
            IssueBox {
                heading: Some("Acceptance criteria".into()),
                text: "c".into(),
                is_ticked: false,
                line: 0,
            },
        ];
        blocked.behind = 13;
        let after = [After {
            number: 2,
            iid: 215,
            why: "needs #2".into(),
            is_hard: true,
        }];
        assert_eq!(
            hold_up_of(&entries(std::slice::from_ref(&blocked))[0], &after),
            [
                "draft",
                "2 work boxes open: UP NEXT row with … +1",
                "1 acceptance box open: c",
                "pipeline failed",
                "conflicts with develop (13 behind)",
                "after !215 (needs #2)",
            ]
        );
        let mut trying = green(1);
        trying.actions = vec![Action {
            id: 1,
            kind: "try".into(),
            text: "the row".into(),
            ..Default::default()
        }];
        assert_eq!(
            hold_up_of(&entries(std::slice::from_ref(&trying))[0], &[]),
            ["try #1 open"]
        );
        let found = order(&[needs(green(1), &[2]), green(2)], &[2, 1]);
        assert_eq!(
            found.open.iter().map(|item| item.hold_up.clone()).collect::<Vec<_>>(),
            vec![vec![], vec!["after !102 (needs #2)".to_string()]]
        );
    }

    fn item(number: i64, after: &[i64]) -> OpenItem {
        OpenItem {
            number,
            reason: String::new(),
            after: after
                .iter()
                .map(|pred| After {
                    number: *pred,
                    iid: 0,
                    why: String::new(),
                    is_hard: true,
                })
                .collect(),
            hold_up: vec![],
        }
    }
    fn drawn(open: &[OpenItem]) -> Vec<String> {
        graph_of(open)
            .into_iter()
            .map(|row| match row.number {
                Some(number) => format!("{} {number} / {}", row.graph, row.below),
                None => row.graph,
            })
            .collect()
    }

    #[test]
    fn draws_independent_mrs_side_by_side_and_joins_them() {
        assert_eq!(
            drawn(&[item(1, &[]), item(2, &[]), item(3, &[1, 2]), item(4, &[3])]),
            ["● 1 / │", "│ ● 2 / │ │", "├─┘", "● 3 / │", "● 4 / "]
        );
        assert_eq!(
            drawn(&[item(1, &[]), item(2, &[1]), item(3, &[1]), item(4, &[2, 3])]),
            ["● 1 / │", "├─┐", "● │ 2 / │ │", "│ ● 3 / │ │", "├─┘", "● 4 / "]
        );
        assert_eq!(
            drawn(&[item(1, &[]), item(2, &[]), item(3, &[1])]),
            ["● 1 / │", "│ ● 2 / │", "● 3 / "]
        );
    }
}
