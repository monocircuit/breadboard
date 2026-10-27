//! The ship row of UP NEXT (#414): the overview suggests shipping the
//! product's batch once enough is in it: `ship after tasks` task MRs (the
//! settings, #415), a hotfix or an issue labelled `ship` among them, or its
//! oldest task waiting `ship after days`. A click runs `wt:batch <product>
//! --ship`, as the band's `ship batch` button does. While a batch shipped
//! before is red (#413), the row of its fix instead, until the fix is in the
//! batch: then the ship replaces the red one (#464).

use super::orchestration::OrchestrationLocal;
use super::order::{Go, Item};
use crate::gitlab::{BatchTask, Remote};
use crate::record::ORCHESTRATOR;

/// The settings' defaults where none is read (`ship_after_tasks`, `ship_after_days`).
const TASKS: i64 = 5;
const DAYS: i64 = 2;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Why the batch should ship now, the first reason that holds; None while none does.
fn reason_of(tasks: &[BatchTask], after_tasks: i64, after_days: i64, now: i64) -> Option<String> {
    let labelled = |label: &str| {
        tasks
            .iter()
            .find(|task| task.labels.iter().any(|it| it.eq_ignore_ascii_case(label)))
    };
    if let Some(task) = labelled("hotfix") {
        return Some(format!("a hotfix is in it (!{})", task.iid));
    }
    if let Some(task) = labelled("ship") {
        return Some(format!("!{} asks to ship", task.iid));
    }
    let count = tasks.len() as i64;
    if count >= after_tasks {
        return Some(format!("{count} tasks are in it"));
    }
    let oldest = tasks
        .iter()
        .filter_map(|task| task.merged_at.as_deref().and_then(crate::time::parse_iso))
        .min()?;
    let days = (now - oldest) / DAY_MS;
    (days >= after_days).then(|| {
        format!(
            "its oldest task has waited {days} {}",
            if days == 1 { "day" } else { "days" }
        )
    })
}

/// The row suggesting to ship the batch the orchestration works on, while
/// it has tasks and no MR into develop; while a batch shipped before is red,
/// the row of its fix instead (#413), or the ship once the fix is in it
/// (#464). None before GitLab answered.
pub fn ship_of(local: &OrchestrationLocal, remote: Option<&Remote>, now: i64) -> Option<Item> {
    let epic = local.epic.as_ref()?;
    let (product, _) = crate::batch::parse(&epic.branch)?;
    let found = remote?.epic.as_ref()?;
    let could_ship = found.mr.is_none() && !found.tasks.is_empty();
    let develop = crate::git::default_branch();
    // A batch shipped before is red (#413): its fix first, into this batch;
    // once it is in, the ship replaces the red one (#464).
    if let Some(red) = super::red_batch::red_of(local, remote) {
        let is_fixed = red.fix.is_some_and(|number| {
            found
                .tasks
                .iter()
                .any(|task| crate::git::issue_of_branch(&task.branch) == Some(number as u64))
        });
        if could_ship && is_fixed {
            return Some(Item {
                key: format!("ship:{}", epic.branch),
                kind: "ship".into(),
                text: format!(
                    "ship {} to {develop}: #{} fixes {} in it, the ship replaces !{}",
                    epic.branch,
                    red.fix.unwrap_or_default(),
                    red.branch,
                    red.mr
                ),
                who: format!("{ORCHESTRATOR} orchestrator"),
                is_auto: true,
                rank: -1,
                go: Go::ShipBatch(product.to_string()),
                ..Default::default()
            });
        }
        return Some(Item {
            key: format!("batch-red:{}:{}", red.branch, red.mr),
            kind: "batch red".into(),
            text: format!("{}: {}", red.said(), red.fixing()),
            who: format!("{ORCHESTRATOR} orchestrator"),
            is_auto: true,
            rank: -1,
            number: red.fix,
            go: match red.fix {
                Some(_) => Go::Nothing,
                None => Go::FixBatch(product.to_string()),
            },
            ..Default::default()
        });
    }
    if !could_ship {
        return None;
    }
    let settings = crate::settings::settings_of(&local.settings);
    let positive = |value: i64, default: i64| if value > 0 { value } else { default };
    let reason = reason_of(
        &found.tasks,
        positive(settings.ship_after_tasks, TASKS),
        positive(settings.ship_after_days, DAYS),
        now,
    )?;
    Some(Item {
        key: format!("ship:{}", epic.branch),
        kind: "ship".into(),
        text: format!("ship {} to {develop}: {reason}", epic.branch),
        who: format!("{ORCHESTRATOR} orchestrator"),
        is_auto: true,
        rank: -1,
        go: Go::ShipBatch(product.to_string()),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::{EpicRemote, Mr};
    use crate::model::epic_row::EpicLocal;

    const NOW: i64 = 1_760_000_000_000;

    fn task(iid: i64, days_ago: i64, labels: &[&str]) -> BatchTask {
        BatchTask {
            iid,
            merged_at: Some(crate::time::to_iso(NOW - days_ago * DAY_MS)),
            labels: labels.iter().map(|label| label.to_string()).collect(),
            ..BatchTask::default()
        }
    }

    fn local(branch: &str) -> OrchestrationLocal {
        OrchestrationLocal {
            name: "b".into(),
            epic: Some(EpicLocal {
                branch: branch.into(),
                behind: Some(0),
                ..EpicLocal::default()
            }),
            ..OrchestrationLocal::default()
        }
    }

    fn remote(tasks: Vec<BatchTask>) -> Remote {
        Remote {
            epic: Some(Box::new(EpicRemote {
                branch: "batch/breadboard-3".into(),
                tasks,
                ..EpicRemote::default()
            })),
            ..Remote::default()
        }
    }

    #[test]
    fn suggests_shipping_at_five_tasks_a_hotfix_a_ship_label_or_an_old_task() {
        let fresh = |count: i64| (0..count).map(|iid| task(iid + 1, 0, &[])).collect::<Vec<_>>();
        assert_eq!(reason_of(&fresh(4), 5, 2, NOW), None);
        assert_eq!(reason_of(&fresh(5), 5, 2, NOW).as_deref(), Some("5 tasks are in it"));
        assert_eq!(
            reason_of(&[task(7, 0, &["hotfix"])], 5, 2, NOW).as_deref(),
            Some("a hotfix is in it (!7)")
        );
        assert_eq!(
            reason_of(&[task(8, 0, &["Ship"])], 5, 2, NOW).as_deref(),
            Some("!8 asks to ship")
        );
        assert_eq!(reason_of(&[task(9, 1, &[])], 5, 2, NOW), None);
        assert_eq!(
            reason_of(&[task(9, 3, &[])], 5, 2, NOW).as_deref(),
            Some("its oldest task has waited 3 days")
        );
        assert_eq!(reason_of(&[], 1, 1, NOW), None, "an empty batch never");
    }

    #[test]
    fn the_row_ships_the_batch_while_it_has_no_mr_into_develop() {
        let five = (1..=5).map(|iid| task(iid, 0, &[])).collect();
        let row = ship_of(&local("batch/breadboard-3"), Some(&remote(five)), NOW).unwrap();
        assert_eq!(row.kind, "ship");
        assert_eq!(row.go, Go::ShipBatch("breadboard".into()));
        assert_eq!(row.text, "ship batch/breadboard-3 to develop: 5 tasks are in it");
        assert_eq!(ship_of(&local("batch/breadboard-3"), None, NOW), None, "before GitLab");
        let mut open = remote(vec![task(1, 0, &["hotfix"])]);
        open.epic.as_mut().unwrap().mr = Some(Mr::default());
        assert_eq!(ship_of(&local("batch/breadboard-3"), Some(&open), NOW), None, "shipped");
        // A batch shipped before is red: its fix first.
        let (red_local, mut red_remote) = crate::model::red_batch::tests::red_world();
        red_remote.epic.as_mut().unwrap().tasks = (1..=5).map(|iid| task(iid, 0, &[])).collect();
        let held = ship_of(&red_local, Some(&red_remote), NOW).unwrap();
        assert_eq!(held.kind, "batch red");
        assert_eq!(held.go, Go::FixBatch("b".into()));
        assert_eq!(held.text, "batch/b-2 is red (check:vita): a click files its fix");
        let five = red_remote.epic.as_ref().unwrap().tasks.clone();
        // Nothing to ship yet: red all the same.
        red_remote.epic.as_mut().unwrap().tasks.clear();
        let red = ship_of(&red_local, Some(&red_remote), NOW).unwrap();
        assert_eq!(red.text, "batch/b-2 is red (check:vita): a click files its fix");
        // Filed: the row names it, the click is done.
        let mut fixed = red_local.clone();
        fixed.epic.as_mut().unwrap().fixes = vec![(460, "batch/b-2".into())];
        let held = ship_of(&fixed, Some(&red_remote), NOW).unwrap();
        assert_eq!((held.go, held.number), (Go::Nothing, Some(460)));
        assert_eq!(held.text, "batch/b-2 is red (check:vita): #460 fixes it");
        // The fix merged into this batch (#464): its ship replaces the red one.
        let mut with_fix = red_remote.clone();
        let mut tasks = five;
        tasks.push(BatchTask {
            branch: "fix/460-batch-b-2-red".into(),
            ..task(6, 0, &[])
        });
        with_fix.epic.as_mut().unwrap().tasks = tasks;
        let ship = ship_of(&fixed, Some(&with_fix), NOW).unwrap();
        assert_eq!((ship.kind.as_str(), ship.go), ("ship", Go::ShipBatch("b".into())));
        assert_eq!(
            ship.text,
            "ship batch/b-3 to develop: #460 fixes batch/b-2 in it, the ship replaces !500"
        );
        let hotfix = remote(vec![task(1, 0, &["hotfix"])]);
        assert_eq!(
            ship_of(&local("epic/15-b"), Some(&hotfix), NOW),
            None,
            "an epic: its milestone row"
        );
    }
}
