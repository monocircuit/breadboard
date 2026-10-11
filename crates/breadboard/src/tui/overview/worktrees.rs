//! WORKTREES of an orchestration (tree.mjs): a folder tree under `⇶ <name>`,
//! a node per issue with its number, its name in herdr's pink, its app and
//! stack in a column after the names (#332: a click opens the app or says
//! how to start it, `▶ stack down` starts the stack), where it stands at the right;
//! its facts below (how far develop moved on among them, #332) with who works
//! in it (#292) at their right, always under the state (the facts that don't
//! fit beside it go under it); where its work collides with develop and the
//! other worktrees (#332), each file a click into that worktree's nvim,
//! whether a merge of develop conflicts, and the migrations its database
//! lacks with `▶ pnpm pg:migrate` (STACK's and DEVELOP's lines, #332);
//! then its title, wrapped under the name (#304), the problem it works on
//! and what its agent is on now (#341), the stems of the tree in
//! front of every line. The queue and the done ones fold
//! (#259); a click anywhere on a node goes to its worktree's space, else to
//! its issue's page; hover lights the whole node (#296).

use super::actor::spawn::Run;
use super::{Act, Ctx, Fold, look};
use crate::tui::scene::{Region, Target, width_of};
use crate::tui::theme;
use crate::tui::{Page, Scene};
use breadboard_core::model::develop::Migrations;
use breadboard_core::model::orchestration::OrchestrationModel;
use breadboard_core::model::order::Go;
use breadboard_core::model::tree::{AgentLine, Node, Worktrees};
use breadboard_core::model::{self, Docker, Paint, counted};
use breadboard_core::time::ago;
use ratatui::layout::Rect;
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
    let note = note.trim().trim_end_matches('·').trim_end().to_string();
    let tree = &worktrees.tree;
    let cells = Cells::of(&tree.live, scene.width(), ctx);
    // Why there is no app and no stack, or why a stack says `?`: said once, not on every node,
    // on a line of its own under the record (a title too long for the line loses its note).
    let why = if !cells.is_any_env && tree.live.iter().any(|node| node.has_tree) {
        Some((
            "no .env.local in any worktree: no stack, the app can't run there",
            theme::dim(),
        ))
    } else if tree.live.iter().any(|node| node.docker == Docker::Silent) {
        Some((
            "docker does not answer: the stacks are unknown",
            look::style_of(&Paint::Yellow),
        ))
    } else {
        None
    };
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
        if let Some((why, style)) = why {
            scene.end_line();
            look::say(scene, why, style);
        }
    });
    let digits = digits_of(worktrees);
    for (i, node) in tree.live.iter().enumerate() {
        let is_last = i + 1 == tree.live.len() && !has_queue && !has_removed;
        node_lines(scene, node, is_last, "", digits, &cells, ctx);
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
                node_lines(scene, node, i + 1 == tree.queued.len(), prefix, digits, &cells, ctx);
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
                node_lines(scene, node, i + 1 == tree.removed.len(), "   ", digits, &cells, ctx);
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
    /// The widest stack cell: at the right edge the apps line up too.
    stack: u16,
    /// Where the cells start on the name line (after the node's gutter); None: with the facts.
    column: Option<u16>,
}

impl Cells {
    fn of(live: &[Node], width: u16, ctx: &Ctx) -> Cells {
        let with_tree: Vec<&Node> = live.iter().filter(|node| node.has_tree).collect();
        let is_any_env = with_tree.iter().any(|node| node.app.is_some());
        let mut cells = Cells {
            is_any_env,
            app: with_tree
                .iter()
                .map(|node| width_of(&look::spans(&[app_of(node)])))
                .max()
                .unwrap_or(0),
            stack: with_tree.iter().map(|node| stack_width(node, ctx)).max().unwrap_or(0),
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
        // By the widest cells: docker's answer coming in later changes no line.
        let is_fitting = with_tree.iter().all(|node| {
            let state = Span::raw(node.state.text.as_str()).width() as u16;
            column + cells.app + 2 + cells.stack + 2 + state <= room
        });
        cells.column = is_fitting.then_some(column);
        cells
    }

    /// `● :3002  stack running`: the app padded to the widest, the stack
    /// after; each a click where it does something (`Link`).
    fn piece(&self, node: &Node, ctx: &Ctx) -> Piece {
        let app = look::spans(&[app_of(node)]);
        let app_width = width_of(&app);
        let mut piece = Piece::default();
        if let (Some(port), Some(dir)) = (node.app_port, &node.dir) {
            piece.links.push(Link {
                at: 0,
                width: app_width,
                label: app.iter().map(|span| span.content.as_ref()).collect(),
                tone: theme::Tone::Green,
                act: Act::OpenApp {
                    dir: dir.clone(),
                    port,
                    is_shared: node.is_shared,
                },
            });
        }
        piece.spans = app;
        let short = self.app.saturating_sub(app_width) + 2;
        piece.spans.push(Span::raw(" ".repeat(usize::from(short))));
        let (stack, act) = stack_cell(node, ctx);
        let stack = look::spans(&[stack]);
        if let Some(act) = act {
            piece.links.push(Link {
                at: self.app + 2,
                width: width_of(&stack),
                label: stack.iter().map(|span| span.content.as_ref()).collect(),
                tone: theme::Tone::Cyan,
                act,
            });
        }
        piece.spans.extend(stack);
        piece
    }

    /// `piece` padded to the widest stack: at the right edge the apps line up.
    fn padded(&self, mut piece: Piece) -> Piece {
        let short = (self.app + 2 + self.stack).saturating_sub(width_of(&piece.spans));
        piece.spans.push(Span::raw(" ".repeat(usize::from(short))));
        piece
    }

    /// The cells of a node, None where there are none to show.
    fn of_node(&self, node: &Node, ctx: &Ctx) -> Option<Piece> {
        (self.is_any_env && node.has_tree).then(|| self.piece(node, ctx))
    }
}

/// Spans with the clicks that lie on parts of them: placed as one, each
/// click a button region on its part (hover lights that part alone).
#[derive(Default)]
struct Piece {
    spans: Vec<Span<'static>>,
    links: Vec<Link>,
}

/// A click on the columns `at..at + width` of a piece.
struct Link {
    at: u16,
    width: u16,
    label: String,
    tone: theme::Tone,
    act: Act,
}

impl Piece {
    fn plain(spans: Vec<Span<'static>>) -> Piece {
        Piece {
            spans,
            links: Vec::new(),
        }
    }

    /// `other` after this one and a gap of `gap` columns.
    fn then(mut self, gap: u16, other: Piece) -> Piece {
        let at = width_of(&self.spans) + gap;
        self.spans.push(Span::raw(" ".repeat(usize::from(gap))));
        self.spans.extend(other.spans);
        self.links.extend(other.links.into_iter().map(|link| Link {
            at: link.at + at,
            ..link
        }));
        self
    }

    /// The regions of its links, where it was placed (`rects` of one line).
    fn place(self, scene: &mut Scene<Act>, rects: &[Rect]) {
        let [rect] = rects else { return };
        for link in self.links {
            if link.at >= rect.width {
                continue;
            }
            scene.regions.push(Region {
                area: Rect::new(rect.x + link.at, rect.y, link.width.min(rect.width - link.at), 1),
                target: Target::Button {
                    label: link.label,
                    tone: link.tone,
                    action: link.act,
                },
            });
        }
    }
}

/// The room a stack of its own (`stack …`, `▶ stack down`) keeps at the
/// least: docker's answer coming in later moves nothing.
const OWN_STACK: &str = "▶ stack exited";

/// The columns a node's stack takes: an own one at least OWN_STACK.
fn stack_width(node: &Node, ctx: &Ctx) -> u16 {
    let stack = stack_cell(node, ctx).0;
    let width = width_of(&look::spans(std::slice::from_ref(&stack)));
    if (node.app_port.is_some() && !node.is_shared) || stack.text.starts_with("stack ") {
        width.max(Span::raw(OWN_STACK).width() as u16)
    } else {
        width
    }
}

/// `stack_of`'s run of `stack:up` from here, where one was started.
fn stack_run<'a>(node: &Node, ctx: &'a Ctx) -> Option<&'a Run> {
    node.dir.as_ref().and_then(|dir| ctx.actor.stacks.get(dir))
}

/// A node's stack cell and its click (#332, STACK's `▶ start the stack`):
/// `◐ starting …` while `stack:up` runs, `stack …` once it ended until
/// docker tells after it, `▶ stack down` where a click starts it; a running
/// one, a shared one and one docker hasn't told are no button (the node's
/// click goes to its space).
fn stack_cell(node: &Node, ctx: &Ctx) -> (model::Span, Option<Act>) {
    let start = node.dir.clone().map(Act::StartStack);
    let run = stack_run(node, ctx);
    if run.is_some_and(|run| run.is_running) {
        return (model::Span::new("◐ starting …", Paint::Yellow), start);
    }
    if run.and_then(|run| run.done_at).is_some_and(|at| at >= node.docker_at) && !node.is_shared {
        return (model::Span::dim("stack …"), None);
    }
    let stack = stack_of(node);
    if node.can_start_stack && start.is_some() {
        let paint = if run.is_some_and(|run| run.error.is_some()) {
            Paint::Red
        } else {
            stack.paint
        };
        return (model::Span::new(format!("▶ {}", stack.text), paint), start);
    }
    (stack, None)
}

/// A worktree's stack, `no .env.local` without one (boxes.mjs `stackCell`).
fn stack_of(node: &Node) -> model::Span {
    node.stack.clone().unwrap_or_else(|| model::Span::dim("no .env.local"))
}

/// A worktree's app, `—` without its .env.local (render.mjs).
fn app_of(node: &Node) -> model::Span {
    node.app.clone().unwrap_or_else(|| model::Span::dim("—"))
}

/// A node: its glyph and number, its name, app and stack, and state; its
/// facts, where it collides and its title under the name.
fn node_lines(
    scene: &mut Scene<Act>,
    node: &Node,
    is_last: bool,
    prefix: &str,
    digits: usize,
    cells: &Cells,
    ctx: &Ctx,
) {
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
            let mut mine = cells.of_node(node, ctx);
            if let Some(column) = cells.column
                && let Some(piece) = mine.take()
            {
                scene.pad_to(column);
                let rects = scene.atom_gap(piece.spans.clone(), 0);
                piece.place(scene, &rects);
            }
            right(
                scene,
                vec![look::span(&node.state.text, look::style_of(&node.state.paint))],
            );
            scene.end_line();
            // The right column under the state: who works in it (#292), then,
            // without a column beside the name, its app and stack, padded so
            // the apps line up down the edge. The facts flow on the left, as
            // many beside each as fit: the agent always stands under the state.
            let agent = node.agent.as_ref().map(|agent| Piece::plain(agent_spans(agent)));
            let aside = mine.map(|piece| cells.padded(piece));
            let facts = look::parts(&node.facts);
            let rights: Vec<Piece> = match (agent, aside) {
                // One line when everything fits on it.
                (Some(agent), Some(aside))
                    if parts_width(&facts) + 2 + width_of(&agent.spans) + 2 + width_of(&aside.spans)
                        <= scene.width() =>
                {
                    vec![agent.then(2, aside)]
                }
                (agent, aside) => agent.into_iter().chain(aside).collect(),
            };
            let mut from = 0;
            for spans in rights {
                from = beside(scene, &facts, from, spans);
            }
            if from < facts.len() || from == 0 {
                put_parts(scene, &facts, from);
                scene.end_line();
            }
            stack_error_line(scene, node, ctx);
            migrations_line(scene, node, ctx);
            conflict_line(scene, node);
            if node
                .collides
                .iter()
                .any(|overlap| !develop_files(node, overlap).is_empty())
            {
                collision_line(scene, node);
                scene.end_line();
            }
            look::say(scene, &node.title, theme::dim());
            scene.end_line();
            said_lines(scene, node, ctx);
        });
    };
    if node.go == Go::Nothing {
        draw(scene);
    } else {
        scene.block(Act::Go(node.go.clone()), draw);
    }
}

/// What the node says of its work in plain words (#341), each wrapped under
/// its text: `problem` from its issue, then `now` (its agent's note) or
/// `said` (the first line of its last answer) with how long ago.
fn said_lines(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) {
    if !node.has_tree {
        return;
    }
    let mut labeled = |label: &str, text: &str, age: Option<String>| {
        scene.atom(vec![look::dim(label)]);
        scene.pad_to(SAID_AT);
        scene.indent(SAID_AT);
        look::say(scene, text, ratatui::style::Style::new());
        if let Some(age) = age {
            look::say(scene, &format!("· {age}"), theme::dim());
        }
        scene.indent(0);
        scene.end_line();
    };
    if let Some(problem) = &node.problem {
        labeled("problem", problem, None);
    }
    if let Some(now) = &node.now {
        let label = if now.is_note { "now" } else { "said" };
        labeled(label, &now.text, now.at.map(|at| ago(at, ctx.now)));
    }
}

/// Where the text of `problem` and `now` starts, 1 after the label `problem`.
const SAID_AT: u16 = 8;

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

/// The facts from `from` on this line while they fit beside `right`, then
/// `right` at the edge; the line ends. Returns the first fact not placed. A
/// first fact wider than the room leaves `right` on a line of its own.
fn beside(scene: &mut Scene<Act>, facts: &[Vec<Span<'static>>], from: usize, right: Piece) -> usize {
    let room = scene.width().saturating_sub(width_of(&right.spans) + 2);
    let mut x = 0;
    let mut to = from;
    while to < facts.len() {
        let width = width_of(&facts[to]) + if to + 1 < facts.len() { 2 } else { 0 };
        let gap = u16::from(to > from);
        if x + gap + width > room {
            break;
        }
        x += gap + width;
        to += 1;
    }
    for at in from..to {
        // The last before the right column has no ` ·`: the agent isn't the next fact.
        let mut atom = fact_atom(facts, at);
        if at + 1 == to && to < facts.len() {
            atom.pop();
        }
        scene.atom(atom);
    }
    let rects = scene.right(right.spans.clone());
    right.place(scene, &rects);
    scene.end_line();
    to
}

/// The facts from `from` on, as `look::whole` parts them.
fn put_parts(scene: &mut Scene<Act>, facts: &[Vec<Span<'static>>], from: usize) {
    let room = scene.width();
    for at in from..facts.len() {
        let atom = fact_atom(facts, at);
        if width_of(&atom) <= room {
            scene.atom(atom);
        } else {
            look::flow(scene, &atom);
        }
    }
}

/// A fact with the dim ` ·` that parts it from the next: the separator ends a line, never starts one.
fn fact_atom(facts: &[Vec<Span<'static>>], at: usize) -> Vec<Span<'static>> {
    let mut atom = facts[at].clone();
    if at + 1 < facts.len() {
        atom.push(look::dim(" ·"));
    }
    atom
}

/// How many files of a collision are named; the rest are `+N more`.
const NAMED: usize = 3;

/// `⚠ collides with develop: tree.mjs · #304: render.mjs, web/src/index.ts ·
/// #302, #292: .env.local` (#332, DIFF's `⚠ also changed on develop` / `in
/// <worktree>`): the worktrees that share the same files are one part; each
/// file by the shortest end of its path none of the files near it shares
/// (core's `near`), the first NAMED of a part, the rest counted. A click on
/// a file opens it in the worktree's nvim at its first change, on `+N more`
/// the first not named (DIFF's click).
fn collision_line(scene: &mut Scene<Act>, node: &Node) {
    let mut groups: Vec<(Vec<&str>, Vec<&String>)> = Vec::new();
    for overlap in &node.collides {
        let is_develop = overlap.name == DEVELOP;
        let files = develop_files(node, overlap);
        if files.is_empty() {
            continue;
        }
        match groups
            .iter_mut()
            .find(|(names, known)| !is_develop && names[0] != DEVELOP && *known == files)
        {
            Some((names, _)) => names.push(&overlap.name),
            None => groups.push((vec![&overlap.name], files)),
        }
    }
    // Its colliding files are near it too, whatever core knew of the rest.
    let near: Vec<&str> = node
        .near
        .iter()
        .chain(node.collides.iter().flat_map(|overlap| &overlap.files))
        .map(String::as_str)
        .collect();
    let open = |file: &str| {
        node.dir.as_ref().map(|dir| Act::OpenFile {
            dir: dir.clone(),
            file: file.to_string(),
            line: None,
        })
    };
    scene.atom(vec![look::span("⚠ collides with", look::style_of(&Paint::Yellow))]);
    // Both changed them, and git merges them cleanly (DEVELOP's `✓ merges cleanly`).
    let is_clean = node.conflicts.as_ref().is_some_and(Vec::is_empty);
    for (i, (names, files)) in groups.iter().enumerate() {
        for (at, name) in names.iter().enumerate() {
            let mut atom = vec![if *name == DEVELOP {
                look::bold(name)
            } else {
                look::span(name, look::pink())
            }];
            if *name == DEVELOP && is_clean {
                atom.push(look::dim(", merges cleanly"));
            }
            atom.push(Span::raw(if at + 1 < names.len() { "," } else { ":" }));
            scene.atom(atom);
        }
        let named = files.len().min(NAMED);
        for (at, file) in files.iter().take(NAMED).enumerate() {
            let name = short_name(file, &near);
            let is_last = at + 1 == named && files.len() <= NAMED;
            let after = match (at + 1 < named, is_last && i + 1 < groups.len()) {
                (true, _) => ",",
                (false, true) => " ·",
                (false, false) => "",
            };
            let link = Span::raw(name);
            link_atom(scene, link, after, open(file));
        }
        if files.len() > NAMED {
            let more = look::dim(format!("+{} more", files.len() - NAMED));
            let after = if i + 1 < groups.len() { " ·" } else { "" };
            link_atom(scene, more, after, open(files[NAMED]));
        }
    }
}

/// The files of `overlap` the collision line names: develop's without the
/// ones a merge conflicts in (the conflict line names those).
fn develop_files<'a>(node: &Node, overlap: &'a model::diff::Overlap) -> Vec<&'a String> {
    let conflicts = node.conflicts.as_deref().unwrap_or_default();
    overlap
        .files
        .iter()
        .filter(|file| overlap.name != DEVELOP || !conflicts.contains(file))
        .collect()
}

/// How many files the conflict line names (DEVELOP's 4).
const CONFLICTS_NAMED: usize = 4;

/// `✗ a merge of develop conflicts in tree.mjs, render.mjs +1 more` (#332,
/// DEVELOP's `git merge-tree` line): only where it would; each name a
/// click into the worktree's nvim, `+N more` the first not named.
fn conflict_line(scene: &mut Scene<Act>, node: &Node) {
    let Some(files) = node.conflicts.as_ref().filter(|files| !files.is_empty()) else {
        return;
    };
    let near: Vec<&str> = node.near.iter().chain(files).map(String::as_str).collect();
    let open = |file: &str| {
        node.dir.as_ref().map(|dir| Act::OpenFile {
            dir: dir.clone(),
            file: file.to_string(),
            line: None,
        })
    };
    scene.atom(vec![look::span(
        "✗ a merge of develop conflicts in",
        look::style_of(&Paint::Red),
    )]);
    let named = files.len().min(CONFLICTS_NAMED);
    for (at, file) in files.iter().take(CONFLICTS_NAMED).enumerate() {
        let after = if at + 1 < named { "," } else { "" };
        link_atom(scene, Span::raw(short_name(file, &near)), after, open(file));
    }
    if files.len() > CONFLICTS_NAMED {
        let more = look::dim(format!("+{} more", files.len() - CONFLICTS_NAMED));
        link_atom(scene, more, "", open(&files[CONFLICTS_NAMED]));
    }
    scene.end_line();
}

/// `✗ the stack didn't start: <why>` under a node whose `stack:up` from here
/// failed, while its stack still isn't running (STACK's red error).
fn stack_error_line(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) {
    let Some(why) = stack_run(node, ctx)
        .filter(|run| !run.is_running && node.can_start_stack)
        .and_then(|run| run.error.as_deref())
    else {
        return;
    };
    look::say(
        scene,
        &format!("✗ the stack didn't start: {why}"),
        look::style_of(&Paint::Red),
    );
    scene.end_line();
}

/// How many migrations the line names (STACK's 3).
const MIGRATIONS_NAMED: usize = 3;

/// The migrations its database lacks (#332, STACK's line), only where
/// something is to do: `⚠ 2 migrations not applied: 0042_a, 0043_b  ▶ pnpm
/// pg:migrate` (`◐ migrating …` while it runs, its error red after it), or
/// `⚠ the database doesn't answer: <why>`; all applied says nothing.
fn migrations_line(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) {
    let Some(dir) = &node.dir else { return };
    let run = ctx.actor.migrations.get(dir);
    let migrate = Act::Migrate(dir.clone());
    let yellow = look::style_of(&Paint::Yellow);
    if let Some(run) = run.filter(|run| run.is_running) {
        let rects = scene.atom(vec![look::span("◐ migrating …", yellow)]);
        button_on(scene, &rects, "◐ migrating …", theme::Tone::Yellow, migrate);
        let last = run.last_line();
        if !last.is_empty() {
            look::say(scene, &last, theme::dim());
        }
        scene.redraw_in(std::time::Duration::from_millis(500));
        scene.end_line();
        return;
    }
    // It ran well after this read: the next one asks the database again.
    if run
        .and_then(|run| run.done_at)
        .is_some_and(|at| at >= node.migrations_at)
    {
        return;
    }
    match &node.migrations {
        Some(Migrations::Error(why)) => {
            look::say(scene, &format!("⚠ the database doesn't answer: {why}"), yellow);
            scene.end_line();
        }
        Some(Migrations::Pending { pending, .. }) if !pending.is_empty() => {
            let names: Vec<&str> = pending.iter().take(MIGRATIONS_NAMED).map(String::as_str).collect();
            let more = if pending.len() > MIGRATIONS_NAMED {
                format!(" +{} more", pending.len() - MIGRATIONS_NAMED)
            } else {
                String::new()
            };
            let text = format!(
                "⚠ {} not applied: {}{more}",
                counted(pending.len() as u64, "migration"),
                names.join(", ")
            );
            look::say(scene, &text, yellow);
            let label = "▶ pnpm pg:migrate";
            let rects = scene.atom_gap(vec![look::span(label, theme::tone(theme::Tone::Cyan))], 2);
            button_on(scene, &rects, label, theme::Tone::Cyan, migrate);
            if let Some(why) = run.and_then(|run| run.error.as_deref()) {
                scene.atom_gap(vec![look::span(format!("✗ {why}"), look::style_of(&Paint::Red))], 2);
            }
            scene.end_line();
        }
        _ => {}
    }
}

/// A button region on text placed as one atom (`rects` of one line).
fn button_on(scene: &mut Scene<Act>, rects: &[Rect], label: &str, tone: theme::Tone, act: Act) {
    if let [rect] = rects {
        scene.regions.push(Region {
            area: *rect,
            target: Target::Button {
                label: label.into(),
                tone,
                action: act,
            },
        });
    }
}

/// `link` and what follows it glued as one atom; a click on `link` runs
/// `act` (a button region on plain text: hover lights it on its own).
fn link_atom(scene: &mut Scene<Act>, link: Span<'static>, after: &str, act: Option<Act>) {
    let width = link.width() as u16;
    let label = link.content.to_string();
    let mut atom = vec![link];
    if !after.is_empty() {
        atom.push(look::dim(after));
    }
    let rects = scene.atom(atom);
    let (Some(act), [rect]) = (act, rects.as_slice()) else {
        return;
    };
    scene.regions.push(Region {
        area: Rect::new(rect.x, rect.y, width.min(rect.width), 1),
        target: Target::Button {
            label,
            tone: theme::Tone::Yellow,
            action: act,
        },
    });
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
    use breadboard_core::model::diff::Overlap;
    use breadboard_core::model::tree::{Lifecycle, TreeOf};
    use breadboard_core::said::Now;
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
            docker: Docker::Answered,
            collides: vec![],
            near: vec![],
            dir: None,
            can_start_stack: false,
            conflicts: None,
            migrations: None,
            is_shared: false,
            app_port: None,
            migrations_at: 0,
            docker_at: 0,
            title: format!("the title of #{}", place + 300),
            problem: None,
            now: None,
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
        draw_by(worktrees, folds, width, super::super::actor::idle())
    }

    fn draw_by(worktrees: &Worktrees, folds: Folds, width: u16, actor: &super::super::actor::Actor) -> Scene<Act> {
        let gitlab = GitlabState::default();
        let ctx = Ctx {
            is_once: false,
            folds,
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor,
            words: super::super::actor::view::no_words(),
        };
        let mut scene = Scene::new(width);
        tree(&mut scene, worktrees, &ctx);
        scene.finish();
        scene
    }

    /// The line of the first node.
    fn first_node(lines: &[String]) -> usize {
        lines.iter().position(|line| line.starts_with("├─ 1.")).unwrap()
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
                "│  no .env.local in any worktree: no stack, the app can't run there",
                "├─ 1. 303-every-button                            waits for the pipeline",
                "│     #301 open · ↑1               ● Mira Albrecht · 3 issues · 2 merged",
                "│     the title of #301",
                "└─ 2. 305-refresh                                 waits for the pipeline",
                "      #302 open · ↑1               ● Mira Albrecht · 3 issues · 2 merged",
                "      the title of #302",
            ]
        );
    }

    #[test]
    fn a_node_says_its_problem_and_what_its_agent_does_now_wrapped_under_their_text() {
        let mut worktrees = worktrees(0, 0);
        let live = &mut worktrees.tree.live;
        live[0].problem =
            Some("The overview's node names the topic of its issue but not the problem it works on.".into());
        live[0].now = Some(Now {
            text: "cause found, fix pushed".into(),
            at: Some(-4 * 60_000),
            is_note: true,
        });
        // Without a note: the problem only.
        live[1].problem = Some("wt:new hangs.".into());
        let lines = text(&draw(&worktrees, Folds::default(), 56));
        let at = lines
            .iter()
            .position(|line| line.ends_with("the title of #301"))
            .unwrap();
        assert_eq!(
            lines[at..at + 4],
            [
                "│     the title of #301",
                "│     problem The overview's node names the topic of its",
                "│             issue but not the problem it works on.",
                "│     now     cause found, fix pushed · 4 min ago",
            ]
        );
        let second = lines
            .iter()
            .position(|line| line.ends_with("the title of #302"))
            .unwrap();
        assert_eq!(
            lines[second..],
            ["      the title of #302", "      problem wt:new hangs."]
        );
        // The first line of the last answer, marked as said, without a time.
        worktrees.tree.live[1].now = Some(Now {
            text: "Pushed the fix.".into(),
            at: None,
            is_note: false,
        });
        let lines = text(&draw(&worktrees, Folds::default(), 56));
        assert_eq!(lines.last().unwrap(), "      said    Pushed the fix.");
    }

    #[test]
    fn facts_that_do_not_fit_beside_the_agent_go_under_it_the_agent_stays_under_the_state() {
        let lines = text(&draw(&worktrees(0, 0), Folds::default(), 56));
        let at = first_node(&lines);
        assert_eq!(
            lines[at..at + 4],
            [
                "├─ 1. 303-every-button            waits for the pipeline",
                "│     #301 open    ● Mira Albrecht · 3 issues · 2 merged",
                "│     ↑1",
                "│     the title of #301",
            ]
        );
        // Not even the first fact fits: the agent alone at the right, the facts under it.
        let lines = text(&draw(&worktrees(0, 0), Folds::default(), 50));
        let at = lines.iter().position(|line| line.contains("● Mira")).unwrap();
        assert_eq!(
            lines[at..at + 2],
            [
                "│            ● Mira Albrecht · 3 issues · 2 merged",
                "│     #301 open · ↑1"
            ]
        );
    }

    #[test]
    fn a_state_wider_than_the_line_stays_at_the_right_edge_on_lines_of_its_own() {
        let mut worktrees = worktrees(0, 0);
        worktrees.tree.live[0].state.text =
            "start: → Elif Kowalczyk: its handover failed: wt:orchestrate: not built yet; a click tries again".into();
        let lines = text(&draw(&worktrees, Folds::default(), 60));
        let at = first_node(&lines);
        assert_eq!(
            lines[at..at + 3],
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
                (4, 3, 72, Act::Go(Go::FocusSpace("/w/303-every-button".into()))),
                (7, 3, 72, Act::Go(Go::FocusSpace("/w/305-refresh".into()))),
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
                model::Span::new("stack running", Paint::Green)
            } else {
                model::Span::dim("shared")
            });
            node.app_port = Some(if i == 0 { 3002 } else { 3000 });
            node.is_shared = i == 1;
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
                "├─ 1. 303-every-button  ● :3002  stack running                                waits for the pipeline",
                "│     #301 open · ↑1                                           ● Mira Albrecht · 3 issues · 2 merged",
                "│     ⚠ collides with develop: tree.mjs · #302: web/src/index.ts, core/src/index.ts, a.md +1 more ·",
                "│     #307, #294: c.md",
                "│     the title of #301",
                "└─ 2. 305-refresh       ⇒ :3000  shared                                       waits for the pipeline",
                "      #302 open · ↑1                                           ● Mira Albrecht · 3 issues · 2 merged",
            ]
        );
        // The whole node is one block: hover lights the collision too.
        assert_eq!(scene.regions[0].area.height, 5);
    }

    #[test]
    fn app_and_stack_go_with_the_facts_where_the_states_leave_no_column() {
        let lines = text(&draw(&with_stacks(), Folds::default(), 66));
        // No room beside the agent either: under it, at the right edge.
        assert_eq!(
            lines[3..6],
            [
                "├─ 1. 303-every-button                      waits for the pipeline",
                "│     #301 open · ↑1         ● Mira Albrecht · 3 issues · 2 merged",
                "│                                          ● :3002  stack running",
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
                "│     #301 open · ↑1                       ● Mira Albrecht · 3 issues · 2 merged",
                "│                                                        ● :3002  stack running",
            ]
        );
        // The apps line up down the right edge.
        let at = |line: &str, mark: &str| line[..line.find(mark).unwrap()].chars().count();
        let shared = lines.iter().find(|line| line.contains("⇒ :3000")).unwrap();
        assert_eq!(at(shared, "⇒ :3000"), at(&lines[5], "● :3002"));
    }

    #[test]
    fn docker_answering_late_moves_nothing_and_unknown_is_never_down() {
        let asking = |stack: model::Span, docker: Docker| {
            let mut worktrees = with_stacks();
            worktrees.tree.live[0].stack = Some(stack);
            worktrees.tree.live[0].docker = docker;
            worktrees
        };
        let before = asking(model::Span::dim("stack …"), Docker::NotYet);
        for width in [66, 80, 100, 120] {
            let mut after = before.clone();
            after.tree.live[0].stack = Some(model::Span::new("stack running", Paint::Green));
            after.tree.live[0].docker = Docker::Answered;
            let (was, is) = (
                draw(&before, Folds::default(), width),
                draw(&after, Folds::default(), width),
            );
            let without = |scene: &Scene<Act>| {
                text(scene)
                    .into_iter()
                    .map(|line| line.replace("stack running", "stack …      ").trim_end().to_string())
                    .collect::<Vec<_>>()
            };
            // The same lines, the cell in the same place: only its words change.
            assert_eq!(without(&was), without(&is), "{width}");
            let at = |lines: Vec<String>, cell: &str| lines.iter().position(|line| line.contains(cell));
            assert_eq!(at(text(&was), "stack …"), at(text(&is), "stack running"), "{width}");
        }
        // docker silent: the cell asks, the title says why once.
        let silent = draw(
            &asking(model::Span::new("stack ?", Paint::Yellow), Docker::Silent),
            Folds::default(),
            100,
        );
        assert_eq!(silent.note.as_deref(), Some("2 wait for GitLab"));
        assert_eq!(text(&silent)[3], "│  docker does not answer: the stacks are unknown");
        assert!(!text(&silent).iter().any(|line| line.contains("down")));
    }

    #[test]
    fn a_colliding_file_opens_in_its_worktree_and_says_which_one_it_is() {
        let mut worktrees = with_stacks();
        let node = &mut worktrees.tree.live[0];
        node.dir = Some("/w/303-every-button".into());
        node.collides.truncate(2);
        node.collides[1].files = vec!["products/vita/web/src/index.ts".into(), "a.md".into()];
        node.collides[1].files.extend(["b.md".into(), "c.md".into()]);
        // The other worktree changed core's index.ts too: web's is named as such.
        node.near = vec!["products/vita/core/src/index.ts".into()];
        let scene = draw(&worktrees, Folds::default(), 100);
        let lines = text(&scene);
        assert!(
            lines.contains(
                &"│     ⚠ collides with develop: tree.mjs · #302: web/src/index.ts, a.md, b.md +1 more".to_string()
            ),
            "{lines:#?}"
        );
        let links: Vec<(String, Act)> = scene
            .regions
            .iter()
            .filter_map(|region| match &region.target {
                Target::Button { label, action, .. } if matches!(action, Act::OpenFile { .. }) => {
                    Some((label.clone(), action.clone()))
                }
                _ => None,
            })
            .collect();
        let open = |file: &str| Act::OpenFile {
            dir: "/w/303-every-button".into(),
            file: file.into(),
            line: None,
        };
        assert_eq!(links[0], ("tree.mjs".into(), open("packages/breadboard/src/tree.mjs")));
        assert_eq!(links[1].1, open("products/vita/web/src/index.ts"));
        // `+1 more`: the first not named.
        assert_eq!(links[4], ("+1 more".into(), open("c.md")));
        // A link lies on its name only, inside the node's block.
        let block = scene.regions.iter().find(|region| !region.is_button()).unwrap().area;
        let tree = scene
            .regions
            .iter()
            .find(|region| region.is_button() && matches!(region.action(), Act::OpenFile { .. }))
            .unwrap()
            .area;
        assert_eq!(tree.width, 8);
        assert!(tree.y >= block.y && tree.y < block.y + block.height);
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

    /// The buttons of a scene: (label, act).
    fn buttons(scene: &Scene<Act>) -> Vec<(String, Act)> {
        scene
            .regions
            .iter()
            .filter_map(|region| match &region.target {
                Target::Button { label, action, .. } => Some((label.clone(), action.clone())),
                Target::Block(_) => None,
            })
            .collect()
    }

    /// `with_stacks` without collisions, the first node's own stack down where docker told at 1000.
    fn down() -> Worktrees {
        let mut worktrees = with_stacks();
        for node in &mut worktrees.tree.live {
            node.dir = Some(PathBuf::from(format!("/w/{}", node.name)));
            node.collides.clear();
            node.docker_at = 1_000;
        }
        let node = &mut worktrees.tree.live[0];
        node.stack = Some(model::Span::new("stack down", Paint::Yellow));
        node.app = Some(model::Span::dim("○ :3002"));
        node.can_start_stack = true;
        worktrees
    }

    #[test]
    fn the_app_opens_where_it_is_and_a_stack_that_is_down_starts_on_a_click() {
        let scene = draw(&down(), Folds::default(), 100);
        let lines = text(&scene);
        assert_eq!(
            lines[3],
            "├─ 1. 303-every-button  ○ :3002  ▶ stack down                                 waits for the pipeline"
        );
        let dir = PathBuf::from("/w/303-every-button");
        assert_eq!(
            buttons(&scene),
            [
                (
                    "○ :3002".to_string(),
                    Act::OpenApp {
                        dir: dir.clone(),
                        port: 3002,
                        is_shared: false
                    }
                ),
                ("▶ stack down".to_string(), Act::StartStack(dir.clone())),
                (
                    "⇒ :3000".to_string(),
                    Act::OpenApp {
                        dir: "/w/305-refresh".into(),
                        port: 3000,
                        is_shared: true
                    }
                ),
            ]
        );
        // The button lies on its words, inside the node's line.
        let start = scene
            .regions
            .iter()
            .find(|region| region.action() == &Act::StartStack(dir.clone()))
            .unwrap();
        let x = usize::from(start.area.x);
        assert_eq!(&lines[3].chars().skip(x).take(12).collect::<String>(), "▶ stack down");
        // A running stack is no button: the node's click goes to its space.
        let scene = draw(&with_stacks(), Folds::default(), 100);
        assert!(!buttons(&scene).iter().any(|(_, act)| matches!(act, Act::StartStack(_))));
    }

    #[test]
    fn a_stack_starting_says_so_then_waits_for_docker_and_a_failure_says_why() {
        let dir = PathBuf::from("/w/303-every-button");
        let mut actor = super::super::actor::Actor::new(
            &breadboard_core::reader::View::Everything,
            std::path::Path::new(""),
            true,
            false,
        );
        let at = |actor: &super::super::actor::Actor| text(&draw_by(&down(), Folds::default(), 100, actor));
        actor.stacks.insert(
            dir.clone(),
            Run {
                is_running: true,
                ..Run::default()
            },
        );
        let lines = at(&actor);
        assert!(lines[3].contains("○ :3002  ◐ starting …  "), "{lines:#?}");
        // Up after docker's last answer: `stack …` until docker tells, never `▶ stack down` again.
        actor.stacks.insert(
            dir.clone(),
            Run {
                done_at: Some(2_000),
                ..Run::default()
            },
        );
        let lines = at(&actor);
        assert!(lines[3].contains("○ :3002  stack …  "), "{lines:#?}");
        actor.stacks.insert(
            dir.clone(),
            Run {
                done_at: Some(500),
                ..Run::default()
            },
        );
        assert!(at(&actor)[3].contains("▶ stack down"));
        actor.stacks.insert(
            dir.clone(),
            Run {
                error: Some("port 5434 is taken".into()),
                ..Run::default()
            },
        );
        let lines = at(&actor);
        assert!(lines[3].contains("▶ stack down"));
        assert_eq!(lines[5], "│     ✗ the stack didn't start: port 5434 is taken");
        // The stack's column holds each of these: the names, cells and state stay where they were.
        let idle = text(&draw(&down(), Folds::default(), 100));
        assert_eq!(lines[3].find("waits"), idle[3].find("waits"));
        assert_eq!(lines[4], idle[4]);
    }

    #[test]
    fn missing_migrations_say_which_with_their_button_a_silent_database_says_so() {
        let mut worktrees = down();
        worktrees.tree.live[1].migrations = Some(Migrations::Pending {
            pending: vec!["0041_a".into(), "0042_b".into(), "0043_c".into(), "0044_d".into()],
            applied: 40,
        });
        worktrees.tree.live[0].migrations = Some(Migrations::Error("connection refused".into()));
        let scene = draw(&worktrees, Folds::default(), 100);
        let lines = text(&scene);
        assert!(lines.contains(&"│     ⚠ the database doesn't answer: connection refused".to_string()));
        assert!(
            lines.contains(
                &"      ⚠ 4 migrations not applied: 0041_a, 0042_b, 0043_c +1 more  ▶ pnpm pg:migrate".to_string()
            ),
            "{lines:#?}"
        );
        assert!(buttons(&scene).contains(&("▶ pnpm pg:migrate".into(), Act::Migrate("/w/305-refresh".into()))));
        // All applied: nothing to say.
        worktrees.tree.live[1].migrations = Some(Migrations::Pending {
            pending: vec![],
            applied: 44,
        });
        assert!(
            !text(&draw(&worktrees, Folds::default(), 100))
                .iter()
                .any(|line| line.contains("migration"))
        );
        // Running, then done after the last ask: `◐ migrating …`, then nothing until asked again.
        let mut actor = super::super::actor::Actor::new(
            &breadboard_core::reader::View::Everything,
            std::path::Path::new(""),
            true,
            false,
        );
        worktrees.tree.live[1].migrations = Some(Migrations::Pending {
            pending: vec!["0044_d".into()],
            applied: 43,
        });
        worktrees.tree.live[1].migrations_at = 1_000;
        let dir = PathBuf::from("/w/305-refresh");
        actor.migrations.insert(
            dir.clone(),
            Run {
                is_running: true,
                ..Run::default()
            },
        );
        let lines = text(&draw_by(&worktrees, Folds::default(), 100, &actor));
        assert!(lines.contains(&"      ◐ migrating …".to_string()), "{lines:#?}");
        actor.migrations.insert(
            dir,
            Run {
                done_at: Some(2_000),
                ..Run::default()
            },
        );
        let lines = text(&draw_by(&worktrees, Folds::default(), 100, &actor));
        assert!(!lines.iter().any(|line| line.contains("migrat")), "{lines:#?}");
    }

    #[test]
    fn a_merge_of_develop_says_whether_it_conflicts_and_the_conflicts_open_in_nvim() {
        let mut worktrees = with_stacks();
        let node = &mut worktrees.tree.live[0];
        node.dir = Some("/w/303-every-button".into());
        node.collides.truncate(2);
        node.collides[0].files.push("packages/breadboard/src/render.mjs".into());
        node.conflicts = Some(vec!["packages/breadboard/src/render.mjs".into()]);
        let scene = draw(&worktrees, Folds::default(), 120);
        let lines = text(&scene);
        assert!(
            lines.contains(&"│     ✗ a merge of develop conflicts in render.mjs".to_string()),
            "{lines:#?}"
        );
        // develop's line keeps the file that merges cleanly, not the conflicting one twice.
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("│     ⚠ collides with develop: tree.mjs · #302:"))
        );
        assert!(buttons(&scene).contains(&(
            "render.mjs".into(),
            Act::OpenFile {
                dir: "/w/303-every-button".into(),
                file: "packages/breadboard/src/render.mjs".into(),
                line: None
            }
        )));
        // Every file develop changed too merges cleanly: said where develop is named.
        worktrees.tree.live[0].conflicts = Some(vec![]);
        let lines = text(&draw(&worktrees, Folds::default(), 120));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("│     ⚠ collides with develop, merges cleanly: tree.mjs, render.mjs ·")),
            "{lines:#?}"
        );
        assert!(!lines.iter().any(|line| line.contains("✗")));
        // Only develop, all of it conflicting: no collision line, the conflict line alone.
        worktrees.tree.live[0].collides.truncate(1);
        worktrees.tree.live[0].conflicts = Some(worktrees.tree.live[0].collides[0].files.clone());
        let lines = text(&draw(&worktrees, Folds::default(), 120));
        assert!(!lines.iter().any(|line| line.contains("collides")));
        assert!(lines.contains(&"│     ✗ a merge of develop conflicts in tree.mjs, render.mjs".to_string()));
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
