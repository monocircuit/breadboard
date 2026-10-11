//! EPIC (#350): the head row of an orchestration that works on its own epic
//! branch: the branch, how far develop has gone on since it, the tasks in it,
//! its last pipeline and its milestone MR into develop.

use super::{Act, look};
use crate::tui::Page;
use breadboard_core::model::orchestration::OrchestrationModel;

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel) {
    let Some(row) = &model.epic else { return };
    let parts: Vec<_> = row.iter().map(|part| look::spans(part)).collect();
    page.section("EPIC", |scene| {
        let room = scene.width();
        look::whole(scene, &parts, room);
    });
}
