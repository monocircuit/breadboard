//! What an open MR still lacks before its merge (#242, mr-blockers.mjs): one
//! reason per item in the order they have to be done, each with who acts on
//! it. Every reason once: the draft behind open boxes, a `wt:wait` behind the
//! merge order's `after` say nothing new.

use super::Tree;
use super::agent_state::RUNNING;
use super::change::is_change_open;
use super::merge::{
    GITLAB_RECHECKS, counted_checks_of, failed_at_head, is_acceptance, is_same_commit, is_work, merge_status_text,
};
use super::merge_order::After;
use super::resolve::is_conflict_settled;
use crate::check::CheckState;
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
/// known merged or closed: a wait for one of them is over. Without
/// `needs_pipeline` (#443) wt:check passed at its pushed head stands for
/// the pipeline.
#[allow(clippy::too_many_arguments)]
pub fn row_blockers_of(
    mr: &Mr,
    boxes: &[IssueBox],
    behind: u64,
    after: &[After],
    jobs: &[Job],
    tree: Option<&Tree>,
    done: &HashSet<i64>,
    needs_pipeline: bool,
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
    let failed = || {
        let job = jobs.iter().find(|job| job.status == "failed");
        format!(
            "pipeline failed{}",
            job.map(|job| format!(" at {}", job.name)).unwrap_or_default()
        )
    };
    match status {
        _ if !needs_pipeline => {
            if failed_at_head(mr) {
                add(failed(), "agent", false);
            }
            if let Some(text) = check_lack_of(mr, tree) {
                add(text, "agent", false);
            }
        }
        None => add("no pipeline yet".into(), "GitLab", false),
        Some(status) if RUNNING.contains(&status) => add(format!("pipeline {status}"), "GitLab", false),
        Some("failed") => add(failed(), "agent", false),
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
    // Its review (#374) once its head is green: serious findings are its agent's.
    let review = tree
        .filter(|_| is_green(mr, tree, needs_pipeline))
        .and_then(|tree| review_hold_of(tree, mr));
    let is_reviewed_back = review.as_ref().is_some_and(|hold| hold.who == "agent");
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
        } else if bar.total > 0 && !is_changed && !is_reviewed_back {
            // The bar is full: breadboard marks it ready by itself (#399).
            add(READY_BAR_FULL.into(), "breadboard", false);
        } else if bar.total == 0 {
            // No boxes: the merge action marks it ready, the agent's or the overview's own once the pipeline passed.
            let is_posted = actions.iter().any(|action| action.kind == "merge");
            add(
                MARK_READY.into(),
                if is_posted || status == Some("success") || !needs_pipeline {
                    "you"
                } else {
                    "agent"
                },
                false,
            );
        }
    }

    // GitLab's `conflict` is one even where `has_conflicts` lags (#416): its agent's, never yours.
    let has_conflicts = mr.has_conflicts || mr.merge_status.as_deref() == Some("conflict");
    if !is_conflict_settled(mr, tree) {
        add("GitLab checks for conflicts".into(), "GitLab", true);
    } else if has_conflicts {
        let files = tree.and_then(|tree| tree.conflicts.as_deref()).unwrap_or(&[]);
        let detail: Vec<String> = [
            (!files.is_empty()).then(|| files_text(files)),
            (behind > 0).then(|| format!("{behind} behind")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(" ({})", detail.join(" · "))
        };
        add(
            format!(
                "conflicts with {target}{detail}: merge {target} in",
                target = mr.target()
            ),
            "agent",
            true,
        );
    } else if let Some(text) = merge_status_text(
        mr.merge_status.as_deref(),
        mr.is_draft,
        if needs_pipeline {
            status != Some("success")
        } else {
            failed_at_head(mr)
        },
        needs_pipeline,
        mr.target(),
    ) {
        // A rebase is its agent's too; what is left (discussions, an approval) is yours.
        let who = if text == GITLAB_RECHECKS {
            "GitLab"
        } else if text.starts_with("needs a rebase") {
            "agent"
        } else {
            "you"
        };
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
    // Due or running, it holds only an MR nothing else holds: the actor starts it on `review due`.
    if let Some(hold) = review.filter(|_| is_reviewed_back || reasons.is_empty()) {
        reasons.push(Reason {
            text: hold.text,
            who: hold.who.into(),
            is_conflict: false,
        });
    }
    reasons
}

/// What the review of `mr`'s head holds (#374), by the `[review]` of its worktree.
pub fn review_hold_of(tree: &Tree, mr: &Mr) -> Option<crate::review::Hold> {
    let is_enabled = crate::config::of(&tree.dir).config.review.enabled;
    crate::review::hold_of(&tree.reviews, mr.sha.as_deref(), is_enabled)
}

/// Whether its head passed what a merge needs of it (#374): its pipeline,
/// or without `needs_pipeline` (#443) wt:check at its pushed head with no
/// pipeline failed there. Its review starts only then.
pub fn is_green(mr: &Mr, tree: Option<&Tree>, needs_pipeline: bool) -> bool {
    if needs_pipeline {
        mr.pipeline_status() == Some("success")
    } else {
        !failed_at_head(mr) && check_lack_of(mr, tree).is_none()
    }
}

/// What its wt:check lacks for a merge without a pipeline (#443): passed at
/// the commit GitLab has as its head, in its worktree here. None once it did.
fn check_lack_of(mr: &Mr, tree: Option<&Tree>) -> Option<String> {
    let Some(tree) = tree else {
        return Some("wt:check not known (no worktree here)".into());
    };
    match &tree.checked {
        CheckState::Tested { sha, .. } if mr.sha.as_deref().is_some_and(|head| is_same_commit(sha, head)) => None,
        CheckState::Tested { .. } => Some("wt:check not at the pushed head".into()),
        CheckState::Checking { .. } => Some("wt:check running".into()),
        CheckState::Failed { .. } => Some("wt:check failed".into()),
        CheckState::Untested { .. } | CheckState::None => Some("wt:check not passed".into()),
    }
}

/// How many conflicting files a reason names before `+n`.
const NAMED_FILES: usize = 2;

/// `README.md, view.rs +2`: the files a merge of develop stops at, by their names.
pub fn files_text(files: &[String]) -> String {
    let names: Vec<&str> = files
        .iter()
        .take(NAMED_FILES)
        .map(|file| file.rsplit('/').next().unwrap_or(file))
        .collect();
    let more = if files.len() > NAMED_FILES {
        format!(" +{}", files.len() - NAMED_FILES)
    } else {
        String::new()
    };
    format!("{}{more}", names.join(", "))
}

/// `conflicts with develop`: a conflict reason as the title row of its
/// worktree says it, without its files and what to do (#416).
pub fn short_of(reason: &Reason) -> String {
    let text = reason.text.as_str();
    if !reason.is_conflict {
        return text.to_string();
    }
    let end = [" (", ": "]
        .iter()
        .filter_map(|cut| text.find(cut))
        .min()
        .unwrap_or(text.len());
    text[..end].to_string()
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
        row_blockers_of(mr, boxes, 0, &[], &[], tree, &HashSet::new(), true)
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
            true,
        );
        assert_eq!(found[0].text, "pipeline failed at check:test");
        assert_eq!(found[0].who, "agent");
    }

    #[test]
    fn without_its_own_pipeline_wt_check_at_the_pushed_head_stands_for_it() {
        // #443: `[merge] needs_pipeline = false`, every push's pipeline skipped.
        let skipped = Mr {
            sha: Some("abc123".into()),
            pipeline: Some(Pipeline {
                status: "skipped".into(),
                sha: Some("abc123".into()),
                ..Default::default()
            }),
            ..mr()
        };
        let checked = |state: CheckState| Tree {
            checked: state,
            ..Default::default()
        };
        let tested = |sha: &str| {
            checked(CheckState::Tested {
                sha: sha.into(),
                packages: Vec::new(),
            })
        };
        let off = |mr: &Mr, tree: Option<&Tree>| -> Vec<String> {
            row_blockers_of(mr, &[work("Tests", true)], 0, &[], &[], tree, &HashSet::new(), false)
                .into_iter()
                .map(|reason| format!("{} → {}", reason.text, reason.who))
                .collect()
        };
        assert!(off(&skipped, Some(&tested("abc1234567"))).is_empty());
        assert!(
            off(
                &Mr {
                    pipeline: None,
                    ..skipped.clone()
                },
                Some(&tested("abc123"))
            )
            .is_empty()
        );
        assert_eq!(
            off(&skipped, Some(&tested("def456"))),
            ["wt:check not at the pushed head → agent"]
        );
        assert_eq!(
            off(&skipped, Some(&checked(CheckState::Checking { since: None }))),
            ["wt:check running → agent"]
        );
        assert_eq!(
            off(&skipped, Some(&checked(CheckState::Untested { count: 2 }))),
            ["wt:check not passed → agent"]
        );
        assert_eq!(off(&skipped, None), ["wt:check not known (no worktree here) → agent"]);
        // A pipeline a click ran that failed at its head still holds it.
        let failed = Mr {
            pipeline: Some(Pipeline {
                status: "failed".into(),
                sha: Some("abc123".into()),
                ..Default::default()
            }),
            ..skipped.clone()
        };
        assert_eq!(off(&failed, Some(&tested("abc123"))), ["pipeline failed → agent"]);
        // On, as before: a skipped pipeline is no pass, whatever wt:check says.
        assert_eq!(
            reasons(&skipped, &[work("Tests", true)], Some(&tested("abc123"))),
            ["pipeline skipped → agent"]
        );
        // GitLab holds it for its setting: the user's step.
        let held = Mr {
            merge_status: Some("ci_must_pass".into()),
            ..skipped
        };
        assert_eq!(
            off(&held, Some(&tested("abc123"))),
            [format!("{} → you", super::super::merge::PIPELINE_SETTING)]
        );
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
    fn serious_review_findings_send_it_back_to_its_agent() {
        use crate::review::{Finding, Review, State, Weight};
        let review = |weight| Review {
            iid: 242,
            head: "abc123".into(),
            model: "haiku".into(),
            state: State::Done,
            findings: vec![Finding {
                weight,
                text: "a.rs:1 x".into(),
            }],
            skipped: None,
            lines: 40,
            tokens: 0,
            dollars: 0.0,
            at: String::new(),
            pid: None,
            session: None,
            failed: None,
        };
        let reviewed = |weight| Tree {
            reviews: vec![review(weight)],
            ..Default::default()
        };
        let at = |mr: Mr, sha: &str| Mr {
            sha: Some(sha.into()),
            ..mr
        };
        let boxes = [work("Tests", true)];
        let serious = reviewed(Weight::Serious);
        assert_eq!(
            reasons(&at(mr(), "abc123"), &boxes, Some(&serious)),
            ["review: 1 serious finding → agent"]
        );
        // Back to draft by its hold: the review says why, not `ready (bar full)`.
        assert_eq!(
            reasons(&at(draft(), "abc123"), &boxes, Some(&serious)),
            ["review: 1 serious finding → agent"]
        );
        assert!(
            reasons(&at(mr(), "def456"), &boxes, Some(&serious)).is_empty(),
            "a new head, [review] off: nothing waits"
        );
        assert!(reasons(&at(mr(), "abc123"), &boxes, Some(&reviewed(Weight::Small))).is_empty());
    }

    #[test]
    fn its_head_is_green_by_the_pipeline_or_by_wt_check_without_one() {
        let at = Mr {
            sha: Some("abc123".into()),
            pipeline: None,
            ..mr()
        };
        let checked = |sha: &str| Tree {
            checked: crate::check::CheckState::Tested {
                sha: sha.into(),
                packages: vec![],
            },
            ..Default::default()
        };
        assert!(!is_green(&at, Some(&checked("abc123")), true), "no pipeline yet");
        assert!(is_green(&mr(), None, true), "the pipeline passed");
        // #443: a merge into develop on wt:check, its review starts on it.
        assert!(is_green(&at, Some(&checked("abc123")), false));
        assert!(
            !is_green(&at, Some(&checked("0ff1ce")), false),
            "checked another commit"
        );
        assert!(!is_green(&at, None, false), "no worktree here");
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
        let all: Vec<String> = row_blockers_of(&found, &[], 4, &after, &[], Some(&tree), &done, true)
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
    fn a_conflict_names_its_files_and_goes_to_the_agent() {
        // GitLab's `conflict` without `has_conflicts` (!381): still the agent's.
        let found = Mr {
            merge_status: Some("conflict".into()),
            ..mr()
        };
        assert_eq!(
            reasons(&found, &[], None),
            ["conflicts with develop: merge develop in → agent"]
        );
        let tree = Tree {
            conflicts: Some(vec!["README.md".into(), "src/a/view.rs".into(), "b.rs".into()]),
            ..Default::default()
        };
        let all = row_blockers_of(&found, &[], 3, &[], &[], Some(&tree), &HashSet::new(), true);
        assert_eq!(
            all[0].text,
            "conflicts with develop (README.md, view.rs +1 · 3 behind): merge develop in"
        );
        assert_eq!(short_of(&all[0]), "conflicts with develop");
        assert_eq!(
            reasons(
                &Mr {
                    merge_status: Some("need_rebase".into()),
                    ..mr()
                },
                &[],
                None
            ),
            ["needs a rebase onto develop → agent"]
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
