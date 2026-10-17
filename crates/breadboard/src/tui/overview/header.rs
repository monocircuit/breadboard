//! The top of a view (boxes.mjs, chat.mjs, gitlab-status.mjs): the title band
//! with the view's name and the age of GitLab's data; in an orchestration the
//! orchestrator's chat row (#287), the CI job watcher's line (#379) and the
//! line of a GitLab read that failed, all on top with NEXT while the rest scrolls.

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
    // The orchestrator's context (#382): `orchestrator 62 % · restart at a quiet point`.
    if let Some(Model::Orchestration(model)) = model
        && let Some(band) = model.orchestrator.band()
    {
        let style = if model.orchestrator.is_due() {
            theme::tone(Tone::Yellow)
        } else {
            quiet
        };
        status.push(Span::styled(format!("{band} · "), style));
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
    // The Claude usage limit holds the runs (#393): until when.
    if let Some(Model::Orchestration(model)) = model
        && let Some(usage) = &model.usage
    {
        status.push(Span::styled(usage.clone(), theme::tone(Tone::Red)));
        status.push(Span::styled(" · ", quiet));
    }
    status.push(Span::styled(ctx.gitlab.fresh(), quiet));
    // The runs paused (#397), first: `agents paused since 13:05 · 3 lines waiting`.
    if let Some(Model::Orchestration(model)) = model
        && let Some(paused) = &model.paused
    {
        status.splice(
            0..0,
            [
                Span::styled(paused.clone(), theme::tone(Tone::Red)),
                Span::styled(" · ", quiet),
            ],
        );
    }
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
            // Its fresh start at a quiet point (#382), by a click unless the overview makes it itself.
            let fresh = &model.orchestrator;
            let is_running = ctx.actor.fresh.as_ref().is_some_and(|run| run.is_running);
            if fresh.is_ready() && (!fresh.is_auto || is_running) {
                scene.block(Act::Fresh, |scene| {
                    scene.pad_to(1);
                    scene.indent(2);
                    if !super::actor::view::button(scene, ctx, "↻ fresh start", Tone::Yellow, Act::Fresh) {
                        look::flow(
                            scene,
                            &[look::dim(
                                "a quiet point: the chat writes its journal entry, then a fresh one starts from the journal; the old stays in /resume",
                            )],
                        );
                    }
                });
            }
            pause_toggle(scene, model, ctx);
        });
    } else if let Some(paused) = &model.paused {
        page.section(Section::new("PAUSED").sticky().plain(), |scene| {
            look::say(scene, &format!("⏸ {paused}"), theme::tone(Tone::Red));
        });
    }
    let journal = &model.orchestrator.journal;
    if !journal.is_empty() {
        page.section(Section::new("JOURNAL"), |scene| {
            for entry in journal {
                look::say(scene, entry, theme::dim());
            }
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
    // The Claude usage limit holds them (#393): nothing to click, they go on after the reset.
    if let Some(usage) = &model.usage {
        page.section(Section::new("USAGE").sticky().plain(), |scene| {
            scene.pad_to(1);
            scene.indent(2);
            look::say(
                scene,
                &format!("⏸ {usage}: no new run starts, the queue waits; the stopped runs go on after it"),
                theme::tone(Tone::Red),
            );
        });
    }
    // A CI job that holds pipelines up (#379): under the band, with NEXT on top.
    if let Some(line) = &model.ci_watch {
        page.section(Section::new("CI").sticky().plain(), |scene| {
            look::say(scene, line, theme::tone(Tone::Yellow));
        });
    }
    if let Some(error) = &ctx.gitlab.error {
        page.section(Section::new("GITLAB").sticky().plain(), |scene| {
            look::say(scene, error, theme::tone(Tone::Red));
        });
    }
}

/// The switch of every headless run (#397), under the chat: `⏸ pause
/// agents`, or while paused why nothing runs and `▶ resume agents`.
fn pause_toggle(scene: &mut crate::tui::scene::Scene<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    scene.block(Act::Pause, |scene| {
        scene.pad_to(1);
        scene.indent(2);
        match &model.paused {
            Some(paused) => {
                if !super::actor::view::button(scene, ctx, "▶ resume agents", Tone::Yellow, Act::Pause) {
                    look::flow(scene, &[look::span(format!("⏸ {paused}"), theme::tone(Tone::Red))]);
                }
            }
            None => {
                if !super::actor::view::button(scene, ctx, "⏸ pause agents", Tone::Cyan, Act::Pause) {
                    look::flow(
                        scene,
                        &[look::dim(
                            "no new run starts, the runs in flight finish, the lines wait",
                        )],
                    );
                }
            }
        }
    });
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
