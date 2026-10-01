//! The view of everything (no flag, render.mjs): every worktree with its app,
//! stack, MR and work; the stacks no worktree has; GitLab's open MRs and
//! issues, the claimed ones first.
//!
//! Each list is a table: its columns as wide as their widest cell, the last
//! one wrapping under itself, nothing cut (#304). The cells that tell most
//! come before the one that wraps: a worktree's MR and pipeline stand on its
//! first line, its work facts wrap after them. When the last column would get
//! fewer than `MIN_WRAP` columns a row becomes a card: its key on one line,
//! the rest under it. Every row is a block (#296); a click goes where it names.

use super::{Act, Ctx, look};
use crate::tui::scene::width_of;
use crate::tui::theme::{self, Tone};
use crate::tui::{Page, Scene, Section};
use breadboard_core::gitlab::read::{EverythingRemote, ListedIssue, ListedMr};
use breadboard_core::herdr;
use breadboard_core::model::everything::{EverythingModel, TreeRow, stack_cell};
use breadboard_core::model::order::Go;
use breadboard_core::model::tree::{check_fact, pipeline_mark};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::cell::Cell;
use std::path::PathBuf;

/// The narrowest a wrapping last column gets before a row becomes a card (lib/tty.mjs `MIN_WRAP`).
const MIN_WRAP: u16 = 16;
/// Columns between two cells.
const GAP: u16 = 2;
/// The most the work column asks for to hold a fact whole: a longer one (a failed check's list) wraps.
const WORK_AT_MOST: u16 = 30;
/// Where a worktree's name starts: after the `→` of the one the overview runs in.
const NAME_AT: u16 = 2;
/// The rows of the title band, as render.mjs counts them for the issues' room.
const BAND_ROWS: u16 = 5;

pub fn build(page: &mut Page<Act>, model: &EverythingModel, ctx: &Ctx) {
    // The rows drawn so far: the issues get what is left of the screen (render.mjs:431).
    let used = Cell::new(BAND_ROWS);
    let count = |scene: &Scene<Act>, is_plain: bool| {
        let lines = scene.y() + u16::from(scene.x() > 0);
        used.set(used.get() + lines + if is_plain { 1 } else { 2 });
    };
    let rows: &[TreeRow] = model.trees.as_ref().map(|rows| &rows[..]).unwrap_or(&[]);
    let main = rows.iter().find(|row| row.tree.is_main).map(|row| row.tree.dir.clone());
    match &model.trees {
        Ok(rows) => page.section("WORKTREES", |scene| {
            worktrees(scene, rows, model, ctx);
            count(scene, false);
        }),
        Err(why) => page.failed("WORKTREES", why),
    }
    if !model.orphans.is_empty() {
        page.section("STACKS WITHOUT A WORKTREE", |scene| {
            orphans(scene, &model.orphans);
            count(scene, false);
        });
    }
    if let Some(error) = &ctx.gitlab.error {
        page.section(Section::new("GITLAB").plain(), |scene| {
            look::say(scene, error, theme::tone(Tone::Red));
            count(scene, true);
        });
    }
    match &model.remote {
        None => {
            if ctx.gitlab.error.is_none() {
                page.section(Section::new("GITLAB").plain(), |scene| {
                    look::say(scene, &ctx.gitlab.progress, theme::dim());
                    count(scene, true);
                });
            }
        }
        Some(remote) => {
            page.section("MERGE REQUESTS", |scene| {
                merge_requests(scene, remote, rows, main.as_ref());
                count(scene, false);
            });
            // Live, the issues fill what the screen has left, at least 3; a print shows all.
            let screen = (!ctx.is_once)
                .then(|| crossterm::terminal::size().ok().map(|(_, rows)| rows))
                .flatten();
            let shown = issues_shown(screen, used.get(), remote.issues.len());
            page.section("ISSUES", |scene| issues(scene, remote, rows, main.as_ref(), shown));
        }
    }
    page.section(Section::new("FOOTER").plain(), |scene| {
        scene.text(
            "pnpm stack:up|down|reset --port <p> · pnpm wt:new <n-feature> · pnpm commands",
            theme::dim(),
        );
    });
}

/// How many of `count` issues show: `max(3, rows − (lines so far + 7))` on a screen, all in a print.
fn issues_shown(screen: Option<u16>, used: u16, count: usize) -> usize {
    match screen {
        Some(rows) => count.min(usize::from(rows.saturating_sub(used + 7)).max(3)),
        None => count,
    }
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
    if tree.changed > 0 {
        parts.push(vec![look::span(
            format!("{} changed", tree.changed),
            theme::tone(Tone::Yellow),
        )]);
    }
    if let Some(fact) = check_fact(&tree.checked) {
        parts.push(look::spans(&[fact]));
    }
    if parts.is_empty() {
        parts.push(vec![look::dim("clean")]);
    }
    parts
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
    scene.note("click one to go to its space");
    // Without an .env.local anywhere the app column says nothing the stack's doesn't.
    let has_app = rows.iter().any(|row| row.tree.has_env);
    let rows = grouped(rows);
    let names: Vec<_> = rows.iter().map(|row| name_cell(row)).collect();
    let apps: Vec<_> = rows
        .iter()
        .map(|row| if has_app { app_cell(row) } else { vec![] })
        .collect();
    let stacks: Vec<_> = rows.iter().map(|row| look::spans(&[stack_cell(&row.tree)])).collect();
    let mrs = mr_cells(&rows);
    let widths = [
        widest(names.iter()),
        widest(apps.iter()),
        widest(stacks.iter()),
        widest(mrs.iter()),
    ];
    // The work column holds its widest fact whole (`untested: 2 commits ·`), a long one wraps.
    let least = rows
        .iter()
        .flat_map(|row| work_of(row))
        .map(|part| width_of(&part) + 2)
        .max()
        .unwrap_or(0)
        .clamp(MIN_WRAP, WORK_AT_MOST);
    let at = columns(NAME_AT, &widths, scene.width(), least);
    // A card's second line lines up as a table of its own when it can.
    let under = columns(NAME_AT, &widths[1..], scene.width(), least);
    let is_in_herdr = herdr::is_in_herdr();
    let cells = names.into_iter().zip(apps).zip(stacks).zip(mrs);
    let mut group: Option<&str> = None;
    for (row, (((name, app), stack), mr)) in rows.iter().zip(cells) {
        // An orchestration's name heads its worktrees (`⇶ breadboard`).
        if let Some(origin) = &row.tree.origin
            && group != Some(origin.name.as_str())
        {
            group = Some(&origin.name);
            scene.end_line();
            scene.pad_to(NAME_AT);
            scene.indent(NAME_AT);
            look::flow(
                scene,
                &[look::span(format!("⇶ {}", origin.name), theme::tone(Tone::Cyan))],
            );
            scene.indent(0);
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
                    table_row(scene, at, vec![name, app, stack, mr], |scene| {
                        facts(scene, &work, column);
                    });
                }
                None => {
                    // A card: the name on its line, the rest under it.
                    scene.pad_to(NAME_AT);
                    scene.indent(NAME_AT);
                    look::flow(scene, &name);
                    scene.end_line();
                    match &under {
                        Some(under) => {
                            let column = under.last().copied().unwrap_or(0);
                            table_row(scene, under, vec![app, stack, mr], |scene| {
                                facts(scene, &work, column);
                            });
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

/// An MR's title, then dim `<branch> · <who>`: on the title's line when both
/// fit on it, else on a line of their own under it.
fn mr_text(scene: &mut Scene<Act>, mr: &ListedMr) {
    let mut tail = vec![vec![look::dim(&mr.branch)]];
    if let Some(who) = &mr.who {
        tail.push(vec![look::dim(who)]);
    }
    let title = look::span(&mr.title, Style::new());
    let tail_width = tail.iter().map(|part| width_of(part)).sum::<u16>() + 3 * (tail.len() as u16 - 1);
    let is_one_line = scene.x() + title.width() as u16 + GAP + tail_width <= scene.width();
    look::flow(scene, &[title]);
    if is_one_line {
        gap(scene, 0);
    } else {
        scene.end_line();
    }
    look::parted(scene, &tail);
}

fn merge_requests(scene: &mut Scene<Act>, remote: &EverythingRemote, rows: &[TreeRow], main: Option<&PathBuf>) {
    scene.note(format!("{} open", remote.mrs.len()));
    let iids: Vec<_> = remote
        .mrs
        .iter()
        .map(|mr| vec![look::span(format!("!{}", mr.iid), Style::new())])
        .collect();
    let states: Vec<_> = remote
        .mrs
        .iter()
        .map(|mr| {
            vec![if mr.is_draft {
                look::dim("draft")
            } else {
                look::span("ready", theme::tone(Tone::Green))
            }]
        })
        .collect();
    let marks: Vec<_> = remote
        .mrs
        .iter()
        .map(|mr| look::spans(&[pipeline_mark(mr.pipeline.as_ref())]))
        .collect();
    let widths = [widest(iids.iter()), widest(states.iter()), widest(marks.iter())];
    let at = columns(0, &widths, scene.width(), prose(scene));
    let cells = iids.into_iter().zip(states).zip(marks);
    for (mr, ((iid, state), mark)) in remote.mrs.iter().zip(cells) {
        let click = Act::Go(Go::Show {
            url: mr.url.clone(),
            dir: group_of(rows, Some(&mr.branch), main),
        });
        scene.block(click, |scene| {
            let rest = |scene: &mut Scene<Act>| mr_text(scene, mr);
            match &at {
                Some(at) => table_row(scene, at, vec![iid, state, mark], rest),
                None => {
                    scene.atom(iid);
                    scene.atom_gap(state, GAP);
                    scene.atom_gap(mark, GAP);
                    scene.end_line();
                    scene.indent(GAP);
                    scene.pad_to(GAP);
                    rest(scene);
                    scene.indent(0);
                }
            }
        });
    }
}

/// An issue's claim: cyan `← <branch> !<mr>`, or dim `unclaimed`.
fn claim_of(issue: &ListedIssue) -> Vec<Span<'static>> {
    vec![match &issue.branch {
        Some(branch) => {
            let mr = issue.mr.map(|mr| format!(" !{mr}")).unwrap_or_default();
            look::span(format!("← {branch}{mr}"), theme::tone(Tone::Cyan))
        }
        None => look::dim("unclaimed"),
    }]
}

/// After an issue's title: its labels and `@who`.
fn issue_tail(issue: &ListedIssue) -> Vec<Span<'static>> {
    let mut tail = vec![];
    if !issue.labels.is_empty() {
        tail.push(look::dim(issue.labels.join(",")));
    }
    if let Some(who) = &issue.who {
        tail.push(look::dim(format!("{}@{who}", if tail.is_empty() { "" } else { " " })));
    }
    tail
}

fn issues(scene: &mut Scene<Act>, remote: &EverythingRemote, rows: &[TreeRow], main: Option<&PathBuf>, shown: usize) {
    scene.note(format!("{} open, claimed first", remote.issues.len()));
    let list = &remote.issues[..shown.min(remote.issues.len())];
    let iid = list
        .iter()
        .map(|issue| format!("#{}", issue.iid).len() as u16)
        .max()
        .unwrap_or(0);
    // The claimed and the unclaimed each line up on their own: one long
    // branch doesn't push sixty titles to the right.
    let (claimed, unclaimed): (Vec<&ListedIssue>, Vec<&ListedIssue>) =
        list.iter().partition(|issue| issue.branch.is_some());
    for group in [claimed, unclaimed] {
        let claims: Vec<_> = group.iter().map(|issue| claim_of(issue)).collect();
        let at = columns(0, &[iid, widest(claims.iter())], scene.width(), prose(scene));
        for (issue, claim) in group.into_iter().zip(claims) {
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
            let number = vec![look::span(format!("#{}", issue.iid), Style::new())];
            scene.block(click, |scene| {
                let rest = |scene: &mut Scene<Act>| {
                    look::say(scene, &issue.title, Style::new());
                    gap(scene, 0);
                    look::flow(scene, &issue_tail(issue));
                };
                match &at {
                    Some(at) => table_row(scene, at, vec![number, claim], rest),
                    None => {
                        scene.atom(padded(number, iid));
                        scene.atom_gap(claim, GAP);
                        scene.end_line();
                        scene.indent(iid + GAP);
                        rest(scene);
                        scene.indent(0);
                    }
                }
            });
        }
    }
    let more = remote.issues.len() - list.len();
    if more > 0 {
        let text = format!("… {more} more: glab issue list");
        // The project's issues in the browser: the part of an issue's URL before `/-/`.
        match remote.issues[0].url.split_once("/-/") {
            Some((project, _)) => {
                let click = Act::Go(Go::Show {
                    url: format!("{project}/-/issues"),
                    dir: main.cloned(),
                });
                scene.block(click, |scene| scene.text(&text, theme::dim()));
            }
            None => scene.text(&text, theme::dim()),
        }
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
        let gitlab = super::super::GitlabState::default();
        let ctx = Ctx {
            is_once: true,
            folds: Default::default(),
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor: super::super::actor::idle(),
            words: super::super::actor::view::no_words(),
        };
        let mut page = Page::new(width);
        page.section(Section::new("TOP").plain(), |_| {});
        build(&mut page, model, &ctx);
        page.finish()
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
        assert!(text[at + 1].trim_start().starts_with("no .env.local"), "{text:#?}");
    }

    #[test]
    fn the_unclaimed_issues_line_up_without_the_long_branch() {
        let text = sheet(&model(3), 120).text();
        let titles: Vec<usize> = text
            .iter()
            .filter(|line| line.contains("unclaimed"))
            .map(|line| line.find("overview:").unwrap())
            .collect();
        assert_eq!(titles.len(), 2);
        assert_eq!(titles[0], titles[1]);
        assert!(titles[0] < 30, "{text:#?}");
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
    fn the_issues_fill_what_the_screen_has_left() {
        assert_eq!(issues_shown(None, 30, 64), 64);
        assert_eq!(issues_shown(Some(40), 30, 64), 3);
        assert_eq!(issues_shown(Some(60), 30, 64), 23);
        assert_eq!(issues_shown(Some(300), 30, 64), 64);
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

    #[test]
    fn live_issues_past_the_room_say_how_many_more() {
        let model = model(10);
        let Some(remote) = &model.remote else { unreachable!() };
        let mut scene = Scene::<Act>::new(118);
        let rows = model.trees.as_ref().unwrap();
        issues(&mut scene, remote, rows, None, 3);
        scene.finish();
        let text: Vec<String> = scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect())
            .collect();
        assert_eq!(text.iter().filter(|line| line.starts_with('#')).count(), 3, "{text:#?}");
        assert_eq!(text.last().map(String::as_str), Some("… 7 more: glab issue list"));
        assert_eq!(
            scene.regions.last().map(|region| region.action().clone()),
            Some(Act::Go(Go::Show {
                url: "https://gitlab.com/monocircuit/platform/-/issues".into(),
                dir: None,
            }))
        );
    }
}
