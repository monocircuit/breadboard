//! BATCH (#411), or EPIC for an epic branch of before (#350): the head row
//! of an orchestration that works on its product's batch: the branch, how
//! far develop has gone on since it, its last pipeline and its MR into
//! develop. A batch's (#414): `ship batch` beside it while tasks are in it
//! and it has no MR into develop. Under it (#491): the batches shipped
//! before it, a red one named apart from the batch's own state with its
//! tasks the likeliest first and `fix batch` (#413); then its tasks against
//! the ship setting, the task MRs merged into it (a click opens one, a click
//! on the count folds them) and those still open into it.

use super::{Act, Ctx, Fold, look, work};
use crate::tui::theme;
use crate::tui::theme::Tone;
use crate::tui::{Page, Scene};
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
            if let Some(replaces) = &batch.replaces {
                scene.atom(vec![look::dim(format!(" {replaces}"))]);
            }
        }
        for line in &batch.shipped {
            scene.end_line();
            look::flow(scene, &look::spans(line));
        }
        if let Some(red) = &batch.red {
            scene.end_line();
            look::flow(scene, &look::spans(red));
            if batch.can_fix {
                scene.atom(vec![look::dim(" ·")]);
                scene.button("fix batch", Tone::Red, Act::Go(Go::FixBatch(batch.product.clone())));
            } else if let Some(number) = batch.fix {
                scene.atom(vec![look::dim(format!(" · #{number} fixes it"))]);
            }
        }
        // A red batch's suspects (#413), always shown: the likeliest first.
        if !batch.suspects.is_empty() {
            scene.end_line();
            scene.atom(vec![look::dim("suspects, the likeliest first:")]);
            for line in &batch.suspects {
                scene.end_line();
                scene.atom(vec![look::dim("  ")]);
                look::flow(scene, &look::spans(line));
            }
        }
        scene.end_line();
        if batch.tasks.is_empty() {
            look::say(scene, &batch.count, theme::dim());
        } else {
            // Shown until folded (#491): folded, the user never saw them.
            let is_folded = !ctx.is_once && ctx.folds.is_batch_folded;
            work::fold_line(scene, Fold::Batch, is_folded, &batch.count, ctx);
            if !is_folded {
                for line in &batch.tasks {
                    scene.end_line();
                    let draw = |scene: &mut Scene<Act>| {
                        scene.atom(vec![look::dim("  ")]);
                        look::flow(scene, &look::spans(line));
                    };
                    // A click opens the MR, as an MR's row does.
                    match line.first().and_then(|first| first.url.clone()) {
                        Some(url) => scene.block(Act::Go(Go::Show { url, dir: None }), draw),
                        None => draw(scene),
                    }
                }
            }
        }
        if !batch.waiting.is_empty() {
            scene.end_line();
            scene.atom(vec![look::dim("  ")]);
            look::flow(scene, &look::spans(&batch.waiting));
        }
    });
}
