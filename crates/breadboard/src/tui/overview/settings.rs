//! SETTINGS (#415): the orchestration's settings panel, opened with `,` or
//! the `⚙ settings` button by the auto mode row. A row per setting: its
//! value, the default when the panel changed it, a line on what it does;
//! a click on a switch flips it, `−`/`+` step a number, `reset` goes back to
//! the team's default. Kept per product (core's `settings`), it holds from
//! the next tick; the journal says what changed.

use super::{Act, Ctx, Overview, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Flow, Page, Section};
use breadboard_core::model::Paint;
use breadboard_core::model::orchestration::OrchestrationModel;
use breadboard_core::settings::{Kind, Row};
use breadboard_core::time::now_ms;
use serde_json::{Value, json};

/// What a click on a setting's row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A switch the other way.
    Flip,
    /// A number up or down by this much.
    Step(i64),
    /// Back to the team's default.
    Reset,
}

/// The panel, under the band, while it is open; nothing in a print.
pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    if !ctx.folds.is_settings_open || ctx.is_once {
        return;
    }
    page.section(Section::new("SETTINGS"), |scene| {
        scene.note(", or ✕ closes · kept per product, they hold from the next tick");
        for row in &model.settings {
            let key = row.def.key.to_string();
            let main_act = match row.def.kind {
                Kind::Switch => Act::Setting(key.clone(), Change::Flip),
                Kind::Count { .. } => Act::Setting(key.clone(), Change::Step(1)),
            };
            scene.block(main_act.clone(), |scene| {
                scene.pad_to(1);
                scene.text(row.def.label, theme::bold());
                scene.text("  ", theme::dim());
                let tone = if row.is_overridden { Tone::Yellow } else { Tone::Green };
                match row.def.kind {
                    Kind::Switch => {
                        let dot = if row.value.as_bool() == Some(true) {
                            "●"
                        } else {
                            "○"
                        };
                        scene.button(&format!("{dot} {}", row.text()), tone, main_act.clone());
                    }
                    Kind::Count { .. } => {
                        scene.button("−", Tone::Cyan, Act::Setting(key.clone(), Change::Step(-1)));
                        scene.text(&format!(" {} ", row.text()), theme::tone(tone));
                        scene.button("+", Tone::Cyan, Act::Setting(key.clone(), Change::Step(1)));
                    }
                }
                if row.is_overridden {
                    scene.text(&format!("  set by you, default {}  ", row.default_text()), theme::dim());
                    scene.button("reset", Tone::Gray, Act::Setting(key.clone(), Change::Reset));
                } else {
                    scene.text("  default", theme::dim());
                }
                scene.end_line();
                scene.indent(3);
                scene.pad_to(3);
                look::flow(scene, &[look::dim(row.def.about)]);
                scene.indent(0);
                scene.end_line();
            });
        }
        scene.block(Act::Settings, |scene| {
            scene.pad_to(1);
            scene.button("✕ close", Tone::Gray, Act::Settings);
        });
    });
}

/// `,` or the button: the panel opens or closes.
pub fn toggle(app: &mut Overview) -> Flow {
    app.folds.is_settings_open = !app.folds.is_settings_open;
    Flow::Redraw
}

/// What a click on a row does: the new value kept, the journal told, the
/// line in the band; the next local read shows it.
pub fn change(app: &mut Overview, key: &str, change: &Change) -> Flow {
    let (Some(name), Some(model)) = (app.orchestration_name(), app.orchestration()) else {
        return Flow::Same;
    };
    let Some(row) = model.settings.iter().find(|row| row.def.key == key) else {
        return Flow::Same;
    };
    let Some(value) = next_value(row, change) else {
        return Flow::Same;
    };
    match breadboard_core::settings::set(&app.actor.main, &name, key, value, now_ms()) {
        Ok(line) => app.actor.log(Paint::Green, format!("settings: {line}")),
        Err(error) => app.actor.log(Paint::Red, format!("settings: not changed: {error}")),
    }
    // Read again at once: the row shows its new value without waiting for the poll.
    if let Some(wake) = &app.wake_local {
        let _ = wake.try_send(());
    }
    Flow::Redraw
}

/// The value `change` sets `row` to: Some(None) back to its default, None
/// for no change (a number at its end).
fn next_value(row: &Row, change: &Change) -> Option<Option<Value>> {
    match (change, row.def.kind) {
        (Change::Reset, _) => Some(None),
        (Change::Flip, Kind::Switch) => Some(Some(json!(row.value.as_bool() != Some(true)))),
        (Change::Step(by), Kind::Count { min, max, .. }) => {
            let now = row.value.as_i64().unwrap_or(min);
            let next = (now + by).clamp(min, max);
            (next != now).then_some(Some(json!(next)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::settings::{DEFS, def_of};

    fn row(key: &str, value: Value) -> Row {
        Row {
            def: def_of(key).unwrap(),
            value: value.clone(),
            default: value,
            is_overridden: false,
        }
    }

    #[test]
    fn a_click_flips_steps_or_resets() {
        let switch = row(DEFS[1].key, json!(true));
        assert_eq!(next_value(&switch, &Change::Flip), Some(Some(json!(false))));
        assert_eq!(next_value(&switch, &Change::Reset), Some(None));
        assert_eq!(next_value(&switch, &Change::Step(1)), None);
        let tasks = row("ship_after_tasks", json!(5));
        assert_eq!(next_value(&tasks, &Change::Step(1)), Some(Some(json!(6))));
        assert_eq!(next_value(&tasks, &Change::Step(-1)), Some(Some(json!(4))));
        let low = row("ship_after_tasks", json!(1));
        assert_eq!(next_value(&low, &Change::Step(-1)), None, "never below its least");
    }
}
