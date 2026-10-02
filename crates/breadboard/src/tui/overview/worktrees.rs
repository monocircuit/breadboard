//! WORKTREES of an orchestration (tree.mjs): a folder tree under `⇶ <name>`,
//! a node per issue with its number, its name in herdr's pink, where it
//! stands at the right; its facts below with who works in it (#292) at
//! their right, under the state (on a line of its own when both don't fit),
//! then its title, wrapped under the name (#304), the stems of the tree in front of every
//! line. The queue and the done ones fold (#259); a click anywhere on a node
//! goes to its worktree's space, else to its issue's page; hover lights the
//! whole node (#296).

use super::{Act, Ctx, Fold, look};
use crate::tui::scene::width_of;
use crate::tui::theme;
use crate::tui::{Page, Scene};
use breadboard_core::model::orchestration::OrchestrationModel;
use breadboard_core::model::order::Go;
use breadboard_core::model::tree::{AgentLine, Node, Worktrees};
use breadboard_core::model::{self, Paint};
use ratatui::text::Span;

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    let worktrees = match &model.worktrees {
        Ok(worktrees) => worktrees,
        Err(why) => return page.failed("WORKTREES", why),
    };
    page.section("WORKTREES", |scene| tree(scene, worktrees, ctx));
}

fn tree(scene: &mut Scene<Act>, worktrees: &Worktrees, ctx: &Ctx) {
    // The counts in the title; memory running low is no aside: red, on a line of its own.
    let (warnings, counts): (Vec<&model::Span>, Vec<&model::Span>) =
        worktrees.head.iter().partition(|span| span.paint == Paint::Red);
    let note: String = counts.iter().map(|span| span.text.as_str()).collect();
    let note = note.trim().trim_end_matches('·').trim_end();
    if !note.is_empty() {
        scene.note(note);
    }
    for warning in warnings {
        look::say(scene, &warning.text, look::style_of(&warning.paint));
        scene.end_line();
    }
    scene.atom(vec![look::bold(format!("⇶ {}", worktrees.name))]);
    scene.atom_gap(vec![look::dim(&worktrees.record)], 2);
    scene.end_line();
    let tree = &worktrees.tree;
    let has_queue = !tree.queued.is_empty();
    let has_removed = !tree.removed.is_empty();
    let is_empty = tree.live.is_empty() && !has_queue && !has_removed;
    let stem = vec![look::dim(if is_empty { "   " } else { "│  " })];
    look::gutter(scene, stem.clone(), stem, |scene| {
        look::say(scene, &worktrees.reuse, theme::dim());
    });
    let digits = tree
        .live
        .iter()
        .chain(&tree.queued)
        .chain(&tree.removed)
        .map(|node| node.place.to_string().len())
        .max()
        .unwrap_or(1);
    for (i, node) in tree.live.iter().enumerate() {
        let is_last = i + 1 == tree.live.len() && !has_queue && !has_removed;
        node_lines(scene, node, is_last, "", digits);
    }
    if has_queue {
        let is_last = !has_removed;
        let is_folded = ctx.folds.is_queued_folded;
        fold_line(
            scene,
            Fold::Queued,
            is_last,
            is_folded,
            &format!("queued ({}) · in the order they start", tree.queued.len()),
            ctx,
        );
        if !is_folded {
            let prefix = if is_last { "   " } else { "│  " };
            for (i, node) in tree.queued.iter().enumerate() {
                node_lines(scene, node, i + 1 == tree.queued.len(), prefix, digits);
            }
        }
    }
    if has_removed {
        let is_folded = ctx.folds.removed.unwrap_or(worktrees.is_removed_folded);
        fold_line(
            scene,
            Fold::Removed,
            true,
            is_folded,
            &format!("done and removed ({})", tree.removed.len()),
            ctx,
        );
        if !is_folded {
            for (i, node) in tree.removed.iter().enumerate() {
                node_lines(scene, node, i + 1 == tree.removed.len(), "   ", digits);
            }
        }
    }
}

/// `└─ ▸ done and removed (42) · click to show`: a click folds or shows what is under it.
fn fold_line(scene: &mut Scene<Act>, fold: Fold, is_last: bool, is_folded: bool, text: &str, ctx: &Ctx) {
    let glyph = if is_last { "└─ " } else { "├─ " };
    let mark = if is_folded { "▸" } else { "▾" };
    let hint = match (ctx.is_once, is_folded) {
        (true, _) => "",
        (false, true) => " · click to show",
        (false, false) => " · click to fold",
    };
    scene.block(Act::Fold(fold), |scene| {
        let line = format!("{mark} {text}{hint}");
        look::gutter(scene, vec![look::dim(glyph)], vec![look::dim("   ")], |scene| {
            look::say(scene, &line, theme::dim());
        });
    });
}

/// A node: its glyph and number, its name and state; its facts and title under the name.
fn node_lines(scene: &mut Scene<Act>, node: &Node, is_last: bool, prefix: &str, digits: usize) {
    let glyph = if is_last { "└─ " } else { "├─ " };
    let stem = if is_last { "   " } else { "│  " };
    let place = format!("{:>digits$}. ", node.place);
    let first = vec![look::dim(format!("{prefix}{glyph}")), look::dim(&place)];
    let rest = vec![
        look::dim(format!("{prefix}{stem}")),
        Span::raw(" ".repeat(place.chars().count())),
    ];
    let draw = |scene: &mut Scene<Act>| {
        look::gutter(scene, first, rest, |scene| {
            let style = if node.has_tree {
                look::pink().add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                look::pink()
            };
            scene.atom(vec![look::span(&node.name, style)]);
            right(
                scene,
                vec![look::span(&node.state.text, look::style_of(&node.state.paint))],
            );
            scene.end_line();
            // Who works in it at the right of the facts, under the state; on a line of its own before them when both don't fit.
            let agent = node.agent.as_ref().map(agent_spans);
            let is_beside = agent
                .as_ref()
                .is_some_and(|agent| facts_width(&node.facts) + 2 + width_of(agent) <= scene.width());
            if let Some(agent) = agent.clone().filter(|_| !is_beside) {
                look::flow(scene, &agent);
                scene.end_line();
            }
            look::facts(scene, &node.facts);
            if let Some(agent) = agent.filter(|_| is_beside) {
                scene.right(agent);
            }
            scene.end_line();
            look::say(scene, &node.title, theme::dim());
            scene.end_line();
        });
    };
    if node.go == Go::Nothing {
        draw(scene);
    } else {
        scene.block(Act::Go(node.go.clone()), draw);
    }
}

/// `● Mira Albrecht · 3 issues · 2 MRs merged` (#292): the dot in the colour
/// of whom the agent waits for, its name, what it did in this orchestration.
fn agent_spans(agent: &AgentLine) -> Vec<Span<'static>> {
    vec![
        look::span("●", look::style_of(&agent.dot)),
        Span::raw(" "),
        look::bold(&agent.name),
        Span::raw(" "),
        look::dim(agent.counts()),
    ]
}

/// The columns the facts take on one line, as `look::facts` sets them.
fn facts_width(facts: &[model::Fact]) -> u16 {
    let parts: Vec<u16> = facts
        .iter()
        .filter(|fact| !model::text_of(fact).trim().is_empty())
        .map(|fact| width_of(&look::spans(fact)))
        .collect();
    let gaps = parts.len().saturating_sub(1) as u16;
    parts.iter().sum::<u16>() + gaps * 3
}

/// `spans` at the right edge: beside what the line has when it fits, else on
/// the next line; wider than a line, its words on lines of their own, each
/// at the right edge (tree.mjs: a state that doesn't fit stays right-aligned).
fn right(scene: &mut Scene<Act>, spans: Vec<Span<'static>>) {
    if width_of(&spans) <= scene.width() {
        scene.right(spans);
        return;
    }
    scene.end_line();
    let mut line: Vec<Span<'static>> = Vec::new();
    for word in look::words_of(&spans) {
        let gap = u16::from(!line.is_empty());
        if !line.is_empty() && width_of(&line) + gap + width_of(&word) > scene.width() {
            scene.right(std::mem::take(&mut line));
            scene.end_line();
        }
        if !line.is_empty() {
            line.push(Span::raw(" "));
        }
        line.extend(word);
    }
    if !line.is_empty() {
        scene.right(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::overview::{Folds, GitlabState};
    use breadboard_core::model::Tone;
    use breadboard_core::model::tree::{Lifecycle, TreeOf};
    use std::path::PathBuf;

    fn node(place: usize, name: &str, key: &'static str, has_tree: bool) -> Node {
        Node {
            index: place,
            number: Some(place as i64 + 300),
            state: Lifecycle {
                key,
                text: if key == "queued" {
                    "start: waits for an agent".into()
                } else {
                    "waits for the pipeline".into()
                },
                paint: Paint::Wait(Tone::Gitlab),
                tone: Some(Tone::Gitlab),
            },
            place,
            name: name.into(),
            has_tree,
            agent: has_tree.then(|| AgentLine {
                name: "Mira Albrecht".into(),
                dot: Paint::Wait(Tone::Gitlab),
                issues: 3,
                merged: 2,
            }),
            facts: vec![
                vec![model::Span::plain(format!("#{} open", place + 300))],
                vec![model::Span::plain("1 ahead")],
            ],
            title: format!("the title of #{}", place + 300),
            go: if has_tree {
                Go::FocusSpace(PathBuf::from(format!("/w/{name}")))
            } else {
                Go::Nothing
            },
        }
    }

    fn worktrees(queued: usize, removed: usize) -> Worktrees {
        Worktrees {
            name: "breadboard".into(),
            record: ".claude/orchestrations/breadboard.json".into(),
            reuse: "agents 2 · issues 3 · reused 0 · handovers failed 0".into(),
            tree: TreeOf {
                live: vec![
                    node(1, "303-every-button", "waitsForMr", true),
                    node(2, "305-refresh", "waitsForMr", true),
                ],
                queued: (0..queued).map(|i| node(10 + i, "q", "queued", false)).collect(),
                removed: (0..removed).map(|i| node(20 + i, "gone", "removed", false)).collect(),
            },
            head: vec![
                model::Span::dim("2 wait for GitLab"),
                model::Span::dim(" · "),
                model::Span::new("0.9 GB free: fresh agents on the shared stack", Paint::Red),
            ],
            is_removed_folded: removed > 3,
        }
    }

    fn draw(worktrees: &Worktrees, folds: Folds, width: u16) -> Scene<Act> {
        let gitlab = GitlabState::default();
        let ctx = Ctx {
            is_once: false,
            folds,
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor: super::super::actor::idle(),
            words: super::super::actor::view::no_words(),
        };
        let mut scene = Scene::new(width);
        tree(&mut scene, worktrees, &ctx);
        scene.finish();
        scene
    }

    fn text(scene: &Scene<Act>) -> Vec<String> {
        scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .map(|line| line.trim_end().to_string())
            .collect()
    }

    #[test]
    fn a_node_is_its_name_and_state_its_facts_and_agent_and_title_under_its_stem() {
        let scene = draw(&worktrees(0, 0), Folds::default(), 72);
        assert_eq!(scene.note.as_deref(), Some("2 wait for GitLab"));
        assert_eq!(
            text(&scene),
            [
                "0.9 GB free: fresh agents on the shared stack",
                "⇶ breadboard  .claude/orchestrations/breadboard.json",
                "│  agents 2 · issues 3 · reused 0 · handovers failed 0",
                "├─ 1. 303-every-button                            waits for the pipeline",
                "│     #301 open · 1 ahead      ● Mira Albrecht · 3 issues · 2 MRs merged",
                "│     the title of #301",
                "└─ 2. 305-refresh                                 waits for the pipeline",
                "      #302 open · 1 ahead      ● Mira Albrecht · 3 issues · 2 MRs merged",
                "      the title of #302",
            ]
        );
    }

    #[test]
    fn an_agent_that_does_not_fit_beside_the_facts_goes_on_its_own_line_before_them() {
        let lines = text(&draw(&worktrees(0, 0), Folds::default(), 60));
        assert_eq!(
            lines[3..7],
            [
                "├─ 1. 303-every-button                waits for the pipeline",
                "│     ● Mira Albrecht · 3 issues · 2 MRs merged",
                "│     #301 open · 1 ahead",
                "│     the title of #301",
            ]
        );
    }

    #[test]
    fn a_state_wider_than_the_line_stays_at_the_right_edge_on_lines_of_its_own() {
        let mut worktrees = worktrees(0, 0);
        worktrees.tree.live[0].state.text =
            "start: → Elif Kowalczyk: its handover failed: wt:orchestrate: not built yet; a click tries again".into();
        let lines = text(&draw(&worktrees, Folds::default(), 60));
        assert_eq!(
            lines[3..6],
            [
                "├─ 1. 303-every-button",
                "│              start: → Elif Kowalczyk: its handover failed:",
                "│         wt:orchestrate: not built yet; a click tries again",
            ]
        );
        for line in &lines {
            assert!(line.chars().count() <= 60, "{line}");
        }
    }

    #[test]
    fn a_click_anywhere_on_a_node_goes_to_it_and_lights_the_whole_node() {
        let scene = draw(&worktrees(0, 0), Folds::default(), 72);
        let blocks: Vec<_> = scene
            .regions
            .iter()
            .map(|region| {
                (
                    region.area.y,
                    region.area.height,
                    region.area.width,
                    region.action().clone(),
                )
            })
            .collect();
        assert_eq!(
            blocks,
            [
                (3, 3, 72, Act::Go(Go::FocusSpace("/w/303-every-button".into()))),
                (6, 3, 72, Act::Go(Go::FocusSpace("/w/305-refresh".into()))),
            ]
        );
    }

    #[test]
    fn the_queue_and_the_removed_fold_and_say_how_to_unfold() {
        let shown = draw(&worktrees(2, 5), Folds::default(), 72);
        let lines = text(&shown);
        assert!(lines.contains(&"├─ ▾ queued (2) · in the order they start · click to fold".to_string()));
        assert!(
            lines.contains(&"│  ├─ 10. q                                    start: waits for an agent".to_string())
        );
        // Five removed: folded by default, past FOLD_OVER.
        assert_eq!(lines.last().unwrap(), "└─ ▸ done and removed (5) · click to show");
        assert!(
            shown
                .regions
                .iter()
                .any(|region| region.action() == &Act::Fold(Fold::Removed))
        );
        let folds = Folds {
            removed: Some(false),
            is_queued_folded: true,
            ..Folds::default()
        };
        let lines = text(&draw(&worktrees(2, 5), folds, 72));
        assert!(lines.contains(&"├─ ▸ queued (2) · in the order they start · click to show".to_string()));
        assert!(!lines.iter().any(|line| line.contains("10. q")));
        assert!(lines.iter().any(|line| line.starts_with("   ├─ 20. gone")));
    }

    #[test]
    fn nothing_runs_past_the_edge_at_any_width() {
        for width in [20, 40, 80] {
            let scene = draw(&worktrees(2, 2), Folds::default(), width);
            for line in &scene.lines {
                assert!(line.width() <= width as usize, "{width}: {line:?}");
            }
        }
    }
}
