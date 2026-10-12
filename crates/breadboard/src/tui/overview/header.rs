//! The top of a view (boxes.mjs, chat.mjs, gitlab-status.mjs): the title band
//! with the view's name and the age of GitLab's data; in an orchestration the
//! orchestrator's chat row (#287) and the line of a GitLab read that failed,
//! both on top with NEXT while the rest scrolls.

use super::{Act, Ctx, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Header, Page, Section};
use breadboard_core::model::orchestration::{ChatState, OrchestrationModel};
use breadboard_core::model::tree::pipeline_mark;
use breadboard_core::reader::{Model, RemoteOf, View};
use ratatui::style::Style;
use ratatui::text::Span;
use std::path::Path;

/// The band: `⇶ <title>`, `<name>  <branch>` or `platform  develop ✓ passed`;
/// on its right the age of GitLab's data, or its progress; no keys in a print.
pub fn header(view: &View, model: Option<&Model>, remote: Option<&RemoteOf>, ctx: &Ctx) -> Header {
    let base = |dir: &Path| {
        dir.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let title = match (model, view) {
        (Some(Model::Orchestration(model)), _) => model.label.clone(),
        (_, View::Orchestration(name)) => format!("⇶ {name}"),
        (Some(Model::Worktree(model)), View::Worktree(dir)) => match &model.local.tree {
            Some(tree) => format!("{}\u{a0} {}", tree.name, tree.branch),
            None => base(dir),
        },
        (_, View::Worktree(dir)) => base(dir),
        (_, View::Everything) => match remote {
            Some(RemoteOf::Everything(remote)) => {
                format!(
                    "{}\u{a0} {} {}",
                    ctx.main_name,
                    breadboard_core::git::default_branch(),
                    pipeline_mark(remote.develop.as_ref()).text
                )
            }
            _ => ctx.main_name.to_string(),
        },
    };
    let quiet = Style::new().fg(theme::BAND_QUIET);
    let mut status = Vec::new();
    // The runner's load while the CI queue runs (#357).
    if let Some(Model::Orchestration(model)) = model
        && let Some(load) = &model.ci_load
    {
        status.push(Span::styled(format!("{load} · "), quiet));
    }
    status.push(Span::styled(ctx.gitlab.fresh(), quiet));
    let mut header = Header::new(title).status(status);
    if ctx.is_once {
        header.keys = String::new();
    }
    // What the overview did by itself or on a click, for a minute: the actor's.
    super::actor::view::log(header, ctx)
}

/// The orchestration's rows on top: its chat (live, inside herdr) and why GitLab didn't answer.
pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    if let Some(chat) = model.chat.as_ref().filter(|_| !ctx.is_once) {
        page.section(Section::new("CHAT").sticky().plain(), |scene| {
            scene.block(Act::Chat, |scene| {
                scene.pad_to(1);
                let (label, tone) = if chat.state == ChatState::Alive {
                    ("◉ orchestrator chat", Tone::Green)
                } else {
                    ("▶ open orchestrator chat", Tone::Cyan)
                };
                scene.indent(2);
                // What the last click did, for a while: the actor's.
                if !super::actor::view::button(scene, ctx, label, tone, Act::Chat) {
                    look::flow(scene, &[look::dim(chat.text())]);
                }
            });
        });
    }
    if let Some(error) = &ctx.gitlab.error {
        page.section(Section::new("GITLAB").sticky().plain(), |scene| {
            look::say(scene, error, theme::tone(Tone::Red));
        });
    }
}
