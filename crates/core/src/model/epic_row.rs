//! The head row of an orchestration's overview on a batch (#411) or an epic
//! of before (#350): its branch, how far develop has gone on since it, how
//! many tasks are in it and not yet in develop, its last pipeline, and its
//! MR into develop; an epic without one is offered to ship as a batch.
//! A batch's row (#414): `batch/breadboard-3 · 6 tasks · fast checks ✓`,
//! the `ship batch` button beside it and the task MRs merged into it in a
//! fold; the batches shipped before it while their MRs into develop run.

use super::orchestration::OrchestrationLocal;
#[cfg(test)]
use super::text_of;
use super::tree::pipeline_mark;
use super::{Paint, Span};
use crate::epic::is_in_epic;
use crate::gitlab::{Pipeline, Remote};

/// The local side of the epic: its branch and how many commits develop has
/// that it lacks (None: git can't tell, origin not fetched).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EpicLocal {
    pub branch: String,
    pub behind: Option<u64>,
}

/// What a batch's row has besides its parts (#414): the button that ships
/// it, the task MRs merged into it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatchFold {
    /// The product whose batch it is: `wt:batch <product> --ship`.
    pub product: String,
    /// Tasks are in it and it has no MR into develop yet: `ship batch` is offered.
    pub can_ship: bool,
    /// `!431 #411 feat(breadboard): …`, one line per task MR, oldest first.
    pub tasks: Vec<Vec<Span>>,
}

/// The row as parts, each whole on a line where it fits (the view parts
/// them with ` · `); the milestone MR's link in its part.
pub fn epic_row_of(local: &OrchestrationLocal, remote: Option<&Remote>) -> Option<Vec<Vec<Span>>> {
    let epic = local.epic.as_ref()?;
    if crate::batch::is_batch(&epic.branch) {
        return Some(batch_row_of(&epic.branch, remote));
    }
    let mut row = vec![vec![Span::new(epic.branch.clone(), Paint::Cyan)]];
    // The default branch the epic merges into (breadboard.toml).
    let develop = crate::git::default_branch();
    match epic.behind {
        Some(0) => row.push(vec![Span::dim(format!("has {develop}"))]),
        Some(behind) => row.push(vec![Span::new(format!("{behind} behind {develop}"), Paint::Yellow)]),
        None => {}
    }
    let Some(remote) = remote else {
        return Some(row);
    };
    let issues = local.record.as_ref().map(|record| &record.issues[..]).unwrap_or(&[]);
    let in_epic = issues
        .iter()
        .filter_map(|issue| issue.branch.as_deref())
        .filter(|branch| {
            let part = remote.of(branch);
            part.mr.as_ref().is_some_and(|mr| is_in_epic(mr, part.issue.as_ref()))
        })
        .count();
    let is_batch = crate::batch::is_batch(&epic.branch);
    if in_epic > 0 {
        let word = if is_batch { "in batch" } else { "in epic" };
        row.push(vec![Span::dim(format!("{in_epic} {word}"))]);
    }
    let Some(found) = remote.epic.as_ref() else {
        return Some(row);
    };
    if let Some(pipeline) = &found.pipeline {
        row.push(vec![pipeline_mark(Some(&Pipeline {
            id: pipeline.id,
            status: pipeline.status.clone(),
            web_url: pipeline.url.clone(),
            ..Pipeline::default()
        }))]);
    }
    row.push(match &found.mr {
        Some(mr) => vec![
            Span::link(format!("!{}", mr.iid), Some(&mr.url)),
            Span::dim(if mr.is_draft {
                format!(" draft → {develop} ")
            } else {
                format!(" → {develop} ")
            }),
            pipeline_mark(mr.pipeline.as_ref()),
        ],
        None if is_batch => vec![Span::dim(format!("not shipped to {develop} yet"))],
        // An epic of before goes to develop as a batch would (#411, #352's MR).
        None => vec![Span::dim(format!("ship as batch: pnpm wt:epic {} --mr", local.name))],
    });
    Some(row)
}

/// A batch's parts (#414): its branch, its tasks, its fast checks, its MR
/// into develop when one is open, then each batch shipped before it with its MR.
fn batch_row_of(branch: &str, remote: Option<&Remote>) -> Vec<Vec<Span>> {
    let mut row = vec![vec![Span::new(branch.to_string(), Paint::Cyan)]];
    let develop = crate::git::default_branch();
    let Some(found) = remote.and_then(|remote| remote.epic.as_ref()) else {
        return row;
    };
    let count = found.tasks.len();
    row.push(vec![Span::dim(match count {
        0 => "no task yet".to_string(),
        1 => "1 task".to_string(),
        _ => format!("{count} tasks"),
    })]);
    if let Some(pipeline) = &found.pipeline {
        let (mark, paint) = match pipeline.status.as_str() {
            "success" => ("✓", Paint::Green),
            "failed" => ("✗", Paint::Red),
            _ => ("…", Paint::Yellow),
        };
        row.push(vec![Span {
            text: format!("fast checks {mark}"),
            paint,
            url: pipeline.url.clone(),
        }]);
    }
    if let Some(mr) = &found.mr {
        row.push(mr_part(mr, develop));
    }
    for mr in &found.shipping {
        let mut part = vec![Span::dim(format!("shipping {} ", mr.branch))];
        part.extend(mr_part(mr, develop));
        row.push(part);
    }
    row
}

/// `!500 → develop ✓ passed`: an MR into develop, its pipeline's mark.
fn mr_part(mr: &crate::gitlab::Mr, develop: &str) -> Vec<Span> {
    vec![
        Span::link(format!("!{}", mr.iid), Some(&mr.url)),
        Span::dim(if mr.is_draft {
            format!(" draft → {develop} ")
        } else {
            format!(" → {develop} ")
        }),
        pipeline_mark(mr.pipeline.as_ref()),
    ]
}

/// The button and the fold of a batch's row (#414), None for an epic or
/// before GitLab answered.
pub fn batch_fold_of(local: &OrchestrationLocal, remote: Option<&Remote>) -> Option<BatchFold> {
    let epic = local
        .epic
        .as_ref()
        .filter(|epic| crate::batch::is_batch(&epic.branch))?;
    let product = crate::batch::parse(&epic.branch)?.0.to_string();
    let found = remote?.epic.as_ref()?;
    let tasks = found
        .tasks
        .iter()
        .rev()
        .map(|task| {
            let mut line = vec![Span::link(format!("!{}", task.iid), Some(&task.url))];
            if let Some(issue) = crate::git::issue_of_branch(&task.branch) {
                line.push(Span::dim(format!(" #{issue}")));
            }
            line.push(Span::plain(format!(" {}", task.title)));
            line
        })
        .collect::<Vec<_>>();
    Some(BatchFold {
        product,
        can_ship: !tasks.is_empty() && found.mr.is_none(),
        tasks,
    })
}

/// The parts as one text, as the view parts them.
#[cfg(test)]
fn text_of_parts(parts: &[Vec<Span>]) -> String {
    parts.iter().map(|part| text_of(part)).collect::<Vec<_>>().join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::{BatchTask, BranchRemote, DevelopPipeline, EpicRemote, Issue, Mr};
    use crate::record::{IssueEntry, Record};

    fn local(behind: Option<u64>) -> OrchestrationLocal {
        OrchestrationLocal {
            name: "b".into(),
            epic: Some(EpicLocal {
                branch: "epic/15-b".into(),
                behind,
            }),
            record: Some(Record {
                issues: vec![IssueEntry {
                    number: Some(7),
                    branch: Some("feature/7-x".into()),
                    ..IssueEntry::default()
                }],
                ..Record::default()
            }),
            ..OrchestrationLocal::default()
        }
    }

    #[test]
    fn says_the_epic_s_branch_its_lag_its_tasks_its_pipeline_and_its_mr() {
        assert_eq!(
            epic_row_of(&OrchestrationLocal::default(), None),
            None,
            "no epic: no row"
        );
        assert_eq!(
            text_of_parts(&epic_row_of(&local(Some(3)), None).unwrap()),
            "epic/15-b · 3 behind develop"
        );
        let mut remote = Remote::default();
        remote.of.insert(
            "feature/7-x".into(),
            BranchRemote {
                mr: Some(Mr {
                    state: "merged".into(),
                    target: "epic/15-b".into(),
                    ..Mr::default()
                }),
                issue: Some(Issue {
                    state: "opened".into(),
                    ..Issue::default()
                }),
                ..BranchRemote::default()
            },
        );
        remote.epic = Some(Box::new(EpicRemote {
            branch: "epic/15-b".into(),
            pipeline: Some(DevelopPipeline {
                status: "success".into(),
                ..DevelopPipeline::default()
            }),
            ..EpicRemote::default()
        }));
        assert_eq!(
            text_of_parts(&epic_row_of(&local(Some(0)), Some(&remote)).unwrap()),
            "epic/15-b · has develop · 1 in epic · ✓ passed · ship as batch: pnpm wt:epic b --mr"
        );
        remote.epic.as_mut().unwrap().mr = Some(Mr {
            iid: 400,
            is_draft: true,
            ..Mr::default()
        });
        assert_eq!(
            text_of_parts(&epic_row_of(&local(None), Some(&remote)).unwrap()),
            "epic/15-b · 1 in epic · ✓ passed · !400 draft → develop no pipeline"
        );
    }

    #[test]
    fn a_batch_says_its_tasks_its_fast_checks_and_the_batches_shipping() {
        let mut on_batch = local(Some(4));
        on_batch.epic.as_mut().unwrap().branch = "batch/b-3".into();
        assert_eq!(text_of_parts(&epic_row_of(&on_batch, None).unwrap()), "batch/b-3");
        assert_eq!(batch_fold_of(&on_batch, None), None, "before GitLab answered");
        let task = |iid: i64, branch: &str, title: &str| BatchTask {
            iid,
            title: title.into(),
            branch: branch.into(),
            ..BatchTask::default()
        };
        let mut remote = Remote {
            epic: Some(Box::new(EpicRemote {
                branch: "batch/b-3".into(),
                pipeline: Some(DevelopPipeline {
                    status: "success".into(),
                    ..DevelopPipeline::default()
                }),
                // Newest first, as GitLab lists them.
                tasks: vec![
                    task(440, "feature/413-red", "feat: a red batch"),
                    task(431, "feature/411-rolling", "feat: rolling batches"),
                ],
                ..EpicRemote::default()
            })),
            ..Remote::default()
        };
        assert_eq!(
            text_of_parts(&epic_row_of(&on_batch, Some(&remote)).unwrap()),
            "batch/b-3 · 2 tasks · fast checks ✓"
        );
        let fold = batch_fold_of(&on_batch, Some(&remote)).unwrap();
        assert_eq!(fold.product, "b");
        assert!(fold.can_ship);
        assert_eq!(
            fold.tasks.iter().map(|line| text_of(line)).collect::<Vec<_>>(),
            ["!431 #411 feat: rolling batches", "!440 #413 feat: a red batch"]
        );
        // Shipped: its MR open, the next batch's row names it.
        let found = remote.epic.as_mut().unwrap();
        found.tasks.clear();
        found.pipeline = None;
        found.shipping = vec![Mr {
            iid: 500,
            branch: "batch/b-2".into(),
            ..Mr::default()
        }];
        assert_eq!(
            text_of_parts(&epic_row_of(&on_batch, Some(&remote)).unwrap()),
            "batch/b-3 · no task yet · shipping batch/b-2 !500 → develop no pipeline"
        );
        assert!(
            !batch_fold_of(&on_batch, Some(&remote)).unwrap().can_ship,
            "nothing in it"
        );
    }
}
