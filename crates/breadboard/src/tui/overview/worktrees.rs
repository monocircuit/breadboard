//! The worktree's pieces of a block in WORK (`work`, #347; tree.mjs before
//! it), each on the line of its kind (#390): who works in it, its dot
//! pulsing while its run is in flight, and what it does in words; its app
//! and stack (#332: a click opens the app or says how to start it, `▶ stack
//! down` starts the stack), the stack's error and the migrations its
//! database lacks with `▶ pnpm pg:migrate`; its commits for the GitLab
//! line, whether a merge of develop conflicts; where its work collides with
//! develop and the other worktrees (#332), folded to one line until a click
//! lists the files, each a click into that worktree's nvim.

use super::actor::spawn::Run;
use super::{Act, Ctx, Fold, boxes, look};
use crate::tui::Scene;
use crate::tui::scene::{Region, Target, width_of};
use crate::tui::theme;
use breadboard_core::model::develop::Migrations;
use breadboard_core::model::tree::Node;
use breadboard_core::model::{self, Paint, counted};
use breadboard_core::time::ago;
use ratatui::layout::Rect;
use ratatui::text::Span;

/// The columns of the live worktrees' app and stack (#332, lazygit's
/// branches): each block's stack line pads its app to the widest, so the
/// stacks line up down the section.
pub(super) struct Cells {
    /// Whether any worktree has an .env.local: without, there is nothing to show.
    pub(super) is_any_env: bool,
    /// The widest app cell: the stacks line up after it.
    app: u16,
}

impl Cells {
    /// The cells of `nodes`.
    pub(super) fn of(nodes: &[&Node]) -> Cells {
        let with_tree: Vec<&Node> = nodes.iter().copied().filter(|node| node.has_tree).collect();
        Cells {
            is_any_env: with_tree.iter().any(|node| node.app.is_some()),
            app: with_tree
                .iter()
                .map(|node| width_of(&look::spans(&[app_of(node)])))
                .max()
                .unwrap_or(0),
        }
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

/// How long the dot of an agent at work is bright, then dim (#390): it
/// pulses while its headless run is in flight.
const PULSE: std::time::Duration = std::time::Duration::from_millis(800);

/// Who works in it (#292, #390): `● Lina Zhou  (2 issues · 1 MR done)
/// [1.6 M tokens · ≈ $33, today 895 k · ≈ $11]  48%`, the dot in the colour
/// of whom it waits for, pulsing while its run is in flight; its name before
/// a handover. Under it, indented, what it does or waits for in words.
pub(super) fn agent_lines(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) {
    if let Some(agent) = &node.agent {
        let mut dot = look::style_of(&agent.dot);
        if node.state.key == "working" && !ctx.is_once {
            let phase = ctx.now.max(0) as u128 / PULSE.as_millis();
            if phase % 2 == 1 {
                dot = dot.add_modifier(ratatui::style::Modifier::DIM);
            } else {
                dot = dot.add_modifier(ratatui::style::Modifier::BOLD);
            }
            scene.redraw_in(PULSE);
        }
        scene.atom_gap(vec![look::span("●", dot), Span::raw(" "), look::bold(&agent.name)], 0);
        if !node.lines.renamed.is_empty() {
            scene.atom(look::spans(&node.lines.renamed));
        }
        scene.atom_gap(vec![look::dim(agent.counts())], 2);
        if let Some(spend) = agent.spend() {
            scene.atom_gap(vec![look::dim(spend)], 2);
        }
        if !node.lines.context.is_empty() {
            scene.atom_gap(look::spans(&node.lines.context), 2);
        }
        scene.end_line();
    }
    let Some(doing) = &node.doing else { return };
    scene.indent(DOING_AT);
    scene.pad_to(DOING_AT);
    let parts: Vec<Vec<Span<'static>>> = doing
        .parts
        .iter()
        .map(|part| look::spans(std::slice::from_ref(part)))
        .collect();
    look::parted(scene, &parts);
    // The age one atom: `5 min ago` never splits over two lines.
    if let Some(at) = doing.at {
        look::say(
            scene,
            &format!("·\u{a0}{}", ago(at, ctx.now).replace(' ', "\u{a0}")),
            theme::dim(),
        );
        look::ticks(scene);
    }
    scene.indent(0);
    scene.end_line();
}

/// Where what an agent does starts: under its name, after its dot.
const DOING_AT: u16 = 2;

/// Its stack (#332, #390): `○ :3015  ▶ stack down`, then the stack's error
/// and the migrations its database lacks. Nothing where no worktree has an
/// .env.local: the section says that once.
pub(super) fn stack_lines(scene: &mut Scene<Act>, node: &Node, cells: &Cells, ctx: &Ctx) {
    if cells.is_any_env {
        let piece = cells.piece(node, ctx);
        let rects = scene.atom_gap(piece.spans.clone(), 0);
        piece.place(scene, &rects);
        scene.end_line();
    }
    stack_error_line(scene, node, ctx);
    migrations_line(scene, node, ctx);
}

/// Its commits for the GitLab line (#390): `↑15 ↓6 · 2 changed`, `14
/// commits not pushed`, `✓ tested 1a2b3c4d`; a check that runs is what its
/// agent does, said there.
pub(super) fn commit_parts(node: &Node) -> Vec<Vec<Span<'static>>> {
    let lines = &node.lines;
    let mut parts = Vec::new();
    if !lines.commits.is_empty() {
        parts.push(look::spans(&lines.commits));
    }
    if let Some(unpushed) = &lines.unpushed {
        parts.push(look::spans(std::slice::from_ref(unpushed)));
    }
    if let Some(tested) = lines.tested.as_ref().filter(|tested| tested.text != "checking…") {
        parts.push(look::spans(std::slice::from_ref(tested)));
    }
    parts
}

/// `▸ 2/6 boxes · 2 open`, the fold of its issue's boxes (#346) in a block
/// without an MR, whose bar would hold it (#347), on its GitLab line: a
/// click or Enter lists them under it (`boxes_list`), another folds them.
/// The line it is on, None without boxes and for `--once`.
pub(super) fn boxes_atom(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) -> Option<u16> {
    let number = node.number.filter(|_| !node.boxes.is_empty() && !ctx.is_once)?;
    let all = node.boxes.iter().flat_map(|section| &section.boxes);
    let (ticked, total) = all.fold((0, 0), |(ticked, total), line| {
        (ticked + usize::from(line.is_ticked), total + 1)
    });
    let is_open = ctx.folds.boxes.worktrees.contains(&number);
    let mut spans = vec![boxes::mark(is_open), look::dim(format!("{ticked}/{total} boxes"))];
    if ticked < total {
        spans.push(look::span(
            format!(" · {} open", total - ticked),
            theme::tone(theme::Tone::Yellow),
        ));
    }
    let label: String = spans.iter().map(|span| span.content.as_ref()).collect();
    let rects = scene.atom(spans);
    button_on(
        scene,
        &rects,
        label.trim(),
        theme::Tone::Gray,
        Act::Fold(Fold::TreeBoxes(number)),
    );
    rects.first().map(|rect| rect.y)
}

/// The boxes `boxes_atom`'s fold lists, while it is open.
pub(super) fn boxes_list(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) {
    if node
        .number
        .is_some_and(|number| ctx.folds.boxes.worktrees.contains(&number))
    {
        boxes::list(scene, &node.boxes, 2);
    }
}

/// Where its work collides (#332), a fold of its own (#390): folded, one
/// line, `▸ collides with develop · 5 MRs: #376 #379 #380 #382 #384`; a
/// click lists the shared files of each (`collision_lines`). Its state is
/// kept with the boxes' (`<name>.folds`). `--once` prints the line alone.
pub(super) fn collides(scene: &mut Scene<Act>, node: &Node, ctx: &Ctx) {
    let groups = groups_of(node);
    if groups.is_empty() {
        return;
    }
    let Some(number) = node.number else {
        collision_lines(scene, node, &groups);
        return;
    };
    let is_open = !ctx.is_once && ctx.folds.boxes.collides.contains(&number);
    let names: Vec<&str> = groups.iter().flat_map(|(names, _)| names.iter().copied()).collect();
    let others: Vec<&str> = names.iter().copied().filter(|name| !is_base(name)).collect();
    let base = names.iter().copied().find(|name| is_base(name));
    let mut said = String::from("collides with ");
    if let Some(base) = base {
        said.push_str(base);
        if !others.is_empty() {
            said.push_str(" · ");
        }
    }
    if !others.is_empty() {
        said.push_str(&format!("{}: {}", counted(others.len() as u64, "MR"), others.join(" ")));
    }
    let yellow = look::style_of(&Paint::Yellow);
    let line = |scene: &mut Scene<Act>| {
        let mut lead = Vec::new();
        if !ctx.is_once {
            lead.push(boxes::mark(is_open));
        }
        lead.push(look::span("⚠", yellow));
        scene.atom_gap(lead, 0);
        look::say(scene, &said, yellow);
        scene.end_line();
    };
    if ctx.is_once {
        line(scene);
    } else {
        scene.block(Act::Fold(Fold::Collides(number)), line);
    }
    if is_open {
        collision_lines(scene, node, &groups);
    }
}

/// Its tries a checker answered or holds (#384): `try #1  checked by haiku ·
/// looks good · <reason>`, a click on it opens what the checker said in the
/// worktree's nvim (its evidence beside it), `overturn` opens the try again
/// for the user while its MR is open.
pub(super) fn checked_lines(scene: &mut Scene<Act>, node: &Node) {
    let Some(dir) = node.dir.as_ref().filter(|_| node.has_tree) else {
        return;
    };
    for checked in &node.checked {
        let line = |scene: &mut Scene<Act>| {
            scene.atom(vec![look::dim(format!("try #{}", checked.id))]);
            scene.pad_to(CHECKED_AT);
            scene.indent(CHECKED_AT);
            look::say(scene, &checked.text, theme::dim());
            scene.indent(0);
            scene.end_line();
        };
        match &checked.evidence {
            Some(file) => scene.block(
                Act::OpenFile {
                    dir: dir.clone(),
                    file: file.clone(),
                    line: None,
                },
                line,
            ),
            None => line(scene),
        }
        if checked.can_overturn {
            scene.pad_to(CHECKED_AT);
            scene.button(
                "overturn",
                theme::Tone::Yellow,
                Act::Overturn {
                    dir: dir.clone(),
                    id: checked.id,
                },
            );
            scene.atom_gap(vec![look::dim("opens the try again for you")], 2);
            scene.end_line();
        }
    }
}

/// Where a checked try's text starts, after `try #N`.
const CHECKED_AT: u16 = 8;

/// The worktrees its work collides with, by the files they share (#332):
/// develop (or the base branch) on its own, the worktrees that share the
/// same files one group; develop's files without the ones a merge conflicts
/// in (the conflict line names those).
fn groups_of(node: &Node) -> Vec<(Vec<&str>, Vec<&String>)> {
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
    groups
}

/// The collisions unfolded (#332, DIFF's `⚠ also changed on develop` / `in
/// <worktree>`), a line per group: `develop, merges cleanly: tree.mjs`,
/// `#304: render.mjs, web/src/index.ts`; each file by the shortest end of
/// its path none of the files near it shares (core's `near`), a click opens
/// it in the worktree's nvim at its first change (DIFF's click).
fn collision_lines(scene: &mut Scene<Act>, node: &Node, groups: &[(Vec<&str>, Vec<&String>)]) {
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
    // Both changed them, and git merges them cleanly (DEVELOP's `✓ merges cleanly`).
    let is_clean = node.conflicts.as_ref().is_some_and(Vec::is_empty);
    for (names, files) in groups {
        scene.end_line();
        scene.indent(COLLISIONS_AT);
        scene.pad_to(COLLISIONS_AT);
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
        for (at, file) in files.iter().enumerate() {
            let after = if at + 1 < files.len() { "," } else { "" };
            link_atom(scene, Span::raw(short_name(file, &near)), after, open(file));
        }
        scene.indent(0);
        scene.end_line();
    }
}

/// Where the unfolded collisions start: under the fold's text.
const COLLISIONS_AT: u16 = 2;

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
pub(super) fn conflict_line(scene: &mut Scene<Act>, node: &Node) {
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
