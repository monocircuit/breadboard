//! The orchestrator's fresh start (#382): above `[orchestrator] fresh_above`
//! of its context and at a quiet point (no question to the user open, its
//! chat idle, no merge in flight), a click on `↻ fresh start` (or the
//! overview by itself with `auto_fresh`) runs `wt:orchestrate <name>
//! --fresh` in the background: the old chat writes its journal entry, then
//! the tab `orchestrator` starts a fresh claude from the journal and the
//! record. Once per conversation: a fresh start that failed is not retried
//! by itself.

use super::merge::Phase;
use super::spawn::{self, Background, Run};
use crate::tui::overview::Overview;
use breadboard_core::model::Paint;
use breadboard_core::model::orchestration::OrchestrationModel;

/// Why no fresh start may run now that the model doesn't know: a merge of this overview in flight.
fn merging(app: &Overview) -> Option<String> {
    app.actor
        .merges
        .of
        .iter()
        .find(|(_, merge)| matches!(merge.phase, Phase::Armed | Phase::Running))
        .map(|(iid, _)| format!("the merge of !{iid} is in flight"))
}

/// Why the fresh start of the model's orchestrator can't run now, None when it can.
pub fn hold(app: &Overview, model: &OrchestrationModel) -> Option<String> {
    if !model.orchestrator.is_due() {
        return Some("its context is below the threshold".into());
    }
    model.orchestrator.hold.clone().or_else(|| merging(app))
}

/// After a read: with `auto_fresh`, the fresh start once it is due and quiet.
pub fn after_read(app: &mut Overview, name: &str, model: &OrchestrationModel) {
    if !model.orchestrator.is_auto || hold(app, model).is_some() {
        return;
    }
    let session = model.chat.as_ref().and_then(|chat| chat.session_id.clone());
    if session.is_some() && app.actor.fresh_for == session {
        return;
    }
    start(app, name, session);
}

/// A click on `↻ fresh start`.
pub fn click(app: &mut Overview) {
    let Some(name) = app.orchestration_name() else { return };
    let Some(model) = app.orchestration().cloned() else {
        return;
    };
    if let Some(why) = hold(app, &model) {
        app.actor
            .log_as("fresh", Paint::Red, format!("no fresh start now: {why}"), None);
        return;
    }
    let session = model.chat.as_ref().and_then(|chat| chat.session_id.clone());
    start(app, &name, session);
}

/// `wt:orchestrate <name> --fresh` in the background, once for `session`.
fn start(app: &mut Overview, name: &str, session: Option<String>) {
    if app.actor.fresh.as_ref().is_some_and(|run| run.is_running) {
        return;
    }
    app.actor.fresh_for = session;
    let log = spawn::log_file(&format!("orchestrator-fresh-{name}.log"));
    let program = spawn::breadboard();
    let poster = app.actor.poster();
    let started = spawn::start(
        Background {
            program: &program,
            args: vec!["wt:orchestrate".into(), name.to_string(), "--fresh".into()],
            cwd: &app.actor.main,
            log: &log,
            env: vec![],
            is_paneless: true,
        },
        move |code, last| {
            poster.post(move |app: &mut Overview| {
                let error = spawn::error_of(code, &last, "");
                match &error {
                    None => app.actor.log_as(
                        "fresh",
                        Paint::Green,
                        format!("orchestrator: {}", spawn::said(&last)),
                        None,
                    ),
                    Some(why) => app
                        .actor
                        .log_as("fresh", Paint::Red, format!("no fresh start: {why}"), None),
                }
                app.actor.fresh = Some(Run {
                    error,
                    ..Run::default()
                });
                app.redraw();
            });
        },
    );
    match started {
        Ok(()) => {
            app.actor.fresh = Some(Run::running(&log));
            app.actor.log_as(
                "fresh",
                Paint::Yellow,
                "orchestrator: its journal entry, then a fresh chat …",
                Some(format!(
                    "{run} wt:orchestrate {name} --fresh",
                    run = breadboard_core::config::run_word()
                )),
            );
        }
        Err(why) => {
            app.actor
                .log_as("fresh", Paint::Red, format!("no fresh start: {why}"), None);
            app.actor.fresh = Some(Run {
                error: Some(why),
                ..Run::default()
            });
        }
    }
    app.redraw();
}
