//! What a merge still lacks (merge.mjs, #182, #280): a merge is offered only
//! when the work is done: the pipeline passed, every box of the issue's Work
//! section and of its acceptance criteria is ticked, and every `try` of its
//! worktree was answered `looks good`. A `try --on test` and an acceptance
//! box marked `(after deploy …)` stay open across the merge.

use crate::actions::Action;
use crate::gitlab::{IssueBox, Mr};
use regex::Regex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

/// How many words of a box name it in a row.
const BOX_WORDS: usize = 4;
/// How many open acceptance boxes a blocker names before `+n`.
const NAMED_BOXES: usize = 2;

/// What a row says while GitLab hasn't settled its mergeability; the read asks it to recheck (#391).
pub const GITLAB_RECHECKS: &str = "GitLab rechecks mergeability";

/// The pipeline mark of an MR that merges without a pipeline of its own (#443).
pub const ON_WT_CHECK: &str = "merges on wt:check (no MR pipeline)";
/// What GitLab needs changed while it holds such an MR for a pipeline (#443): the user's step.
pub const PIPELINE_SETTING: &str = "GitLab wants a pipeline: Settings → Merge requests → Merge checks, turn on \"Skipped pipelines are considered successful\"";

/// Which MRs merge on `wt:check` at their pushed head, without a pipeline of
/// their own: into the default branch while `[merge] needs_pipeline = false`
/// (#443, temporary), a task's into its batch while `[merge]
/// batch_needs_pipeline = false` (#461). A batch's own MR never does.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OnCheck {
    pub into_default: bool,
    pub into_batch: bool,
}

impl OnCheck {
    pub fn of(merge: &crate::config::Merge) -> OnCheck {
        OnCheck {
            into_default: !merge.needs_pipeline,
            into_batch: !merge.batch_needs_pipeline,
        }
    }

    /// Of the repo at `main`.
    pub fn of_main(main: &std::path::Path) -> OnCheck {
        OnCheck::of(&crate::config::of_main(main).config.merge)
    }

    /// Whether an MR from `source` into `target` merges on `wt:check`.
    pub fn covers(&self, source: &str, target: &str) -> bool {
        if crate::batch::is_batch(source) {
            return false;
        }
        (self.into_default && target == crate::git::default_branch())
            || (self.into_batch && crate::batch::is_batch(target))
    }
}

/// Whether `mr` needs a passed pipeline of its own before its merge: always,
/// but where `on_check` covers it: there `wt:check` at its pushed head is enough.
pub fn needs_pipeline(mr: &Mr, on_check: OnCheck) -> bool {
    !on_check.covers(&mr.branch, mr.target())
}

/// Whether the commits `a` and `b` are one, either maybe abbreviated.
pub fn is_same_commit(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

/// What a pipeline of `mr` holds when it needs none (#443): one that failed
/// at its head (a click ran it) says the head is broken; any other, or
/// none, holds nothing.
pub fn failed_at_head(mr: &Mr) -> bool {
    mr.pipeline.as_ref().is_some_and(|pipeline| {
        pipeline.status == "failed"
            && pipeline
                .sha
                .as_deref()
                .zip(mr.sha.as_deref())
                .is_some_and(|(ran, head)| is_same_commit(ran, head))
    })
}

/// GitLab's `detailed_merge_status` in words, for what a blocker doesn't say
/// already (the draft, the pipeline); None for `mergeable`.
pub fn merge_status_text(
    status: Option<&str>,
    is_draft: bool,
    is_pipeline_blocked: bool,
    needs_pipeline: bool,
    target: &str,
) -> Option<String> {
    let status = status.filter(|status| !status.is_empty() && *status != "mergeable")?;
    if ["checking", "unchecked", "preparing", "approvals_syncing"].contains(&status) {
        return Some(GITLAB_RECHECKS.into());
    }
    // It needs none, GitLab wants one: its setting is what to change (#443).
    if status == "ci_must_pass" && !needs_pipeline {
        return Some(PIPELINE_SETTING.into());
    }
    if status == "draft_status" && is_draft {
        return None;
    }
    if (status == "ci_must_pass" || status == "ci_still_running") && is_pipeline_blocked {
        return None;
    }
    Some(
        match status {
            "conflict" => return Some(format!("conflicts with {target}")),
            "need_rebase" => return Some(format!("needs a rebase onto {target}")),
            "discussions_not_resolved" => "open discussions",
            "not_approved" => "needs an approval",
            "blocked_status" => "blocked by another MR",
            "not_open" => "not open",
            "ci_must_pass" => "pipeline must pass",
            "ci_still_running" => "pipeline still running",
            "draft_status" => "draft",
            other => return Some(format!("GitLab: {}", other.replace('_', " "))),
        }
        .to_string(),
    )
}

static WORK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^work\b").unwrap());
static ACCEPTANCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)acceptance").unwrap());
static AFTER_DEPLOY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bafter deploy").unwrap());

/// A box of the issue's Work section.
pub fn is_work(issue_box: &IssueBox) -> bool {
    WORK.is_match(issue_box.heading.as_deref().unwrap_or(""))
}
/// A box of its acceptance criteria.
pub fn is_acceptance(issue_box: &IssueBox) -> bool {
    ACCEPTANCE.is_match(issue_box.heading.as_deref().unwrap_or(""))
}
/// A box only the deployment can check: `… (after deploy, try #3)`.
pub fn is_after_deploy(issue_box: &IssueBox) -> bool {
    AFTER_DEPLOY.is_match(&issue_box.text)
}

/// The first words of a box, without its markdown.
pub fn words_of(issue_box: &IssueBox) -> String {
    let text: String = issue_box
        .text
        .chars()
        .filter(|c| !matches!(c, '`' | '*' | '_'))
        .collect();
    let words: Vec<&str> = text.split_whitespace().collect();
    let shown = words.iter().take(BOX_WORDS).copied().collect::<Vec<_>>().join(" ");
    if words.len() > BOX_WORDS {
        format!("{shown} …")
    } else {
        shown
    }
}

/// The tries of a worktree that still hold its merge, oldest first: one not
/// answered `looks good` yet, and one answered `broken` that no later try
/// answered `looks good`. A try on the test instance never holds it.
/// `actions` open, `answered` delivered, as its actions file holds them.
pub fn open_tries_of<'a>(actions: &'a [Action], answered: &'a [Action]) -> Vec<&'a Action> {
    let is_try = |action: &&Action| action.is_try() && action.on.as_deref() != Some("test");
    let open: Vec<&Action> = actions.iter().filter(is_try).collect();
    let done: Vec<&Action> = answered.iter().filter(is_try).collect();
    let good: Vec<i64> = done
        .iter()
        .chain(open.iter())
        .filter(|action| action.answer.as_deref() == Some("looks good"))
        .map(|action| action.id)
        .collect();
    let mut tries: Vec<&Action> = done
        .into_iter()
        .filter(|action| action.answer.as_deref() == Some("broken") && !good.iter().any(|later| *later > action.id))
        .chain(
            open.into_iter()
                .filter(|action| action.answer.as_deref() != Some("looks good")),
        )
        .collect();
    tries.sort_by_key(|action| action.id);
    tries
}

/// `try #1 open`, `1 acceptance box open: Two clicks merge a …`: what the
/// human's checks still lack before a merge; none when they are all done.
pub fn checks_of(boxes: &[IssueBox], actions: &[Action], answered: &[Action]) -> Vec<String> {
    checks(boxes, actions, answered, true)
}

/// `checks_of` with the open acceptance boxes counted, not named: `2
/// acceptance boxes open` (an MR row's, whose fold lists them, #346).
pub fn counted_checks_of(boxes: &[IssueBox], actions: &[Action], answered: &[Action]) -> Vec<String> {
    checks(boxes, actions, answered, false)
}

fn checks(boxes: &[IssueBox], actions: &[Action], answered: &[Action], is_named: bool) -> Vec<String> {
    let open: Vec<&IssueBox> = boxes
        .iter()
        .filter(|found| is_acceptance(found) && !found.is_ticked && !is_after_deploy(found))
        .collect();
    let mut checks: Vec<String> = open_tries_of(actions, answered)
        .iter()
        .map(|action| {
            format!(
                "try #{} {}",
                action.id,
                if action.answer.as_deref() == Some("broken") {
                    "broken"
                } else {
                    "open"
                }
            )
        })
        .collect();
    if !open.is_empty() {
        let counted = format!(
            "{} acceptance box{} open",
            open.len(),
            if open.len() == 1 { "" } else { "es" }
        );
        if !is_named {
            checks.push(counted);
            return checks;
        }
        let named = open
            .iter()
            .take(NAMED_BOXES)
            .map(|found| words_of(found))
            .collect::<Vec<_>>()
            .join(", ");
        let more = if open.len() > NAMED_BOXES {
            format!(" +{}", open.len() - NAMED_BOXES)
        } else {
            String::new()
        };
        checks.push(format!("{counted}: {named}{more}"));
    }
    checks
}

/// What keeps the MR from being merged, each in a few words; none when two
/// clicks merge it. `can_mark_ready`: its agent posted the merge, so a draft
/// is marked ready first. `needs_pipeline`: its own pipeline must pass; else
/// (#443) only one that failed at its head holds it, its wt:check is the caller's.
pub fn blockers_of(
    mr: Option<&Mr>,
    boxes: &[IssueBox],
    can_mark_ready: bool,
    actions: &[Action],
    answered: &[Action],
    needs_pipeline: bool,
) -> Vec<String> {
    let Some(mr) = mr else {
        return vec!["no merge request".into()];
    };
    if !mr.state.is_empty() && mr.state != "opened" {
        return vec![format!("it is {}", mr.state)];
    }
    let mut blockers = Vec::new();
    if mr.is_draft && !can_mark_ready {
        blockers.push("draft: its agent posts the merge".to_string());
    }
    let open: Vec<&IssueBox> = boxes
        .iter()
        .filter(|found| is_work(found) && !found.is_ticked)
        .collect();
    if !open.is_empty() {
        blockers.push(format!(
            "{} work box{} open: {}",
            open.len(),
            if open.len() == 1 { "" } else { "es" },
            open.iter().map(|found| words_of(found)).collect::<Vec<_>>().join(", ")
        ));
    }
    blockers.extend(checks_of(boxes, actions, answered));
    let status = mr.pipeline_status();
    let is_pipeline_blocked = if needs_pipeline {
        status != Some("success")
    } else {
        failed_at_head(mr)
    };
    if is_pipeline_blocked {
        blockers.push(format!("pipeline {}", status.unwrap_or("not there yet")));
    }
    if mr.has_conflicts {
        blockers.push(format!("conflicts with {}", mr.target()));
    } else if let Some(text) = merge_status_text(
        mr.merge_status.as_deref(),
        mr.is_draft,
        is_pipeline_blocked,
        needs_pipeline,
        mr.target(),
    ) {
        blockers.push(text);
    }
    blockers
}

/// How many checks wait for the deployment: the open acceptance boxes marked
/// after deploy, the open tries on the test instance.
pub fn to_check_of(boxes: &[IssueBox], actions: &[Action]) -> usize {
    boxes
        .iter()
        .filter(|found| is_acceptance(found) && !found.is_ticked && is_after_deploy(found))
        .count()
        + actions
            .iter()
            .filter(|action| {
                action.is_try()
                    && action.on.as_deref() == Some("test")
                    && action.answer.as_deref() != Some("looks good")
            })
            .count()
}

/// The files of `mine` the merged MR touched too, sorted.
pub fn shared_files(merged: &[String], mine: &BTreeSet<String>) -> Vec<String> {
    let set: BTreeSet<&String> = merged.iter().filter(|file| mine.contains(*file)).collect();
    set.into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::Pipeline;

    fn ready() -> Mr {
        Mr {
            iid: 7,
            state: "opened".into(),
            url: "https://gitlab.example/mr/7".into(),
            pipeline: Some(Pipeline {
                status: "success".into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn issue_box(heading: &str, text: &str, is_ticked: bool) -> IssueBox {
        IssueBox {
            heading: Some(heading.into()),
            text: text.into(),
            is_ticked,
            line: 0,
        }
    }
    fn boxes() -> Vec<IssueBox> {
        vec![
            issue_box("Work", "`merge` button: the first click arms it", true),
            issue_box("Work", "After the merge: the agent gets told", false),
            issue_box("Acceptance criteria", "Two clicks merge a ready MR", false),
        ]
    }
    fn try_of(id: i64, answer: Option<&str>, on: Option<&str>) -> Action {
        Action {
            id,
            kind: "try".into(),
            text: format!("t{id}"),
            answer: answer.map(String::from),
            on: on.map(String::from),
            ..Default::default()
        }
    }
    fn blockers(
        mr: &Mr,
        boxes: &[IssueBox],
        can_mark_ready: bool,
        actions: &[Action],
        answered: &[Action],
    ) -> Vec<String> {
        blockers_of(Some(mr), boxes, can_mark_ready, actions, answered, true)
    }

    #[test]
    fn names_what_blocks_a_merge() {
        assert!(blockers(&ready(), &[], false, &[], &[]).is_empty());
        assert_eq!(
            blockers(
                &Mr {
                    is_draft: true,
                    ..ready()
                },
                &[],
                false,
                &[],
                &[]
            ),
            ["draft: its agent posts the merge"]
        );
        let running = Mr {
            pipeline: Some(Pipeline {
                status: "running".into(),
                ..Default::default()
            }),
            ..ready()
        };
        assert_eq!(blockers(&running, &[], false, &[], &[]), ["pipeline running"]);
        assert_eq!(
            blockers(
                &Mr {
                    pipeline: None,
                    ..ready()
                },
                &[],
                false,
                &[],
                &[]
            ),
            ["pipeline not there yet"]
        );
        assert_eq!(
            blockers(
                &Mr {
                    state: "merged".into(),
                    ..ready()
                },
                &[],
                false,
                &[],
                &[]
            ),
            ["it is merged"]
        );
        assert_eq!(blockers_of(None, &[], false, &[], &[], true), ["no merge request"]);
    }

    #[test]
    fn marks_a_posted_draft_ready_once_every_box_is_ticked() {
        let draft = Mr {
            is_draft: true,
            ..ready()
        };
        let ticked: Vec<IssueBox> = boxes()
            .into_iter()
            .map(|found| IssueBox {
                is_ticked: true,
                ..found
            })
            .collect();
        assert!(blockers(&draft, &ticked, true, &[], &[]).is_empty());
        let running = Mr {
            pipeline: Some(Pipeline {
                status: "running".into(),
                ..Default::default()
            }),
            ..draft
        };
        assert_eq!(
            blockers(&running, &boxes(), true, &[], &[]),
            [
                "1 work box open: After the merge: the …",
                "1 acceptance box open: Two clicks merge a …",
                "pipeline running",
            ]
        );
        assert_eq!(
            blockers(&ready(), &boxes()[2..], false, &[], &[]),
            ["1 acceptance box open: Two clicks merge a …"]
        );
    }

    #[test]
    fn an_open_try_blocks_and_looks_good_unblocks() {
        assert_eq!(
            blockers(&ready(), &[], false, &[try_of(1, None, None)], &[]),
            ["try #1 open"]
        );
        assert!(blockers(&ready(), &[], false, &[try_of(1, Some("looks good"), None)], &[]).is_empty());
        assert!(blockers(&ready(), &[], false, &[], &[try_of(1, Some("looks good"), None)]).is_empty());
        let check = Action {
            kind: "browser-check".into(),
            ..try_of(2, None, None)
        };
        assert_eq!(
            open_tries_of(&[check], &[]).iter().map(|a| a.id).collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn a_broken_blocks_until_a_later_try_looks_good() {
        let broken = try_of(1, Some("broken"), None);
        assert_eq!(
            blockers(&ready(), &[], false, std::slice::from_ref(&broken), &[]),
            ["try #1 broken"]
        );
        assert_eq!(
            blockers(&ready(), &[], false, &[], std::slice::from_ref(&broken)),
            ["try #1 broken"]
        );
        assert_eq!(
            blockers(
                &ready(),
                &[],
                false,
                &[try_of(2, None, None)],
                std::slice::from_ref(&broken)
            ),
            ["try #1 broken", "try #2 open"]
        );
        assert!(
            blockers(
                &ready(),
                &[],
                false,
                &[],
                &[broken, try_of(2, Some("looks good"), None)]
            )
            .is_empty()
        );
    }

    #[test]
    fn tries_on_test_and_boxes_after_deploy_count_after_the_merge() {
        let actions = [try_of(1, None, Some("test"))];
        let after = issue_box("Acceptance criteria", "It shows on test (after deploy, try #1)", false);
        assert!(checks_of(std::slice::from_ref(&after), &actions, &[]).is_empty());
        assert!(blockers(&ready(), std::slice::from_ref(&after), false, &actions, &[]).is_empty());
        assert_eq!(to_check_of(&[after], &actions), 2);
        assert_eq!(to_check_of(&boxes(), &[]), 0);
    }

    #[test]
    fn names_what_gitlab_refuses() {
        let fresh = Mr {
            merge_status: Some("mergeable".into()),
            ..ready()
        };
        assert!(blockers(&fresh, &[], false, &[], &[]).is_empty());
        let with = |status: &str, has_conflicts: bool| {
            blockers(
                &Mr {
                    merge_status: Some(status.into()),
                    has_conflicts,
                    ..fresh.clone()
                },
                &[],
                false,
                &[],
                &[],
            )
        };
        assert_eq!(with("checking", false), [GITLAB_RECHECKS]);
        assert_eq!(with("conflict", false), ["conflicts with develop"]);
        assert_eq!(with("discussions_not_resolved", false), ["open discussions"]);
        assert_eq!(with("conflict", true), ["conflicts with develop"]);
        assert_eq!(with("not_approved", false), ["needs an approval"]);
    }

    #[test]
    fn without_its_own_pipeline_only_a_failed_one_at_its_head_holds_it() {
        // #443: `[merge] needs_pipeline = false`, a push's skipped pipeline.
        let with = |status: &str, ran: &str| Mr {
            sha: Some("abc123".into()),
            pipeline: Some(Pipeline {
                status: status.into(),
                sha: Some(ran.into()),
                ..Default::default()
            }),
            ..ready()
        };
        let off = |mr: &Mr| blockers_of(Some(mr), &[], false, &[], &[], false);
        assert!(off(&with("skipped", "abc123")).is_empty());
        assert!(
            off(&Mr {
                pipeline: None,
                ..ready()
            })
            .is_empty()
        );
        assert!(off(&with("running", "abc123")).is_empty());
        assert!(off(&with("failed", "0ld")).is_empty(), "failed on an older commit");
        assert_eq!(off(&with("failed", "abc")), ["pipeline failed"]);
        // On, as before: a skipped one is no pass.
        assert_eq!(
            blockers_of(Some(&with("skipped", "abc123")), &[], false, &[], &[], true),
            ["pipeline skipped"]
        );
        // GitLab holds it for its setting: the row names what to change.
        let held = Mr {
            merge_status: Some("ci_must_pass".into()),
            ..with("skipped", "abc123")
        };
        assert_eq!(off(&held), [PIPELINE_SETTING]);
        let into_default = OnCheck {
            into_default: true,
            into_batch: false,
        };
        assert!(needs_pipeline(
            &Mr {
                target: "epic/15-x".into(),
                ..ready()
            },
            into_default
        ));
        assert!(!needs_pipeline(&ready(), into_default));
        assert!(needs_pipeline(&ready(), OnCheck::default()));
    }

    #[test]
    fn a_task_into_its_batch_merges_on_wt_check_and_the_batch_never() {
        // #461: `[merge] batch_needs_pipeline = false`.
        let into_batch = OnCheck {
            into_default: false,
            into_batch: true,
        };
        let task = Mr {
            branch: "feature/461-x".into(),
            target: "batch/breadboard-2".into(),
            ..ready()
        };
        assert!(!needs_pipeline(&task, into_batch));
        assert!(needs_pipeline(&task, OnCheck::default()));
        // Into develop, an epic, or the batch's own MR: its pipeline, as before.
        assert!(needs_pipeline(&ready(), into_batch));
        let shipped = Mr {
            branch: "batch/breadboard-2".into(),
            ..ready()
        };
        assert!(needs_pipeline(&shipped, into_batch));
        assert!(needs_pipeline(
            &shipped,
            OnCheck {
                into_default: true,
                into_batch: true
            }
        ));
    }

    #[test]
    fn merge_status_leaves_out_what_another_blocker_says() {
        assert_eq!(
            merge_status_text(Some("mergeable"), false, false, true, "develop"),
            None
        );
        assert_eq!(
            merge_status_text(Some("draft_status"), true, false, true, "develop"),
            None
        );
        assert_eq!(
            merge_status_text(Some("ci_must_pass"), false, true, true, "develop"),
            None
        );
        assert_eq!(
            merge_status_text(Some("need_rebase"), false, false, true, "develop").as_deref(),
            Some("needs a rebase onto develop")
        );
        assert_eq!(
            merge_status_text(Some("some_new_thing"), false, false, true, "develop").as_deref(),
            Some("GitLab: some new thing")
        );
    }

    #[test]
    fn names_the_files_both_touched() {
        let mine: BTreeSet<String> = ["a.mjs", "b.mjs"].iter().map(|f| f.to_string()).collect();
        assert_eq!(
            shared_files(&["b.mjs".into(), "c.mjs".into(), "a.mjs".into(), "a.mjs".into()], &mine),
            ["a.mjs", "b.mjs"]
        );
    }
}
