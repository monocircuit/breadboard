//! The head row of an orchestration's overview on a batch (#411) or an epic
//! of before (#350): its branch, how far develop has gone on since it, how
//! many tasks are in it and not yet in develop, its last pipeline, and its
//! MR into develop; an epic without one is offered to ship as a batch.

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

/// The row as parts, each whole on a line where it fits (the view parts
/// them with ` · `); the milestone MR's link in its part.
pub fn epic_row_of(local: &OrchestrationLocal, remote: Option<&Remote>) -> Option<Vec<Vec<Span>>> {
    let epic = local.epic.as_ref()?;
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

/// The parts as one text, as the view parts them.
#[cfg(test)]
fn text_of_parts(parts: &[Vec<Span>]) -> String {
    parts.iter().map(|part| text_of(part)).collect::<Vec<_>>().join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::{BranchRemote, DevelopPipeline, EpicRemote, Issue, Mr};
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
            mr: None,
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
        // On a batch: its tasks `in batch`, shipped by the user (#414).
        let mut on_batch = local(Some(0));
        on_batch.epic.as_mut().unwrap().branch = "batch/b-1".into();
        let part = remote.of.get_mut("feature/7-x").unwrap();
        part.mr.as_mut().unwrap().target = "batch/b-1".into();
        remote.epic.as_mut().unwrap().mr = None;
        assert_eq!(
            text_of_parts(&epic_row_of(&on_batch, Some(&remote)).unwrap()),
            "batch/b-1 · has develop · 1 in batch · ✓ passed · not shipped to develop yet"
        );
    }
}
