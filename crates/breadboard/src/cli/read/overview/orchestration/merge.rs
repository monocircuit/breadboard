//! What an open MR still lacks before its merge, and the merge order
//! (merge.mjs, merge-order.mjs, mr-blockers.mjs, resolve.mjs): the human's
//! checks (tries, acceptance boxes), GitLab's word, the MRs it comes after,
//! computed on each read from what the view knows; `wt:plan` is the record
//! and the tie-breaker.

use super::local::Tree;
use super::remote::{Job, Mr};
use crate::cli::read::issues::Box as IssueBox;
use crate::cli::read::overview::agents::Agents;
use crate::cli::read::overview::paint::{button, dim, yellow};
use std::collections::{BTreeSet, HashSet};

/// How many words of a box name it.
const BOX_WORDS: usize = 4;
/// How many open boxes a blocker names before `+n`.
const NAMED_BOXES: usize = 2;
/// Who acts on a reason; an MR it waits for is named by its `!N`.
pub const AGENT: &str = "agent";
pub const YOU: &str = "you";
pub const GITLAB: &str = "GitLab";
/// The pipeline states GitLab is still working on.
pub const RUNNING: [&str; 6] = [
    "created",
    "waiting_for_resource",
    "preparing",
    "pending",
    "running",
    "scheduled",
];
/// Between a reason's last word and who acts: a wrap never parts them.
const GLUE: char = '\u{a0}';

pub fn is_work(found: &IssueBox) -> bool {
    found.heading.as_deref().is_some_and(|heading| {
        heading.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("work"))
            && !heading[4..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

fn is_acceptance(found: &IssueBox) -> bool {
    found
        .heading
        .as_deref()
        .is_some_and(|heading| heading.to_lowercase().contains("acceptance"))
}

/// A box only the deployment can check: `… (after deploy, try #3)`.
fn is_after_deploy(found: &IssueBox) -> bool {
    let text = found.text.to_lowercase();
    text.match_indices("after deploy")
        .any(|(at, _)| at == 0 || !text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// The first words of a box, without its markdown.
pub fn words_of(found: &IssueBox) -> String {
    let plain: String = found.text.chars().filter(|c| !matches!(c, '`' | '*' | '_')).collect();
    let words: Vec<&str> = plain.split_whitespace().collect();
    let more = if words.len() > BOX_WORDS { " …" } else { "" };
    format!(
        "{}{more}",
        words.iter().take(BOX_WORDS).copied().collect::<Vec<_>>().join(" ")
    )
}

/// The tries of a worktree that still hold its merge, oldest first: one not
/// answered `looks good` yet, and one answered `broken` no later try
/// answered `looks good`. A try on the test instance never holds it.
pub fn open_tries_of(tree: Option<&Tree>) -> Vec<(i64, bool)> {
    let Some(tree) = tree else { return Vec::new() };
    let is_try = |kind: &str, on: Option<&str>| (kind == "try" || kind == "browser-check") && on != Some("test");
    let open: Vec<_> = tree
        .actions
        .iter()
        .filter(|action| is_try(&action.kind, action.on.as_deref()))
        .collect();
    let answered: Vec<_> = tree
        .answered
        .iter()
        .filter(|action| is_try(&action.kind, action.on.as_deref()))
        .collect();
    let good: Vec<i64> = answered
        .iter()
        .chain(&open)
        .filter(|action| action.answer.as_deref() == Some("looks good"))
        .map(|action| action.id)
        .collect();
    let mut tries: Vec<(i64, bool)> = answered
        .iter()
        .filter(|action| action.answer.as_deref() == Some("broken") && !good.iter().any(|later| *later > action.id))
        .map(|action| (action.id, true))
        .chain(
            open.iter()
                .filter(|action| action.answer.as_deref() != Some("looks good"))
                .map(|action| (action.id, action.answer.as_deref() == Some("broken"))),
        )
        .collect();
    tries.sort_by_key(|(id, _)| *id);
    tries
}

/// What the human's checks still lack before a merge: the open tries
/// (`try #1 open`, `try #2 broken`), the acceptance boxes not ticked but
/// those after the deployment.
pub fn checks_of(boxes: &[IssueBox], tree: Option<&Tree>) -> Vec<String> {
    let open: Vec<&IssueBox> = boxes
        .iter()
        .filter(|found| is_acceptance(found) && !found.is_ticked && !is_after_deploy(found))
        .collect();
    let mut checks: Vec<String> = open_tries_of(tree)
        .into_iter()
        .map(|(id, is_broken)| format!("try #{id} {}", if is_broken { "broken" } else { "open" }))
        .collect();
    if !open.is_empty() {
        checks.push(format!(
            "{} acceptance box{} open: {}{}",
            open.len(),
            if open.len() == 1 { "" } else { "es" },
            open.iter()
                .take(NAMED_BOXES)
                .map(|found| words_of(found))
                .collect::<Vec<_>>()
                .join(", "),
            if open.len() > NAMED_BOXES {
                format!(" +{}", open.len() - NAMED_BOXES)
            } else {
                String::new()
            }
        ));
    }
    checks
}

/// GitLab's `detailed_merge_status` as a blocker in words; None for
/// `mergeable` and for what another blocker says already.
pub fn merge_status_text(status: Option<&str>, is_draft: bool, is_pipeline_blocked: bool) -> Option<String> {
    let status = status.filter(|status| *status != "mergeable")?;
    if ["checking", "unchecked", "preparing", "approvals_syncing"].contains(&status) {
        return Some("GitLab is checking".into());
    }
    if status == "draft_status" && is_draft {
        return None;
    }
    if ["ci_must_pass", "ci_still_running"].contains(&status) && is_pipeline_blocked {
        return None;
    }
    let words = match status {
        "conflict" => "conflicts with develop",
        "discussions_not_resolved" => "open discussions",
        "need_rebase" => "needs a rebase onto develop",
        "not_approved" => "needs an approval",
        "blocked_status" => "blocked by another MR",
        "not_open" => "not open",
        "ci_must_pass" => "pipeline must pass",
        "ci_still_running" => "pipeline still running",
        "draft_status" => "draft",
        other => return Some(format!("GitLab: {}", other.replace('_', " "))),
    };
    Some(words.into())
}

/// The boxes open in the Work section, as a blocker: `2 work boxes open: a, b`.
fn open_work(boxes: &[IssueBox]) -> Vec<&IssueBox> {
    boxes
        .iter()
        .filter(|found| is_work(found) && !found.is_ticked)
        .collect()
}

/// What keeps an MR from being merged by two clicks (merge.mjs `blockersOf`).
pub fn blockers_of(mr: &Mr, boxes: &[IssueBox], can_mark_ready: bool, tree: Option<&Tree>) -> Vec<String> {
    if !mr.is_open() {
        return vec![format!("it is {}", mr.state)];
    }
    let mut blockers = Vec::new();
    if mr.is_draft && !can_mark_ready {
        blockers.push("draft: its agent posts the merge".to_string());
    }
    let open = open_work(boxes);
    if !open.is_empty() {
        blockers.push(format!(
            "{} work box{} open: {}",
            open.len(),
            if open.len() == 1 { "" } else { "es" },
            open.iter().map(|found| words_of(found)).collect::<Vec<_>>().join(", ")
        ));
    }
    blockers.extend(checks_of(boxes, tree));
    let status = mr.status();
    let is_pipeline_blocked = status != Some("success");
    if is_pipeline_blocked {
        blockers.push(format!("pipeline {}", status.unwrap_or("not there yet")));
    }
    if mr.has_conflicts {
        blockers.push("conflicts with develop".into());
    } else if let Some(text) = merge_status_text(mr.merge_status.as_deref(), mr.is_draft, is_pipeline_blocked) {
        blockers.push(text);
    }
    blockers
}

/// How many checks wait for the deployment: open acceptance boxes marked
/// after deploy, open tries on the test instance.
fn to_check_of(boxes: &[IssueBox], tree: Option<&Tree>) -> usize {
    boxes
        .iter()
        .filter(|found| is_acceptance(found) && !found.is_ticked && is_after_deploy(found))
        .count()
        + tree.map_or(0, |tree| {
            tree.actions
                .iter()
                .filter(|action| {
                    action.is_try()
                        && action.on.as_deref() == Some("test")
                        && action.answer.as_deref() != Some("looks good")
                })
                .count()
        })
}

/// A merge action's text: what blocks it, or what its two clicks do (merge.mjs `mergeText`).
pub fn merge_text(mr: &Mr, text: &str, boxes: &[IssueBox], can_mark_ready: bool, tree: Option<&Tree>) -> String {
    let verb = if mr.is_draft { "mark ready and merge" } else { "merge" };
    let blockers = blockers_of(mr, boxes, can_mark_ready, tree);
    if !blockers.is_empty() {
        let draft = (mr.is_draft && can_mark_ready).then(|| "draft".to_string());
        let parts: Vec<String> = draft.into_iter().chain(blockers).collect();
        return dim(&format!("{text} · {}", parts.join(" · ")));
    }
    let to_check = to_check_of(boxes, tree);
    let after = if to_check > 0 {
        format!(" · {to_check} to check after merge")
    } else {
        String::new()
    };
    format!("{text} {}", dim(&format!("· two clicks {verb}{after}")))
}

/// Whether a change requested for a worktree still blocks its MR (change.mjs `isChangeOpen`).
pub fn is_change_open(tree: Option<&Tree>, mr: Option<&Mr>) -> bool {
    let Some(tree) = tree else { return false };
    let Some(change) = &tree.change else { return false };
    if tree.pushed.is_none() || tree.pushed == change.head || tree.unpushed > 0 {
        return true;
    }
    let Some(pipeline) = mr.and_then(|mr| mr.pipeline.as_ref()) else {
        return true;
    };
    if pipeline.status != "success" {
        return true;
    }
    pipeline.sha.is_some() && pipeline.sha != tree.pushed
}

/// An issue with an MR, as the merge order takes it.
#[derive(Debug, Clone)]
pub struct Entry<'a> {
    pub number: i64,
    pub mr: &'a Mr,
    pub needs: Vec<i64>,
    pub files: BTreeSet<String>,
    pub boxes: &'a [IssueBox],
    pub behind: u64,
    pub tree: Option<&'a Tree>,
}

/// An MR it comes directly after, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Pred {
    pub number: i64,
    pub iid: i64,
    pub why: String,
}

/// An open MR placed in the merge order.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub number: i64,
    pub reason: String,
    pub after: Vec<Pred>,
    pub hold_up: Vec<String>,
}

/// The merge order (merge-order.mjs `mergeOrderOf`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Order {
    pub open: Vec<Placed>,
    /// The merged in the order they were merged, then the closed.
    pub merged: Vec<i64>,
    /// The order to write into the plan.
    pub planned: Vec<i64>,
    pub is_off_plan: bool,
}

/// How far along an open MR is, lower is further: 0 ready and green, 1
/// ready with its pipeline to come, 2 a failed pipeline or conflicts, 3 a draft.
fn progress_of(mr: &Mr) -> u8 {
    if mr.is_draft {
        return 3;
    }
    let status = mr.status();
    if status == Some("failed") || mr.has_conflicts {
        return 2;
    }
    if status == Some("success") { 0 } else { 1 }
}

/// What holds an open MR up, each in a few words (merge-order.mjs `holdUpOf`).
pub fn hold_up_of(entry: &Entry, after: &[Pred]) -> Vec<String> {
    let blockers = blockers_of(entry.mr, entry.boxes, true, entry.tree)
        .into_iter()
        .map(|blocker| {
            if blocker == "conflicts with develop" && entry.behind > 0 {
                return format!("{blocker} ({} behind)", entry.behind);
            }
            let parts: Vec<&str> = blocker.split(", ").collect();
            let is_boxes = blocker.split_once(' ').is_some_and(|(count, rest)| {
                count.parse::<u64>().is_ok() && (rest.starts_with("work box") || rest.starts_with("acceptance box"))
            });
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
    if !after.is_empty() {
        let named: Vec<String> = after
            .iter()
            .map(|pred| format!("!{} ({})", pred.iid, pred.why))
            .collect();
        hold_up.push(format!("after {}", named.join(", ")));
    }
    hold_up
}

/// `render-orchestration.mjs +2`: the base name of the first file, how many more.
fn files_text(files: &[&String]) -> String {
    let first = files
        .first()
        .map(|file| file.rsplit('/').next().unwrap_or(file).to_string())
        .unwrap_or_default();
    if files.len() > 1 {
        format!("{first} +{}", files.len() - 1)
    } else {
        first
    }
}

/// A predecessor: its number, the reason it goes first, the edge's phrase.
type Edge = (i64, String, String);
/// Each open MR's issue with its predecessors.
type Graph = Vec<(i64, Vec<Edge>)>;

/// The merge order of `entries` with `plan` (wt:plan's merge order) as the
/// tie-breaker: prerequisites first, of two MRs sharing files the one
/// further along, a ready one past the blocked ones ahead of it.
pub fn merge_order_of(entries: &[Entry], plan: &[i64]) -> Order {
    let place = |number: i64| plan.iter().position(|each| *each == number).unwrap_or(usize::MAX);
    let mut base: Vec<&Entry> = entries.iter().filter(|entry| entry.mr.is_open()).collect();
    base.sort_by_key(|entry| (place(entry.number), entry.number));
    let mut merged: Vec<&Entry> = entries.iter().filter(|entry| entry.mr.is_merged()).collect();
    merged.sort_by(|a, b| {
        a.mr.merged_at
            .clone()
            .unwrap_or_default()
            .cmp(&b.mr.merged_at.clone().unwrap_or_default())
            .then(a.number.cmp(&b.number))
    });
    let mut closed: Vec<&Entry> = entries
        .iter()
        .filter(|entry| !entry.mr.is_open() && !entry.mr.is_merged())
        .collect();
    closed.sort_by_key(|entry| (place(entry.number), entry.number));

    // number → its predecessors: (number, reason, why).
    let mut before: Graph = base.iter().map(|entry| (entry.number, Vec::new())).collect();
    fn preds_of(before: &[(i64, Vec<Edge>)], number: i64) -> &[Edge] {
        before
            .iter()
            .find(|(each, _)| *each == number)
            .map(|(_, preds)| preds.as_slice())
            .unwrap_or_default()
    }
    fn reaches(before: &[(i64, Vec<Edge>)], from: i64, to: i64, seen: &mut HashSet<i64>) -> bool {
        if from == to {
            return true;
        }
        if !seen.insert(from) {
            return false;
        }
        let nexts: Vec<i64> = before
            .iter()
            .filter(|(_, preds)| preds.iter().any(|pred| pred.0 == from))
            .map(|(next, _)| *next)
            .collect();
        nexts.into_iter().any(|next| reaches(before, next, to, seen))
    }
    let has = |before: &[(i64, Vec<Edge>)], number: i64| before.iter().any(|(each, _)| *each == number);
    for entry in &base {
        for need in &entry.needs {
            if has(&before, *need)
                && *need != entry.number
                && let Some((_, preds)) = before.iter_mut().find(|(each, _)| *each == entry.number)
            {
                preds.push((*need, format!("after #{need} (needs it)"), format!("needs #{need}")));
            }
        }
    }
    for (at, a) in base.iter().enumerate() {
        for b in &base[at + 1..] {
            let shared: Vec<&String> = a.files.intersection(&b.files).collect();
            if shared.is_empty() {
                continue;
            }
            let (first, second) = if progress_of(b.mr) < progress_of(a.mr) {
                (b, a)
            } else {
                (a, b)
            };
            if !reaches(&before, second.number, first.number, &mut HashSet::new()) {
                let text = files_text(&shared);
                if let Some((_, preds)) = before.iter_mut().find(|(each, _)| *each == second.number) {
                    preds.push((
                        first.number,
                        format!("after !{}: shares {text}", first.mr.iid),
                        format!("shares {text}"),
                    ));
                }
            }
        }
    }

    let mut open: Vec<Placed> = Vec::new();
    let mut placed: HashSet<i64> = HashSet::new();
    let mut remaining: Vec<&Entry> = base.clone();
    while !remaining.is_empty() {
        let available: Vec<&Entry> = remaining
            .iter()
            .copied()
            .filter(|entry| {
                preds_of(&before, entry.number)
                    .iter()
                    .all(|pred| placed.contains(&pred.0))
            })
            .collect();
        let (pick, mut reason) = if available.is_empty() {
            (
                remaining[0],
                Some("as planned: its prerequisites go in a circle".to_string()),
            )
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
                    (
                        available[ready_at],
                        Some(format!(
                            "ready while {} {} blocked",
                            blocked.join(", "),
                            if ready_at > 1 { "are" } else { "is" }
                        )),
                    )
                }
                _ => (available[0], None),
            }
        };
        let preds = preds_of(&before, pick.number).to_vec();
        if reason.is_none() {
            let last = open
                .iter()
                .rev()
                .find(|item| preds.iter().any(|pred| pred.0 == item.number));
            reason = Some(match last {
                Some(last) => preds
                    .iter()
                    .find(|pred| pred.0 == last.number)
                    .map(|pred| pred.1.clone())
                    .unwrap_or_default(),
                None if place(pick.number) == usize::MAX => "not in the plan: by number".into(),
                None => "as planned".into(),
            });
        }
        // The ones it comes directly after: not those another of them comes after.
        let after: Vec<Pred> = preds
            .iter()
            .enumerate()
            .filter(|(at, pred)| {
                preds.iter().position(|other| other.0 == pred.0) == Some(*at)
                    && !preds
                        .iter()
                        .any(|other| other.0 != pred.0 && reaches(&before, pred.0, other.0, &mut HashSet::new()))
            })
            .map(|(_, pred)| Pred {
                number: pred.0,
                iid: base
                    .iter()
                    .find(|entry| entry.number == pred.0)
                    .map_or(0, |entry| entry.mr.iid),
                why: pred.2.clone(),
            })
            .collect();
        open.push(Placed {
            number: pick.number,
            reason: reason.unwrap_or_default(),
            hold_up: hold_up_of(pick, &after),
            after,
        });
        placed.insert(pick.number);
        remaining.retain(|entry| entry.number != pick.number);
    }

    let open_numbers: Vec<i64> = open.iter().map(|item| item.number).collect();
    let listed: HashSet<i64> = entries.iter().map(|entry| entry.number).collect();
    let plan_rest: Vec<i64> = plan.iter().copied().filter(|number| !listed.contains(number)).collect();
    let plan_open: Vec<i64> = plan.iter().copied().filter(|number| has(&before, *number)).collect();
    Order {
        merged: merged.iter().chain(&closed).map(|entry| entry.number).collect(),
        planned: merged
            .iter()
            .map(|entry| entry.number)
            .chain(open_numbers.iter().copied())
            .chain(plan_rest)
            .collect(),
        is_off_plan: !open_numbers.is_empty() && open_numbers != plan_open,
        open,
    }
}

/// A row of the merge graph: an MR's (`number`) with its `●` in its lane,
/// `below` the lanes on its further lines; or a line that joins or forks lanes.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphRow {
    pub number: Option<i64>,
    pub graph: String,
    pub below: String,
}

/// The open MRs as a graph like `git log --graph` (merge-order.mjs `graphOf`):
/// independent ones in lanes side by side, one that comes after others below
/// them, their lanes joined into its own; a lane is two characters wide.
pub fn graph_of(open: &[Placed]) -> Vec<GraphRow> {
    let successors = |number: i64| -> Vec<i64> {
        open.iter()
            .filter(|item| item.after.iter().any(|pred| pred.number == number))
            .map(|item| item.number)
            .collect()
    };
    let mut lanes: Vec<Option<i64>> = Vec::new();
    let mut rows = Vec::new();
    let bar = |lane: &Option<i64>| if lane.is_none() { "  " } else { "│ " };
    let tie = |lanes: &[Option<i64>], cols: &[usize], corner: &str, middle: &str| {
        let (first, last) = (cols[0], cols[cols.len() - 1]);
        let mut graph = String::new();
        for (col, lane) in lanes.iter().enumerate() {
            if col == first {
                graph += "├─";
            } else if col == last {
                graph += &format!("{corner} ");
            } else if cols.contains(&col) {
                graph += &format!("{middle}─");
            } else if col > first && col < last {
                graph += if lane.is_none() { "──" } else { "┼─" };
            } else {
                graph += bar(lane);
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
            .collect();
        let next = successors(item.number);
        lanes[col] = next.first().copied();
        rows.push(GraphRow {
            number: Some(item.number),
            graph: graph.trim_end().to_string(),
            below: lanes.iter().map(bar).collect::<String>().trim_end().to_string(),
        });
        if next.len() > 1 {
            let mut forks = vec![col];
            for number in &next[1..] {
                let at = lanes
                    .iter()
                    .enumerate()
                    .position(|(at, lane)| lane.is_none() && at > col)
                    .unwrap_or(lanes.len());
                if at == lanes.len() {
                    lanes.push(None);
                }
                lanes[at] = Some(*number);
                forks.push(at);
            }
            forks.sort_unstable();
            rows.push(tie(&lanes, &forks, "┐", "┬"));
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
    }
    rows
}

/// A reason an open MR can't be merged yet, and who acts on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reason {
    pub text: String,
    pub who: String,
    pub is_conflict: bool,
}

/// GitLab's `merge_status` while it works the conflicts out again; its `detailed_merge_status` while it does.
fn is_conflict_settled(mr: &Mr, tree: Option<&Tree>) -> bool {
    let is_rechecking = matches!(
        mr.merge_check.as_deref(),
        Some("unchecked" | "checking" | "cannot_be_merged_recheck")
    );
    let is_checking = matches!(mr.merge_status.as_deref(), Some("unchecked" | "checking"));
    let is_behind = tree
        .and_then(|tree| tree.pushed.as_ref())
        .is_some_and(|pushed| mr.sha.as_ref().is_some_and(|sha| sha != pushed));
    !is_rechecking && !is_checking && !is_behind
}

/// The reasons an open MR can't be merged yet, in the order they have to be
/// done (mr-blockers.mjs `rowBlockersOf`); none when it is ready.
pub fn row_blockers_of(
    mr: &Mr,
    boxes: &[IssueBox],
    behind: u64,
    after: &[Pred],
    jobs: &[Job],
    tree: Option<&Tree>,
    done: &HashSet<i64>,
) -> Vec<Reason> {
    let mut reasons = Vec::new();
    let mut add = |text: String, who: &str, is_conflict: bool| {
        reasons.push(Reason {
            text,
            who: who.to_string(),
            is_conflict,
        })
    };
    let status = mr.status();
    match status {
        None => add("no pipeline yet".into(), GITLAB, false),
        Some(status) if RUNNING.contains(&status) => add(format!("pipeline {status}"), GITLAB, false),
        Some("failed") => {
            let job = jobs.iter().find(|job| job.status == "failed");
            let at = job.map(|job| format!(" at {}", job.name)).unwrap_or_default();
            add(format!("pipeline failed{at}"), AGENT, false);
        }
        Some("success") => {}
        Some(status) => add(format!("pipeline {status}"), AGENT, false),
    }
    let open = open_work(boxes);
    if !open.is_empty() {
        let named: Vec<String> = open.iter().take(NAMED_BOXES).map(|found| words_of(found)).collect();
        let more = if open.len() > NAMED_BOXES {
            format!(" +{}", open.len() - NAMED_BOXES)
        } else {
            String::new()
        };
        add(
            format!(
                "{} Work box{} open: {}{more}",
                open.len(),
                if open.len() == 1 { "" } else { "es" },
                named.join(", ")
            ),
            AGENT,
            false,
        );
    }
    let checks = checks_of(boxes, tree);
    for check in &checks {
        add(
            check.clone(),
            if check.starts_with("try ") { YOU } else { AGENT },
            false,
        );
    }
    if open.is_empty() && checks.is_empty() && mr.is_draft {
        let is_posted = tree.is_some_and(|tree| tree.actions.iter().any(|action| action.kind == "merge"));
        add(
            "draft: mark ready".into(),
            if is_posted || status == Some("success") {
                YOU
            } else {
                AGENT
            },
            false,
        );
    }
    if !is_conflict_settled(mr, tree) {
        add("GitLab checks for conflicts".into(), GITLAB, true);
    } else if mr.has_conflicts {
        let behind = if behind > 0 {
            format!(" ({behind} behind)")
        } else {
            String::new()
        };
        add(format!("conflicts with develop{behind}: merge develop in"), AGENT, true);
    } else if let Some(text) = merge_status_text(mr.merge_status.as_deref(), mr.is_draft, status != Some("success")) {
        let who = if text == "GitLab is checking" { GITLAB } else { YOU };
        add(text, who, false);
    }
    for pred in after {
        add(
            format!("after !{} ({})", pred.iid, pred.why),
            &format!("!{}", pred.iid),
            false,
        );
    }
    let named: Vec<i64> = after.iter().map(|pred| pred.iid).collect();
    for iid in tree.and_then(|tree| tree.wait.as_ref()).into_iter().flatten() {
        if !named.contains(iid) && !done.contains(iid) {
            add(format!("waits for !{iid}"), &format!("!{iid}"), false);
        }
    }
    if is_change_open(tree, Some(mr))
        && let Some(change) = tree.and_then(|tree| tree.change.as_ref())
    {
        add(format!("change requested: {}", change.text), AGENT, false);
    }
    if let Some(unpushed) = tree.map(|tree| tree.unpushed).filter(|count| *count > 0) {
        add(
            format!("{unpushed} commit{} not pushed", if unpushed == 1 { "" } else { "s" }),
            AGENT,
            false,
        );
    }
    reasons
}

/// The reasons as the row's text below its title: `pipeline running → GitLab · …`.
pub fn blockers_text(reasons: &[Reason]) -> String {
    reasons
        .iter()
        .filter(|reason| !reason.is_conflict)
        .map(|reason| {
            format!(
                "{}{GLUE}{}",
                yellow(&reason.text),
                dim(&format!("→{GLUE}{}", reason.who))
            )
        })
        .collect::<Vec<_>>()
        .join(&dim(" · "))
}

/// The conflict state of an open MR as the start of its second line
/// (resolve.mjs `conflictStateOf`, without the clicks of a live view): GitLab
/// still checking, none, or the button that has its agent resolve them,
/// grey with why when a click couldn't reach it.
pub fn conflict_text(mr: &Mr, tree: Option<&Tree>, behind: u64, agents: &Agents) -> String {
    if !is_conflict_settled(mr, tree) {
        return dim("checking for conflicts…");
    }
    if !mr.has_conflicts {
        return dim("no merge conflicts");
    }
    let label = "resolve merge conflicts".replace(' ', "\u{a0}");
    let grey = |why: &str| format!("{} {}", button(&label, "gray"), dim(why));
    if let Some(tree) = tree
        && tree.unpushed > 0
    {
        return grey(&format!(
            "conflicts as of the pushed commits · {} not pushed: its agent pushes first",
            tree.unpushed
        ));
    }
    if let Some(why) = no_agent_of(tree, agents) {
        return grey(why);
    }
    let behind = if behind > 0 {
        format!(" {}", yellow(&format!("{behind} behind")))
    } else {
        String::new()
    };
    format!("{}{behind}", button(&label, "yellow"))
}

/// Why a click can't reach the agent of `tree`, None when it can (resolve.mjs `noAgentOf`).
fn no_agent_of(tree: Option<&Tree>, agents: &Agents) -> Option<&'static str> {
    let Some(tree) = tree else {
        return Some("no worktree, so no agent: merge develop in yourself");
    };
    let Some(claude) = &tree.claude else {
        return Some("no agent in its worktree");
    };
    if agents.is_empty() {
        return None;
    }
    match agents.get(&claude.id) {
        None => Some("its agent isn't running"),
        Some(agent) if agent.status == "blocked" => Some("its agent is at a question: answer it first"),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::remote::Pipeline;
    use super::*;

    fn mr(iid: i64, status: Option<&str>, is_draft: bool) -> Mr {
        Mr {
            iid,
            state: "opened".into(),
            is_draft,
            merge_status: Some("mergeable".into()),
            merge_check: Some("can_be_merged".into()),
            pipeline: status.map(|status| Pipeline {
                status: status.into(),
                ..Pipeline::default()
            }),
            ..Mr::default()
        }
    }

    fn entry<'a>(number: i64, mr: &'a Mr, files: &[&str]) -> Entry<'a> {
        Entry {
            number,
            mr,
            needs: Vec::new(),
            files: files.iter().map(|file| file.to_string()).collect(),
            boxes: &[],
            behind: 0,
            tree: None,
        }
    }

    fn boxed(text: &str, heading: &str, is_ticked: bool) -> IssueBox {
        IssueBox {
            is_ticked,
            text: text.into(),
            heading: Some(heading.into()),
            ..IssueBox::default()
        }
    }

    #[test]
    fn checks_name_open_tries_and_acceptance_boxes() {
        let boxes = vec![
            boxed("The user can run `pnpm x` and see", "Acceptance criteria", false),
            boxed("The decision record says why", "Acceptance criteria", false),
            boxed("works after deploy on test", "Acceptance criteria", false),
            boxed("done", "Acceptance criteria", true),
        ];
        assert_eq!(
            checks_of(&boxes, None),
            vec!["2 acceptance boxes open: The user can run …, The decision record says …"]
        );
    }

    #[test]
    fn a_draft_with_a_green_pipeline_waits_for_you() {
        let draft = mr(308, Some("success"), true);
        let reasons = row_blockers_of(&draft, &[], 0, &[], &[], None, &HashSet::new());
        assert_eq!(
            reasons
                .iter()
                .map(|reason| (reason.text.as_str(), reason.who.as_str()))
                .collect::<Vec<_>>(),
            vec![("draft: mark ready", "you")]
        );
        let pending = mr(311, Some("pending"), true);
        let reasons = row_blockers_of(&pending, &[], 0, &[], &[], None, &HashSet::new());
        assert_eq!(
            blockers_text(&reasons),
            "pipeline pending\u{a0}→\u{a0}GitLab · draft: mark ready\u{a0}→\u{a0}agent"
        );
    }

    #[test]
    fn the_merge_order_puts_a_ready_one_past_blocked_ones_and_shared_files_after() {
        let (a, b, c) = (
            mr(1, Some("failed"), false),
            mr(2, Some("success"), false),
            mr(3, Some("success"), true),
        );
        let entries = vec![
            entry(10, &a, &[]),
            entry(20, &b, &["x/view.mjs"]),
            entry(30, &c, &["x/view.mjs"]),
        ];
        let order = merge_order_of(&entries, &[10, 20, 30]);
        let numbers: Vec<i64> = order.open.iter().map(|item| item.number).collect();
        assert_eq!(numbers, vec![20, 10, 30]);
        assert_eq!(order.open[0].reason, "ready while !1 is blocked");
        assert_eq!(
            order.open[2].after,
            vec![Pred {
                number: 20,
                iid: 2,
                why: "shares view.mjs".into()
            }]
        );
        assert_eq!(order.open[2].hold_up, vec!["draft", "after !2 (shares view.mjs)"]);
        assert!(order.is_off_plan);
        let graph = graph_of(&order.open);
        let lines: Vec<&str> = graph.iter().map(|row| row.graph.as_str()).collect();
        assert_eq!(lines, vec!["●", "│ ●", "●"]);
    }

    #[test]
    fn a_merge_text_says_what_blocks_it_or_what_two_clicks_do() {
        let ready = mr(5, Some("success"), false);
        assert_eq!(merge_text(&ready, "go", &[], false, None), "go · two clicks merge");
        let draft = mr(6, Some("running"), true);
        assert_eq!(
            merge_text(&draft, "go", &[], true, None),
            "go · draft · pipeline running"
        );
    }
}
