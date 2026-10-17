//! What an open MR still lacks before its merge (#242, mr-blockers.mjs): one
//! reason per item in the order they have to be done, each with who acts on
//! it. Every reason once: the draft behind open boxes, a `wt:wait` behind the
//! merge order's `after` say nothing new.

use super::Tree;
use super::agent_state::RUNNING;
use super::change::is_change_open;
use super::merge::{GITLAB_RECHECKS, counted_checks_of, is_acceptance, is_work, merge_status_text};
use super::merge_order::After;
use super::resolve::is_conflict_settled;
use crate::gitlab::{IssueBox, Job, Mr};
use std::collections::HashSet;

/// The reason of a draft without boxes: the merge action marks it ready.
pub const MARK_READY: &str = "draft: mark ready";
/// The reason of a draft whose bar is full (#399): breadboard marks it ready.
pub const READY_BAR_FULL: &str = "ready (bar full)";
/// The reason of such a draft a person set back after breadboard marked it ready.
pub const DRAFT_BY_HAND: &str = "draft by hand: ready after the next push";

/// The reasons that are the draft alone: the merge's own step, no lack.
pub fn is_draft_reason(text: &str) -> bool {
    matches!(text, MARK_READY | READY_BAR_FULL | DRAFT_BY_HAND)
}

/// A reason an open MR can't be merged yet: what, who acts on it (`agent`,
/// `you`, `GitLab`, or the `!N` it waits for), and whether it is about
/// conflicts (the row's conflict cell says those).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub text: String,
    pub who: String,
    pub is_conflict: bool,
}

/// rowBlockersOf: the reasons, in order: pipeline, open Work boxes, the
/// human's checks, the draft, conflicts, the MRs it waits for, a requested
/// change, unpushed commits. None when it is ready to merge. `done` the iids
/// known merged or closed: a wait for one of them is over.
pub fn row_blockers_of(
    mr: &Mr,
    boxes: &[IssueBox],
    behind: u64,
    after: &[After],
    jobs: &[Job],
    tree: Option<&Tree>,
    done: &HashSet<i64>,
) -> Vec<Reason> {
    let mut reasons = Vec::new();
    let mut add = |text: String, who: &str, is_conflict: bool| {
        reasons.push(Reason {
            text,
            who: who.into(),
            is_conflict,
        })
    };

    let status = mr.pipeline_status();
    match status {
        None => add("no pipeline yet".into(), "GitLab", false),
        Some(status) if RUNNING.contains(&status) => add(format!("pipeline {status}"), "GitLab", false),
        Some("failed") => {
            let job = jobs.iter().find(|job| job.status == "failed");
            add(
                format!(
                    "pipeline failed{}",
                    job.map(|job| format!(" at {}", job.name)).unwrap_or_default()
                ),
                "agent",
                false,
            );
        }
        Some("success") => {}
        Some(status) => add(format!("pipeline {status}"), "agent", false),
    }

    let open: Vec<&IssueBox> = boxes
        .iter()
        .filter(|found| is_work(found) && !found.is_ticked)
        .collect();
    // Counted only: the fold under the row's bar lists them whole (#346).
    if !open.is_empty() {
        add(
            format!(
                "{} Work box{} open",
                open.len(),
                if open.len() == 1 { "" } else { "es" }
            ),
            "agent",
            false,
        );
    }
    // A try waits for you, an acceptance box for its agent to tick it (#280).
    let (actions, answered) = tree
        .map(|tree| (&tree.actions[..], &tree.answered[..]))
        .unwrap_or((&[], &[]));
    let checks = counted_checks_of(boxes, actions, answered);
    for check in &checks {
        add(
            check.clone(),
            if check.starts_with("try ") { "you" } else { "agent" },
            false,
        );
    }
    if open.is_empty() && checks.is_empty() && mr.is_draft {
        let bar = progress_of(boxes).unwrap_or_default();
        let is_changed = tree.is_some_and(|tree| is_change_open(Some(tree), Some(mr)));
        if bar.total > 0 && bar.ticked < bar.total {
            // Boxes of neither Work nor acceptance (`## Scope`): the bar isn't full.
            let left = bar.total - bar.ticked;
            add(
                format!("{left} box{} open", if left == 1 { "" } else { "es" }),
                "agent",
                false,
            );
        } else if bar.total > 0 && !is_changed {
            // The bar is full: breadboard marks it ready by itself (#399).
            add(READY_BAR_FULL.into(), "breadboard", false);
        } else if bar.total == 0 {
            // No boxes: the merge action marks it ready, the agent's or the overview's own once the pipeline passed.
            let is_posted = actions.iter().any(|action| action.kind == "merge");
            add(
                MARK_READY.into(),
                if is_posted || status == Some("success") {
                    "you"
                } else {
                    "agent"
                },
                false,
            );
        }
    }

    if !is_conflict_settled(mr, tree) {
        add("GitLab checks for conflicts".into(), "GitLab", true);
    } else if mr.has_conflicts {
        let behind = if behind > 0 {
            format!(" ({behind} behind)")
        } else {
            String::new()
        };
        add(
            format!(
                "conflicts with {target}{behind}: merge {target} in",
                target = mr.target()
            ),
            "agent",
            true,
        );
    } else if let Some(text) = merge_status_text(
        mr.merge_status.as_deref(),
        mr.is_draft,
        status != Some("success"),
        mr.target(),
    ) {
        let who = if text == GITLAB_RECHECKS { "GitLab" } else { "you" };
        add(text, who, false);
    }

    // Only a prerequisite holds the merge: shared files just rank (#392).
    for pred in after.iter().filter(|pred| pred.is_hard) {
        add(
            format!("after !{} ({})", pred.iid, pred.why),
            &format!("!{}", pred.iid),
            false,
        );
    }
    let named: HashSet<i64> = after.iter().filter(|pred| pred.is_hard).map(|pred| pred.iid).collect();
    for iid in tree
        .and_then(|tree| tree.wait.as_ref())
        .map(|wait| wait.mrs.clone())
        .unwrap_or_default()
    {
        if !named.contains(&iid) && !done.contains(&iid) {
            add(format!("waits for !{iid}"), &format!("!{iid}"), false);
        }
    }
    if let Some(tree) = tree {
        if let Some(change) = tree.change.as_ref().filter(|_| is_change_open(Some(tree), Some(mr))) {
            add(format!("change requested: {}", change.text), "agent", false);
        }
        if tree.unpushed > 0 {
            add(
                format!("{} not pushed", super::counted(tree.unpushed, "commit")),
                "agent",
                false,
            );
        }
    }
    reasons
}

/// How far an MR is by its issue's boxes (#294, mr-blockers.mjs `progressOf`):
/// ticked of all, and `(ticked, all)` of the Work and the acceptance boxes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    pub ticked: usize,
    pub total: usize,
    pub work: (usize, usize),
    pub acceptance: (usize, usize),
    /// The boxes under neither (an issue's `## Scope`): no reason names them,
    /// they hold no merge up, but `total` counts them.
    pub other: (usize, usize),
    /// Their section's name when they share one (`scope`), else `other`.
    pub other_name: String,
}

/// The longest section name the bar's counts take; a longer one is `other`.
const SECTION_NAME: usize = 16;

/// None for an issue without boxes: its row shows no bar.
pub fn progress_of(boxes: &[IssueBox]) -> Option<Progress> {
    if boxes.is_empty() {
        return None;
    }
    let count = |is: fn(&IssueBox) -> bool| {
        let all: Vec<&IssueBox> = boxes.iter().filter(|issue_box| is(issue_box)).collect();
        (all.iter().filter(|issue_box| issue_box.is_ticked).count(), all.len())
    };
    let is_other = |issue_box: &IssueBox| !is_work(issue_box) && !is_acceptance(issue_box);
    let (ticked, total) = count(|_| true);
    let mut headings = boxes
        .iter()
        .filter(|issue_box| is_other(issue_box))
        .map(|issue_box| issue_box.heading.as_deref().map(str::trim).unwrap_or(""));
    let first = headings.next().unwrap_or("");
    let is_shared = !first.is_empty() && first.chars().count() <= SECTION_NAME && headings.all(|name| name == first);
    Some(Progress {
        ticked,
        total,
        work: count(is_work),
        acceptance: count(is_acceptance),
        other: count(is_other),
        other_name: if is_shared {
            first.to_lowercase()
        } else {
            "other".into()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Action, Change};
    use crate::gitlab::Pipeline;
    use crate::wait::Wait;

    fn mr() -> Mr {
        Mr {
            iid: 242,
            state: "opened".into(),
            pipeline: Some(Pipeline {
                status: "success".into(),
                sha: Some("b".into()),
                ..Default::default()
            }),
            merge_status: Some("mergeable".into()),
            ..Default::default()
        }
    }
    fn with_pipeline(status: Option<&str>) -> Mr {
        Mr {
            pipeline: status.map(|status| Pipeline {
                status: status.into(),
                ..Default::default()
            }),
            ..mr()
        }
    }
    fn work(text: &str, is_ticked: bool) -> IssueBox {
        IssueBox {
            heading: Some("Work".into()),
            text: text.into(),
            is_ticked,
            line: 0,
        }
    }
    fn acceptance(text: &str) -> IssueBox {
        IssueBox {
            heading: Some("Acceptance criteria".into()),
            text: text.into(),
            is_ticked: false,
            line: 0,
        }
    }
    fn reasons(mr: &Mr, boxes: &[IssueBox], tree: Option<&Tree>) -> Vec<String> {
        row_blockers_of(mr, boxes, 0, &[], &[], tree, &HashSet::new())
            .into_iter()
            .map(|reason| format!("{} → {}", reason.text, reason.who))
            .collect()
    }
    fn draft() -> Mr {
        Mr {
            is_draft: true,
            merge_status: Some("draft_status".into()),
            ..mr()
        }
    }

    #[test]
    fn has_none_for_a_ready_green_mr() {
        assert!(reasons(&mr(), &[work("Tests", true)], None).is_empty());
    }

    #[test]
    fn says_who_acts_on_the_pipeline() {
        assert_eq!(
            reasons(&with_pipeline(Some("running")), &[], None),
            ["pipeline running → GitLab"]
        );
        assert_eq!(reasons(&with_pipeline(None), &[], None), ["no pipeline yet → GitLab"]);
        let jobs = [
            Job {
                name: "check:lint".into(),
                status: "success".into(),
                ..Default::default()
            },
            Job {
                name: "check:test".into(),
                status: "failed".into(),
                ..Default::default()
            },
        ];
        let found = row_blockers_of(
            &with_pipeline(Some("failed")),
            &[],
            0,
            &[],
            &jobs,
            None,
            &HashSet::new(),
        );
        assert_eq!(found[0].text, "pipeline failed at check:test");
        assert_eq!(found[0].who, "agent");
    }

    #[test]
    fn counts_the_open_work_boxes_and_says_the_draft_once() {
        let boxes = [
            work("Row layout: first line as now", true),
            work("The blockers in order", false),
            work("Say who acts", false),
            work("Tests", false),
            acceptance("Checked"),
        ];
        assert_eq!(
            reasons(&draft(), &boxes, None),
            ["3 Work boxes open → agent", "1 acceptance box open → agent",]
        );
    }

    #[test]
    fn names_open_tries_for_you_the_draft_after_them() {
        let tree = Tree {
            actions: vec![
                Action {
                    id: 1,
                    kind: "try".into(),
                    text: "the row".into(),
                    ..Default::default()
                },
                Action {
                    id: 2,
                    kind: "try".into(),
                    text: "on test".into(),
                    on: Some("test".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let boxes = [
            work("Tests", true),
            acceptance("No merge while a try is open"),
            acceptance("Looks good unblocks"),
            acceptance("On the test instance (after deploy, try #2)"),
        ];
        assert_eq!(
            reasons(&draft(), &boxes, Some(&tree)),
            ["try #1 open → you", "2 acceptance boxes open → agent",]
        );
        assert_eq!(
            reasons(&draft(), &[work("Tests", true)], Some(&Tree::default())),
            ["ready (bar full) → breadboard"]
        );
    }

    #[test]
    fn leaves_a_full_bar_to_breadboard_and_names_boxes_of_neither_kind() {
        // #399: every box ticked, breadboard marks it ready, whatever the pipeline.
        let boxes = [work("Tests", true)];
        assert_eq!(reasons(&draft(), &boxes, None), ["ready (bar full) → breadboard"]);
        let scope = |is_ticked| IssueBox {
            heading: Some("Scope".into()),
            text: "Marked ready by the daemon".into(),
            is_ticked,
            line: 0,
        };
        assert_eq!(
            reasons(&draft(), &[scope(false), scope(true), scope(false)], None),
            ["2 boxes open → agent"]
        );
        assert_eq!(
            reasons(&draft(), &[scope(true)], None),
            ["ready (bar full) → breadboard"]
        );
        // A change requested holds the draft: the change says it, no ready.
        let changed = Tree {
            change: Some(crate::actions::Change {
                text: "rename it".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            reasons(&draft(), &boxes, Some(&changed)),
            ["change requested: rename it → agent"]
        );
    }

    #[test]
    fn gives_a_green_draft_without_boxes_to_the_user() {
        let boxes = [];
        assert_eq!(reasons(&draft(), &boxes, None), ["draft: mark ready → you"]);
        let running = Mr {
            pipeline: Some(Pipeline {
                status: "running".into(),
                ..Default::default()
            }),
            ..draft()
        };
        assert_eq!(
            reasons(&running, &boxes, None),
            ["pipeline running → GitLab", "draft: mark ready → agent"]
        );
        let posted = Tree {
            actions: vec![Action {
                kind: "merge".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            reasons(&running, &boxes, Some(&posted)),
            ["pipeline running → GitLab", "draft: mark ready → you"]
        );
    }

    #[test]
    fn puts_conflicts_waits_a_change_and_unpushed_commits_in_order() {
        let found = Mr {
            has_conflicts: true,
            merge_status: Some("conflict".into()),
            ..mr()
        };
        let tree = Tree {
            wait: Some(Wait {
                mrs: vec![253, 249, 248],
                ..Default::default()
            }),
            change: Some(Change {
                text: "shorter title".into(),
                head: Some("a".into()),
                ..Default::default()
            }),
            pushed: Some("a".into()),
            unpushed: 2,
            ..Default::default()
        };
        // GitLab's head is the pushed one: its conflicts count.
        let found = Mr {
            sha: Some("a".into()),
            ..found
        };
        let after = [After {
            number: 1,
            iid: 253,
            why: "needs #1".into(),
            is_hard: true,
        }];
        let done: HashSet<i64> = [248].into();
        let all: Vec<String> = row_blockers_of(&found, &[], 4, &after, &[], Some(&tree), &done)
            .into_iter()
            .map(|reason| format!("{} → {}", reason.text, reason.who))
            .collect();
        assert_eq!(
            all,
            [
                "conflicts with develop (4 behind): merge develop in → agent",
                "after !253 (needs #1) → !253",
                "waits for !249 → !249",
                "change requested: shorter title → agent",
                "2 commits not pushed → agent",
            ]
        );
    }

    #[test]
    fn names_what_else_gitlab_holds_it_for() {
        let with = |status: &str| Mr {
            merge_status: Some(status.into()),
            ..mr()
        };
        assert_eq!(
            reasons(&with("discussions_not_resolved"), &[], None),
            ["open discussions → you"]
        );
        assert_eq!(
            reasons(&with("checking"), &[], None),
            ["GitLab checks for conflicts → GitLab"]
        );
    }

    #[test]
    fn progress_counts_the_boxes_by_section() {
        assert_eq!(progress_of(&[]), None);
        let boxes = [work("a", true), work("b", false), acceptance("c")];
        assert_eq!(
            progress_of(&boxes),
            Some(Progress {
                ticked: 1,
                total: 3,
                work: (1, 2),
                acceptance: (0, 1),
                other: (0, 0),
                other_name: "other".into(),
            })
        );
    }

    #[test]
    fn progress_names_the_section_of_the_other_boxes() {
        let under = |heading: Option<&str>, is_ticked: bool| IssueBox {
            heading: heading.map(Into::into),
            text: "x".into(),
            is_ticked,
            line: 0,
        };
        let scope = [under(Some("Scope"), true), under(Some("Scope"), false), acceptance("c")];
        let found = progress_of(&scope).unwrap();
        assert_eq!((found.other, found.other_name.as_str()), ((1, 2), "scope"));
        // Two sections, or boxes under no heading: `other`.
        let mixed = [under(Some("Scope"), false), under(Some("Later"), false)];
        assert_eq!(progress_of(&mixed).unwrap().other_name, "other");
        assert_eq!(progress_of(&[under(None, false)]).unwrap().other_name, "other");
    }
}
