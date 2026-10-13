//! The worktree's pieces of a block in WORK (`work`, #347; tree.mjs before
//! it): its app and stack in a column after the names (#332: a click opens
//! the app or says how to start it, `▶ stack down` starts the stack); its
//! facts (how far develop moved on among them, #332) with who works in it
//! (#292) at their right, always under the state (the facts that don't fit
//! beside it go under it); where its work collides with develop and the
//! other worktrees (#332), each file a click into that worktree's nvim,
//! whether a merge of develop conflicts, and the migrations its database
//! lacks with `▶ pnpm pg:migrate` (STACK's and DEVELOP's lines, #332).

use super::actor::spawn::Run;
use super::{Act, Ctx, Fold, boxes, look};
use crate::tui::Scene;
use crate::tui::scene::{Region, Target, width_of};
use crate::tui::theme;
use breadboard_core::model::develop::Migrations;
use breadboard_core::model::tree::{AgentLine, Node};
use breadboard_core::model::{self, Paint, counted};
use breadboard_core::time::ago;
use ratatui::layout::Rect;
use ratatui::text::Span;

/// The columns of the live worktrees' app and stack (#332, lazygit's
/// branches): in a column after the widest name when every node's state
/// still fits beside them, else at the end of each node's facts. Chosen once
/// for all nodes, so they line up and one long state reflows none.
pub(super) struct Cells {
    /// Whether any worktree has an .env.local: without, there is nothing to show.
    pub(super) is_any_env: bool,
    /// The widest app cell: the stacks line up after it.
    app: u16,
    /// The widest stack cell: at the right edge the apps line up too.
    stack: u16,
    /// Where the cells start on the name line (after the node's gutter); None: with the facts.
    pub(super) column: Option<u16>,
}

impl Cells {
    /// The cells of `nodes` in a line of `room` columns, beside the name
    /// line's lead (`column`: its name and MR columns, #347) where every
    /// node's state still fits after them.
    pub(super) fn of(nodes: &[&Node], column: u16, room: u16, ctx: &Ctx) -> Cells {
        let with_tree: Vec<&Node> = nodes.iter().copied().filter(|node| node.has_tree).collect();
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
    pub(super) fn of_node(&self, node: &Node, ctx: &Ctx) -> Option<Piece> {
        (self.is_any_env && node.has_tree).then(|| self.piece(node, ctx))
    }
}

/// Spans with the clicks that lie on parts of them: placed as one, each
/// click a button region on its part (hover lights that part alone).
#[derive(Default)]
pub(super) struct Piece {
    pub(super) spans: Vec<Span<'static>>,
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
    pub(super) fn place(self, scene: &mut Scene<Act>, rects: &[Rect]) {
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
    node.stack
        .clone()
        .unwrap_or_else(|| model::Span::dim(breadboard_core::config::env_missing()))
}

/// A worktree's app, `—` without its .env.local (render.mjs).
fn app_of(node: &Node) -> model::Span {
    node.app.clone().unwrap_or_else(|| model::Span::dim("—"))
}

/// What a node says under its name (#347: in its block of WORK): its facts
/// with who works in it at their right (#292), its app and stack there
/// where `mine` stands in no column beside the name; then its stack's
/// error, the migrations it lacks, whether a merge of develop conflicts,
/// where it collides.
pub(super) fn body(scene: &mut Scene<Act>, node: &Node, cells: &Cells, mine: Option<Piece>, ctx: &Ctx) {
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
            if parts_width(&facts) + 2 + width_of(&agent.spans) + 2 + width_of(&aside.spans) <= scene.width() =>
        {
            vec![agent.then(2, aside)]
        }
        (agent, aside) => agent.into_iter().chain(aside).collect(),
    };
    let mut from = 0;
    for spans in rights {
        from = beside(scene, &facts, from, spans);
    }
    if from < facts.len() || (from == 0 && !facts.is_empty()) {
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
}

/// `▸ 2/6 boxes · 2 open`, the fold of its issue's boxes (#346) in a block
/// without an MR, whose bar would hold it (#347): a click or Enter lists
/// them under it, another folds them. None without boxes, and none for
/// `--once`.
pub(super) fn boxes_line(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) {
    let Some(number) = node.number.filter(|_| !node.boxes.is_empty() && !ctx.is_once) else {
        return;
    };
    let all = node.boxes.iter().flat_map(|section| &section.boxes);
    let (ticked, total) = all.fold((0, 0), |(ticked, total), line| {
        (ticked + usize::from(line.is_ticked), total + 1)
    });
    let is_open = ctx.folds.boxes.worktrees.contains(&number);
    scene.block(Act::Fold(Fold::TreeBoxes(number)), |scene| {
        let mut spans = vec![boxes::mark(is_open), look::dim(format!("{ticked}/{total} boxes"))];
        if ticked < total {
            spans.push(look::span(
                format!(" · {} open", total - ticked),
                theme::tone(theme::Tone::Yellow),
            ));
        }
        scene.atom_gap(spans, 0);
        scene.end_line();
        if is_open {
            boxes::list(scene, &node.boxes, 2);
        }
    });
}

/// What the node says of its work in plain words (#341), each wrapped under
/// its text: `problem` from its issue, then `now` (its agent's note) or
/// `said` (the first line of its last answer) with how long ago.
pub(super) fn said_lines(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) {
    if !node.has_tree {
        return;
    }
    let mut labeled = |label: &str, text: &str, age: Option<String>| {
        scene.atom(vec![look::dim(label)]);
        scene.pad_to(SAID_AT);
        scene.indent(SAID_AT);
        look::say(scene, text, ratatui::style::Style::new());
        // The age one atom: `5 min ago` never splits over two lines.
        if let Some(age) = age {
            look::say(scene, &format!("·\u{a0}{}", age.replace(' ', "\u{a0}")), theme::dim());
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
        let is_develop = is_base(&overlap.name);
        let files = develop_files(node, overlap);
        if files.is_empty() {
            continue;
        }
        match groups
            .iter_mut()
            .find(|(names, known)| !is_develop && !is_base(names[0]) && *known == files)
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
            let mut atom = vec![if is_base(name) {
                look::bold(name)
            } else {
                look::span(name, look::pink())
            }];
            if is_base(name) && is_clean {
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
        .filter(|file| !is_base(&overlap.name) || !conflicts.contains(file))
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
    let base = node
        .collides
        .iter()
        .map(|overlap| overlap.name.as_str())
        .find(|name| is_base(name))
        .unwrap_or(DEVELOP);
    scene.atom(vec![look::span(
        format!("✗ a merge of {base} conflicts in"),
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
            let label = format!(
                "▶ {}",
                node.dir.as_deref().map_or_else(|| "migrate".into(), migrate_run)
            );
            let rects = scene.atom_gap(vec![look::span(&label, theme::tone(theme::Tone::Cyan))], 2);
            button_on(scene, &rects, &label, theme::Tone::Cyan, migrate);
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

/// What applies a worktree's migrations (`[stack.migrations] run`), as its button says it.
pub fn migrate_run(dir: &std::path::Path) -> String {
    breadboard_core::stack::of(dir)
        .and_then(|stack| stack.migrations)
        .map_or_else(|| "migrate".into(), |migrations| migrations.run)
}

/// The name of develop among the collisions (core's tree).
const DEVELOP: &str = "develop";

/// Whether a collision is with the worktree's base, not another worktree:
/// develop or a branch (`epic/12-x`); worktrees are named `#12` or kebab-case.
fn is_base(name: &str) -> bool {
    name == DEVELOP || name.contains('/')
}

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
pub(super) fn right(scene: &mut Scene<Act>, spans: Vec<Span<'static>>) {
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
}
