//! The end of an orchestration's page (end.mjs, render-orchestration.mjs):
//! the button that ends it (grey with what is left while worktrees or queued
//! issues remain, two clicks once none is), then how to reach an agent.

use super::{Act, Ctx, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Page, Section};
use breadboard_core::model::orchestration::OrchestrationModel;
use breadboard_core::model::order::Go;

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    page.section(Section::new("END").plain(), |scene| {
        scene.pad_to(1);
        scene.indent(2);
        if !model.left.is_empty() {
            scene.inactive("✕ end orchestration");
            let what = if model.left.len() == 1 { "worktree" } else { "worktrees" };
            look::say(
                scene,
                &format!("not yet, {what} left: {}", model.left.join(", ")),
                theme::dim(),
            );
            return;
        }
        let end = Act::Go(Go::End);
        scene.block(end.clone(), |scene| {
            scene.pad_to(1);
            // Armed, ending or refused: the actor says so in place of the hint.
            if super::actor::view::button(scene, ctx, "✕ end orchestration", Tone::Red, end) {
                return;
            }
            let hint = if ctx.is_once {
                format!(
                    "{run} wt:orchestrate {} --end",
                    model.name,
                    run = breadboard_core::config::run_word()
                )
            } else {
                "no worktree left: its space, record and checkout go".into()
            };
            look::say(scene, &hint, theme::dim());
        });
    });
    page.section(Section::new("FOOTER").plain(), |scene| {
        scene.text(
            &breadboard_core::config::in_repo(
                "SendMessage(to: \"<feature>\") reaches an agent · pnpm overview (everything)",
            ),
            theme::dim(),
        );
    });
}
