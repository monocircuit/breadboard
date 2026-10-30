//! The commands of the daemon's table (#498, core's `command`) that the
//! actor of an orchestration does, on its headless overview in the daemon
//! (`daemon/acts.rs`): each the click that does the same, its log entry and
//! what its rows say included. A merge or a start is done once it started;
//! how it ends is in the actor's log.

use super::{Overview, act, actions};
use breadboard_core::command::Command;
use breadboard_core::model::order::{Go, Item};

/// Why a command waits: the actor hasn't read yet (the daemon asks again for a while).
pub const NOT_YET: &str = "is still reading";

/// `command` done by the actor of this overview's orchestration, or why not.
pub fn act(app: &mut Overview, command: &Command) -> Result<String, String> {
    // Before its first read it holds nothing and knows no action yet.
    if app.model.is_none() {
        return Err(NOT_YET.into());
    }
    if !app.state.is_deliverer {
        return Err("another overview acts for this orchestration".into());
    }
    match command {
        Command::StartIssue { number, to, .. } => start_issue(app, *number, to.as_deref()),
        Command::AnswerAction {
            dir, id, answer, note, ..
        } => actions::answer_as_command(
            app,
            &breadboard_core::actions::actions_file(dir),
            *id,
            answer,
            note.as_deref(),
        ),
        Command::MergeIntoBatch { iid, .. } => merge_into_batch(app, *iid),
        Command::ShipBatch { product, .. } => {
            act::go_to(app, Go::ShipBatch(product.clone()));
            match &app.actor.ship {
                Some(run) if run.is_running => Ok(format!("shipping the batch of {product}")),
                Some(run) => Err(run.error.clone().unwrap_or_else(|| "it shipped a moment ago".into())),
                None => Err(format!("the batch of {product} didn't start shipping")),
            }
        }
        Command::PauseRuns { .. }
        | Command::ResumeRuns
        | Command::Tell { .. }
        | Command::StopAgent { .. }
        | Command::StopStack { .. } => Err("not the actor's: the daemon does it".into()),
    }
}

/// A fresh agent's start of `number`, or its hand-over to `to`, as its click.
fn start_issue(app: &mut Overview, number: i64, to: Option<&str>) -> Result<String, String> {
    if app.state.runs.get(&number).is_some_and(|run| run.is_running) {
        return Err(format!("#{number} is being started already"));
    }
    let go = match to {
        Some(to) => Go::HandOver {
            number,
            to: to.to_string(),
            dir: None,
        },
        None => Go::StartFresh(number),
    };
    act::go_to(app, go);
    match app.state.runs.get(&number) {
        Some(run) if run.is_running => Ok(match to {
            Some(to) => format!("handing #{number} to {to}"),
            None => format!("starting a fresh agent for #{number}"),
        }),
        Some(run) => Err(run.error.clone().unwrap_or_else(|| format!("#{number} didn't start"))),
        None => Err(format!("#{number} didn't start")),
    }
}

/// The merge of `iid` into its batch, as the second click of its merge row
/// (the reads again of the MR, its boxes and tries, then the merge as the
/// user): only while its turn offers it, only into a batch or an epic.
fn merge_into_batch(app: &mut Overview, iid: i64) -> Result<String, String> {
    if app
        .actor
        .merges
        .of
        .values()
        .any(|merge| merge.phase == super::actor::merge::Phase::Running)
    {
        return Err("another merge runs: one at a time".into());
    }
    let model = app.orchestration().ok_or("the orchestration isn't read yet")?;
    let section = model.actions.as_ref().map_err(Clone::clone)?;
    let item: Item = section
        .actions
        .iter()
        .chain(&section.up_next)
        .find(|item| item.merge.as_ref().is_some_and(|offer| offer.iid == iid))
        .cloned()
        .ok_or_else(|| format!("!{iid} isn't offered to merge: its turn hasn't come, or something blocks it"))?;
    let (Go::Merge { branch, action }, Some(offer)) = (item.go.clone(), item.merge.clone()) else {
        return Err(format!("!{iid} isn't offered to merge"));
    };
    if !offer.blockers.is_empty() {
        return Err(format!("!{iid} waits: {}", offer.blockers.join(", ")));
    }
    let remote = app.orchestration_remote().ok_or("GitLab didn't answer yet")?;
    let target = remote
        .of(&branch)
        .mr
        .as_ref()
        .map(|mr| mr.target().to_string())
        .unwrap_or_default();
    if !breadboard_core::batch::is_auto_target(&target) {
        return Err(format!(
            "!{iid} goes into {target}, not into a batch: that merge stays a click"
        ));
    }
    super::actor::merge::start(app, &branch, &action, &offer);
    Ok(format!("merging !{iid} into {target}"))
}
