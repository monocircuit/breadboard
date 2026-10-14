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
    // What the agents spent (#372), red once the daily limit holds the runs.
    if let Some(Model::Orchestration(model)) = model
        && let Some(spent) = &model.spent
    {
        let style = if model.held.is_some() {
            theme::tone(Tone::Red)
        } else {
            quiet
        };
        status.push(Span::styled(spent.clone(), style));
        status.push(Span::styled(" · ", quiet));
    }
    status.push(Span::styled(ctx.gitlab.fresh(), quiet));
    // How the main checkout followed develop (#348): in the views that look at all of it.
    if let Some((text, is_warning)) = ctx.develop.filter(|_| !matches!(view, View::Worktree(_))) {
        let style = if *is_warning { theme::tone(Tone::Yellow) } else { quiet };
        status.splice(0..0, [Span::styled(text.clone(), style), Span::styled(" · ", quiet)]);
    }
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
    // The daily limit holds the headless runs (#372): why, and the click that lifts it.
    if let Some(held) = &model.held {
        page.section(Section::new("LIMIT").sticky().plain(), |scene| {
            scene.block(Act::LiftLimit, |scene| {
                scene.pad_to(1);
                scene.indent(2);
                look::say(
                    scene,
                    &format!("⏸ {held}: no new run starts, the queue waits"),
                    theme::tone(Tone::Red),
                );
                if !ctx.is_once {
                    super::actor::view::button(scene, ctx, "▶ continue anyway", Tone::Yellow, Act::LiftLimit);
                    look::flow(scene, &[look::dim("until midnight")]);
                }
            });
        });
    }
    // Its auto mode (#384): a click switches it.
    if !ctx.is_once || model.is_auto {
        page.section(Section::new("AUTO").sticky().plain(), |scene| {
            scene.block(Act::Auto, |scene| {
                scene.pad_to(1);
                scene.indent(2);
                let (label, tone, says) = if model.is_auto {
                    (
                        "● auto mode",
                        Tone::Green,
                        format!(
                            "{} checks and answers the tries · a click switches it off",
                            model.checker
                        ),
                    )
                } else {
                    (
                        "○ auto mode off",
                        Tone::Gray,
                        format!("a click switches it on: {} checks and answers the tries", model.checker),
                    )
                };
                if !super::actor::view::button(scene, ctx, label, tone, Act::Auto) {
                    look::flow(scene, &[look::dim(says)]);
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

#[cfg(test)]
mod tests {
    use super::super::{Folds, GitlabState, actor};
    use super::*;

    fn status_of(view: &View, develop: Option<&(String, bool)>) -> String {
        let gitlab = GitlabState {
            progress: "GitLab …".into(),
            ..GitlabState::default()
        };
        let ctx = Ctx {
            is_once: true,
            folds: Folds::default(),
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor: actor::idle(),
            words: actor::view::no_words(),
            develop,
        };
        header(view, None, None, &ctx)
            .status
            .iter()
            .map(|span| span.content.to_string())
            .collect()
    }

    #[test]
    fn says_how_develop_was_updated_beside_gitlab_in_the_views_of_all() {
        let updated = ("develop updated 17:42 · +4 commits (!331, !332…)".to_string(), false);
        assert_eq!(
            status_of(&View::Everything, Some(&updated)),
            "develop updated 17:42 · +4 commits (!331, !332…) · GitLab …"
        );
        let skipped = (
            "develop: not updated (↓2), local changes in README.md".to_string(),
            true,
        );
        assert!(status_of(&View::Orchestration("b".into()), Some(&skipped)).starts_with("develop: not updated"));
        // A worktree's view is about its branch.
        assert_eq!(status_of(&View::Worktree("/w".into()), Some(&updated)), "GitLab …");
        assert_eq!(status_of(&View::Everything, None), "GitLab …");
    }
}
