//! The head row of an orchestration's overview on a batch (#411) or an epic
//! of before (#350): its branch, how far develop has gone on since it, how
//! many tasks are in it and not yet in develop, its last pipeline, and its
//! MR into develop; an epic without one is offered to ship as a batch.
//! A batch's row (#414): `batch/breadboard-3 · fast checks ✓`, the `ship
//! batch` button beside it; under it the batches shipped before it while
//! their MRs into develop run, a red one apart from the batch's own state,
//! then its tasks against the ship setting, the task MRs merged into it and
//! those still open into it (#491).

use super::orchestration::OrchestrationLocal;
#[cfg(test)]
use super::text_of;
use super::tree::pipeline_mark;
use super::{Paint, Span};
use crate::epic::is_in_epic;
use crate::gitlab::{BatchTask, Pipeline, Remote};

/// The local side of the epic: its branch and how many commits develop has
/// that it lacks (None: git can't tell, origin not fetched).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EpicLocal {
    pub branch: String,
    pub behind: Option<u64>,
    /// The product's batches shipped before it, as git has them (#413):
    /// a red one's suspects.
    pub shipped: Vec<crate::batch_red::Shipped>,
    /// The fix issues filed for red shipped batches (#413): (issue, batch).
    pub fixes: Vec<(i64, String)>,
}

/// What a batch's row has besides its parts (#414): the button that ships
/// it, the batches shipped before it, the task MRs merged into it and those
/// still open into it (#491).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatchFold {
    /// The product whose batch it is: `wt:batch <product> --ship`.
    pub product: String,
    /// Tasks are in it and it has no MR into develop yet: `ship batch` is offered.
    pub can_ship: bool,
    /// The red batch shipped before whose MR the ship replaces (#464): `replaces batch/b-2 (red)`.
    pub replaces: Option<String>,
    /// The batches shipped before it whose MRs into develop are open (#491),
    /// one line each: `batch 6 shipped: !450 → develop ✓ passed · 4 tasks: !431 …`.
    pub shipped: Vec<Vec<Span>>,
    /// The red one among them, apart from its own state (#491): `batch 7 (!468, shipped) failed: …`.
    pub red: Option<Vec<Span>>,
    /// The red batch's suspects, the likeliest first (#413).
    pub suspects: Vec<Vec<Span>>,
    /// A red batch without its fix issue yet: `fix batch` files it (#413).
    pub can_fix: bool,
    /// The fix issue filed for the red batch: `#460 fixes it`.
    pub fix: Option<i64>,
    /// Its tasks against the ship setting (#491): `3 of 5 tasks in batch 8`.
    pub count: String,
    /// `!431 #411 feat(breadboard): …`, one line per task MR, oldest first.
    pub tasks: Vec<Vec<Span>>,
    /// The task MRs open into it, oldest first (#491): `waiting: !469 !471`, empty for none.
    pub waiting: Vec<Span>,
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

/// A batch's parts (#414): its branch, its fast checks, its MR into develop
/// when one is open. The batches shipped before it and its tasks have lines
/// of their own under it (#491, `BatchFold`).
fn batch_row_of(branch: &str, remote: Option<&Remote>) -> Vec<Vec<Span>> {
    let mut row = vec![vec![Span::new(branch.to_string(), Paint::Cyan)]];
    let develop = crate::git::default_branch();
    let Some(found) = remote.and_then(|remote| remote.epic.as_ref()) else {
        return row;
    };
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

/// `!431 #411 feat(breadboard): …`: a task MR's line, its number a link.
fn task_line(task: &BatchTask) -> Vec<Span> {
    let mut line = vec![Span::link(format!("!{}", task.iid), Some(&task.url))];
    if let Some(issue) = crate::git::issue_of_branch(&task.branch) {
        line.push(Span::dim(format!(" #{issue}")));
    }
    line.push(Span::plain(format!(" {}", task.title)));
    line
}

/// `3 of 5 tasks in batch 8`, `7 tasks in batch 8, it ships at 5`: the count against the setting.
fn count_of(count: usize, after: i64, name: &str) -> String {
    match count {
        0 => format!("no task in {name} yet, it ships at {after}"),
        _ if (count as i64) < after => format!("{count} of {after} tasks in {name}"),
        1 => format!("1 task in {name}, it ships at {after}"),
        _ => format!("{count} tasks in {name}, it ships at {after}"),
    }
}

/// A batch shipped before, its MR into develop open: `batch 6 shipped: !450
/// → develop ✓ passed · 4 tasks: !431 !440`, its tasks as git has them, a red
/// one's it replaced with them (#464).
fn shipped_line(mr: &crate::gitlab::Mr, local: &EpicLocal, develop: &str) -> Vec<Span> {
    let mut line = vec![Span::dim(format!(
        "{} shipped: ",
        super::red_batch::name_of(&mr.branch)
    ))];
    line.extend(mr_part(mr, develop));
    let Some(shipped) = local.shipped.iter().find(|shipped| shipped.branch == mr.branch) else {
        return line;
    };
    let count = shipped.tasks.len();
    line.push(Span::dim(match count {
        0 => " · no task".to_string(),
        1 => " · 1 task:".to_string(),
        _ => format!(" · {count} tasks:"),
    }));
    for task in shipped.tasks.iter().rev() {
        line.push(match task.mr {
            Some(iid) => Span::link(format!(" !{iid}"), Some(&super::red_batch::mr_url_of(&mr.url, iid))),
            None => Span::dim(format!(" {}", task.sha.chars().take(8).collect::<String>())),
        });
    }
    line
}

/// The button and the lines under a batch's row (#414, #491), None for an
/// epic or before GitLab answered.
pub fn batch_fold_of(local: &OrchestrationLocal, remote: Option<&Remote>) -> Option<BatchFold> {
    let epic = local
        .epic
        .as_ref()
        .filter(|epic| crate::batch::is_batch(&epic.branch))?;
    let product = crate::batch::parse(&epic.branch)?.0.to_string();
    let found = remote?.epic.as_ref()?;
    let develop = crate::git::default_branch();
    let could_ship = !found.tasks.is_empty() && found.mr.is_none();
    let red = super::red_batch::red_of(local, remote);
    let shipped = found
        .shipping
        .iter()
        .filter(|mr| red.as_ref().is_none_or(|red| red.mr != mr.iid))
        .map(|mr| shipped_line(mr, epic, develop))
        .collect();
    let mut waiting = Vec::new();
    for (i, task) in found.waiting.iter().rev().enumerate() {
        waiting.push(if i == 0 {
            Span::dim("waiting: ")
        } else {
            Span::plain(" ")
        });
        waiting.push(Span::link(format!("!{}", task.iid), Some(&task.url)));
    }
    Some(BatchFold {
        product,
        can_ship: could_ship,
        replaces: red
            .as_ref()
            .filter(|_| could_ship)
            .map(|red| format!("replaces {} (red)", red.branch)),
        shipped,
        red: red
            .as_ref()
            .map(|red| red.line(&epic.branch, red.is_fixed_in(&found.tasks))),
        suspects: red.as_ref().map(super::red_batch::Red::lines).unwrap_or_default(),
        can_fix: red.as_ref().is_some_and(|red| red.fix.is_none()),
        fix: red.as_ref().and_then(|red| red.fix),
        count: count_of(
            found.tasks.len(),
            super::ship::after_tasks(local),
            &super::red_batch::name_of(&epic.branch),
        ),
        tasks: found.tasks.iter().rev().map(task_line).collect(),
        waiting,
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
                ..EpicLocal::default()
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
        remote.epic.as_mut().unwrap().waiting = vec![
            task(452, "feature/420-y", "feat: y"),
            task(451, "feature/419-x", "feat: x"),
        ];
        assert_eq!(
            text_of_parts(&epic_row_of(&on_batch, Some(&remote)).unwrap()),
            "batch/b-3 · fast checks ✓"
        );
        let fold = batch_fold_of(&on_batch, Some(&remote)).unwrap();
        assert_eq!(fold.product, "b");
        assert!(fold.can_ship);
        assert_eq!(fold.count, "2 of 5 tasks in batch 3", "the ship setting's default");
        assert_eq!(
            fold.tasks.iter().map(|line| text_of(line)).collect::<Vec<_>>(),
            ["!431 #411 feat: rolling batches", "!440 #413 feat: a red batch"]
        );
        assert_eq!(text_of(&fold.waiting), "waiting: !451 !452");
        assert_eq!(count_of(7, 5, "batch 3"), "7 tasks in batch 3, it ships at 5");
        // Shipped: its MR open, the next batch's row names it.
        let found = remote.epic.as_mut().unwrap();
        found.tasks.clear();
        found.pipeline = None;
        found.shipping = vec![Mr {
            iid: 500,
            branch: "batch/b-2".into(),
            ..Mr::default()
        }];
        on_batch.epic.as_mut().unwrap().shipped = vec![crate::batch_red::Shipped {
            branch: "batch/b-2".into(),
            tasks: vec![crate::batch_red::Merged {
                mr: Some(431),
                ..crate::batch_red::Merged::default()
            }],
            ..crate::batch_red::Shipped::default()
        }];
        assert_eq!(
            text_of_parts(&epic_row_of(&on_batch, Some(&remote)).unwrap()),
            "batch/b-3"
        );
        let fold = batch_fold_of(&on_batch, Some(&remote)).unwrap();
        assert!(!fold.can_ship, "nothing in it");
        assert_eq!(fold.count, "no task in batch 3 yet, it ships at 5");
        assert_eq!(
            fold.shipped.iter().map(|line| text_of(line)).collect::<Vec<_>>(),
            ["batch 2 shipped: !500 → develop no pipeline · 1 task: !431"]
        );
    }

    #[test]
    fn a_red_batch_says_so_lists_its_suspects_and_the_ship_replaces_it() {
        let (local, mut remote) = crate::model::red_batch::tests::red_world();
        remote.epic.as_mut().unwrap().tasks = vec![BatchTask {
            iid: 450,
            ..BatchTask::default()
        }];
        assert_eq!(text_of_parts(&epic_row_of(&local, Some(&remote)).unwrap()), "batch/b-3");
        let fold = batch_fold_of(&local, Some(&remote)).unwrap();
        // Named apart from the current batch's own state (#491), not among the shipped.
        assert_eq!(
            text_of(fold.red.as_ref().unwrap()),
            "batch 2 (!500, shipped) failed: check:vita +1 · ship of batch 3 held until it is green"
        );
        assert!(fold.shipped.is_empty());
        assert!(fold.can_ship);
        assert_eq!(fold.replaces.as_deref(), Some("replaces batch/b-2 (red)"));
        assert!(fold.can_fix && fold.fix.is_none());
        assert_eq!(
            text_of(&fold.suspects[0]),
            "!431 #411 feat: vita's page · its files run in check:vita"
        );
        // Its fix filed: named, no second click.
        let mut fixed = local.clone();
        fixed.epic.as_mut().unwrap().fixes = vec![(460, "batch/b-2".into())];
        let fold = batch_fold_of(&fixed, Some(&remote)).unwrap();
        assert!(!fold.can_fix);
        assert_eq!(fold.fix, Some(460));
        // Green again: shipped as before.
        remote.epic.as_mut().unwrap().shipping[0]
            .pipeline
            .as_mut()
            .unwrap()
            .status = "success".into();
        let fold = batch_fold_of(&local, Some(&remote)).unwrap();
        assert!(fold.can_ship && fold.replaces.is_none() && fold.suspects.is_empty() && !fold.can_fix);
    }
}
