//! The view of everything (no flag, render.mjs): every worktree with its app,
//! stack, MR and work; the stacks no worktree has; GitLab's open MRs and
//! issues, the claimed ones first.
//!
//! Each list is a table: its columns as wide as their widest cell, the last
//! one wrapping under itself, nothing cut (#304). The cells that tell most
//! come before the one that wraps: a worktree's MR and pipeline stand on its
//! line, its work after them, its check first; a column too narrow for a
//! fact holds the check in a word and the whole work goes on the row's next
//! line, under the name. Where the app and stack columns leave the work too
//! little room (80 columns), one column holds a row's MR, or its app and
//! stack when it has none; the usual env (no .env.local, the shared stack)
//! is counted once in the title. An MR is its title, then its branch and owner;
//! where its cells leave the title too little room they start the line under
//! the title, in columns, whichever takes fewer rows. An orchestration's
//! worktrees stand under its `⇶ <name>`, a row of its own that brings its
//! space to the front. Live, the MRs (those of no worktree here first) and
//! the issues share what the pane has left, at least one each, then `… N
//! more · click to show them`; `--once` prints them all. The stacks no
//! worktree has come after them: they get rows only from what the lists'
//! first items leave. Every row is a block (#296); a click goes where it names.

use super::actor::{spawn::Run, tries};
use super::{Act, Ctx, Fold, Overview, look};
use crate::tui::band;
use crate::tui::page::guarded;
use crate::tui::scene::width_of;
use crate::tui::theme::{self, Tone};
use crate::tui::{Flow, KeyEvent};
use crate::tui::{Header, Page, Scene, Section};
use breadboard_core::check::CheckState;
use breadboard_core::git::orchestrations_dir;
use breadboard_core::gitlab::read::{ListedIssue, ListedMr};
use breadboard_core::herdr;
use breadboard_core::model::everything::{EverythingModel, TreeRow};
use breadboard_core::model::order::Go;
use breadboard_core::model::tree::{check_fact, pipeline_mark, stack_of};
use breadboard_core::model::{Docker, Tree};
use breadboard_core::reader::Model;
use crossterm::event::KeyCode;
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
/// The rows of the two lists that are no item: each one's title and the blank after it.
const LIST_FRAMES: u16 = 4;
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
            let width = page.inner();
            let folds = (ctx.folds.is_every_mr_shown, ctx.folds.is_every_issue_shown);
            // What the pane has left under the band and the rows so far, the
            // two lists' titles and the blank after each: the lists share it,
            // so both stand on the first screen (`--once`: all of them).
            let room = height.map(|height| {
                let band = band::shown(&Header::default(), "", width + 2, height).len() as u16;
                height.saturating_sub(band + page.rows() + LIST_FRAMES)
            });
            // The stacks no worktree has come under the lists and get their
            // rows only from what the lists' first items leave (stray stacks give way).
            let stray = orphan_rows(&model.orphans, width);
            let ordered = others_first(&remote.mrs, rows);
            // Laid out before either is drawn, guarded as a section is (#306).
            let planned = guarded(|| {
                let mrs = MrTable::new(&ordered, rows, width);
                let list = IssueTable::new(&remote.issues, width);
                let (heights_mrs, heights_issues) = (mrs.heights(width), list.heights(width));
                let least = rows_of(&heights_mrs, heights_mrs.len().min(1))
                    + rows_of(&heights_issues, heights_issues.len().min(1));
                let share = |folds| match room {
                    None => (remote.mrs.len(), remote.issues.len()),
                    Some(room) => {
                        let room = room.saturating_sub(stray).max(least.min(room));
                        shares(&heights_mrs, &heights_issues, room, folds)
                    }
                };
                let shown = share(folds);
                // An opened list's fold stays where its line about more was:
                // the rows it opened come under it, where the click was.
                let folded = (share((false, folds.1)).0, share((folds.0, false)).1);
                (mrs, list, shown, folded)
            });
            match planned {
                Ok((mrs, list, (shown_mrs, shown_issues), (folded_mrs, folded_issues))) => {
                    page.section("MERGE REQUESTS", |scene| {
                        let fold = Shown::new(shown_mrs, folds.0, folded_mrs);
                        merge_requests(scene, &mrs, fold, rows, main.as_ref())
                    });
                    page.section("ISSUES", |scene| {
                        let fold = Shown::new(shown_issues, folds.1, folded_issues);
                        issues(scene, &list, fold, rows, main.as_ref())
                    });
                }
                Err(why) => {
                    page.failed("MERGE REQUESTS", &why);
                    page.failed("ISSUES", &why);
                }
            }
        }
    }
    if !model.orphans.is_empty() {
        page.section("STACKS WITHOUT A WORKTREE", |scene| orphans(scene, &model.orphans));
    }
    page.section(Section::new("FOOTER").plain(), |scene| {
        scene.text(
            &breadboard_core::config::in_repo(
                "pnpm stack:up|down|reset --port <p> · pnpm wt:new <n-feature> · pnpm commands",
            ),
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
fn app_cell(tree: &Tree) -> Vec<Span<'static>> {
    vec![if !tree.has_env {
        look::dim("—")
    } else if !tree.has_own_port {
        look::dim(format!("⇒ :{}", tree.app_port))
    } else if tree.is_up {
        look::span(format!("● :{}", tree.app_port + tree.offset), theme::tone(Tone::Green))
    } else {
        look::dim(format!("○ :{}", tree.app_port + tree.offset))
    }]
}

/// What a checkout's .env.local gives it, as its app and stack cells say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Env {
    Missing,
    Shared,
    Own,
}

fn env_of(tree: &Tree) -> Env {
    if !tree.has_env {
        Env::Missing
    } else if !tree.has_own_port {
        Env::Shared
    } else {
        Env::Own
    }
}

/// The env a narrow table says once in its title instead of on each row:
/// the commoner of no .env.local and the shared stack (an own stack has
/// its own port, said on its row).
fn usual_env(rows: &[&TreeRow]) -> Option<Env> {
    let count = |env| rows.iter().filter(|row| env_of(&row.tree) == env).count();
    match (count(Env::Missing), count(Env::Shared)) {
        (0, 0) => None,
        (missing, shared) if missing >= shared => Some(Env::Missing),
        _ => Some(Env::Shared),
    }
}

/// The title's word for the rows of `env` that say nothing about it.
fn env_note(env: Env, count: usize, of: usize) -> String {
    match env {
        Env::Missing if !breadboard_core::config::has_stack() => "no stack (breadboard.toml)".into(),
        Env::Missing if count == of => format!("{} in any", breadboard_core::config::env_missing()),
        Env::Missing => format!("{} in {count} of {of}", breadboard_core::config::env_missing()),
        _ => format!("{count} of {of} on the shared stack"),
    }
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
/// `is_short`: the pipeline as its glyph alone (`✓`, `◐`), where the
/// column would leave the work too little room; the MR list says it in words.
fn mr_cells(rows: &[&TreeRow], is_short: bool) -> Vec<Vec<Span<'static>>> {
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
            let mut mark = pipeline_mark(mr.pipeline.as_ref());
            if let Some((glyph, _)) = mark.text.split_once(' ')
                && is_short
                && glyph.chars().count() == 1
            {
                mark.text = glyph.into();
            }
            spans.extend(look::spans(&[mark]));
            spans
        })
        .collect()
}

/// What a worktree's work is, its check first so the column starts with it
/// on every row: `✓ tested 1a2b3c4d · 2 changed · 1 ahead`, or `clean`.
fn work_of(row: &TreeRow) -> Vec<Vec<Span<'static>>> {
    let tree = &row.tree;
    let mut parts = Vec::new();
    if let Some(fact) = check_fact(&tree.checked) {
        parts.push(look::spans(&[fact]));
    }
    if tree.changed > 0 {
        parts.push(vec![look::span(
            format!("{} changed", tree.changed),
            theme::tone(Tone::Yellow),
        )]);
    }
    if tree.ahead > 0 {
        parts.push(vec![look::span(format!("{} ahead", tree.ahead), Style::new())]);
    }
    if parts.is_empty() {
        parts.push(vec![look::dim("clean")]);
    }
    parts
}

/// The check in a word (`untested`, `✓ tested`, `✗ failed`, `checking…`):
/// `2 untested` when fewer commits than its ahead are untested.
fn check_word(row: &TreeRow) -> Option<Span<'static>> {
    let (word, tone) = match &row.tree.checked {
        CheckState::Tested { no_test: None, .. } => ("✓ tested".to_string(), Tone::Green),
        CheckState::Tested { .. } => ("✓ tested · no test".to_string(), Tone::Green),
        CheckState::Checking { .. } => ("checking…".into(), Tone::Cyan),
        CheckState::Failed {
            refused: Some(refused), ..
        } => (format!("✗ {refused}"), Tone::Red),
        CheckState::Failed { .. } => ("✗ failed".into(), Tone::Red),
        CheckState::Untested { count } if *count > 0 && *count != row.tree.ahead => {
            (format!("{count} untested"), Tone::Yellow)
        }
        CheckState::Untested { .. } => ("untested".into(), Tone::Yellow),
        CheckState::None => return None,
    };
    Some(look::span(word, theme::tone(tone)))
}

/// The work in short, as lazygit counts a branch: the check in a word and
/// `↑N` commits ahead of develop (`untested ↑1`), then `N changed`.
fn short_work(row: &TreeRow) -> Vec<Vec<Span<'static>>> {
    let tree = &row.tree;
    let mut first: Vec<Span<'static>> = check_word(row).into_iter().collect();
    if tree.ahead > 0 {
        if !first.is_empty() {
            first.push(Span::raw(" "));
        }
        first.push(look::span(format!("↑{}", tree.ahead), Style::new()));
    }
    let mut parts: Vec<_> = [first].into_iter().filter(|part| !part.is_empty()).collect();
    if tree.changed > 0 {
        parts.push(vec![look::span(
            format!("{} changed", tree.changed),
            theme::tone(Tone::Yellow),
        )]);
    }
    if parts.is_empty() {
        parts.push(vec![look::dim("clean")]);
    }
    parts
}

/// Facts on one line, parted by ` · `.
fn line_width(parts: &[Vec<Span<'static>>]) -> u16 {
    let gaps = 3 * parts.len().saturating_sub(1) as u16;
    parts.iter().map(|part| width_of(part)).sum::<u16>() + gaps
}

/// The work for a column `room` wide, the first that holds: all of it on
/// one line; in short on one line (a failure's what then stands under the
/// name); in short wrapping in the column; the check in a word there and
/// all of it on a line of its own under the name. Nothing is lost (#304).
fn work_in(row: &TreeRow, room: u16) -> (Vec<Vec<Span<'static>>>, Option<Vec<Vec<Span<'static>>>>) {
    let all = work_of(row);
    if line_width(&all) <= room {
        return (all, None);
    }
    let short = short_work(row);
    let is_failed = matches!(row.tree.checked, CheckState::Failed { .. });
    let detail = is_failed.then(|| all.clone());
    // Each but the last carries its ` ·`.
    let fits = |part: &Vec<Span<'static>>| width_of(part) + 2 <= room;
    if line_width(&short) <= room || short.iter().all(fits) {
        return (short, detail);
    }
    let word = check_word(row)
        .map(|word| vec![word])
        .or_else(|| short.first().cloned())
        .filter(fits);
    (word.into_iter().collect(), Some(all))
}

/// Whether a column `room` wide holds a row's work on the row's line, in
/// short at least.
fn is_one_line(row: &TreeRow, room: u16) -> bool {
    line_width(&work_of(row)) <= room || line_width(&short_work(row)) <= room
}

/// A row's app and stack where its MR holds their column: on a line of
/// their own in that column, under the MR.
fn env_line(scene: &mut Scene<Act>, column: u16, env: Option<Vec<Span<'static>>>) {
    let Some(env) = env else { return };
    scene.end_line();
    scene.pad_to(column);
    scene.indent(column);
    scene.atom(env);
    scene.indent(0);
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
        let port = tree.app_port + if tree.has_own_port { tree.offset } else { 0 };
        Act::Go(Go::Show {
            url: format!("http://localhost:{port}"),
            dir: Some(tree.dir.clone()),
        })
    } else {
        Act::Go(Go::Detail(tree.dir.clone()))
    }
}

/// The width of each column of `cells`, one row each.
fn widths_of(cells: &[Vec<Vec<Span<'static>>>]) -> Vec<u16> {
    let count = cells.first().map_or(0, Vec::len);
    (0..count).map(|i| widest(cells.iter().map(|row| &row[i]))).collect()
}

/// How the rows of WORKTREES stand.
struct Layout {
    /// Each row's cells between its name and its work.
    cells: Vec<Vec<Vec<Span<'static>>>>,
    /// Each row's app and stack where its MR holds their column: under it.
    envs: Vec<Option<Vec<Span<'static>>>>,
    /// The env the title says once, its rows saying nothing of it.
    usual: Option<Env>,
    /// Where the columns start; None: cards.
    at: Option<Vec<u16>>,
}

/// A checkout's app and stack in one cell: `○ :3000 stack down`, `⇒ :3000
/// shared`, `— no .env.local`.
fn env_cell(tree: &Tree) -> Vec<Span<'static>> {
    [app_cell(tree), vec![Span::raw(" ")], stack_word(tree)].concat()
}

/// A checkout's stack in WORKTREES' words at every width (`stack down`,
/// `stack running`, `stack 2 running, 1 exited`, `shared`): the app's port
/// says its offset; `no .env.local` without one.
fn stack_word(tree: &Tree) -> Vec<Span<'static>> {
    match env_of(tree) {
        Env::Missing => vec![look::dim(breadboard_core::config::env_missing())],
        _ => look::spans(&[stack_of(tree)]),
    }
}

/// The layout of WORKTREES in `width`: of these, the first with the most
/// rows whose work stands on their line (in short at least): wide, the
/// app, the stack and the MR in columns of their own; narrow (80 columns,
/// long names), one column that holds the MR, or the app and stack of a
/// row without one, the env of a row with an MR under it unless it is
/// the usual one (no .env.local, the shared stack), which the title
/// counts once; narrow with each pipeline as its glyph. None with room for
/// the work column: cards.
fn layout(rows: &[&TreeRow], names: &[Vec<Span<'static>>], width: u16) -> Layout {
    let is_bare = rows.iter().all(|row| !row.tree.has_env);
    let wide = Layout {
        cells: rows
            .iter()
            .zip(mr_cells(rows, false))
            .map(|(row, mr)| match is_bare {
                true => vec![vec![], vec![], mr],
                false => vec![app_cell(&row.tree), stack_word(&row.tree), mr],
            })
            .collect(),
        envs: vec![None; rows.len()],
        usual: is_bare.then_some(Env::Missing),
        at: None,
    };
    let usual = usual_env(rows);
    let is_usual = |row: &TreeRow| Some(env_of(&row.tree)) == usual;
    let narrow = |is_short| {
        let mut cells = vec![];
        let mut envs = vec![];
        for (row, mr) in rows.iter().zip(mr_cells(rows, is_short)) {
            let has_mr = !mr.is_empty();
            envs.push((has_mr && !is_usual(row)).then(|| env_cell(&row.tree)));
            cells.push(vec![match (has_mr, is_usual(row)) {
                (true, _) => mr,
                (false, true) => vec![],
                (false, false) => env_cell(&row.tree),
            }]);
        }
        Layout {
            cells,
            envs,
            usual,
            at: None,
        }
    };
    let mut layouts = [wide, narrow(false), narrow(true)];
    for layout in &mut layouts {
        let widths = [vec![widest(names.iter())], widths_of(&layout.cells)].concat();
        layout.at = columns(NAME_AT, &widths, width, WORK_AT_LEAST);
    }
    // How many rows stand on one line; the first with the most wins.
    let one_line = |layout: &Layout| {
        let column = layout.at.as_ref().and_then(|at| at.last().copied())?;
        let room = width.saturating_sub(column);
        let rows = rows.iter().zip(&layout.envs);
        Some(
            rows.filter(|(row, env)| env.is_none() && is_one_line(row, room))
                .count(),
        )
    };
    let counts: Vec<_> = layouts.iter().map(one_line).collect();
    let most = counts.iter().flatten().max().copied();
    let pick = counts
        .iter()
        .position(|count| count.is_some() && *count == most)
        .unwrap_or(1);
    let [wide, narrow, short] = layouts;
    match pick {
        0 => wide,
        1 => narrow,
        _ => short,
    }
}

fn worktrees(scene: &mut Scene<Act>, rows: &[TreeRow], model: &EverythingModel, ctx: &Ctx) {
    let main = rows
        .iter()
        .find(|row| row.tree.is_main)
        .map(|row| row.tree.dir.as_path());
    let rows = grouped(rows);
    let names: Vec<_> = rows.iter().map(|row| name_cell(row)).collect();
    let Layout { cells, envs, usual, at } = layout(&rows, &names, scene.width());
    match usual {
        Some(env) => {
            let count = rows.iter().filter(|row| env_of(&row.tree) == env).count();
            scene.note(format!(
                "click one to go to its space · {}",
                env_note(env, count, rows.len())
            ));
        }
        None => scene.note("click one to go to its space"),
    }
    // A card's second line lines up as a table of its own when it can.
    let under = columns(NAME_AT, &widths_of(&cells), scene.width(), WORK_AT_LEAST);
    let is_in_herdr = herdr::is_in_herdr();
    let mut group: Option<&str> = None;
    for (row, ((name, cells), env)) in rows.iter().zip(names.into_iter().zip(cells).zip(envs)) {
        // An orchestration's name heads its worktrees (`⇶ breadboard`) in
        // cyan: a line once, where Node said it after every name.
        if let Some(origin) = &row.tree.origin
            && group != Some(origin.name.as_str())
        {
            group = Some(&origin.name);
            scene.end_line();
            // A click brings its space `⇶ <title>` to the front, which stands on its checkout.
            let checkout = main.map(|main| orchestrations_dir(main).join(&origin.name));
            let click = checkout.map_or(Act::Go(Go::Nothing), |dir| Act::Go(Go::Detail(dir)));
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
            if let Some(at) = &at {
                let column = at.last().copied().unwrap_or(0);
                let (work, detail) = work_in(row, scene.width().saturating_sub(column));
                let cells = [vec![name], cells].concat();
                table_row(scene, at, cells, |scene| facts(scene, &work, column));
                env_line(scene, at.get(1).copied().unwrap_or(column), env);
                work_line(scene, detail);
                return;
            }
            // A card: the name on its line, the rest under it, unless the
            // row has only its work and that fits after the name
            // (`→ platform develop  clean`): the aligned cells stay aligned.
            let work = work_of(row);
            let is_one_line = cells.iter().all(Vec::is_empty)
                && env.is_none()
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
                    table_row(scene, under, cells, |scene| facts(scene, &work, column));
                    env_line(scene, under.first().copied().unwrap_or(column), env);
                    work_line(scene, detail);
                }
                None => {
                    for cell in cells {
                        if !cell.is_empty() {
                            scene.atom_gap(cell, GAP);
                        }
                    }
                    gap(scene, NAME_AT);
                    facts(scene, &work, NAME_AT);
                    env_line(scene, NAME_AT, env);
                }
            }
            scene.indent(0);
        });
    }
    if model.can_start_main
        && let Some(main) = rows.iter().find(|row| row.tree.is_main)
    {
        // Right under the rows: a blank there cost the issues their row at 80x24.
        scene.end_line();
        let start = Act::StartStack(main.tree.dir.clone());
        let run = ctx.actor.stacks.get(&main.tree.dir);
        scene.block(start.clone(), |scene| default_stack(scene, start, run, ctx.is_once));
    }
    if !model.has_docker {
        scene.blank();
        scene.text(
            "docker does not answer: the stacks are unknown",
            theme::tone(Tone::Yellow),
        );
    }
}

/// The default stack's button (actions.mjs `pushStackButton`): `▶ start
/// the default stack  click or u`, a failed run's last line red after it;
/// while it runs `◐ starting the default stack …` and its log's last line.
fn default_stack(scene: &mut Scene<Act>, start: Act, run: Option<&Run>, is_once: bool) {
    if let Some(run) = run.filter(|run| run.is_running) {
        scene.button("◐ starting the default stack …", Tone::Yellow, start);
        look::say(scene, &run.last_line(), theme::dim());
        return;
    }
    scene.button("▶ start the default stack", Tone::Cyan, start);
    let up = breadboard_core::config::in_repo("pnpm stack:up");
    scene.text(if is_once { &up } else { "click or u" }, theme::dim());
    if let Some(why) = run.and_then(|run| run.error.as_deref()) {
        gap(scene, 0);
        look::say(scene, &format!("✗ {why}"), theme::tone(Tone::Red));
    }
}

/// `u` in the view of everything: the default stack whose button shows (overview.mjs `upTarget`).
pub fn key(app: &mut Overview, key: KeyEvent) -> Option<Flow> {
    let Some(Model::Everything(model)) = &app.model else {
        return None;
    };
    if key.code != KeyCode::Char('u') || !key.modifiers.is_empty() || !model.can_start_main {
        return None;
    }
    let main = model
        .trees
        .as_ref()
        .ok()?
        .iter()
        .find(|row| row.tree.is_main)?
        .tree
        .dir
        .clone();
    tries::start_stack(app, &main, None);
    app.redraw();
    Some(Flow::Redraw)
}

/// The rows STACKS WITHOUT A WORKTREE takes in a page `width` wide: its
/// title, its rows and the blank after it; none without one.
fn orphan_rows(orphans: &[(String, String)], width: u16) -> u16 {
    if orphans.is_empty() {
        return 0;
    }
    2 + rows_in(width, |scene| self::orphans(scene, orphans))
}

/// vita's stacks no worktree has, and how to drop one with its data.
fn orphans(scene: &mut Scene<Act>, orphans: &[(String, String)]) {
    let names: Vec<_> = orphans
        .iter()
        .map(|(name, _)| vec![look::span(name, Style::new())])
        .collect();
    // WORKTREES' words for a stack (`stack exited`, `stack 2 running, 1
    // exited`), never compose's `exited(1)`: one fact, one way on the page.
    let states: Vec<_> = orphans
        .iter()
        .map(|(_, status)| {
            let tree = Tree {
                has_own_port: true,
                docker: Docker::Answered,
                stack: Some(status.clone()),
                ..Tree::default()
            };
            look::spans(&[stack_of(&tree)])
        })
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

/// The open MRs, those of no worktree here first (GitLab's order in each):
/// a worktree's MR stands in its row above with its pipeline, so where the
/// pane shows a few, they are the ones said nowhere else.
fn others_first(mrs: &[ListedMr], rows: &[TreeRow]) -> Vec<ListedMr> {
    let is_here = |mr: &ListedMr| rows.iter().any(|row| row.tree.branch == mr.branch);
    let (mut others, here): (Vec<_>, Vec<_>) = mrs.iter().cloned().partition(|mr| !is_here(mr));
    others.extend(here);
    others
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
fn prose(width: u16) -> u16 {
    (width / 2).max(MIN_WRAP)
}

/// How many rows `draw` writes in a scene `width` wide: a list is laid out
/// the way that takes fewer, and the lists share the pane by rows.
fn rows_in(width: u16, draw: impl FnOnce(&mut Scene<Act>)) -> u16 {
    let mut scene = Scene::new(width);
    draw(&mut scene);
    scene.end_line();
    scene.y()
}

/// `spans` GAP columns after what is on the line: whole there or on the
/// next line when they fit one from `indent`, else as words.
fn trail(scene: &mut Scene<Act>, spans: Vec<Span<'static>>, indent: u16) {
    if spans.is_empty() {
        return;
    }
    if width_of(&spans) <= scene.width().saturating_sub(indent) {
        scene.atom_gap(spans, GAP);
    } else {
        gap(scene, indent);
        look::flow(scene, &spans);
    }
}

/// How many of a list fit `share` rows: all when they do, else as many as
/// fit with the line about the rest under them, at least one.
fn fitting(heights: &[u16], share: u16) -> usize {
    let total: u32 = heights.iter().map(|&height| u32::from(height)).sum();
    if total <= u32::from(share) {
        return heights.len();
    }
    let mut used = 1u32;
    let mut count = 0;
    for &height in heights {
        if used + u32::from(height) > u32::from(share) {
            break;
        }
        used += u32::from(height);
        count += 1;
    }
    count.max(1).min(heights.len())
}

/// The rows the first `count` of a list take, with the line about the rest.
fn rows_of(heights: &[u16], count: usize) -> u16 {
    let rows: u16 = heights[..count].iter().sum();
    rows + u16::from(count < heights.len())
}

/// How many MRs and issues a pane with `room` rows for both shows: all when
/// they fit; else each gets half, a list that needs less leaves the rest to
/// the other, and one the user opened (`… N more`) shows whole.
fn shares(mrs: &[u16], issues: &[u16], room: u16, folds: (bool, bool)) -> (usize, usize) {
    let need = |heights: &[u16]| heights.iter().map(|&height| u32::from(height)).sum::<u32>();
    let (all_mrs, all_issues) = (mrs.len(), issues.len());
    let rest = |used: u32| u16::try_from(u32::from(room).saturating_sub(used)).unwrap_or(0);
    match folds {
        (true, true) => return (all_mrs, all_issues),
        (true, false) => return (all_mrs, fitting(issues, rest(need(mrs)))),
        (false, true) => return (fitting(mrs, rest(need(issues))), all_issues),
        (false, false) => {}
    }
    if need(mrs) + need(issues) <= u32::from(room) {
        return (all_mrs, all_issues);
    }
    let half = room - room / 2;
    if need(mrs) <= u32::from(half) {
        return (all_mrs, fitting(issues, rest(need(mrs))));
    }
    if need(issues) <= u32::from(room / 2) {
        return (fitting(mrs, rest(need(issues))), all_issues);
    }
    let shown = fitting(mrs, half);
    (shown, fitting(issues, rest(u32::from(rows_of(mrs, shown)))))
}

/// How much of a list stands: its rows shown, whether the user opened it,
/// and how many the pane showed before that (where its fold stays).
#[derive(Clone, Copy, Debug)]
struct Shown {
    rows: usize,
    is_open: bool,
    folded: usize,
}

impl Shown {
    fn new(rows: usize, is_open: bool, folded: usize) -> Self {
        Self { rows, is_open, folded }
    }

    /// Whether the line about the fold goes before row `i` of `len`: under
    /// the rows the pane showed by itself, where `… N more` was clicked, so
    /// what it opened shows under the pointer (lazygit's fold stays put);
    /// at the end when there is nothing to open.
    fn is_fold_before(&self, i: usize, len: usize) -> bool {
        if self.is_open && self.folded < len {
            i == self.folded
        } else {
            i == self.rows
        }
    }
}

/// The rows of a list with the line about its fold: under a list the pane
/// cut, how many more and that a click shows them; in one the user opened,
/// where its line was, how many came and that a click folds them again.
fn rows_with_fold(
    scene: &mut Scene<Act>,
    len: usize,
    shown: Shown,
    fold: Fold,
    glab: &str,
    mut row: impl FnMut(&mut Scene<Act>, usize),
) {
    for i in 0..=shown.rows.min(len) {
        if shown.is_fold_before(i, len) {
            let text = match (len - shown.rows, shown.is_open) {
                (0, true) if shown.folded < len => format!("{} more below · click to show fewer", len - shown.folded),
                (0, true) => "click to show fewer".to_string(),
                (0, false) => String::new(),
                (more, _) => format!("… {more} more · click to show them · {glab}"),
            };
            if !text.is_empty() {
                scene.end_line();
                scene.block(Act::Fold(fold), |scene| look::say(scene, &text, theme::dim()));
            }
        }
        if i < shown.rows.min(len) {
            row(scene, i);
        }
    }
}

/// The open MRs as a table: `!iid`, draft or ready, the pipeline, the
/// title, then the branch and owner.
struct MrTable<'a> {
    mrs: &'a [ListedMr],
    cells: Vec<Vec<Vec<Span<'static>>>>,
    widths: Vec<u16>,
    at: Option<Vec<u16>>,
    is_stacked: bool,
    /// The branches of the worktrees here: their MR's row names no branch,
    /// the worktree's row above has it with the same `!iid`.
    locals: Vec<&'a str>,
}

impl<'a> MrTable<'a> {
    fn new(mrs: &'a [ListedMr], rows: &'a [TreeRow], width: u16) -> Self {
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
        let cells: Vec<_> = mrs.iter().map(cells_of).collect();
        let widths: Vec<u16> = (0..3).map(|i| widest(cells.iter().map(|row| &row[i]))).collect();
        let at = columns(0, &widths, width, MIN_WRAP);
        let locals = rows.iter().map(|row| row.tree.branch.as_str()).collect();
        let mut table = Self {
            mrs,
            cells,
            widths,
            at,
            is_stacked: false,
            locals,
        };
        // The cells beside the title, unless under it takes fewer rows (80 columns).
        table.is_stacked = table.at.is_none() || {
            let wide: u32 = table.heights(width).iter().map(|&rows| u32::from(rows)).sum();
            table.is_stacked = true;
            let stacked: u32 = table.heights(width).iter().map(|&rows| u32::from(rows)).sum();
            stacked < wide
        };
        table
    }

    fn heights(&self, width: u16) -> Vec<u16> {
        (0..self.mrs.len())
            .map(|i| rows_in(width, |scene| self.draw(scene, i)))
            .collect()
    }

    /// Dim `<branch> · <who>` (render.mjs), after an MR's title; a worktree's MR only `<who>`.
    fn tail(&self, mr: &ListedMr) -> Vec<Span<'static>> {
        let is_local = self.locals.contains(&mr.branch.as_str());
        let parts: Vec<&str> = [(!is_local).then_some(mr.branch.as_str()), mr.who.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if parts.is_empty() {
            return vec![];
        }
        vec![look::dim(parts.join(" · "))]
    }

    fn draw(&self, scene: &mut Scene<Act>, i: usize) {
        let (mr, cells) = (&self.mrs[i], self.cells[i].clone());
        match (&self.at, self.is_stacked) {
            (Some(at), false) => self.wide(scene, at, cells, mr),
            _ => self.stacked(scene, cells, mr),
        }
    }

    /// An MR as a table row: its cells, then its title wrapping under the
    /// title column, its branch and owner after the title, on the title's
    /// last line when they fit there.
    fn wide(&self, scene: &mut Scene<Act>, at: &[u16], cells: Vec<Vec<Span<'static>>>, mr: &ListedMr) {
        let column = at.last().copied().unwrap_or(0);
        table_row(scene, at, cells, |scene| {
            look::flow(scene, &[look::span(&mr.title, Style::new())]);
            trail(scene, self.tail(mr), column);
        });
    }

    /// An MR where its cells leave the title too little room: `!iid`, then
    /// its title with the room of the line; under it, always at the start of
    /// its own line, its state and pipeline padded to their columns, so the
    /// eye runs down them as at 120 columns, then branch and owner where
    /// they fit whole.
    fn stacked(&self, scene: &mut Scene<Act>, cells: Vec<Vec<Span<'static>>>, mr: &ListedMr) {
        let column = self.widths[0] + GAP;
        let mut cells = cells.into_iter();
        scene.atom(cells.next().unwrap_or_default());
        scene.indent(column);
        scene.pad_to(column);
        look::flow(scene, &[look::span(&mr.title, Style::new())]);
        scene.end_line();
        scene.pad_to(column);
        let mut state: Vec<Span<'static>> = vec![];
        for (cell, &width) in cells.zip(&self.widths[1..]) {
            if !state.is_empty() {
                state.push(Span::raw(" ".repeat(usize::from(GAP))));
            }
            state.extend(padded(cell, width));
        }
        scene.atom(state);
        trail(scene, self.tail(mr), column);
        scene.indent(0);
    }
}

fn merge_requests(scene: &mut Scene<Act>, table: &MrTable, shown: Shown, rows: &[TreeRow], main: Option<&PathBuf>) {
    scene.note(format!("{} open", table.mrs.len()));
    rows_with_fold(scene, table.mrs.len(), shown, Fold::Mrs, "glab mr list", |scene, i| {
        let mr = &table.mrs[i];
        let click = Act::Go(Go::Show {
            url: mr.url.clone(),
            dir: group_of(rows, Some(&mr.branch), main),
        });
        scene.block(click, |scene| table.draw(scene, i));
    });
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

/// After an issue's title: the branch that claims it in cyan when no MR
/// names it yet, then dim `<labels> @<who>` (render.mjs), each whole on a
/// line when it fits one.
fn issue_tail(issue: &ListedIssue) -> Vec<Vec<Span<'static>>> {
    let mut tail = vec![];
    if let Some(branch) = issue.branch.as_ref().filter(|_| issue.mr.is_none()) {
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

/// The open issues as a table: `#iid`, the claim, the title and what follows
/// it; cards (the number and claim on a line, the rest under it) where
/// there's no room.
struct IssueTable<'a> {
    issues: &'a [ListedIssue],
    cells: Vec<Vec<Vec<Span<'static>>>>,
    at: Option<Vec<u16>>,
}

impl<'a> IssueTable<'a> {
    fn new(issues: &'a [ListedIssue], width: u16) -> Self {
        let cells_of = |issue: &ListedIssue| {
            vec![
                vec![look::span(format!("#{}", issue.iid), Style::new())],
                claim_of(issue),
            ]
        };
        let mut cells: Vec<_> = issues.iter().map(cells_of).collect();
        let widths: Vec<u16> = (0..2).map(|i| widest(cells.iter().map(|row| &row[i]))).collect();
        // The title column holds a claiming branch whole.
        let branch = issues
            .iter()
            .filter(|issue| issue.mr.is_none())
            .filter_map(|issue| issue.branch.as_ref())
            .map(|branch| branch.chars().count() as u16)
            .max()
            .unwrap_or(0);
        let at = columns(0, &widths, width, prose(width).max(branch));
        if at.is_none() {
            // A card's number padded, so the claims line up.
            for row in &mut cells {
                row[0] = padded(std::mem::take(&mut row[0]), widths[0]);
            }
        }
        Self { issues, cells, at }
    }

    fn heights(&self, width: u16) -> Vec<u16> {
        (0..self.issues.len())
            .map(|i| rows_in(width, |scene| self.draw(scene, i)))
            .collect()
    }

    fn draw(&self, scene: &mut Scene<Act>, i: usize) {
        let (issue, cells) = (&self.issues[i], self.cells[i].clone());
        let column = self.at.as_ref().and_then(|at| at.last().copied()).unwrap_or(GAP);
        let rest = |scene: &mut Scene<Act>| {
            look::say(scene, &issue.title, Style::new());
            for part in issue_tail(issue) {
                trail(scene, part, column);
            }
        };
        match &self.at {
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
}

/// The first `shown` issues, claimed first, then how many more there are.
fn issues(scene: &mut Scene<Act>, table: &IssueTable, shown: Shown, rows: &[TreeRow], main: Option<&PathBuf>) {
    scene.note(format!("{} open, claimed first", table.issues.len()));
    rows_with_fold(
        scene,
        table.issues.len(),
        shown,
        Fold::Issues,
        "glab issue list",
        |scene, i| {
            let issue = &table.issues[i];
            let tree = issue
                .branch
                .as_ref()
                .and_then(|branch| rows.iter().find(|row| &row.tree.branch == branch));
            let click = match tree {
                Some(row) => Act::Go(Go::Detail(row.tree.dir.clone())),
                None => Act::Go(Go::Show {
                    url: issue.url.clone(),
                    dir: main.cloned(),
                }),
            };
            scene.block(click, |scene| table.draw(scene, i));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::Section;
    use crate::tui::page::Sheet;
    use breadboard_core::check::CheckState;
    use breadboard_core::gitlab::Pipeline;
    use breadboard_core::gitlab::read::EverythingRemote;
    use breadboard_core::model::{Docker, Tree};
    use breadboard_core::record::Origin;

    fn tree(name: &str, is_main: bool) -> Tree {
        Tree {
            dir: PathBuf::from(format!("/repo/{name}")),
            app_port: 3000,
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
        sheet_with(model, width, height, Default::default())
    }

    fn sheet_with(model: &EverythingModel, width: u16, height: Option<u16>, folds: super::super::Folds) -> Sheet<Act> {
        let gitlab = super::super::GitlabState::default();
        let ctx = Ctx {
            is_once: height.is_none(),
            folds,
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor: super::super::actor::idle(),
            words: super::super::actor::view::no_words(),
            develop: None,
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

    /// The MRs of the world: its own worktrees', a long title and a short one of others.
    fn with_others(mut model: EverythingModel) -> EverythingModel {
        if let Some(remote) = &mut model.remote {
            remote.mrs.push(ListedMr {
                title:
                    "perf(breadboard): an orchestration's GitLab refresh in a few calls, final things read once (#305)"
                        .into(),
                ..mr(318, "fix/305-overview-gitlab-refresh-makes-312", "pending")
            });
            remote.mrs.push(ListedMr {
                title: "fix(scripts): wt:mr and wt:tick pass the description as text".into(),
                ..mr(187, "fix/157-wt-mr-wt-tick-fail", "success")
            });
            remote.mrs.push(ListedMr {
                title: "build: short".into(),
                ..mr(102, "feature/102-x", "success")
            });
        }
        model
    }

    #[test]
    fn an_mr_is_its_title_then_branch_and_who_after_it() {
        let sheet = sheet(&with_others(model(0)), 120);
        let text = sheet.text();
        // A worktree's MR names no branch, its row in WORKTREES does: one line.
        let own = row_of(&text, "!308");
        assert!(text[own].ends_with("under the pointer  lukasdiegelmann1"), "{text:#?}");
        assert_eq!(block_rows(&sheet, "!308"), 1);
        // A long title wraps in its column, branch and owner after its last word.
        let long = row_of(&text, "!318");
        let column = |line: &str, word: &str| line.find(word).map(|at| line[..at].chars().count());
        assert_eq!(
            column(&text[long + 1], "(#305)"),
            column(&text[long], "perf(breadboard)"),
            "{text:#?}"
        );
        assert!(
            text[long + 1].ends_with("(#305)  fix/305-overview-gitlab-refresh-makes-312 · lukasdiegelmann1"),
            "{text:#?}"
        );
        // A short one is one line, as in Node.
        let short = &text[row_of(&text, "!102")];
        assert!(
            short.contains("build: short  feature/102-x · lukasdiegelmann1"),
            "{text:#?}"
        );
        assert_eq!(block_rows(&sheet, "!102"), 1);
    }

    #[test]
    fn at_80_every_mr_keeps_its_state_in_one_column_in_few_rows() {
        let sheet = sheet(&with_others(model(0)), 80);
        let text = sheet.text();
        let mut columns = vec![];
        for iid in ["!308", "!318", "!187", "!102"] {
            let at = row_of(&text, iid);
            let rows = block_rows(&sheet, iid);
            // A title of two lines and a long branch: 4, the rest 2 at most.
            assert!(rows <= if iid == "!318" { 4 } else { 2 }, "{iid}: {text:#?}");
            let block = &text[at..at + usize::from(rows)];
            let state = block
                .iter()
                .find_map(|line| line.find(" draft ").or_else(|| line.find(" ready ")))
                .unwrap();
            columns.push(state);
            assert!(
                block.iter().any(|line| line.contains("lukasdiegelmann1")),
                "{iid}: {text:#?}"
            );
        }
        assert!(
            columns.windows(2).all(|pair| pair[0] == pair[1]),
            "{columns:?} {text:#?}"
        );
        let all = text.join("\n");
        assert!(
            all.contains("fix/157-wt-mr-wt-tick-fail") && !all.contains("feature/296"),
            "{all}"
        );
    }

    #[test]
    fn at_80_a_worktree_is_one_line_with_its_work_in_short() {
        let mut model = model(0);
        if let Ok(rows) = &mut model.trees {
            rows[2].tree.changed = 2;
        }
        let sheet = sheet(&model, 80);
        let text = sheet.text();
        let at = |name: &str| text.iter().position(|line| line.contains(name)).unwrap();
        let first = at("292-overview-every-agent-has-name");
        assert!(
            text[first].contains("!303") && text[first].contains("✓ passed") && text[first].ends_with("untested ↑1"),
            "{text:#?}"
        );
        assert_eq!(block_rows(&sheet, "292-"), 1, "{text:#?}");
        assert_eq!(block_rows(&sheet, "303-"), 1, "{text:#?}");
        // What the column can't hold in short goes under the name whole: nothing dropped.
        let second = at("296-");
        assert!(text[second].contains("!308 draft ✓ passed  untested"), "{text:#?}");
        assert_eq!(
            text[second + 1].trim(),
            "untested: 1 commit · 2 changed · 1 ahead",
            "{text:#?}"
        );
        // The check stands in one column on every row.
        let column = |row: usize| text[row].find("untested").unwrap();
        assert_eq!(column(first), column(second), "{text:#?}");
        // Where the column holds it all, the row says it in words.
        let wide = sheet_of(&model, 200);
        let line = wide.iter().position(|line| line.contains("292-")).unwrap();
        assert!(wide[line].ends_with("untested: 1 commit · 1 ahead"), "{wide:#?}");
        assert!(wide[line + 1].contains("296-"), "{wide:#?}");
    }

    #[test]
    fn the_work_is_said_whole_else_short_else_under_the_name() {
        let text = |parts: &[Vec<Span<'static>>]| {
            parts
                .iter()
                .map(|part| part.iter().map(|span| span.content.to_string()).collect::<String>())
                .collect::<Vec<_>>()
                .join(" · ")
        };
        let mut row = TreeRow {
            tree: tree("292-x", false),
            mr: None,
            branch: None,
        };
        row.tree.changed = 1;
        // Room for all: one line in words.
        let (column, rest) = work_in(&row, 60);
        assert_eq!(text(&column), "untested: 1 commit · 1 changed · 1 ahead");
        assert!(rest.is_none());
        // Less: in short on one line, lazygit's ↑ for ahead.
        let (column, rest) = work_in(&row, 26);
        assert_eq!(text(&column), "untested ↑1 · 1 changed");
        assert!(rest.is_none());
        // Less: in short, wrapping in the column.
        let (column, rest) = work_in(&row, 14);
        assert_eq!(text(&column), "untested ↑1 · 1 changed");
        assert!(rest.is_none());
        // Too narrow for that: the check's word, all of it under the name.
        let (column, rest) = work_in(&row, 10);
        assert_eq!(text(&column), "untested");
        assert_eq!(text(&rest.unwrap()), "untested: 1 commit · 1 changed · 1 ahead");
        // Fewer untested commits than ahead: the count stays.
        row.tree.checked = CheckState::Untested { count: 1 };
        row.tree.ahead = 3;
        assert_eq!(text(&work_in(&row, 20).0), "1 untested ↑3 · 1 changed");
        // A failure's what never goes: in short beside, whole under the name.
        row.tree.checked = CheckState::Failed {
            sha: "1a2b3c4d".into(),
            failures: vec![],
            refused: None,
        };
        let (column, rest) = work_in(&row, 26);
        assert_eq!(text(&column), "✗ failed ↑3 · 1 changed");
        assert!(text(&rest.unwrap()).starts_with("✗ check failed: see pnpm wt:check"));
    }

    #[test]
    fn every_issue_lines_up_its_title_on_one_line_at_120() {
        let text = sheet_of(&model(3), 120);
        let issues: Vec<&String> = text.iter().filter(|line| line.trim_start().starts_with('#')).collect();
        assert_eq!(issues.len(), 3, "{text:#?}");
        let title_at = |line: &str| line[..line.find("overview:").unwrap()].chars().count();
        assert_eq!(title_at(issues[0]), 18, "{text:#?}");
        assert!(issues.iter().all(|line| title_at(line) == 18), "{text:#?}");
        // Its MR names the claim: the branch isn't said again.
        assert!(issues[0].contains("← !308"), "{text:#?}");
        let all = text.join("\n");
        assert!(!all.contains("feature/296-"), "{all}");
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
            Some(Act::Go(Go::Detail("/repo/292-overview-every-agent-has-name".into())))
        );
        assert_eq!(
            action("!308  draft"),
            Some(Act::Go(Go::Show {
                url: "https://gitlab.com/monocircuit/platform/-/merge_requests/308".into(),
                dir: Some("/repo/296-overview-hover-highlights-whole-block".into()),
            }))
        );
        assert_eq!(
            action("⇶ breadboard"),
            Some(Act::Go(Go::Detail(
                "/repo/platform/.claude/orchestrations/breadboard".into()
            )))
        );
        assert_eq!(
            action("#300"),
            Some(Act::Go(Go::Detail(
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

    /// The row of the first line that starts with `start`.
    fn row_of(text: &[String], start: &str) -> usize {
        text.iter()
            .position(|line| line.trim_start().starts_with(start))
            .unwrap()
    }

    /// The model with the main checkout's .env.local, and `each` done to every worktree.
    fn with_env(mut model: EverythingModel, each: impl Fn(&mut Tree)) -> EverythingModel {
        if let Ok(rows) = &mut model.trees {
            for row in rows {
                if row.tree.is_main {
                    row.tree.has_env = true;
                    row.tree.has_own_port = true;
                } else {
                    each(&mut row.tree);
                }
            }
        }
        model
    }

    /// The rows of each worktree's block, by its name.
    fn tree_rows(sheet: &Sheet<Act>) -> Vec<(String, u16)> {
        ["292-", "296-", "303-"]
            .iter()
            .map(|name| (name.to_string(), block_rows(sheet, name)))
            .collect()
    }

    #[test]
    fn at_80_a_worktree_is_one_line_whatever_its_env() {
        // Only the main checkout has a .env.local (the critic's world): the
        // rows without one say nothing, the title counts them.
        let model = with_env(model(0), |_| {});
        let view = sheet(&model, 80);
        let text = view.text();
        assert!(tree_rows(&view).iter().all(|(_, rows)| *rows == 1), "{text:#?}");
        let all = text.join("\n");
        assert!(all.contains("no .env.local in 3 of 4"), "{all}");
        assert_eq!(all.matches("no .env.local").count(), 1, "{all}");
        // The main checkout has no MR: its app and stack stand in that column.
        let main = &text[row_of(&text, "→ platform")];
        let first = &text[row_of(&text, "292-")];
        assert!(main.contains("○ :3000 stack down"), "{text:#?}");
        let column = |line: &str, word: &str| line.find(word).map(|at| line[..at].chars().count());
        assert_eq!(column(main, "○ :3000"), column(first, "!303"), "{text:#?}");
        assert!(first.ends_with("untested ↑1"), "{text:#?}");
        // Every worktree on the shared stack, the live setup: the same, said once.
        let model = with_env(model, |tree| tree.has_env = true);
        let view = sheet(&model, 80);
        let all = view.text().join("\n");
        assert!(tree_rows(&view).iter().all(|(_, rows)| *rows == 1), "{all}");
        assert!(
            all.contains("3 of 4 on the shared stack") && !all.contains("⇒"),
            "{all}"
        );
        // A worktree without an MR names its env, the usual one too.
        let mut model = model;
        if let Ok(rows) = &mut model.trees {
            rows[3].tree.has_env = false;
        }
        let text = sheet(&model, 80).text();
        assert!(text[row_of(&text, "303-")].contains("— no .env.local"), "{text:#?}");
        assert!(text.join("\n").contains("2 of 4 on the shared stack"), "{text:#?}");
        // At 120 every row has its columns, nothing said once.
        let text = sheet(&model, 120).text();
        let line = &text[row_of(&text, "292-")];
        assert!(line.contains("⇒ :3000  shared") && line.contains("!303"), "{text:#?}");
        assert!(!text.join("\n").contains("on the shared stack"), "{text:#?}");
    }

    #[test]
    fn an_own_stack_beside_an_mr_goes_under_it() {
        let mut model = with_env(model(0), |_| {});
        if let Ok(rows) = &mut model.trees {
            let tree = &mut rows[2].tree;
            (tree.has_env, tree.has_own_port, tree.offset) = (true, true, 2);
            tree.stack = Some("running(5)".into());
        }
        let text = sheet_of(&model, 80);
        let all = text.join("\n");
        assert!(all.contains("no .env.local in 2 of 4"), "{all}");
        let at = row_of(&text, "296-");
        assert!(text[at].contains("!308"), "{text:#?}");
        assert!(text[at].ends_with("untested ↑1"), "{text:#?}");
        // Under its MR, in the column of the main checkout's.
        let column = |line: &str, word: &str| line.find(word).map(|at| line[..at].chars().count());
        assert_eq!(
            column(&text[at + 1], "○ :3002 stack running"),
            column(&text[at], "!308"),
            "{text:#?}"
        );
        let main = &text[row_of(&text, "→ platform")];
        assert_eq!(column(main, "○ :3000"), column(&text[at], "!308"), "{text:#?}");
        // Wide enough: in its column.
        let text = sheet_of(&model, 200);
        assert!(
            text[row_of(&text, "296-")].contains("○ :3002  stack running  !308"),
            "{text:#?}"
        );
    }

    #[test]
    fn the_mrs_of_no_worktree_here_come_first() {
        let model = with_others(model(0));
        let text = sheet_of(&model, 120);
        let order: Vec<usize> = ["!318", "!187", "!102", "!308"]
            .iter()
            .map(|iid| row_of(&text, iid))
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text:#?}");
    }

    #[test]
    fn with_docker_silent_an_own_stack_is_unknown() {
        let mut model = with_env(model(0), |_| {});
        model.has_docker = false;
        if let Ok(rows) = &mut model.trees {
            rows[0].tree.docker = Docker::Silent;
        }
        let all = sheet_of(&model, 120).join("\n");
        assert!(all.contains("○ :3000  stack ?") && !all.contains("stack down"), "{all}");
        let all = sheet_of(&model, 80).join("\n");
        assert!(all.contains("○ :3000 stack ?") && !all.contains("stack down"), "{all}");
    }

    #[test]
    fn at_80x24_with_a_main_env_both_lists_start_on_the_first_screen() {
        let mut model = with_env(model(64), |_| {});
        if let (Some(remote), Ok(rows)) = (&mut model.remote, &mut model.trees) {
            remote.mrs = (0..14)
                .map(|i| mr(100 + i, &format!("feature/{i}-x"), "success"))
                .collect();
            // The gauntlet's world: 8 worktrees, no stack without one.
            let template = rows[1].clone();
            for i in 0..4 {
                let mut row = template.clone();
                row.tree.name = format!("31{i}-a-worktree-with-a-long-name-for-80");
                rows.push(row);
            }
        }
        model.orphans.clear();
        let height = 24;
        let band = band::shown(&Header::default(), "", 80, height).len();
        let text = sheet_in(&model, 80, Some(height)).text();
        let screen = usize::from(height) - band;
        assert!(row_of(&text, "!100") < screen, "{text:#?}");
        assert!(row_of(&text, "#300") < screen, "{text:#?}");
    }

    #[test]
    fn a_stack_reads_the_same_at_every_width() {
        let mut model = with_env(model(0), |_| {});
        if let Ok(rows) = &mut model.trees {
            let tree = &mut rows[3].tree;
            (tree.has_env, tree.has_own_port, tree.offset) = (true, true, 4);
            tree.stack = Some("exited(1), running(2)".into());
        }
        for width in [80, 120, 200] {
            let all = sheet_of(&model, width).join("\n");
            assert!(all.contains(":3000") && all.contains("stack down"), "{width}: {all}");
            assert!(all.contains("stack 2 running, 1 exited"), "{width}: {all}");
            assert!(!all.contains("default") && !all.contains("+4"), "{width}: {all}");
        }
    }

    #[test]
    fn at_80x24_stray_stacks_give_way_to_both_lists() {
        // The critic's world: 9 worktrees, the default stack down, two stacks of no worktree.
        let mut model = with_env(model(64), |_| {});
        model.can_start_main = true;
        if let (Some(remote), Ok(rows)) = (&mut model.remote, &mut model.trees) {
            remote.mrs = (0..14)
                .map(|i| ListedMr {
                    title: "refactor(vita): import @monocircuit/vita/core and /ui (#299)".into(),
                    ..mr(100 + i, &format!("feature/{i}-import-vita-s-parts-as"), "running")
                })
                .collect();
            let template = rows[1].clone();
            for i in 0..5 {
                let mut row = template.clone();
                row.tree.name = format!("30{i}-overview-pipeline-cuts-its-jobs");
                rows.push(row);
            }
        }
        model.orphans = vec![
            ("vita-gone".into(), "exited(1)".into()),
            ("vita-old-thing".into(), "running(1)".into()),
        ];
        let height = 24;
        let band = band::shown(&Header::default(), "", 80, height).len();
        let text = sheet_in(&model, 80, Some(height)).text();
        let screen = usize::from(height) - band;
        assert!(row_of(&text, "!100") < screen, "{text:#?}");
        assert!(row_of(&text, "#300") < screen, "{text:#?}");
        // The stray stacks come after the lists, whole.
        let stray = row_of(&text, "STACKS WITHOUT A WORKTREE");
        assert!(stray > row_of(&text, "ISSUES"), "{text:#?}");
        // In WORKTREES' words for a stack, never compose's `exited(1)`.
        let said = |name: &str, state: &str| text.iter().any(|line| line.contains(name) && line.contains(state));
        assert!(said("vita-gone", "stack exited"), "{text:#?}");
        assert!(said("vita-old-thing", "stack running"), "{text:#?}");
        assert!(!text.iter().any(|line| line.contains("exited(1)")), "{text:#?}");
        assert!(
            text[stray + 1].contains("docker compose -p vita-gone down -v"),
            "{text:#?}"
        );
        // A tall pane: every one of them on the first screen with both lists.
        let text = sheet_in(&model, 120, Some(60)).text();
        let band = band::shown(&Header::default(), "", 120, 60).len();
        assert!(row_of(&text, "vita-old-thing") < 60 - band, "{text:#?}");
        // `--once` prints them after the lists too.
        let text = sheet_of(&model, 80);
        assert!(
            row_of(&text, "STACKS WITHOUT A WORKTREE") > row_of(&text, "#237"),
            "{text:#?}"
        );
    }

    #[test]
    fn the_default_stack_says_where_its_run_stands() {
        let draw = |run: Option<Run>| {
            let mut scene = Scene::new(120);
            let start = Act::StartStack("/repo/platform".into());
            default_stack(&mut scene, start, run.as_ref(), false);
            let mut page = Page::new(120);
            page.section(Section::new("X").plain(), |into| {
                default_stack(into, Act::Go(Go::Nothing), run.as_ref(), false)
            });
            page.finish().text().join("\n")
        };
        assert_eq!(draw(None).trim(), "▶ start the default stack  click or u");
        let running = Run {
            is_running: true,
            ..Run::default()
        };
        assert!(draw(Some(running)).contains("◐ starting the default stack …"));
        let failed = Run {
            error: Some("pnpm stack:up failed".into()),
            ..Run::default()
        };
        let text = draw(Some(failed));
        assert!(
            text.contains("▶ start the default stack  click or u  ✗ pnpm stack:up failed"),
            "{text}"
        );
    }

    #[test]
    fn live_the_mrs_and_issues_share_what_the_pane_has_left() {
        let mut model = model(64);
        if let Some(remote) = &mut model.remote {
            remote.mrs = (0..14)
                .map(|i| mr(100 + i, &format!("feature/{i}-x"), "success"))
                .collect();
        }
        // 80x24: both lists start on the first screen, each with what it holds.
        let height = 24;
        let band = band::shown(&Header::default(), "", 80, height).len();
        let sheet = sheet_in(&model, 80, Some(height));
        let text = sheet.text();
        let screen = usize::from(height) - band;
        assert!(row_of(&text, "MERGE REQUESTS") < screen, "{text:#?}");
        assert!(row_of(&text, "#300") < screen, "{text:#?}");
        assert!(row_of(&text, "!100") < screen, "{text:#?}");
        assert!(
            text.iter()
                .any(|line| line.trim_start().starts_with("… 63 more · click")),
            "{text:#?}"
        );
        // The line about the rest opens its list.
        let more = row_of(&text, "… 63 more") as u16;
        assert_eq!(
            sheet.hit(4, more).map(|(_, region)| region.action().clone()),
            Some(Act::Fold(Fold::Issues))
        );
        let mrs_more = text
            .iter()
            .position(|line| line.contains("more · click to show them · glab mr list"))
            .unwrap();
        assert_eq!(
            sheet.hit(4, mrs_more as u16).map(|(_, region)| region.action().clone()),
            Some(Act::Fold(Fold::Mrs))
        );
        // Opened: every issue, and a line that folds it again.
        let folds = super::super::Folds {
            is_every_issue_shown: true,
            ..Default::default()
        };
        let opened = sheet_with(&model, 80, Some(height), folds);
        let text = opened.text();
        assert_eq!(issues_in(&text), 64);
        // Its fold stays on the row clicked: the issues it opened come under it.
        let fewer = row_of(&text, "63 more below · click to show fewer");
        assert_eq!(fewer, usize::from(more), "{text:#?}");
        assert!(text[fewer + 1].trim_start().starts_with("#299"), "{text:#?}");
        assert_eq!(
            opened.hit(4, fewer as u16).map(|(_, region)| region.action().clone()),
            Some(Act::Fold(Fold::Issues))
        );
        // The MRs opened: theirs stays put too, the rest of them under it.
        let folds = super::super::Folds {
            is_every_mr_shown: true,
            ..Default::default()
        };
        let text = sheet_with(&model, 80, Some(height), folds).text();
        let fewer = row_of(&text, "13 more below · click to show fewer");
        assert_eq!(fewer, mrs_more, "{text:#?}");
        assert!(text[fewer + 1].trim_start().starts_with("!101"), "{text:#?}");
        // A tall pane: all of both, no line about more.
        let text = sheet_in(&model, 120, Some(500)).text();
        assert_eq!(issues_in(&text), 64);
        assert!(!text.iter().any(|line| line.contains("more ·")), "{text:#?}");
    }

    #[test]
    fn a_list_that_needs_less_than_half_leaves_the_rest_to_the_other() {
        let mrs = [2u16; 3];
        let issues = [1u16; 40];
        // Room for all.
        assert_eq!(shares(&mrs, &issues[..4], 10, (false, false)), (3, 4));
        // The MRs need 6 of 20: the issues get 14, one of them the line about more.
        assert_eq!(shares(&mrs, &issues, 20, (false, false)), (3, 13));
        // Both need more than half: half each, the line about more in each half.
        assert_eq!(shares(&[2; 10], &issues, 10, (false, false)), (2, 4));
        // No room: one of each still.
        assert_eq!(shares(&[2; 10], &issues, 0, (false, false)), (1, 1));
        // Opened by the user: whole, the other gets what is left.
        assert_eq!(shares(&[2; 10], &issues, 10, (true, false)), (10, 1));
        assert_eq!(shares(&[], &[], 0, (false, false)), (0, 0));
    }
}
