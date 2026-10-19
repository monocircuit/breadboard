//! BATCH (#411), or EPIC for an epic branch of before (#350): the head row
//! of an orchestration that works on its product's batch: the branch, how
//! far develop has gone on since it, the tasks in it, its last pipeline and
//! its MR into develop.

use super::{Act, look};
use crate::tui::Page;
use breadboard_core::model::orchestration::OrchestrationModel;

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel) {
    let Some(row) = &model.epic else { return };
    let parts: Vec<_> = row.iter().map(|part| look::spans(part)).collect();
    let is_batch = row
        .first()
        .and_then(|part| part.first())
        .is_some_and(|branch| breadboard_core::batch::is_batch(&branch.text));
    page.section(if is_batch { "BATCH" } else { "EPIC" }, |scene| {
        let room = scene.width();
        look::whole(scene, &parts, room);
    });
}
