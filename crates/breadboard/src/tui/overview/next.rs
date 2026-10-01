//! NEXT (#278, next.mjs): the answer to "what now?" on top of the
//! orchestration while the rest scrolls: the first action not answered as
//! the block ACTIONS draws, else the first row of UP NEXT, else the merge
//! that is ready, else one dim line of what goes on. The plan's why under a merge.

use super::{Act, Ctx, actions, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Page, Scene, Section};
use breadboard_core::model::next::once_text;
use breadboard_core::model::orchestration::{NextBand, OrchestrationModel};
use ratatui::style::Style;
use ratatui::text::Span;

/// The label's width: what follows it starts under this column.
const LABEL: &str = "NEXT  ";

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    let next = match &model.next {
        Ok(next) => next,
        Err(why) => return page.failed(Section::new("NEXT").sticky(), why),
    };
    page.section(Section::new("NEXT").sticky().plain(), |scene| {
        let label = vec![look::bold("NEXT"), Span::raw("  ")];
        let under = || vec![Span::raw(" ".repeat(LABEL.len()))];
        match &next.band {
            NextBand::Nothing(text) => look::say(scene, text, theme::dim()),
            NextBand::Action(item) => {
                look::gutter(scene, label, under(), |scene| {
                    actions::action_block(scene, item, ctx, true)
                });
                // Off the block: a click on the hint does nothing.
                if let Some(hint) = actions::key_hint(item) {
                    look::gutter(scene, under(), under(), |scene| look::say(scene, &hint, theme::dim()));
                }
            }
            NextBand::UpNext(item) => {
                look::gutter(scene, label, under(), |scene| actions::next_row(scene, item, ctx));
            }
            NextBand::Merge(merge) => {
                look::gutter(scene, label, under(), |scene| {
                    scene.block(Act::Open(merge.url.clone()), |scene| {
                        if merge.is_ready {
                            scene.text("ready to merge", theme::tone(Tone::Green));
                        } else {
                            scene.text("next merge", theme::dim());
                        }
                        scene.atom_gap(vec![look::span(format!("!{}", merge.iid), Style::new())], 2);
                        scene.atom(vec![look::bold(&merge.feature)]);
                        if !merge.is_ready && !merge.hold_up.is_empty() {
                            look::say(scene, &format!("once {}", once_text(&merge.hold_up)), theme::dim());
                        }
                        if merge.more_ready > 0 {
                            look::say(scene, &format!("· {} more ready", merge.more_ready), theme::dim());
                        }
                    });
                });
            }
        }
        if let Some(why) = &next.why {
            look::gutter(scene, under(), under(), |scene| look::say(scene, why, theme::dim()));
        }
    });
}

/// An orchestration without issues: how to give it one, nothing else.
pub fn empty(page: &mut Page<Act>, model: &OrchestrationModel) {
    page.section(Section::new("EMPTY").plain(), |scene: &mut Scene<Act>| {
        let text = format!(
            "no issues recorded for {}: pnpm wt:orchestrate {} <issue>…",
            model.label, model.name
        );
        look::say(scene, &text, theme::dim());
    });
}
