//! WORKTREES of an orchestration (tree.mjs): a folder tree under `⇶ <name>`,
//! a node per issue with its number, its name in herdr's pink, its app and
//! stack in a column after the names (#332), where it stands at the right;
//! its facts below (how far develop moved on among them, #332) with who works
//! in it (#292) at their right, under the state (on a line of its own when
//! both don't fit); where its work collides with develop and the other
//! worktrees (#332); then its title, wrapped under the name (#304), the stems
//! of the tree in front of every line. The queue and the done ones fold
//! (#259); a click anywhere on a node goes to its worktree's space, else to
//! its issue's page; hover lights the whole node (#296).

use super::{Act, Ctx, Fold, look};
use crate::tui::scene::width_of;
use crate::tui::theme;
use crate::tui::{Page, Scene};
use breadboard_core::model::diff::Overlap;
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
    let mut note = note.trim().trim_end_matches('·').trim_end().to_string();
    let tree = &worktrees.tree;
    let cells = Cells::of(&tree.live, scene.width());
    if !cells.is_any_env && tree.live.iter().any(|node| node.has_tree) {
        // Why there is no app and no stack: said once, not on every node.
        if !note.is_empty() {
            note.push_str(" · ");
        }
        note.push_str("no .env.local in any");
    }
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
    let has_queue = !tree.queued.is_empty();
    let has_removed = !tree.removed.is_empty();
    let is_empty = tree.live.is_empty() && !has_queue && !has_removed;
    let stem = vec![look::dim(if is_empty { "   " } else { "│  " })];
    look::gutter(scene, stem.clone(), stem, |scene| {
        look::say(scene, &worktrees.reuse, theme::dim());
    });
    let digits = digits_of(worktrees);
    for (i, node) in tree.live.iter().enumerate() {
        let is_last = i + 1 == tree.live.len() && !has_queue && !has_removed;
        node_lines(scene, node, is_last, "", digits, &cells);
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
                node_lines(scene, node, i + 1 == tree.queued.len(), prefix, digits, &cells);
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
                node_lines(scene, node, i + 1 == tree.removed.len(), "   ", digits, &cells);
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

/// The digits of the widest place: the names start in one column.
fn digits_of(worktrees: &Worktrees) -> usize {
    let tree = &worktrees.tree;
    tree.live
        .iter()
        .chain(&tree.queued)
        .chain(&tree.removed)
        .map(|node| node.place.to_string().len())
        .max()
        .unwrap_or(1)
}

/// The columns of the live worktrees' app and stack (#332, lazygit's
/// branches): in a column after the widest name when every node's state
/// still fits beside them, else at the end of each node's facts. Chosen once
/// for all nodes, so they line up and one long state reflows none.
struct Cells {
    /// Whether any worktree has an .env.local: without, there is nothing to show.
    is_any_env: bool,
    /// The widest app cell: the stacks line up after it.
    app: u16,
    /// Where the cells start on the name line (after the node's gutter); None: with the facts.
    column: Option<u16>,
}

impl Cells {
    fn of(live: &[Node], width: u16) -> Cells {
        let with_tree: Vec<&Node> = live.iter().filter(|node| node.has_tree).collect();
        let is_any_env = with_tree.iter().any(|node| node.app.is_some());
        let mut cells = Cells {
            is_any_env,
            app: with_tree
                .iter()
                .map(|node| width_of(&look::spans(&[app_of(node)])))
                .max()
                .unwrap_or(0),
            column: None,
        };
        if !is_any_env {
            return cells;
        }
        let digits = live.iter().map(|node| node.place.to_string().len()).max().unwrap_or(1) as u16;
        // `├─ ` and `NN. ` before the name.
        let room = width.saturating_sub(3 + digits + 2);
        let column = with_tree
            .iter()
            .map(|node| Span::raw(node.name.as_str()).width() as u16)
            .max()
            .unwrap_or(0)
            + 2;
        let is_fitting = with_tree.iter().all(|node| {
            let state = Span::raw(node.state.text.as_str()).width() as u16;
            column + width_of(&cells.spans(node)) + 2 + state <= room
        });
        cells.column = is_fitting.then_some(column);
        cells
    }

    /// `● :3002  +2 running(5)`: the app padded to the widest, the stack after.
    fn spans(&self, node: &Node) -> Vec<Span<'static>> {
        let mut spans = look::spans(&[app_of(node)]);
        let short = self.app.saturating_sub(width_of(&spans)) + 2;
        spans.push(Span::raw(" ".repeat(usize::from(short))));
        spans.extend(look::spans(&[node
            .stack
            .clone()
            .unwrap_or_else(|| model::Span::dim("no .env.local"))]));
        spans
    }

    /// The cells of a node, None where there are none to show.
    fn of_node(&self, node: &Node) -> Option<Vec<Span<'static>>> {
        (self.is_any_env && node.has_tree).then(|| self.spans(node))
    }
}

/// A worktree's app, `—` without its .env.local (render.mjs).
fn app_of(node: &Node) -> model::Span {
    node.app.clone().unwrap_or_else(|| model::Span::dim("—"))
}

/// A node: its glyph and number, its name, app and stack, and state; its
/// facts, where it collides and its title under the name.
fn node_lines(scene: &mut Scene<Act>, node: &Node, is_last: bool, prefix: &str, digits: usize, cells: &Cells) {
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
            let mine = cells.of_node(node);
            if let (Some(spans), Some(column)) = (&mine, cells.column) {
                scene.pad_to(column);
                scene.atom_gap(spans.clone(), 0);
            }
            right(
                scene,
                vec![look::span(&node.state.text, look::style_of(&node.state.paint))],
            );
            scene.end_line();
            // Who works in it at the right of the facts, under the state; on a line of its own before them when both don't fit.
            let agent = node.agent.as_ref().map(agent_spans);
            let mut facts = look::parts(&node.facts);
            // Not beside the name: the app and stack end the facts; where those
            // leave the agent no room, at the right of the agent's own line.
            let mut aside = mine.filter(|_| cells.column.is_none());
            let with_aside = |facts: &[Vec<Span<'static>>], aside: &Option<Vec<Span<'static>>>| {
                parts_width(facts) + aside.as_ref().map_or(0, |aside| 3 + width_of(aside))
            };
            let is_beside = agent
                .as_ref()
                .is_some_and(|agent| with_aside(&facts, &aside) + 2 + width_of(agent) <= scene.width());
            if let Some(agent) = agent.clone().filter(|_| !is_beside) {
                look::flow(scene, &agent);
                if let Some(spans) = aside.take_if(|spans| width_of(&agent) + 2 + width_of(spans) <= scene.width()) {
                    scene.right(spans);
                }
                scene.end_line();
            }
            facts.extend(aside);
            let room = scene.width();
            look::whole(scene, &facts, room);
            if let Some(agent) = agent.filter(|_| is_beside) {
                scene.right(agent);
            }
            scene.end_line();
            if !node.collides.is_empty() {
                let room = scene.width();
                look::whole(scene, &collisions(&node.collides), room);
                scene.end_line();
            }
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

/// The columns parts take on one line, as `look::whole` sets them.
fn parts_width(parts: &[Vec<Span<'static>>]) -> u16 {
    let gaps = parts.len().saturating_sub(1) as u16;
    parts.iter().map(|part| width_of(part)).sum::<u16>() + gaps * 3
}

/// How many files of a collision are named; the rest are `+N more`.
const NAMED: usize = 3;

/// `⚠ collides with develop: tree.mjs · #304: render.mjs, index.ts · #302, #292: .env.local`
/// (#332, DIFF's `⚠ also changed on develop` / `in <worktree>`): the
/// worktrees that share the same files are one part; each file by the
/// shortest end of its path no other file of the node shares, the first
/// NAMED of a part, the rest counted.
fn collisions(collides: &[Overlap]) -> Vec<Vec<Span<'static>>> {
    let mut groups: Vec<(Vec<&str>, &[String])> = Vec::new();
    for overlap in collides {
        let is_develop = overlap.name == DEVELOP;
        match groups
            .iter_mut()
            .find(|(names, files)| !is_develop && names[0] != DEVELOP && *files == overlap.files.as_slice())
        {
            Some((names, _)) => names.push(&overlap.name),
            None => groups.push((vec![&overlap.name], &overlap.files)),
        }
    }
    let all: Vec<&str> = collides
        .iter()
        .flat_map(|overlap| overlap.files.iter().map(String::as_str))
        .collect();
    groups
        .iter()
        .enumerate()
        .map(|(i, (names, files))| {
            let mut part = Vec::new();
            if i == 0 {
                part.push(look::span("⚠ collides with ", look::style_of(&Paint::Yellow)));
            }
            for (at, name) in names.iter().enumerate() {
                if at > 0 {
                    part.push(Span::raw(", "));
                }
                part.push(if *name == DEVELOP {
                    look::bold(name)
                } else {
                    look::span(name, look::pink())
                });
            }
            let named: Vec<String> = files.iter().take(NAMED).map(|file| short_name(file, &all)).collect();
            part.push(Span::raw(format!(": {}", named.join(", "))));
            if files.len() > NAMED {
                part.push(look::dim(format!(" +{} more", files.len() - NAMED)));
            }
            part
        })
        .collect()
}

/// The name of develop among the collisions (core's tree).
const DEVELOP: &str = "develop";

/// The shortest end of `file`'s path no other of `all` ends in: `tree.mjs`,
/// `web/src/index.ts` beside `core/src/index.ts`.
fn short_name(file: &str, all: &[&str]) -> String {
    let parts: Vec<&str> = file.split('/').collect();
    for k in 1..parts.len() {
        let end = parts[parts.len() - k..].join("/");
        let is_shared = all
            .iter()
            .any(|other| *other != file && (*other == end || other.ends_with(&format!("/{end}"))));
        if !is_shared {
            return end;
        }
    }
    file.to_string()
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
                vec![model::Span::plain("↑1")],
            ],
            app: None,
            stack: None,
            collides: vec![],
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
        assert_eq!(scene.note.as_deref(), Some("2 wait for GitLab · no .env.local in any"));
        assert_eq!(
            text(&scene),
            [
                "0.9 GB free: fresh agents on the shared stack",
                "⇶ breadboard  .claude/orchestrations/breadboard.json",
                "│  agents 2 · issues 3 · reused 0 · handovers failed 0",
                "├─ 1. 303-every-button                            waits for the pipeline",
                "│     #301 open · ↑1           ● Mira Albrecht · 3 issues · 2 MRs merged",
                "│     the title of #301",
                "└─ 2. 305-refresh                                 waits for the pipeline",
                "      #302 open · ↑1           ● Mira Albrecht · 3 issues · 2 MRs merged",
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
                "│     #301 open · ↑1",
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

    /// The worktrees with an app and a stack each, the first colliding.
    fn with_stacks() -> Worktrees {
        let mut worktrees = worktrees(0, 0);
        for (i, node) in worktrees.tree.live.iter_mut().enumerate() {
            node.app = Some(if i == 0 {
                model::Span::new("● :3002", Paint::Green)
            } else {
                model::Span::dim("⇒ :3000")
            });
            node.stack = Some(if i == 0 {
                model::Span::new("+2 running(5)", Paint::Green)
            } else {
                model::Span::dim("shared")
            });
        }
        worktrees.tree.live[0].collides = vec![
            Overlap {
                name: "develop".into(),
                files: vec!["packages/breadboard/src/tree.mjs".into()],
            },
            Overlap {
                name: "#302".into(),
                files: vec![
                    "products/vita/web/src/index.ts".into(),
                    "products/vita/core/src/index.ts".into(),
                    "a.md".into(),
                    "b.md".into(),
                ],
            },
            Overlap {
                name: "#307".into(),
                files: vec!["c.md".into()],
            },
            Overlap {
                name: "#294".into(),
                files: vec!["c.md".into()],
            },
        ];
        worktrees
    }

    #[test]
    fn app_and_stack_stand_in_a_column_after_the_names_where_it_collides_under_the_facts() {
        let scene = draw(&with_stacks(), Folds::default(), 100);
        assert_eq!(scene.note.as_deref(), Some("2 wait for GitLab"));
        assert_eq!(
            text(&scene)[3..10],
            [
                "├─ 1. 303-every-button  ● :3002  +2 running(5)                                waits for the pipeline",
                "│     #301 open · ↑1                                       ● Mira Albrecht · 3 issues · 2 MRs merged",
                "│     ⚠ collides with develop: tree.mjs · #302: web/src/index.ts, core/src/index.ts, a.md +1 more ·",
                "│     #307, #294: c.md",
                "│     the title of #301",
                "└─ 2. 305-refresh       ⇒ :3000  shared                                       waits for the pipeline",
                "      #302 open · ↑1                                       ● Mira Albrecht · 3 issues · 2 MRs merged",
            ]
        );
        // The whole node is one block: hover lights the collision too.
        assert_eq!(scene.regions[0].area.height, 5);
    }

    #[test]
    fn app_and_stack_go_with_the_facts_where_the_states_leave_no_column() {
        let lines = text(&draw(&with_stacks(), Folds::default(), 66));
        // No room beside the agent either: they end the facts.
        assert_eq!(
            lines[3..6],
            [
                "├─ 1. 303-every-button                      waits for the pipeline",
                "│     ● Mira Albrecht · 3 issues · 2 MRs merged",
                "│     #301 open · ↑1 · ● :3002  +2 running(5)",
            ]
        );
        // A long state leaves no column: at the right of the agent's own line.
        let mut long = with_stacks();
        long.tree.live[1].state.text = "waits for you: try #1, decide #2, look #3, merge".into();
        let lines = text(&draw(&long, Folds::default(), 80));
        assert_eq!(
            lines[3..6],
            [
                "├─ 1. 303-every-button                                    waits for the pipeline",
                "│     ● Mira Albrecht · 3 issues · 2 MRs merged           ● :3002  +2 running(5)",
                "│     #301 open · ↑1",
            ]
        );
    }

    #[test]
    fn a_file_is_named_by_the_shortest_end_no_other_shares() {
        let all = [
            "products/vita/web/src/index.ts",
            "products/vita/core/src/index.ts",
            "x/tree.mjs",
        ];
        assert_eq!(short_name(all[0], &all), "web/src/index.ts");
        assert_eq!(short_name(all[2], &all), "tree.mjs");
        assert_eq!(short_name("tree.mjs", &["tree.mjs", "x/tree.mjs"]), "tree.mjs");
    }

    #[test]
    fn nothing_runs_past_the_edge_at_any_width() {
        for width in [20, 40, 80] {
            for worktrees in [worktrees(2, 2), with_stacks()] {
                let scene = draw(&worktrees, Folds::default(), width);
                for line in &scene.lines {
                    assert!(line.width() <= width as usize, "{width}: {line:?}");
                }
            }
        }
    }
}
