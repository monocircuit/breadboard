//! NEXT (#278, next.mjs): the answer to "what now?" on top of the
//! orchestration while the rest scrolls: the first action not answered as
//! the block ACTIONS draws, else the first row of UP NEXT, else the merge
//! that is ready, else one dim line of what goes on. The plan's why under a merge.

use super::{Act, Ctx, actions, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Page, Scene, Section};
use breadboard_core::model::next::once_text;
use breadboard_core::model::orchestration::{GITLAB_PENDING, NextBand, OrchestrationModel};
use ratatui::style::Style;
use ratatui::text::Span;

/// The label's width: what follows it starts under this column.
const LABEL: &str = "NEXT  ";

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    let next = match &model.next {
        Ok(next) => next,
        Err(why) => return page.failed(Section::new("NEXT").sticky(), why),
    };
    let pick = actions::pick_of(model, page.picked());
    page.section(Section::new("NEXT").sticky().plain(), |scene| {
        let label = vec![look::bold("NEXT"), Span::raw("  ")];
        let under = || vec![Span::raw(" ".repeat(LABEL.len()))];
        match &next.band {
            // GitLab failed before its first answer: nothing comes by waiting, and NEXT can't tell.
            NextBand::Nothing(text) if text == GITLAB_PENDING && ctx.gitlab.error.is_some() => look::say(
                scene,
                "what is for you can't be told: GitLab didn't answer",
                theme::dim(),
            ),
            NextBand::Nothing(text) => look::say(scene, text, theme::dim()),
            NextBand::Action(item) => {
                // Its digits are keys while nothing is picked with Tab or it is: never two blocks' at once.
                let is_keyed = pick.keys(item, true);
                look::gutter(scene, label, under(), |scene| {
                    actions::action_block(scene, item, ctx, is_keyed)
                });
                // Off the block: a click on the hint does nothing.
                let hint = match &pick {
                    actions::Pick::Action { option, .. } if is_keyed => actions::picked_hint(item, option.as_deref()),
                    _ if is_keyed => actions::hint(item),
                    _ => vec![],
                };
                if !hint.is_empty() {
                    look::gutter(scene, under(), under(), |scene| {
                        let room = scene.width();
                        look::whole(scene, &hint, room);
                    });
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
