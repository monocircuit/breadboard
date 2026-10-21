//! BATCH (#411), or EPIC for an epic branch of before (#350): the head row
//! of an orchestration that works on its product's batch: the branch, how
//! far develop has gone on since it, the tasks in it, its last pipeline and
//! its MR into develop. A batch's (#414): `ship batch` beside it while
//! tasks are in it and it has no MR into develop, and the task MRs merged
//! into it in a fold.

use super::{Act, Ctx, Fold, look, work};
use crate::tui::Page;
use crate::tui::theme::Tone;
use breadboard_core::model::orchestration::OrchestrationModel;
use breadboard_core::model::order::Go;

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    let Some(row) = &model.epic else { return };
    let parts: Vec<_> = row.iter().map(|part| look::spans(part)).collect();
    let is_batch = row
        .first()
        .and_then(|part| part.first())
        .is_some_and(|branch| breadboard_core::batch::is_batch(&branch.text));
    page.section(if is_batch { "BATCH" } else { "EPIC" }, |scene| {
        let room = scene.width();
        look::whole(scene, &parts, room);
        let Some(batch) = &model.batch else { return };
        if batch.can_ship {
            scene.atom(vec![look::dim(" ·")]);
            scene.button("ship batch", Tone::Green, Act::Go(Go::ShipBatch(batch.product.clone())));
        }
        if batch.tasks.is_empty() {
            return;
        }
        let count = batch.tasks.len();
        let text = format!("{count} task {} merged into it", if count == 1 { "MR" } else { "MRs" });
        // Folded until opened; a print shows them.
        let is_folded = !ctx.is_once && !ctx.folds.is_batch_shown;
        scene.end_line();
        work::fold_line(scene, Fold::Batch, is_folded, &text, ctx);
        if !is_folded {
            for line in &batch.tasks {
                scene.end_line();
                scene.atom(vec![look::dim("  ")]);
                look::flow(scene, &look::spans(line));
            }
        }
    });
}
