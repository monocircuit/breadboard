//! The view of everything (no flag, render.mjs): every worktree with its app,
//! stack, MR and work; the stacks no worktree has; GitLab's open MRs and
//! issues, the claimed ones first.
//!
//! Each list is a table: its columns as wide as their widest cell, the last
//! one wrapping under itself, nothing cut (#304). The cells that tell most
//! come before the one that wraps: a worktree's MR and pipeline stand on its
//! line, its work after them, shortened to its check when the line is short
//! (a worktree is one line at 80 columns, as in Node). An MR is its title,
//! then its branch and owner; where its cells leave the title too little room
//! the cells go under the title, whichever takes fewer rows. Live, the issues
//! get what the pane has left (render.mjs), at least 3, then `… N more: glab
//! issue list`; `--once` prints them all. Every row is a block (#296); a
//! click goes where it names.

use super::{Act, Ctx, look};
use crate::tui::band;
use crate::tui::scene::width_of;
use crate::tui::theme::{self, Tone};
use crate::tui::{Header, Page, Scene, Section};
use breadboard_core::check::CheckState;
use breadboard_core::git::orchestrations_dir;
use breadboard_core::gitlab::read::{EverythingRemote, ListedIssue, ListedMr};
use breadboard_core::herdr;
use breadboard_core::model::everything::{EverythingModel, TreeRow, stack_cell};
use breadboard_core::model::order::Go;
use breadboard_core::model::tree::{check_fact, pipeline_mark};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::path::PathBuf;

/// The narrowest a wrapping last column gets before a row becomes a card (lib/tty.mjs `MIN_WRAP`).
const MIN_WRAP: u16 = 16;
/// Columns between two cells.
const GAP: u16 = 2;
/// The least a worktree's work column gets before its rows become cards: its
/// check in short (`untested`, `✓ tested`, `checking…`), wider ones wrap in it.
const WORK_AT_LEAST: u16 = 9;
/// The fewest issues the live view shows, however full the pane (render.mjs).
const ISSUES_AT_LEAST: usize = 3;
/// The rows under the issues: the `… N more` line, the blank after the
/// section, the footer and the blank after it.
const UNDER_ISSUES: u16 = 4;
/// Where a worktree's name starts: after the `→` of the one the overview runs in.
const NAME_AT: u16 = 2;

pub fn build(page: &mut Page<Act>, model: &EverythingModel, ctx: &Ctx) {
    // Node: `process.stdout.rows || 40`.
    let height = (!ctx.is_once).then(|| crossterm::terminal::size().map_or(40, |(_, rows)| rows));
    build_in(page, model, ctx, height);
}

/// The view in a pane `height` rows high; None (`--once`): every issue.
fn build_in(page: &mut Page<Act>, model: &EverythingModel, ctx: &Ctx, height: Option<u16>) {
    let rows: &[TreeRow] = model.trees.as_ref().map(|rows| &rows[..]).unwrap_or(&[]);
    let main = rows.iter().find(|row| row.tree.is_main).map(|row| row.tree.dir.clone());
    match &model.trees {
        Ok(rows) => page.section("WORKTREES", |scene| worktrees(scene, rows, model, ctx)),
        Err(why) => page.failed("WORKTREES", why),
    }
    if !model.orphans.is_empty() {
        page.section("STACKS WITHOUT A WORKTREE", |scene| orphans(scene, &model.orphans));
    }
    if let Some(error) = &ctx.gitlab.error {
        page.section(Section::new("GITLAB").plain(), |scene| {
            look::say(scene, error, theme::tone(Tone::Red));
        });
    }
    match &model.remote {
        None => {
            if ctx.gitlab.error.is_none() {
                page.section(Section::new("GITLAB").plain(), |scene| {
                    look::say(scene, &ctx.gitlab.progress, theme::dim());
                });
            }
        }
        Some(remote) => {
            page.section("MERGE REQUESTS", |scene| {
                merge_requests(scene, remote, rows, main.as_ref())
            });
            // What the pane has left under the band, the rows so far, the
            // section's title and what comes under it (render.mjs `used`).
            let room = height.map(|height| {
                let band = band::lines(&Header::default(), "", page.inner() + 2).len() as u16;
                height.saturating_sub(band + page.rows() + 1 + UNDER_ISSUES)
            });
            page.section("ISSUES", |scene| issues(scene, remote, rows, main.as_ref(), room));
        }
    }
    page.section(Section::new("FOOTER").plain(), |scene| {
        scene.text(
            "pnpm stack:up|down|reset --port <p> · pnpm wt:new <n-feature> · pnpm commands",
            theme::dim(),
        );
    });
}

/// Where the columns of a table start: one per fixed width (a width of 0
/// takes no column), then the last, the one that wraps. None when the last
/// would get fewer than `least` columns of `room`: then rows are cards.
fn columns(start: u16, widths: &[u16], room: u16, least: u16) -> Option<Vec<u16>> {
    let mut at = Vec::with_capacity(widths.len() + 1);
    let mut x = start;
    for &width in widths {
        at.push(x);
        if width > 0 {
            x += width + GAP;
        }
    }
    at.push(x);
    (x + least <= room).then_some(at)
}

/// The widest of some cells.
fn widest<'a>(cells: impl Iterator<Item = &'a Vec<Span<'static>>>) -> u16 {
    cells.map(|cell| width_of(cell)).max().unwrap_or(0)
}

/// A row of a table: each fixed cell at its column, then `last` from the
/// last column on, wrapping under it.
fn table_row(scene: &mut Scene<Act>, at: &[u16], cells: Vec<Vec<Span<'static>>>, last: impl FnOnce(&mut Scene<Act>)) {
    for (cell, &column) in cells.into_iter().zip(at) {
        if cell.is_empty() {
            continue;
        }
        scene.pad_to(column);
        scene.indent(column);
        scene.atom_gap(cell, 0);
    }
    let column = at.last().copied().unwrap_or(0);
    scene.pad_to(column);
    scene.indent(column);
    last(scene);
    scene.indent(0);
}

/// GAP columns before the next atom, unless the line has nothing on it yet.
fn gap(scene: &mut Scene<Act>, indent: u16) {
    if scene.x() > indent {
        scene.pad_to(scene.x() + GAP - 1);
    }
}

/// Spaces after `spans` up to `width`: a cell padded to its column.
fn padded(mut spans: Vec<Span<'static>>, width: u16) -> Vec<Span<'static>> {
    let short = width.saturating_sub(width_of(&spans));
    if short > 0 {
        spans.push(Span::raw(" ".repeat(usize::from(short))));
    }
    spans
}

/// The app of a checkout: `● :3002` up, `○ :3002` down, `⇒ :3000` shared, `—` without env.
fn app_cell(row: &TreeRow) -> Vec<Span<'static>> {
    let tree = &row.tree;
    vec![if !tree.has_env {
        look::dim("—")
    } else if !tree.has_own_port {
        look::dim("⇒ :3000")
    } else if tree.is_up {
        look::span(format!("● :{}", 3000 + tree.offset), theme::tone(Tone::Green))
    } else {
        look::dim(format!("○ :{}", 3000 + tree.offset))
    }]
}

/// A worktree's name in herdr's pink, its branch when that says more.
fn name_cell(row: &TreeRow) -> Vec<Span<'static>> {
    let mut spans = vec![look::span(&row.tree.name, look::pink().add_modifier(Modifier::BOLD))];
    if let Some(branch) = &row.branch {
        spans.push(look::dim(format!(" {branch}")));
    }
    spans
}

/// The rows under their orchestration: first the ones without (the main
/// checkout leads), then each orchestration's in the order it first shows,
/// `git worktree list`'s order inside each.
fn grouped(rows: &[TreeRow]) -> Vec<&TreeRow> {
    let mut names: Vec<&str> = vec![];
    for row in rows {
        if let Some(origin) = &row.tree.origin
            && !names.contains(&origin.name.as_str())
        {
            names.push(&origin.name);
        }
    }
    let group = |row: &TreeRow| match &row.tree.origin {
        None => 0,
        Some(origin) => 1 + names.iter().position(|name| *name == origin.name).unwrap_or(0),
    };
    let mut order: Vec<&TreeRow> = rows.iter().collect();
    order.sort_by_key(|row| group(row));
    order
}

/// Facts parted by a dim ` ·`: each whole on a line when it fits one, else as words.
fn facts(scene: &mut Scene<Act>, parts: &[Vec<Span<'static>>], indent: u16) {
    let room = scene.width().saturating_sub(indent);
    for (i, part) in parts.iter().enumerate() {
        let mut spans = part.clone();
        if i + 1 < parts.len() {
            spans.push(look::dim(" ·"));
        }
        if width_of(&spans) <= room {
            scene.atom(spans);
        } else {
            for word in look::words_of(&spans) {
                scene.atom(word);
            }
        }
    }
}

/// The open MR of each row, `!303 draft ✓ passed`, its parts aligned over the rows.
fn mr_cells(rows: &[&TreeRow]) -> Vec<Vec<Span<'static>>> {
    let iid = rows
        .iter()
        .filter_map(|row| row.mr.as_ref())
        .map(|mr| format!("!{}", mr.iid).len() as u16)
        .max()
        .unwrap_or(0);
    let is_any_draft = rows.iter().any(|row| row.mr.as_ref().is_some_and(|mr| mr.is_draft));
    rows.iter()
        .map(|row| {
            let Some(mr) = &row.mr else { return vec![] };
            let mut spans = padded(vec![look::span(format!("!{}", mr.iid), Style::new())], iid);
            if is_any_draft {
                spans.push(look::dim(if mr.is_draft { " draft" } else { "      " }));
            }
            spans.push(Span::raw(" "));
            spans.extend(look::spans(&[pipeline_mark(mr.pipeline.as_ref())]));
            spans
        })
        .collect()
}

/// What a worktree's work is: `1 ahead · 2 changed · ✓ tested 1a2b3c4d`, or `clean`.
fn work_of(row: &TreeRow) -> Vec<Vec<Span<'static>>> {
    let tree = &row.tree;
    let mut parts = Vec::new();
    if tree.ahead > 0 {
        parts.push(vec![look::span(format!("{} ahead", tree.ahead), Style::new())]);
    }
    parts.extend(changed_of(row));
    if let Some(fact) = check_fact(&tree.checked) {
        parts.push(look::spans(&[fact]));
    }
    if parts.is_empty() {
        parts.push(vec![look::dim("clean")]);
    }
    parts
}

fn changed_of(row: &TreeRow) -> Option<Vec<Span<'static>>> {
    (row.tree.changed > 0).then(|| {
        vec![look::span(
            format!("{} changed", row.tree.changed),
            theme::tone(Tone::Yellow),
        )]
    })
}

/// The work in a word, for a column too narrow for all of it: the check
/// (`untested`, `✓ tested`, `✗ failed`, `checking…`), else what there is
/// (`2 changed`, `1 ahead`, `clean`), first.
fn short_work_of(row: &TreeRow) -> Vec<Vec<Span<'static>>> {
    let check = match &row.tree.checked {
        CheckState::Tested { .. } => Some(("✓ tested", Tone::Green)),
        CheckState::Checking { .. } => Some(("checking…", Tone::Cyan)),
        CheckState::Failed { .. } => Some(("✗ failed", Tone::Red)),
        CheckState::Untested { .. } => Some(("untested", Tone::Yellow)),
        CheckState::None => None,
    };
    match check {
        Some((word, tone)) => vec![vec![look::span(word, theme::tone(tone))]],
        None => work_of(row).into_iter().take(1).collect(),
    }
}

/// How wide facts are on one line: each, then ` ·` and a space between.
fn facts_width(parts: &[Vec<Span<'static>>]) -> u16 {
    let words: u16 = parts.iter().map(|part| width_of(part)).sum();
    words + 3 * (parts.len().max(1) as u16 - 1)
}

/// The work for a column `room` wide: all of it when it fits; else its
/// check in a word there, and all of it on a line of its own under the
/// name (the second): nothing is dropped (#304).
fn work_in(row: &TreeRow, room: u16) -> (Vec<Vec<Span<'static>>>, Option<Vec<Vec<Span<'static>>>>) {
    let all = work_of(row);
    if facts_width(&all) <= room {
        return (all, None);
    }
    (short_work_of(row), Some(all))
}

/// A worktree's work that its column could not hold, on the row's next line
/// under its name: the column above keeps the check, this says the rest.
fn work_line(scene: &mut Scene<Act>, detail: Option<Vec<Vec<Span<'static>>>>) {
    let Some(detail) = detail else { return };
    let indent = NAME_AT + GAP;
    scene.end_line();
    scene.pad_to(indent);
    scene.indent(indent);
    facts(scene, &detail, indent);
    scene.indent(0);
}

/// A row's click: its space in herdr; outside herdr its app when that runs (render.mjs:366).
fn tree_click(row: &TreeRow, is_in_herdr: bool) -> Act {
    let tree = &row.tree;
    if !is_in_herdr && tree.is_up && tree.has_env {
        let port = 3000 + if tree.has_own_port { tree.offset } else { 0 };
        Act::Go(Go::Show {
            url: format!("http://localhost:{port}"),
            dir: Some(tree.dir.clone()),
        })
    } else {
        Act::Go(Go::FocusSpace(tree.dir.clone()))
    }
}

fn worktrees(scene: &mut Scene<Act>, rows: &[TreeRow], model: &EverythingModel, ctx: &Ctx) {
    // Without an .env.local anywhere the app and stack cells of every row
    // would say the same: the title says it once.
    let has_env = rows.iter().any(|row| row.tree.has_env);
    scene.note(if has_env {
        "click one to go to its space"
    } else {
        "click one to go to its space · no .env.local in any"
    });
    let main = rows
        .iter()
        .find(|row| row.tree.is_main)
        .map(|row| row.tree.dir.as_path());
    let rows = grouped(rows);
    let names: Vec<_> = rows.iter().map(|row| name_cell(row)).collect();
    let cell = |row: &TreeRow, of: fn(&TreeRow) -> Vec<Span<'static>>| if has_env { of(row) } else { vec![] };
    let apps: Vec<_> = rows.iter().map(|row| cell(row, app_cell)).collect();
    let stacks: Vec<_> = rows
        .iter()
        .map(|row| cell(row, |row| look::spans(&[stack_cell(&row.tree)])))
        .collect();
    let mrs = mr_cells(&rows);
    let widths = [
        widest(names.iter()),
        widest(apps.iter()),
        widest(stacks.iter()),
        widest(mrs.iter()),
    ];
    // The work column holds its check in short at least, then a row stays one line.
    let at = columns(NAME_AT, &widths, scene.width(), WORK_AT_LEAST);
    // A card's second line lines up as a table of its own when it can.
    let under = columns(NAME_AT, &widths[1..], scene.width(), WORK_AT_LEAST);
    let is_in_herdr = herdr::is_in_herdr();
    let cells = names.into_iter().zip(apps).zip(stacks).zip(mrs);
    let mut group: Option<&str> = None;
    for (row, (((name, app), stack), mr)) in rows.iter().zip(cells) {
        // An orchestration's name heads its worktrees (`⇶ breadboard`) in
        // cyan: a line once, where Node said it after every name.
        if let Some(origin) = &row.tree.origin
            && group != Some(origin.name.as_str())
        {
            group = Some(&origin.name);
            scene.end_line();
            // A click brings its space `⇶ <title>` to the front, which stands on its checkout.
            let checkout = main.map(|main| orchestrations_dir(main).join(&origin.name));
            let click = checkout.map_or(Act::Go(Go::Nothing), |dir| Act::Go(Go::FocusSpace(dir)));
            scene.block(click, |scene| {
                scene.pad_to(NAME_AT);
                scene.indent(NAME_AT);
                look::flow(
                    scene,
                    &[look::span(format!("⇶ {}", origin.name), theme::tone(Tone::Cyan))],
                );
                scene.indent(0);
            });
            scene.end_line();
        }
        scene.block(tree_click(row, is_in_herdr), |scene| {
            if row.tree.is_here {
                scene.atom(vec![look::span("→", theme::tone(Tone::Cyan))]);
            }
            let work = work_of(row);
            match &at {
                Some(at) => {
                    let column = at.last().copied().unwrap_or(0);
                    let (work, detail) = work_in(row, scene.width().saturating_sub(column));
                    table_row(scene, at, vec![name, app, stack, mr], |scene| {
                        facts(scene, &work, column);
                    });
                    work_line(scene, detail);
                }
                None => {
                    // A card: the name on its line, the rest under it, unless
                    // the row has only its work and that fits after the name
                    // (`→ platform develop  clean`): the aligned cells stay aligned.
                    let is_one_line = [&app, &stack, &mr].iter().all(|cell| cell.is_empty())
                        && NAME_AT + width_of(&name) + work.iter().map(|part| width_of(part) + GAP).sum::<u16>()
                            <= scene.width();
                    scene.pad_to(NAME_AT);
                    scene.indent(NAME_AT);
                    look::flow(scene, &name);
                    if is_one_line {
                        gap(scene, NAME_AT);
                        facts(scene, &work, NAME_AT);
                        scene.indent(0);
                        return;
                    }
                    scene.end_line();
                    match &under {
                        Some(under) => {
                            let column = under.last().copied().unwrap_or(0);
                            let (work, detail) = work_in(row, scene.width().saturating_sub(column));
                            table_row(scene, under, vec![app, stack, mr], |scene| {
                                facts(scene, &work, column);
                            });
                            work_line(scene, detail);
                        }
                        None => {
                            for cell in [app, stack, mr] {
                                if !cell.is_empty() {
                                    scene.atom_gap(cell, GAP);
                                }
                            }
                            gap(scene, NAME_AT);
                            facts(scene, &work, NAME_AT);
                        }
                    }
                    scene.indent(0);
                }
            }
        });
    }
    if model.can_start_main
        && let Some(main) = rows.iter().find(|row| row.tree.is_main)
    {
        scene.blank();
        let start = Act::StartStack(main.tree.dir.clone());
        scene.block(start.clone(), |scene| {
            scene.button("▶ start the default stack", Tone::Cyan, start);
            let hint = if ctx.is_once { "pnpm stack:up" } else { "click or u" };
            scene.text(hint, theme::dim());
        });
    }
    if !model.has_docker {
        scene.blank();
        scene.text(
            "docker does not answer: the stacks are unknown",
            theme::tone(Tone::Yellow),
        );
    }
}

/// vita's stacks no worktree has, and how to drop one with its data.
fn orphans(scene: &mut Scene<Act>, orphans: &[(String, String)]) {
    let names: Vec<_> = orphans
        .iter()
        .map(|(name, _)| vec![look::span(name, Style::new())])
        .collect();
    let states: Vec<_> = orphans
        .iter()
        .map(|(_, status)| vec![look::span(status, theme::tone(Tone::Yellow))])
        .collect();
    let at = columns(
        NAME_AT,
        &[widest(names.iter()), widest(states.iter())],
        scene.width(),
        MIN_WRAP,
    );
    for ((name, state), (project, _)) in names.into_iter().zip(states).zip(orphans) {
        let down = format!("docker compose -p {project} down -v");
        match &at {
            Some(at) => table_row(scene, at, vec![name, state], |scene| {
                look::flow(scene, &[look::dim(&down)]);
            }),
            None => {
                scene.pad_to(NAME_AT);
                scene.indent(NAME_AT);
                scene.atom(name);
                scene.atom_gap(state, GAP);
                scene.end_line();
                look::flow(scene, &[look::dim(&down)]);
                scene.indent(0);
            }
        }
        scene.end_line();
    }
}

/// The worktree with `branch`, else the main checkout: where a page of it opens.
fn group_of(rows: &[TreeRow], branch: Option<&str>, main: Option<&PathBuf>) -> Option<PathBuf> {
    branch
        .and_then(|branch| rows.iter().find(|row| row.tree.branch == branch))
        .map(|row| row.tree.dir.clone())
        .or_else(|| main.cloned())
}

/// The least a column of titles gets: half the line, so a title reads in
/// two lines, not five; less, and rows are cards.
fn prose(scene: &Scene<Act>) -> u16 {
    (scene.width() / 2).max(MIN_WRAP)
}

/// How many rows `draw` writes in a scene `width` wide: a list is laid out
/// the way that takes fewer, and the issues are cut by rows.
fn rows_in(width: u16, draw: impl FnOnce(&mut Scene<Act>)) -> u16 {
    let mut scene = Scene::new(width);
    draw(&mut scene);
    scene.end_line();
    scene.y()
}

/// `spans` GAP columns after what is on the line: whole there or on the
/// next line when they fit one from `indent`, else as words.
fn trail(scene: &mut Scene<Act>, spans: Vec<Span<'static>>, indent: u16) {
    if width_of(&spans) <= scene.width().saturating_sub(indent) {
        scene.atom_gap(spans, GAP);
    } else {
        gap(scene, indent);
        look::flow(scene, &spans);
    }
}

/// Dim `<branch> · <who>` (render.mjs), after an MR's title.
fn mr_tail(mr: &ListedMr) -> Vec<Span<'static>> {
    let mut tail = vec![look::dim(&mr.branch)];
    if let Some(who) = &mr.who {
        tail.push(look::dim(format!(" · {who}")));
    }
    tail
}

/// An MR as a table row: its cells, then its title wrapping under the title
/// column, its branch and owner after the title, on the title's last line
/// when they fit there.
fn mr_wide(scene: &mut Scene<Act>, at: &[u16], cells: Vec<Vec<Span<'static>>>, mr: &ListedMr) {
    let column = at.last().copied().unwrap_or(0);
    table_row(scene, at, cells, |scene| {
        look::flow(scene, &[look::span(&mr.title, Style::new())]);
        trail(scene, mr_tail(mr), column);
    });
}

/// An MR where its cells leave the title too little room: `!iid`, then its
/// title with the room of the line; under it, always at the start of its
/// own line, its state and pipeline padded to their columns, so the eye runs
/// down them as at 120 columns, then branch and owner where they fit whole.
fn mr_stacked(scene: &mut Scene<Act>, column: u16, widths: &[u16], cells: Vec<Vec<Span<'static>>>, mr: &ListedMr) {
    let mut cells = cells.into_iter();
    scene.atom(cells.next().unwrap_or_default());
    scene.indent(column);
    scene.pad_to(column);
    look::flow(scene, &[look::span(&mr.title, Style::new())]);
    scene.end_line();
    scene.pad_to(column);
    let mut state: Vec<Span<'static>> = vec![];
    for (cell, &width) in cells.zip(&widths[1..]) {
        if !state.is_empty() {
            state.push(Span::raw(" ".repeat(usize::from(GAP))));
        }
        state.extend(padded(cell, width));
    }
    scene.atom(state);
    trail(scene, mr_tail(mr), column);
    scene.indent(0);
}

fn merge_requests(scene: &mut Scene<Act>, remote: &EverythingRemote, rows: &[TreeRow], main: Option<&PathBuf>) {
    scene.note(format!("{} open", remote.mrs.len()));
    let cells_of = |mr: &ListedMr| {
        vec![
            vec![look::span(format!("!{}", mr.iid), Style::new())],
            vec![if mr.is_draft {
                look::dim("draft")
            } else {
                look::span("ready", theme::tone(Tone::Green))
            }],
            look::spans(&[pipeline_mark(mr.pipeline.as_ref())]),
        ]
    };
    let cells: Vec<_> = remote.mrs.iter().map(cells_of).collect();
    let widths: Vec<u16> = (0..3).map(|i| widest(cells.iter().map(|row| &row[i]))).collect();
    let width = scene.width();
    let at = columns(0, &widths, width, MIN_WRAP);
    let column = widths[0] + GAP;
    // The cells beside the title, unless under it takes fewer rows (80 columns).
    let is_stacked = at.as_ref().is_none_or(|at| {
        let count = |stacked: bool| -> u32 {
            remote
                .mrs
                .iter()
                .zip(&cells)
                .map(|(mr, cells)| {
                    u32::from(rows_in(width, |scene| match stacked {
                        true => mr_stacked(scene, column, &widths, cells.clone(), mr),
                        false => mr_wide(scene, at, cells.clone(), mr),
                    }))
                })
                .sum()
        };
        count(true) < count(false)
    });
    for (mr, cells) in remote.mrs.iter().zip(cells) {
        let click = Act::Go(Go::Show {
            url: mr.url.clone(),
            dir: group_of(rows, Some(&mr.branch), main),
        });
        scene.block(click, |scene| match (&at, is_stacked) {
            (Some(at), false) => mr_wide(scene, at, cells, mr),
            _ => mr_stacked(scene, column, &widths, cells, mr),
        });
    }
}

/// An issue's claim, short so the titles of all issues line up: cyan
/// `← !316` (its MR), `← claimed` (a branch, no MR yet), or dim `unclaimed`.
fn claim_of(issue: &ListedIssue) -> Vec<Span<'static>> {
    vec![match (&issue.branch, issue.mr) {
        (Some(_), Some(mr)) => look::span(format!("← !{mr}"), theme::tone(Tone::Cyan)),
        (Some(_), None) => look::span("← claimed", theme::tone(Tone::Cyan)),
        (None, _) => look::dim("unclaimed"),
    }]
}

/// After an issue's title: the branch that claims it in cyan, then dim
/// `<labels> @<who>` (render.mjs), each whole on a line when it fits one.
fn issue_tail(issue: &ListedIssue) -> Vec<Vec<Span<'static>>> {
    let mut tail = vec![];
    if let Some(branch) = &issue.branch {
        tail.push(vec![look::span(branch, theme::tone(Tone::Cyan))]);
    }
    let who = issue.who.as_ref().map(|who| format!("@{who}"));
    let rest: Vec<String> = [Some(issue.labels.join(",")).filter(|labels| !labels.is_empty()), who]
        .into_iter()
        .flatten()
        .collect();
    if !rest.is_empty() {
        tail.push(vec![look::dim(rest.join(" "))]);
    }
    tail
}

/// An issue's row: `#iid`, its claim, its title and what follows it; a card
/// (the number and claim on a line, the rest under it) where there's no room.
fn issue_row(scene: &mut Scene<Act>, at: Option<&[u16]>, cells: Vec<Vec<Span<'static>>>, issue: &ListedIssue) {
    let column = at.and_then(|at| at.last().copied()).unwrap_or(GAP);
    let rest = |scene: &mut Scene<Act>| {
        look::say(scene, &issue.title, Style::new());
        for part in issue_tail(issue) {
            trail(scene, part, column);
        }
    };
    match at {
        Some(at) => table_row(scene, at, cells, rest),
        None => {
            for (i, cell) in cells.into_iter().enumerate() {
                scene.atom_gap(cell, if i == 0 { 0 } else { GAP });
            }
            scene.end_line();
            scene.indent(GAP);
            scene.pad_to(GAP);
            rest(scene);
            scene.indent(0);
        }
    }
}

/// The issues, claimed first; with a `room` (live) as many as fit in that
/// many rows, at least ISSUES_AT_LEAST, then how many more there are.
fn issues(
    scene: &mut Scene<Act>,
    remote: &EverythingRemote,
    rows: &[TreeRow],
    main: Option<&PathBuf>,
    room: Option<u16>,
) {
    scene.note(format!("{} open, claimed first", remote.issues.len()));
    let cells_of = |issue: &ListedIssue| {
        vec![
            vec![look::span(format!("#{}", issue.iid), Style::new())],
            claim_of(issue),
        ]
    };
    let cells: Vec<_> = remote.issues.iter().map(cells_of).collect();
    let widths: Vec<u16> = (0..2).map(|i| widest(cells.iter().map(|row| &row[i]))).collect();
    // The title column holds a claiming branch whole.
    let branch = remote
        .issues
        .iter()
        .filter_map(|issue| issue.branch.as_ref())
        .map(|branch| branch.chars().count() as u16)
        .max()
        .unwrap_or(0);
    let at = columns(0, &widths, scene.width(), prose(scene).max(branch));
    let width = scene.width();
    let mut used = 0u16;
    let mut shown = 0;
    for (issue, mut cells) in remote.issues.iter().zip(cells) {
        if at.is_none() {
            // A card's number padded, so the claims line up.
            cells[0] = padded(std::mem::take(&mut cells[0]), widths[0]);
        }
        if let Some(room) = room {
            let height = rows_in(width, |scene| issue_row(scene, at.as_deref(), cells.clone(), issue));
            if shown >= ISSUES_AT_LEAST && used + height > room {
                break;
            }
            used += height;
        }
        shown += 1;
        let tree = issue
            .branch
            .as_ref()
            .and_then(|branch| rows.iter().find(|row| &row.tree.branch == branch));
        let click = match tree {
            Some(row) => Act::Go(Go::FocusSpace(row.tree.dir.clone())),
            None => Act::Go(Go::Show {
                url: issue.url.clone(),
                dir: main.cloned(),
            }),
        };
        scene.block(click, |scene| issue_row(scene, at.as_deref(), cells, issue));
    }
    let more = remote.issues.len() - shown;
    if more > 0 {
        scene.end_line();
        scene.text(&format!("… {more} more: glab issue list"), theme::dim());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::Section;
    use crate::tui::page::Sheet;
    use breadboard_core::check::CheckState;
    use breadboard_core::gitlab::Pipeline;
    use breadboard_core::model::Tree;
    use breadboard_core::record::Origin;

    fn tree(name: &str, is_main: bool) -> Tree {
        Tree {
            dir: PathBuf::from(format!("/repo/{name}")),
            name: name.into(),
            branch: if is_main {
                "develop".into()
            } else {
                format!("feature/{name}")
            },
            is_main,
            is_here: is_main,
            origin: (!is_main).then(|| Origin {
                name: "breadboard".into(),
                slot: 0,
            }),
            ahead: u64::from(!is_main),
            checked: if is_main {
                CheckState::None
            } else {
                CheckState::Untested { count: 1 }
            },
            ..Default::default()
        }
    }

    fn mr(iid: i64, branch: &str, status: &str) -> ListedMr {
        ListedMr {
            iid,
            title: "feat(breadboard): hover lights the whole block under the pointer".into(),
            branch: branch.into(),
            is_draft: iid % 2 == 0,
            who: Some("lukasdiegelmann1".into()),
            url: format!("https://gitlab.com/monocircuit/platform/-/merge_requests/{iid}"),
            pipeline: Some(Pipeline {
                status: status.into(),
                ..Default::default()
            }),
        }
    }

    fn model(issues: usize) -> EverythingModel {
        let names = [
            "292-overview-every-agent-has-name",
            "296-overview-hover-highlights-whole-block",
            "303-overview-every-drawn-button-works",
        ];
        let mut trees = vec![TreeRow {
            tree: tree("platform", true),
            mr: None,
            branch: Some("develop".into()),
        }];
        for (i, name) in names.iter().enumerate() {
            let branch = format!("feature/{name}");
            trees.push(TreeRow {
                tree: tree(name, false),
                mr: (i < 2).then(|| mr(303 + i as i64 * 5, &branch, "success")),
                branch: None,
            });
        }
        let mut list: Vec<ListedIssue> = (0..issues)
            .map(|i| ListedIssue {
                iid: 300 - i as i64,
                title: "overview: hover highlights the whole block (worktree, MR, action), not one line".into(),
                url: format!("https://gitlab.com/monocircuit/platform/-/issues/{}", 300 - i),
                labels: vec!["enhancement".into()],
                who: Some("lukasdiegelmann1".into()),
                ..Default::default()
            })
            .collect();
        if let Some(first) = list.first_mut() {
            first.branch = Some("feature/296-overview-hover-highlights-whole-block".into());
            first.mr = Some(308);
        }
        EverythingModel {
            trees: Ok(trees),
            orphans: vec![("vita-7".into(), "exited(3)".into())],
            has_docker: true,
            can_start_main: false,
            remote: Some(EverythingRemote {
                mrs: vec![mr(308, "feature/296-overview-hover-highlights-whole-block", "running")],
                issues: list,
                ..Default::default()
            }),
        }
    }

    fn sheet(model: &EverythingModel, width: u16) -> Sheet<Act> {
        sheet_in(model, width, None)
    }

    /// The live view in a pane `height` rows high; None: `--once`.
    fn sheet_in(model: &EverythingModel, width: u16, height: Option<u16>) -> Sheet<Act> {
        let gitlab = super::super::GitlabState::default();
        let ctx = Ctx {
            is_once: height.is_none(),
            folds: Default::default(),
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor: super::super::actor::idle(),
            words: super::super::actor::view::no_words(),
        };
        let mut page = Page::new(width);
        page.section(Section::new("TOP").plain(), |_| {});
        build_in(&mut page, model, &ctx, height);
        page.finish()
    }

    fn sheet_of(model: &EverythingModel, width: u16) -> Vec<String> {
        sheet(model, width).text()
    }

    #[test]
    fn nothing_is_cut_or_runs_past_the_edge() {
        let cut = |line: &str| {
            let chars: Vec<char> = line.chars().collect();
            chars
                .windows(2)
                .any(|pair| pair[1] == '…' && (pair[0].is_alphanumeric() || "_)(#".contains(pair[0])))
        };
        for width in [40, 80, 120, 200] {
            let sheet = sheet(&model(5), width);
            for row in &sheet.rows {
                assert!(row.line.width() <= usize::from(width), "{width}: {:?}", row.line);
            }
            let text = sheet.text();
            assert!(!text.iter().any(|line| cut(line)), "{width}: {text:#?}");
            let all = text.join("\n");
            for word in ["!303", "!308", "✓ passed", "◐ running", "lukasdiegelmann1", "vita-7"] {
                assert!(all.contains(word), "{width}: {word}\n{all}");
            }
        }
    }

    #[test]
    fn a_worktree_is_one_line_with_its_mr_aligned_when_there_is_room() {
        let text = sheet(&model(0), 200).text();
        let line = |name: &str| text.iter().find(|line| line.contains(name)).unwrap().clone();
        let first = line("292-overview-every-agent-has-name");
        let second = line("296-overview-hover-highlights-whole-block");
        assert!(first.contains("!303") && first.contains("✓ passed") && first.contains("1 ahead"));
        assert_eq!(first.find("!303"), second.find("!308"));
        assert_eq!(first.find("✓ passed"), second.find("✓ passed"));
        assert!(text.iter().any(|line| line.starts_with(" → platform")));
    }

    #[test]
    fn at_120_the_mr_and_its_pipeline_stay_on_the_first_line() {
        let text = sheet(&model(0), 120).text();
        let line = text
            .iter()
            .find(|line| line.contains("296-overview-hover-highlights-whole-block"))
            .unwrap();
        assert!(line.contains("!308 draft ✓ passed"), "{line}");
    }

    #[test]
    fn a_narrow_screen_shows_cards_with_the_name_first() {
        let text = sheet(&model(0), 60).text();
        let at = text
            .iter()
            .position(|line| line.contains("292-overview-every-agent-has-name"))
            .unwrap();
        assert!(
            text[at + 1].trim_start().starts_with("!303       ✓ passed"),
            "{text:#?}"
        );
    }

    #[test]
    fn a_card_that_fits_one_line_stays_one() {
        let text = sheet_of(&model(0), 60);
        assert!(
            text.iter().any(|line| line.starts_with(" → platform develop  clean")),
            "{text:#?}"
        );
    }

    #[test]
    fn without_any_env_the_title_says_it_once() {
        let sheet = sheet(&model(0), 120);
        let all = sheet.text().join("\n");
        assert!(all.contains("no .env.local in any"), "{all}");
        assert_eq!(all.matches("no .env.local").count(), 1, "{all}");
        let mut model = model(0);
        if let Ok(rows) = &mut model.trees {
            rows[1].tree.has_env = true;
        }
        let all = sheet_of(&model, 120).join("\n");
        assert!(
            all.contains("—") && all.contains("⇒ :3000") && all.contains("no .env.local"),
            "{all}"
        );
    }

    /// The rows of the block that starts on the line beginning with `start`.
    fn block_rows(sheet: &Sheet<Act>, start: &str) -> u16 {
        let text = sheet.text();
        let row = text
            .iter()
            .position(|line| line.trim_start().starts_with(start))
            .unwrap() as u16;
        sheet.block_at(1, row).map(|(_, region)| region.area.height).unwrap()
    }

    #[test]
    fn an_mr_is_its_title_then_branch_and_who_after_it() {
        let mut model = model(0);
        if let Some(remote) = &mut model.remote {
            remote.mrs.push(ListedMr {
                title: "build: short".into(),
                ..mr(102, "feature/102-x", "success")
            });
        }
        let sheet = sheet(&model, 120);
        let text = sheet.text();
        // A long title: branch and owner on the line under it, in the title's column.
        let at = text
            .iter()
            .position(|line| line.trim_start().starts_with("!308"))
            .unwrap();
        assert!(!text[at].contains("lukasdiegelmann1"), "{text:#?}");
        let column = |line: &str, word: &str| line.find(word).map(|at| line[..at].chars().count());
        assert_eq!(
            column(&text[at + 1], "feature/296"),
            column(&text[at], "feat(breadboard)"),
            "{text:#?}"
        );
        // A short one is one line, as in Node.
        let short = text.iter().find(|line| line.trim_start().starts_with("!102")).unwrap();
        assert!(
            short.contains("build: short  feature/102-x · lukasdiegelmann1"),
            "{text:#?}"
        );
        assert_eq!(block_rows(&sheet, "!102"), 1);
    }

    #[test]
    fn at_80_an_mr_puts_its_cells_under_its_title_and_takes_at_most_3_rows() {
        let mut model = model(0);
        if let Some(remote) = &mut model.remote {
            remote.mrs.push(ListedMr {
                title: "fix(scripts): wt:mr and wt:tick pass the description as text".into(),
                ..mr(187, "fix/157-wt-mr-wt-tick-fail", "success")
            });
        }
        let sheet = sheet(&model, 80);
        let text = sheet.text();
        let at = text
            .iter()
            .position(|line| line.trim_start().starts_with("!187"))
            .unwrap();
        assert!(text[at].contains("!187  fix(scripts): wt:mr"), "{text:#?}");
        assert!(
            text[at + 1]
                .trim_start()
                .starts_with("ready  ✓ passed   fix/157-wt-mr-wt-tick-fail · lukasdiegelmann1"),
            "{text:#?}"
        );
        assert_eq!(block_rows(&sheet, "!187"), 2, "{text:#?}");
        // State and pipeline start every MR's second line, in columns: the eye runs down them.
        let other = text
            .iter()
            .position(|line| line.trim_start().starts_with("!308"))
            .unwrap();
        assert_eq!(text[other + 1].find("draft"), text[at + 1].find("ready"), "{text:#?}");
        assert_eq!(
            text[other + 1].find("◐ running"),
            text[at + 1].find("✓ passed"),
            "{text:#?}"
        );
        assert!(block_rows(&sheet, "!308") <= 3, "{text:#?}");
    }

    #[test]
    fn at_80_a_worktree_keeps_its_check_in_the_column_and_all_its_work_under_it() {
        let mut model = model(0);
        if let Ok(rows) = &mut model.trees {
            rows[2].tree.changed = 2;
        }
        let sheet = sheet(&model, 80);
        let text = sheet.text();
        let at = |name: &str| text.iter().position(|line| line.contains(name)).unwrap();
        let first = at("292-overview-every-agent-has-name");
        assert!(
            text[first].contains("!303") && text[first].contains("✓ passed") && text[first].ends_with("untested"),
            "{text:#?}"
        );
        // Nothing dropped: `1 ahead` and the commit count on the row's second line, under its name.
        assert_eq!(text[first + 1], "     1 ahead · untested: 1 commit", "{text:#?}");
        let second = at("296-");
        assert!(text[second].contains("!308 draft ✓ passed  untested"), "{text:#?}");
        assert_eq!(
            text[second + 1].trim(),
            "1 ahead · 2 changed · untested: 1 commit",
            "{text:#?}"
        );
        for name in ["292-", "296-", "303-"] {
            assert_eq!(block_rows(&sheet, name), 2, "{text:#?}");
        }
        // The check stands in one column on every row.
        let column = |row: usize| text[row].find("untested").unwrap();
        assert_eq!(column(first), column(second), "{text:#?}");
        // Where the column holds it all, the row stays one line.
        let wide = sheet_of(&model, 200);
        let line = wide.iter().position(|line| line.contains("292-")).unwrap();
        assert!(wide[line].ends_with("1 ahead · untested: 1 commit"), "{wide:#?}");
        assert!(wide[line + 1].contains("296-"), "{wide:#?}");
    }

    #[test]
    fn every_issue_lines_up_its_title_on_one_line_at_120() {
        let text = sheet_of(&model(3), 120);
        let issues: Vec<&String> = text.iter().filter(|line| line.trim_start().starts_with('#')).collect();
        assert_eq!(issues.len(), 3, "{text:#?}");
        let title_at = |line: &str| line[..line.find("overview:").unwrap()].chars().count();
        assert_eq!(title_at(issues[0]), 18, "{text:#?}");
        assert!(issues.iter().all(|line| title_at(line) == 18), "{text:#?}");
        assert!(issues[0].contains("← !308"), "{text:#?}");
        let all = text.join("\n");
        assert!(
            all.contains("feature/296-overview-hover-highlights-whole-block"),
            "{all}"
        );
    }

    #[test]
    fn worktrees_stand_under_their_orchestration() {
        let model = model(0);
        let mut rows = model.trees.clone().unwrap();
        let mut loose = TreeRow {
            tree: tree("9-loose", false),
            mr: None,
            branch: None,
        };
        loose.tree.origin = None;
        rows.insert(2, loose);
        let names: Vec<&str> = grouped(&rows).iter().map(|row| row.tree.name.as_str()).collect();
        assert_eq!(names[..2], ["platform", "9-loose"]);
        let text = sheet(
            &EverythingModel {
                trees: Ok(rows),
                ..model
            },
            200,
        )
        .text();
        let head = text.iter().position(|line| line.trim() == "⇶ breadboard").unwrap();
        assert!(
            text[head - 1].contains("9-loose") && text[head + 1].contains("292-"),
            "{text:#?}"
        );
    }

    #[test]
    fn a_click_goes_where_the_row_names() {
        let sheet = sheet(&model(2), 120);
        let text = sheet.text();
        let action = |needle: &str| {
            let row = text.iter().position(|line| line.contains(needle)).unwrap() as u16;
            sheet.hit(60, row).map(|(_, region)| region.action().clone())
        };
        assert_eq!(
            action("292-overview-every-agent-has-name"),
            Some(Act::Go(Go::FocusSpace(
                "/repo/292-overview-every-agent-has-name".into()
            )))
        );
        assert_eq!(
            action("!308  draft"),
            Some(Act::Go(Go::Show {
                url: "https://gitlab.com/monocircuit/platform/-/merge_requests/308".into(),
                dir: Some("/repo/296-overview-hover-highlights-whole-block".into()),
            }))
        );
        assert_eq!(
            action("#300"),
            Some(Act::Go(Go::FocusSpace(
                "/repo/296-overview-hover-highlights-whole-block".into()
            )))
        );
        assert_eq!(
            action("#299"),
            Some(Act::Go(Go::Show {
                url: "https://gitlab.com/monocircuit/platform/-/issues/299".into(),
                dir: Some("/repo/platform".into()),
            }))
        );
    }

    fn issues_in(text: &[String]) -> usize {
        text.iter().filter(|line| line.trim_start().starts_with('#')).count()
    }

    #[test]
    fn once_shows_every_issue() {
        let text = sheet_of(&model(64), 120);
        assert_eq!(issues_in(&text), 64);
        assert!(!text.iter().any(|line| line.contains("more: glab issue list")));
    }

    #[test]
    fn live_the_issues_get_what_the_pane_has_left() {
        let model = model(64);
        // A full pane: at least 3, then how many more.
        let text = sheet_in(&model, 120, Some(24)).text();
        assert_eq!(issues_in(&text), 3, "{text:#?}");
        assert!(
            text.iter().any(|line| line.trim() == "… 61 more: glab issue list"),
            "{text:#?}"
        );
        // A tall one: as many as fit, the page no taller than the pane.
        let sheet = sheet_in(&model, 120, Some(60));
        let text = sheet.text();
        let shown = issues_in(&text);
        assert!(shown > 3 && shown < 64, "{text:#?}");
        let band = band::lines(&Header::default(), "", 120).len();
        assert!(band + text.len() <= 60, "{} rows: {text:#?}", band + text.len());
        // Room for all: no line about more.
        let text = sheet_in(&model, 120, Some(500)).text();
        assert_eq!(issues_in(&text), 64);
        assert!(!text.iter().any(|line| line.contains("more: glab issue list")));
    }
}
