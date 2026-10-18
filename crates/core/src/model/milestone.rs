//! The milestone row of UP NEXT (#402): once every task up to a milestone
//! mark of the plan (`wt:plan <name> --milestone after #n`) is merged into
//! the epic, the overview offers the milestone MR epic → develop by itself;
//! a click runs `wt:epic <name> --mr`.

use super::orchestration::OrchestrationLocal;
use super::order::{Go, Item};
use crate::epic::is_epic;
use crate::gitlab::Remote;
use crate::record::{Milestone, ORCHESTRATOR};

/// Where a task of a mark stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Task {
    /// Its MR is merged into the epic, its issue open: the milestone MR closes it.
    InEpic,
    /// Its issue is closed: in develop already, or dropped.
    Done,
    /// Still on its way into the epic.
    Open,
}

/// Where each task of `mark` stands; one the record or GitLab doesn't know is left out.
fn tasks_of(mark: &Milestone, local: &OrchestrationLocal, remote: &Remote) -> Vec<Task> {
    let issues = local.record.as_ref().map(|record| &record.issues[..]).unwrap_or(&[]);
    mark.issues
        .iter()
        .filter_map(|number| {
            let branch = issues
                .iter()
                .rev()
                .find(|issue| issue.number == Some(*number))?
                .branch
                .as_deref()?;
            let part = remote.of(branch);
            if part.issue.as_ref().is_some_and(|issue| issue.state == "closed") {
                return Some(Task::Done);
            }
            let is_in = part
                .mr
                .as_ref()
                .is_some_and(|mr| mr.is_merged() && is_epic(mr.target()));
            Some(if is_in { Task::InEpic } else { Task::Open })
        })
        .collect()
}

/// The row offering the milestone MR: for the last mark whose tasks are all
/// in the epic (one at least, not yet in develop), while the epic has no MR
/// into develop open. None before GitLab answered.
pub fn milestone_of(local: &OrchestrationLocal, remote: Option<&Remote>) -> Option<Item> {
    let remote = remote?;
    let epic = local.epic.as_ref()?;
    if remote.epic.as_ref().is_none_or(|found| found.mr.is_some()) {
        return None;
    }
    let plan = local.record.as_ref()?.plan.as_ref()?;
    let mark = plan.milestones.iter().rev().find(|mark| {
        let tasks = tasks_of(mark, local, remote);
        tasks.contains(&Task::InEpic) && !tasks.contains(&Task::Open)
    })?;
    let develop = crate::git::default_branch();
    Some(Item {
        key: format!("milestone:{}:{}", local.name, mark.after),
        kind: "milestone".into(),
        text: format!(
            "milestone after #{}: its tasks are in the epic, open the MR {} → {develop}",
            mark.after, epic.branch
        ),
        who: format!("{ORCHESTRATOR} orchestrator"),
        is_auto: true,
        rank: -1,
        number: Some(mark.after),
        go: Go::EpicMr,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::{BranchRemote, EpicRemote, Issue, Mr};
    use crate::model::epic_row::EpicLocal;
    use crate::record::{IssueEntry, Plan, Record};

    const EPIC: &str = "epic/15-b";

    fn local(milestones: Vec<Milestone>) -> OrchestrationLocal {
        let entry = |number: i64| IssueEntry {
            number: Some(number),
            branch: Some(format!("feature/{number}-x")),
            ..IssueEntry::default()
        };
        OrchestrationLocal {
            name: "b".into(),
            epic: Some(EpicLocal {
                branch: EPIC.into(),
                behind: Some(0),
            }),
            record: Some(Record {
                issues: vec![entry(1), entry(2), entry(3)],
                plan: Some(Plan {
                    merge: vec![1, 2, 3],
                    milestones,
                    ..Plan::default()
                }),
                ..Record::default()
            }),
            ..OrchestrationLocal::default()
        }
    }

    /// GitLab with each issue's MR in `state` into `target`, its issue in `issue`.
    fn remote(tasks: &[(i64, &str, &str, &str)]) -> Remote {
        let mut remote = Remote::default();
        for (number, state, target, issue) in tasks {
            remote.of.insert(
                format!("feature/{number}-x"),
                BranchRemote {
                    mr: Some(Mr {
                        state: (*state).into(),
                        target: (*target).into(),
                        ..Mr::default()
                    }),
                    issue: Some(Issue {
                        state: (*issue).into(),
                        ..Issue::default()
                    }),
                    ..BranchRemote::default()
                },
            );
        }
        remote.epic = Some(Box::new(EpicRemote {
            branch: EPIC.into(),
            ..EpicRemote::default()
        }));
        remote
    }

    fn after(number: i64) -> Milestone {
        Milestone::in_order(&[1, 2, 3], number).unwrap()
    }

    #[test]
    fn a_mark_takes_the_merge_order_up_to_it() {
        assert_eq!(after(2).issues, [1, 2]);
        assert_eq!(Milestone::in_order(&[1, 2], 7), None);
    }

    #[test]
    fn offers_the_milestone_mr_once_every_task_up_to_the_mark_is_in_the_epic() {
        let marks = vec![after(2)];
        let half = remote(&[(1, "merged", EPIC, "opened"), (2, "opened", EPIC, "opened")]);
        assert_eq!(milestone_of(&local(marks.clone()), Some(&half)), None, "#2 still open");
        let both = remote(&[
            (1, "merged", EPIC, "opened"),
            (2, "merged", EPIC, "opened"),
            (3, "opened", EPIC, "opened"),
        ]);
        let row = milestone_of(&local(marks.clone()), Some(&both)).unwrap();
        assert_eq!(row.kind, "milestone");
        assert_eq!(row.go, Go::EpicMr);
        assert_eq!(
            row.text,
            "milestone after #2: its tasks are in the epic, open the MR epic/15-b → develop"
        );
        assert_eq!(milestone_of(&local(marks), None), None, "before GitLab answered");
        assert_eq!(milestone_of(&local(vec![]), Some(&both)), None, "no mark");
    }

    #[test]
    fn a_closed_issue_counts_as_done_and_a_mark_all_in_develop_is_passed() {
        // #1 went to develop at the last milestone, #2 is in the epic.
        let next = remote(&[(1, "merged", EPIC, "closed"), (2, "merged", EPIC, "opened")]);
        assert!(milestone_of(&local(vec![after(2)]), Some(&next)).is_some());
        let passed = remote(&[(1, "merged", EPIC, "closed"), (2, "merged", EPIC, "closed")]);
        assert_eq!(milestone_of(&local(vec![after(2)]), Some(&passed)), None);
        // A task merged straight into develop isn't in the epic.
        let develop = remote(&[(1, "merged", "develop", "opened")]);
        assert_eq!(milestone_of(&local(vec![after(1)]), Some(&develop)), None);
    }

    #[test]
    fn the_last_reached_mark_while_the_epic_has_no_mr_into_develop() {
        let all = remote(&[
            (1, "merged", EPIC, "opened"),
            (2, "merged", EPIC, "opened"),
            (3, "merged", EPIC, "opened"),
        ]);
        let row = milestone_of(&local(vec![after(1), after(3)]), Some(&all)).unwrap();
        assert_eq!(row.number, Some(3));
        let mut open = all.clone();
        open.epic.as_mut().unwrap().mr = Some(Mr::default());
        assert_eq!(
            milestone_of(&local(vec![after(3)]), Some(&open)),
            None,
            "its MR is open"
        );
        let mut no_epic = local(vec![after(3)]);
        no_epic.epic = None;
        assert_eq!(milestone_of(&no_epic, Some(&all)), None, "works on develop");
    }
}
