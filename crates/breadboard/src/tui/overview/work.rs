//! WORK of an orchestration (#347): what WORKTREES and MERGE REQUESTS
//! showed apart, one block per piece of work. Under the orchestration's
//! record (`⇶ <name>`, how reuse went) the blocks with an open MR come in
//! the merge order, the MR graph's lanes in front of them ("side by side
//! where independent"), then the worktrees without one in the work order;
//! the queue and the done ones (merged, closed, removed) fold, shown until
//! folded, as the merged MRs were.
//!
//! A block (#390), a line per kind, each piece said once: its head (`!M`,
//! its name, where it stands at the right); who works in it, its dot
//! pulsing while its run is in flight, and under it what it does in words;
//! its app and stack; its GitLab line (commits, pipeline, the bar of its
//! boxes with their fold, where its conflicts stand with the button that
//! resolves them, what its MR still lacks); where it collides, a fold of
//! its own; then the problem it works on (#341), two lines until a click,
//! after its title where that says more. A click anywhere goes to the
//! worktree's space (its MR, else its issue, where it has none), `!M` opens
//! the MR, the pipeline its page; hover lights the whole block (#296).

use super::actor::merge::Phase;
use super::merge_requests::{self as mrs};
use super::worktrees::{self as nodes, Cells};
use super::{Act, Ctx, Fold, boxes, look};
use crate::tui::links;
use crate::tui::scene::{Region, Target, width_of};
use crate::tui::theme;
use crate::tui::{Page, Scene};
use breadboard_core::model::mr_blockers::Reason;
use breadboard_core::model::orchestration::{MergeRequests, MrFacts, OrchestrationModel};
use breadboard_core::model::order::Go;
use breadboard_core::model::tree::{Node, Worktrees};
use breadboard_core::model::{self, Docker, Paint};
use ratatui::layout::Rect;
use ratatui::text::Span;

/// The section's name.
const WORK: &str = "WORK";
/// The widest the bar of the boxes gets: past it a line of dashes says nothing more.
const MAX_BAR: u16 = 72;

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    let worktrees = match &model.worktrees {
        Ok(worktrees) => worktrees,
        Err(why) => return page.failed(WORK, why),
    };
    let none = MergeRequests::default();
    let (merge_requests, broken) = match &model.merge_requests {
        Ok(merge_requests) => (merge_requests, None),
        Err(why) => (&none, Some(why.as_str())),
    };
    page.section(WORK, |scene| {
        work(scene, worktrees, merge_requests, broken, model.has_remote, ctx);
    });
}

/// A piece of work: its worktree's node, its MR, the MR graph's lanes on its
/// first line and on the lines after.
#[derive(Clone, Copy)]
struct Block<'a> {
    node: Option<&'a Node>,
    mr: Option<&'a MrFacts>,
    graph: &'a str,
    below: &'a str,
    /// Merged, closed or removed: its head and title say all of it.
    is_done: bool,
}

/// A row of the open part: a block, or a line of the MR graph alone (`├─┘`).
enum Row<'a> {
    Block(Block<'a>),
    Graph(&'a str),
}

/// The blocks of the section, where they stand.
struct Parts<'a> {
    open: Vec<Row<'a>>,
    queued: Vec<Block<'a>>,
    done: Vec<Block<'a>>,
}

impl Parts<'_> {
    fn blocks(&self) -> impl Iterator<Item = &Block<'_>> {
        self.open
            .iter()
            .filter_map(|row| match row {
                Row::Block(block) => Some(block),
                Row::Graph(_) => None,
            })
            .chain(&self.queued)
            .chain(&self.done)
    }
}

/// The nodes and MRs joined by their issue: the open MRs in the merge order
/// with their worktrees, the worktrees without one in the work order (with
/// their merged MR, while the worktree is still there), the queue, then the
/// done: the merged and closed MRs, the last merged first, with their
/// removed worktree, and the removed worktrees without an MR.
fn parts_of<'a>(worktrees: &'a Worktrees, merge_requests: &'a MergeRequests) -> Parts<'a> {
    let tree = &worktrees.tree;
    let mut used: Vec<&Node> = Vec::new();
    let mut used_mrs: Vec<i64> = Vec::new();
    let all = || tree.live.iter().chain(&tree.removed).chain(&tree.queued);
    let is_used = |used: &[&Node], node: &Node| used.iter().any(|was| std::ptr::eq(*was, node));
    let mut open = Vec::new();
    for row in &merge_requests.open {
        let Some(mr) = &row.mr else {
            open.push(Row::Graph(&row.graph));
            continue;
        };
        let node = row
            .number
            .and_then(|number| all().find(|node| node.number == Some(number)));
        used.extend(node);
        used_mrs.push(mr.iid);
        open.push(Row::Block(Block {
            node,
            mr: Some(mr),
            graph: &row.graph,
            below: &row.below,
            is_done: false,
        }));
    }
    let merged_of = |number: Option<i64>| {
        let number = number?;
        merge_requests
            .merged
            .iter()
            .find(|row| row.number == Some(number))
            .and_then(|row| row.mr.as_ref())
    };
    for node in &tree.live {
        if is_used(&used, node) {
            continue;
        }
        used.push(node);
        let mr = merged_of(node.number);
        used_mrs.extend(mr.map(|mr| mr.iid));
        open.push(Row::Block(Block {
            node: Some(node),
            mr,
            graph: "",
            below: "",
            is_done: false,
        }));
    }
    let mut queued = Vec::new();
    for node in &tree.queued {
        if !is_used(&used, node) {
            used.push(node);
            queued.push(Block {
                node: Some(node),
                mr: None,
                graph: "",
                below: "",
                is_done: false,
            });
        }
    }
    let mut done = Vec::new();
    for mr in mrs::newest_first(merge_requests) {
        if used_mrs.contains(&mr.iid) {
            continue;
        }
        let number = merge_requests
            .merged
            .iter()
            .find(|row| row.mr.as_ref().is_some_and(|its| its.iid == mr.iid))
            .and_then(|row| row.number);
        let node = number.and_then(|number| {
            tree.removed
                .iter()
                .find(|node| node.number == Some(number) && !is_used(&used, node))
        });
        used.extend(node);
        done.push(Block {
            node,
            mr: Some(mr),
            graph: "",
            below: "",
            is_done: true,
        });
    }
    for node in &tree.removed {
        if !is_used(&used, node) {
            done.push(Block {
                node: Some(node),
                mr: None,
                graph: "",
                below: "",
                is_done: true,
            });
        }
    }
    Parts { open, queued, done }
}

/// The columns every block shares, so they line up across the folds: the
/// graph's, the number's, `!M`'s, the name's (the pipeline mark of a block
/// without a worktree after it).
struct Layout {
    graph: u16,
    digits: usize,
    iid: u16,
    name: u16,
    cells: Cells,
}

impl Layout {
    fn of(parts: &Parts) -> Layout {
        let blocks: Vec<&Block> = parts.blocks().collect();
        let lanes = blocks
            .iter()
            .map(|block| Span::raw(block.graph).width().max(Span::raw(block.below).width()) as u16)
            .max()
            .unwrap_or(0);
        let graph = if lanes > 0 { lanes + 2 } else { 0 };
        let digits = blocks
            .iter()
            .filter_map(|block| block.node)
            .map(|node| node.place.to_string().len())
            .max()
            .unwrap_or(1);
        let iid = blocks
            .iter()
            .filter_map(|block| block.mr)
            .map(|mr| Span::raw(format!("!{}", mr.iid)).width() as u16 + 2)
            .max()
            .unwrap_or(0);
        let name = blocks
            .iter()
            .map(|block| Span::raw(name_of(block)).width() as u16)
            .max()
            .unwrap_or(0)
            + 2;
        // The cells of the worktrees at work: the queue's and the done ones' have none.
        let live: Vec<&Node> = parts
            .open
            .iter()
            .filter_map(|row| match row {
                Row::Block(block) => block.node,
                Row::Graph(_) => None,
            })
            .collect();
        Layout {
            graph,
            digits,
            iid,
            name,
            cells: Cells::of(&live),
        }
    }
}

/// A block's name: its feature, else the MR's branch without its kind
/// (`feature/302-x` → `302-x`).
fn name_of<'a>(block: &Block<'a>) -> &'a str {
    match (block.node, block.mr) {
        (Some(node), _) => &node.name,
        (None, Some(mr)) => mr.branch.split_once('/').map_or(mr.branch.as_str(), |(_, name)| name),
        (None, None) => "",
    }
}

fn work(
    scene: &mut Scene<Act>,
    worktrees: &Worktrees,
    merge_requests: &MergeRequests,
    broken: Option<&str>,
    has_remote: bool,
    ctx: &Ctx,
) {
    // The counts of both in the title; memory running low is no aside: red, on a line of its own.
    let (warnings, counts): (Vec<&model::Span>, Vec<&model::Span>) =
        worktrees.head.iter().partition(|span| span.paint == Paint::Red);
    let counts: String = counts.iter().map(|span| span.text.as_str()).collect();
    let counts = counts.trim().trim_end_matches('·').trim_end();
    let note: Vec<&str> = [counts, merge_requests.counts.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    if !note.is_empty() {
        scene.note(note.join(" · "));
    }
    for warning in warnings {
        look::say(scene, &warning.text, look::style_of(&warning.paint));
        scene.end_line();
    }
    if let Some(why) = broken {
        look::say(scene, &format!("merge requests: {why}"), look::style_of(&Paint::Red));
        scene.end_line();
    }
    scene.atom(vec![look::bold(format!("⇶ {}", worktrees.name))]);
    scene.atom_gap(vec![look::dim(&worktrees.record)], 2);
    scene.end_line();
    let parts = parts_of(worktrees, merge_requests);
    let layout = Layout::of(&parts);
    let tree = &worktrees.tree;
    // Why there is no app and no stack, or why a stack says `?`: said once, not on every block.
    let no_env = if breadboard_core::config::has_stack() {
        format!(
            "{} in any worktree: no stack, the app can't run there",
            breadboard_core::config::env_missing()
        )
    } else {
        "no stack: breadboard.toml has no [stack]".to_string()
    };
    let why = if !layout.cells.is_any_env && tree.live.iter().any(|node| node.has_tree) {
        Some((no_env.as_str(), theme::dim()))
    } else if tree.live.iter().any(|node| node.docker == Docker::Silent) {
        Some((
            "docker does not answer: the stacks are unknown",
            look::style_of(&Paint::Yellow),
        ))
    } else {
        None
    };
    let indent = vec![Span::raw(" ".repeat(usize::from(layout.graph) + layout.digits + 2))];
    look::gutter(scene, indent.clone(), indent, |scene| {
        look::say(scene, &worktrees.reuse, theme::dim());
        if let Some((why, style)) = why {
            scene.end_line();
            look::say(scene, why, style);
        }
    });
    for row in &parts.open {
        match row {
            Row::Block(block) => block_lines(scene, block, &layout, ctx),
            Row::Graph(graph) => {
                scene.end_line();
                scene.atom(mrs::lanes(graph, layout.graph as usize));
                scene.end_line();
            }
        }
    }
    if !parts.queued.is_empty() {
        let is_folded = ctx.folds.is_queued_folded;
        let text = format!("queued ({}) · in the order they start", parts.queued.len());
        fold_line(scene, Fold::Queued, is_folded, &text, ctx);
        if !is_folded {
            for block in &parts.queued {
                block_lines(scene, block, &layout, ctx);
            }
        }
    }
    if !parts.done.is_empty() {
        let is_folded = ctx.folds.is_done_folded && !ctx.is_once;
        fold_line(scene, Fold::Removed, is_folded, &done_text(&parts, merge_requests), ctx);
        if !is_folded {
            for block in &parts.done {
                block_lines(scene, block, &layout, ctx);
            }
        }
    }
    scene.end_line();
    let fresh = match (has_remote, &ctx.gitlab.error) {
        (true, _) => ctx.gitlab.fresh(),
        (false, Some(_)) => "GitLab does not answer yet".into(),
        (false, None) => ctx.gitlab.progress.clone(),
    };
    look::say(scene, &fresh, theme::dim());
}

/// `done (44) · 40 merged · 1 closed · 3 removed`: the merged MRs and the
/// removed worktrees in one fold, each counted once (a removed worktree
/// whose MR was merged is the MR's).
fn done_text(parts: &Parts, merge_requests: &MergeRequests) -> String {
    let removed = parts.done.iter().filter(|block| block.mr.is_none()).count();
    let mut text = format!("done ({})", parts.done.len());
    if !merge_requests.merged.is_empty() {
        text.push_str(&format!(" · {}", mrs::done_counts(merge_requests)));
    }
    if removed > 0 {
        text.push_str(&format!(" · {removed} removed"));
    }
    text
}

/// `▸ done (42) · click to show`: a click folds or shows what is under it.
fn fold_line(scene: &mut Scene<Act>, fold: Fold, is_folded: bool, text: &str, ctx: &Ctx) {
    let mark = if is_folded { "▸" } else { "▾" };
    let hint = match (ctx.is_once, is_folded) {
        (true, _) => "",
        (false, true) => " · click to show",
        (false, false) => " · click to fold",
    };
    let line = format!("{mark} {text}{hint}");
    if ctx.is_once {
        scene.end_line();
        look::say(scene, &line, theme::dim());
        scene.end_line();
        return;
    }
    scene.block(Act::Fold(fold), |scene| look::say(scene, &line, theme::dim()));
}

/// Where a click on a block goes: the worktree's space, else its MR, else its issue.
fn go_of(block: &Block) -> Go {
    match (block.node, block.mr) {
        (Some(node), _) if node.has_tree && node.go != Go::Nothing => node.go.clone(),
        (_, Some(mr)) => mrs::show(mr),
        (Some(node), None) => node.go.clone(),
        (None, None) => Go::Nothing,
    }
}

/// A block: its gutter (the MR's lane, its number), then its lines.
fn block_lines(scene: &mut Scene<Act>, block: &Block, layout: &Layout, ctx: &Ctx) {
    let width = layout.graph as usize;
    let digits = layout.digits;
    let place = match block.node {
        Some(node) => format!("{:>digits$}. ", node.place),
        None => " ".repeat(digits + 2),
    };
    let mut first = match block.mr {
        Some(mr) if !block.graph.is_empty() => mrs::node(block.graph, mr, width),
        _ => mrs::lanes("", width),
    };
    first.push(look::dim(&place));
    let mut rest = mrs::lanes(block.below, width);
    rest.push(Span::raw(" ".repeat(place.chars().count())));
    let draw = |scene: &mut Scene<Act>| {
        look::gutter(scene, first, rest, |scene| lines(scene, block, layout, ctx));
    };
    match go_of(block) {
        Go::Nothing => draw(scene),
        go => scene.block(Act::Go(go), draw),
    }
}

/// A block's lines, in their order (#390).
fn lines(scene: &mut Scene<Act>, block: &Block, layout: &Layout, ctx: &Ctx) {
    let live = block.node.filter(|node| node.has_tree && !block.is_done);
    head(scene, block, layout, ctx);
    if let Some(node) = live {
        nodes::agent_lines(scene, node, ctx);
        nodes::stack_lines(scene, node, &layout.cells, ctx);
        gitlab_line(scene, block, ctx);
        nodes::collides(scene, node, ctx);
        description(scene, block, node, ctx);
        nodes::checked_lines(scene, node);
        return;
    }
    if block.is_done {
        // A done one: its MR's title said what it was, else its issue's.
        match (block.mr, block.node) {
            (Some(_), _) => title(scene, block, ctx),
            (None, Some(node)) => {
                look::say(scene, &node.title, theme::dim());
                scene.end_line();
            }
            (None, None) => {}
        }
        return;
    }
    // An open MR whose worktree is gone, an issue without a worktree, the queue.
    if block.mr.is_some() {
        gitlab_line(scene, block, ctx);
        title(scene, block, ctx);
    } else if let Some(node) = block.node {
        look::say(scene, &node.title, theme::dim());
        scene.end_line();
    }
}

/// The head line (#390): `!M` (a click opens the MR), the name, where it
/// stands at the right; a done one has its pipeline mark after the name
/// (GitLab merges on a red pipeline too), the others their GitLab line.
fn head(scene: &mut Scene<Act>, block: &Block, layout: &Layout, ctx: &Ctx) {
    if let Some(mr) = block.mr {
        iid_atom(scene, mr);
    }
    scene.pad_to(layout.iid);
    let style = match block.node {
        Some(node) if node.has_tree => look::pink().add_modifier(ratatui::style::Modifier::BOLD),
        _ => look::pink(),
    };
    scene.atom_gap(vec![look::span(name_of(block), style)], 0);
    if let Some(mr) = block.mr.filter(|_| block.is_done) {
        scene.pad_to(layout.iid + layout.name);
        for rect in scene.atom_gap(mrs::mark_of(mr), 0) {
            mrs::pipeline_region(scene, mr, rect);
        }
    }
    let state = match (block.node, block.mr) {
        (Some(node), _) => vec![look::span(&node.state.text, look::style_of(&node.state.paint))],
        // An open MR without its worktree: where it stands is its status.
        (None, Some(mr)) if !block.is_done => mrs::status_of(mr, mrs::merge_of(ctx, mr)),
        (None, _) => vec![],
    };
    if !state.is_empty() {
        nodes::right(scene, state);
    }
    scene.end_line();
}

/// `!M`, the MR's link, a click of its own (§11.3): a Ctrl-click in herdr opens its page too.
fn iid_atom(scene: &mut Scene<Act>, mr: &MrFacts) {
    let iid = format!("!{}", mr.iid);
    let style = if mr.is_merged { theme::dim() } else { theme::bold() };
    let rects = scene.atom_gap(vec![Span::styled(iid.clone(), links::style(style, Some(&mr.url)))], 0);
    if let [rect] = rects.as_slice() {
        scene.regions.push(Region {
            area: *rect,
            target: Target::Button {
                label: iid,
                tone: theme::Tone::Gray,
                action: Act::Go(mrs::show(mr)),
            },
        });
    }
}

/// The fewest columns the bar gets beside the line's other parts; with
/// less it goes on a line of its own.
const BAR_BESIDE: u16 = 32;

/// The GitLab line (#390): its commits (`↑15 ↓6 · 14 commits not pushed ·
/// ✓ tested 1a2b3c4d`), the MR's pipeline (a click opens it), the bar of
/// its issue's boxes with their fold, then where its conflicts stand (the
/// button that resolves them) and what its MR still lacks, without what
/// another line says (the pipeline, unpushed commits, the MRs it waits
/// for). Without an MR: its issue, `no MR yet`, the fold of its boxes.
/// The boxes a fold shows follow the line; a merge of develop that would
/// conflict, under them.
fn gitlab_line(scene: &mut Scene<Act>, block: &Block, ctx: &Ctx) {
    let node = block.node.filter(|node| node.has_tree);
    let mut parts: Vec<Vec<Span<'static>>> = Vec::new();
    let has_bar = block.mr.is_some_and(|mr| mr.progress.is_some());
    if let Some(node) = block.node {
        // The bar names the issue; without it, or once closed, the line does.
        if !node.lines.issue.is_empty() && (!has_bar || node.lines.issue.len() > 1) {
            parts.push(look::spans(&node.lines.issue));
        }
        if block.mr.is_none() && !node.lines.mr.is_empty() {
            parts.push(look::spans(&node.lines.mr));
        }
    }
    if let Some(node) = node {
        parts.extend(nodes::commit_parts(node));
    }
    let reasons: Vec<Reason> = block
        .mr
        .map(|mr| {
            mr.reasons
                .iter()
                .filter(|reason| !is_said_elsewhere(reason, node.is_some()))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut shown = None;
    // The line its bar (or its boxes' fold) is on, and that fold's act.
    let mut fold: Option<(u16, Act)> = None;
    let has_more = |at: usize, of: usize| at + 1 < of;
    // The parts before the pipeline, each whole with its ` ·`.
    let put = |scene: &mut Scene<Act>, part: Vec<Span<'static>>, is_last: bool| {
        let mut atom = part;
        if !is_last {
            atom.push(look::dim(" ·"));
        }
        if width_of(&atom) <= scene.width() {
            scene.atom(atom);
        } else {
            look::flow(scene, &atom);
        }
    };
    match block.mr {
        Some(mr) => {
            for part in parts {
                put(scene, part, false);
            }
            let has_lacks = mrs::has_lacks(mr, &reasons, ctx);
            // `pipeline ✗ failed at check:x`: the job its reason names, once.
            let mut mark = mrs::mark_of(mr);
            if let Some(job) = mr
                .reasons
                .iter()
                .find_map(|reason| reason.text.strip_prefix("pipeline failed"))
            {
                mark.push(look::dim(job));
            }
            let mut atom = vec![look::dim("pipeline ")];
            let at = width_of(&atom);
            let width = width_of(&mrs::mark_of(mr));
            atom.extend(mark);
            if has_bar || has_lacks {
                atom.push(look::dim(" ·"));
            }
            let rects = scene.atom(atom);
            if let [rect] = rects.as_slice()
                && rect.width >= at + width
            {
                mrs::pipeline_region(scene, mr, Rect::new(rect.x + at, rect.y, width, 1));
            }
            if let Some(progress) = &mr.progress {
                // Its gap before it, its ` ·` after it.
                let tail = if has_lacks { 2 } else { 0 };
                let room = scene.width().saturating_sub(scene.x() + 1 + tail);
                let room = if room < BAR_BESIDE {
                    scene.end_line();
                    scene.width().saturating_sub(tail)
                } else {
                    room
                };
                let after = has_lacks.then(|| look::dim(" ·"));
                let (boxes, y) = mrs::bar_atom(scene, mr, progress, room.min(MAX_BAR), after, ctx);
                shown = boxes;
                fold = y.map(|y| (y, Act::Fold(Fold::MrBoxes(mr.iid))));
            }
            if has_lacks {
                mrs::lacks(scene, mr, &reasons, ctx);
            }
            // While the overview merges it: `merging …`, then `merged`.
            if let Some(phase) = mrs::merge_of(ctx, mr).map(|merge| merge.phase)
                && matches!(phase, Phase::Running | Phase::Merged)
            {
                nodes::right(scene, mrs::status_of(mr, mrs::merge_of(ctx, mr)));
            }
        }
        None => {
            let of = parts.len();
            let has_boxes = node.is_some_and(|node| !node.boxes.is_empty() && !ctx.is_once);
            for (at, part) in parts.into_iter().enumerate() {
                put(scene, part, !has_more(at, of) && !has_boxes);
            }
            if let Some((node, number)) = node.zip(node.and_then(|node| node.number)) {
                fold = nodes::boxes_atom(scene, node, ctx).map(|y| (y, Act::Fold(Fold::TreeBoxes(number))));
            }
        }
    }
    scene.end_line();
    if let Some(sections) = shown {
        boxes::list(scene, sections, 2);
    }
    if let Some(node) = node.filter(|_| block.mr.is_none()) {
        nodes::boxes_list(scene, node, ctx);
    }
    // The fold's block, inside the block's (#346, as before #390): from the
    // bar's line down its list, a click or Tab and Enter folds it.
    if let Some((y, act)) = fold {
        let area = Rect::new(0, y, scene.width(), scene.y() - y);
        scene.regions.push(Region {
            area,
            target: Target::Block(act),
        });
    }
    if let Some(node) = node {
        nodes::conflict_line(scene, node);
    }
}

/// A reason of an MR its block says on another line (#390): the pipeline
/// (the GitLab line's mark), unpushed commits (its commits), and with a
/// worktree the MRs it waits for (its state, or what its agent does).
fn is_said_elsewhere(reason: &Reason, has_tree: bool) -> bool {
    reason.text == "no pipeline yet"
        || reason.text.starts_with("pipeline ")
        || (has_tree && reason.text.ends_with(" not pushed"))
        || (has_tree && mrs::is_wait(reason))
}

/// How many lines the problem shows before its fold (#390).
const PROBLEM_LINES: usize = 2;

/// What the block works on (#390): its title where that says more than the
/// problem (the MR's, else its issue's, never both when one repeats the
/// other), then the problem from its issue's Goal (#341), its first
/// PROBLEM_LINES lines and `▸ more` until a click shows it whole.
fn description(scene: &mut Scene<Act>, block: &Block, node: &Node, ctx: &Ctx) {
    let problem = node.problem.as_deref().unwrap_or("");
    let titles = [block.mr.map(|mr| mr.title.as_str()), Some(node.title.as_str())];
    let shown = titles
        .into_iter()
        .flatten()
        .find(|title| problem.is_empty() || !is_repeated(title, problem));
    if let Some(title) = shown {
        let spans = match block.mr {
            Some(mr) if mr.title == title => mrs::title_spans(title, mr.is_merged),
            _ => vec![look::dim(title)],
        };
        look::flow(scene, &spans);
        scene.end_line();
    }
    if problem.is_empty() {
        return;
    }
    let Some(number) = node.number.filter(|_| !ctx.is_once) else {
        look::say(scene, problem, ratatui::style::Style::new());
        scene.end_line();
        return;
    };
    let is_open = ctx.folds.boxes.problems.contains(&number);
    let words: Vec<&str> = problem.split_whitespace().collect();
    let fits = |count: usize, more: bool| {
        let mut trial: Scene<Act> = Scene::new(scene.width());
        for word in &words[..count] {
            trial.atom(vec![Span::raw(word.to_string())]);
        }
        if more {
            trial.atom(vec![Span::raw("… ▸ more")]);
        }
        trial.finish();
        trial.lines.len() <= PROBLEM_LINES
    };
    if is_open || fits(words.len(), false) {
        look::say(scene, problem, ratatui::style::Style::new());
        if is_open {
            scene.atom(vec![look::dim("▾ less")]);
            fold_region(scene, number, "▾ less");
        }
        scene.end_line();
        return;
    }
    let count = (0..words.len()).rev().find(|&count| fits(count, true)).unwrap_or(0);
    for word in &words[..count] {
        scene.atom(vec![Span::raw(word.to_string())]);
    }
    scene.atom(vec![look::dim("… ▸ more")]);
    fold_region(scene, number, "… ▸ more");
    scene.end_line();
}

/// The last atom placed (`label`) folds or shows the whole problem of `number`.
fn fold_region(scene: &mut Scene<Act>, number: i64, label: &str) {
    let width = Span::raw(label).width() as u16;
    let area = Rect::new(scene.x().saturating_sub(width), scene.y(), width, 1);
    scene.regions.push(Region {
        area,
        target: Target::Button {
            label: label.into(),
            tone: theme::Tone::Gray,
            action: Act::Fold(Fold::Problem(number)),
        },
    });
}

/// Whether `title` says little more than `text` (#390): most of its words
/// (those of four letters or more, a conventional-commit prefix left out)
/// are in it.
fn is_repeated(title: &str, text: &str) -> bool {
    let words = |text: &str| -> Vec<String> {
        text.split(|ch: char| !ch.is_alphanumeric())
            .filter(|word| word.chars().count() >= 4)
            .map(str::to_lowercase)
            .collect()
    };
    let title = title.split_once(": ").map_or(title, |(_, rest)| rest);
    let said = words(text);
    let own = words(title);
    if own.is_empty() {
        return true;
    }
    let shared = own.iter().filter(|word| said.contains(word)).count();
    shared * 5 >= own.len() * 3
}

/// The MR's title with its status at the right (`ready to merge`, `merging
/// …`, a merged one's age), padded to the status before a merge: merging
/// never moves the block.
fn title(scene: &mut Scene<Act>, block: &Block, ctx: &Ctx) {
    let Some(mr) = block.mr else { return };
    look::flow(scene, &mrs::title_spans(&mr.title, mr.is_merged));
    // An open one without its worktree has its status on its head.
    if mr.is_merged || block.is_done {
        let mut status = mrs::status_of(mr, mrs::merge_of(ctx, mr));
        let short = width_of(&mrs::status_of(mr, None)).saturating_sub(width_of(&status));
        if short > 0 {
            status.insert(0, Span::raw(" ".repeat(usize::from(short))));
        }
        if !status.is_empty() {
            scene.right(status);
        }
    }
    if mr.is_merged {
        look::ticks(scene);
    }
    scene.end_line();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::overview::actor::spawn::Run;
    use crate::tui::overview::{Folds, GitlabState};
    use breadboard_core::model::Tone;
    use breadboard_core::model::develop::Migrations;
    use breadboard_core::model::diff::Overlap;
    use breadboard_core::model::mr_blockers::{Progress, Reason};
    use breadboard_core::model::orchestration::MrRow;
    use breadboard_core::model::resolve::Conflict;
    use breadboard_core::model::tree::AgentLine;
    use breadboard_core::model::tree::{Doing, Lifecycle, Lines, TreeOf};
    use std::path::PathBuf;
    use std::time::Duration;

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
                today: None,
                spent: None,
            }),
            lines: Lines {
                issue: vec![model::Span::plain(format!("#{}", place + 300))],
                mr: if has_tree {
                    vec![model::Span::dim("no MR yet")]
                } else {
                    vec![]
                },
                commits: vec![model::Span::plain("↑1")],
                ..Lines::default()
            },
            doing: None,
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
            checked: Vec::new(),
            boxes: vec![],
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
        draw_all(worktrees, &MergeRequests::default(), folds, width, actor)
    }

    fn draw_all(
        worktrees: &Worktrees,
        merge_requests: &MergeRequests,
        folds: Folds,
        width: u16,
        actor: &super::super::actor::Actor,
    ) -> Scene<Act> {
        let gitlab = GitlabState::default();
        let ctx = Ctx {
            is_once: false,
            folds,
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor,
            words: super::super::actor::view::no_words(),
            develop: None,
        };
        let mut scene = Scene::new(width);
        work(&mut scene, worktrees, merge_requests, None, true, &ctx);
        scene.finish();
        scene
    }

    /// The line of the first node.
    fn first_node(lines: &[String]) -> usize {
        lines.iter().position(|line| line.starts_with("1. ")).unwrap()
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
    fn a_block_is_its_head_its_agent_its_gitlab_line_and_its_title() {
        let scene = draw(&worktrees(0, 0), Folds::default(), 72);
        assert_eq!(scene.note.as_deref(), Some("2 wait for GitLab"));
        assert_eq!(
            text(&scene),
            [
                "0.9 GB free: fresh agents on the shared stack",
                "⇶ breadboard  .claude/orchestrations/breadboard.json",
                "   agents 2 · issues 3 · reused 0 · handovers failed 0",
                "   no .env.local in any worktree: no stack, the app can't run there",
                "1. 303-every-button                               waits for the pipeline",
                "   ● Mira Albrecht  (3 issues · 2 MRs done)",
                "   #301 · no MR yet · ↑1",
                "   the title of #301",
                "2. 305-refresh                                    waits for the pipeline",
                "   ● Mira Albrecht  (3 issues · 2 MRs done)",
                "   #302 · no MR yet · ↑1",
                "   the title of #302",
            ]
        );
    }

    #[test]
    fn a_block_says_what_its_agent_does_under_it_and_its_problem_after_its_title() {
        let mut worktrees = worktrees(0, 0);
        let live = &mut worktrees.tree.live;
        live[0].problem =
            Some("The overview's node names the topic of its issue but not the problem it works on.".into());
        live[0].doing = Some(Doing {
            parts: vec![
                model::Span::plain("cause found, fix pushed"),
                model::Span::new("wt:check running", Paint::Cyan),
            ],
            at: Some(-4 * 60_000),
        });
        // A title the problem repeats is left out.
        live[1].title = "breadboard: wt:new hangs".into();
        live[1].problem = Some("wt:new hangs when the branch exists.".into());
        let lines = text(&draw(&worktrees, Folds::default(), 56));
        let at = first_node(&lines);
        assert_eq!(
            lines[at..at + 12],
            [
                "1. 303-every-button               waits for the pipeline",
                "   ● Mira Albrecht  (3 issues · 2 MRs done)",
                "     cause found, fix pushed · wt:check running",
                "     ·\u{a0}4\u{a0}min\u{a0}ago",
                "   #301 · no MR yet · ↑1",
                "   the title of #301",
                "   The overview's node names the topic of its issue but",
                "   not the problem it works on.",
                "2. 305-refresh                    waits for the pipeline",
                "   ● Mira Albrecht  (3 issues · 2 MRs done)",
                "   #302 · no MR yet · ↑1",
                "   wt:new hangs when the branch exists.",
            ],
            "{lines:#?}"
        );
        // No `said`: the agent's last words stay in its conversation (#390).
        assert!(!lines.iter().any(|line| line.contains("said")), "{lines:#?}");
    }

    #[test]
    fn a_long_problem_shows_two_lines_until_a_click_shows_it_whole() {
        let mut worktrees = worktrees(0, 0);
        worktrees.tree.live[0].problem = Some(
            "The worktree block is readable at a glance: a fixed order of lines, each with one kind of \
             information, nothing said twice, and the problem it works on last."
                .into(),
        );
        let fold = Act::Fold(Fold::Problem(301));
        let folded = draw(&worktrees, Folds::default(), 50);
        let lines = text(&folded);
        let at = lines
            .iter()
            .position(|line| line.contains("The worktree block"))
            .unwrap();
        assert!(lines[at + 1].ends_with("… ▸ more"), "{lines:#?}");
        assert!(!lines[at + 2].starts_with("   information") && !lines.join(" ").contains("works on last"));
        let more = folded.regions.iter().find(|region| *region.action() == fold).unwrap();
        assert_eq!((more.area.y as usize, more.is_button()), (at + 1, true));
        let mut folds = Folds::default();
        folds.boxes.problems.insert(301);
        let lines = text(&draw(&worktrees, folds, 50));
        let all = lines.join(" ");
        assert!(all.contains("works on last.") && all.contains("▾ less"), "{lines:#?}");
    }

    #[test]
    fn the_dot_of_an_agent_at_work_pulses_and_asks_to_be_drawn_again() {
        let mut worktrees = worktrees(0, 0);
        worktrees.tree.live[0].state.key = "working";
        let dot = |scene: &Scene<Act>| {
            let line = scene
                .lines
                .iter()
                .find(|line| line.spans.iter().any(|span| span.content.contains("Mira")))
                .unwrap();
            line.spans.iter().find(|span| span.content == "●").unwrap().style
        };
        let at = |now: i64| {
            let gitlab = GitlabState::default();
            let ctx = Ctx {
                is_once: false,
                folds: Folds::default(),
                gitlab: &gitlab,
                now,
                main_name: "platform",
                actor: super::super::actor::idle(),
                words: super::super::actor::view::no_words(),
                develop: None,
            };
            let mut scene = Scene::new(72);
            work(&mut scene, &worktrees, &MergeRequests::default(), None, true, &ctx);
            scene.finish();
            scene
        };
        let (bright, dim) = (at(0), at(800));
        assert_ne!(dot(&bright), dot(&dim));
        assert_eq!(dot(&at(1600)), dot(&bright));
        assert_eq!(bright.redraw_in, Some(Duration::from_millis(800)));
        // An agent that waits: a steady dot, nothing to draw again.
        let steady = draw(&super::tests::worktrees(0, 0), Folds::default(), 72);
        assert_eq!(steady.redraw_in, None);
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
                "1. 303-every-button",
                "               start: → Elif Kowalczyk: its handover failed:",
                "          wt:orchestrate: not built yet; a click tries again",
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
                (4, 4, 72, Act::Go(Go::FocusSpace("/w/303-every-button".into()))),
                (8, 4, 72, Act::Go(Go::FocusSpace("/w/305-refresh".into()))),
            ]
        );
    }

    #[test]
    fn the_queue_and_the_removed_fold_and_say_how_to_unfold() {
        let shown = draw(&worktrees(2, 5), Folds::default(), 72);
        let lines = text(&shown);
        assert!(lines.contains(&"▾ queued (2) · in the order they start · click to fold".to_string()));
        assert!(
            lines.contains(&"10. q                                          start: waits for an agent".to_string()),
            "{lines:#?}"
        );
        // The done ones show until folded, as the merged MRs did.
        assert!(
            lines.contains(&"▾ done (5) · 5 removed · click to fold".to_string()),
            "{lines:#?}"
        );
        assert!(
            shown
                .regions
                .iter()
                .any(|region| region.action() == &Act::Fold(Fold::Removed))
        );
        let folds = Folds {
            is_done_folded: false,
            is_queued_folded: true,
            ..Folds::default()
        };
        let lines = text(&draw(&worktrees(2, 5), folds, 72));
        assert!(lines.contains(&"▸ queued (2) · in the order they start · click to show".to_string()));
        assert!(!lines.iter().any(|line| line.contains("10. q")));
        assert!(lines.iter().any(|line| line.starts_with("20. gone")));
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
    fn the_stack_has_its_own_line_and_the_collisions_fold_to_one() {
        let scene = draw(&with_stacks(), Folds::default(), 100);
        assert_eq!(scene.note.as_deref(), Some("2 wait for GitLab"));
        let lines = text(&scene);
        assert_eq!(
            lines[3..12],
            [
                "1. 303-every-button                                                           waits for the pipeline",
                "   ● Mira Albrecht  (3 issues · 2 MRs done)",
                "   ● :3002  stack running",
                "   #301 · no MR yet · ↑1",
                "   ▸ ⚠ collides with develop · 3 MRs: #302 #307 #294",
                "   the title of #301",
                "2. 305-refresh                                                                waits for the pipeline",
                "   ● Mira Albrecht  (3 issues · 2 MRs done)",
                "   ⇒ :3000  shared",
            ]
        );
        // The stacks line up after the widest app.
        let at = |line: &str, mark: &str| line[..line.find(mark).unwrap()].chars().count();
        assert_eq!(at(&lines[5], "stack running"), at(&lines[11], "shared"));
        // The whole node is one block, the collisions' line a fold inside it.
        let block = scene
            .regions
            .iter()
            .find(|region| *region.action() == Act::Go(Go::FocusSpace("/w/303-every-button".into())))
            .unwrap();
        assert_eq!((block.area.y, block.area.height), (3, 6));
        let fold = Act::Fold(Fold::Collides(301));
        let line = scene.regions.iter().find(|region| *region.action() == fold).unwrap();
        assert_eq!((line.area.y, line.area.height), (7, 1));
        // A click lists the files each shares, a line each; the merge order keeps all of them.
        let mut folds = Folds::default();
        folds.boxes.collides.insert(301);
        let lines = text(&draw(&with_stacks(), folds, 100));
        assert_eq!(
            lines[7..11],
            [
                "   ▾ ⚠ collides with develop · 3 MRs: #302 #307 #294",
                "     develop: tree.mjs",
                "     #302: web/src/index.ts, core/src/index.ts, a.md, b.md",
                "     #307, #294: c.md",
            ]
        );
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
        assert_eq!(text(&silent)[3], "   docker does not answer: the stacks are unknown");
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
        let mut folds = Folds::default();
        folds.boxes.collides.insert(301);
        let scene = draw(&worktrees, folds, 100);
        let lines = text(&scene);
        assert!(
            lines.contains(&"     #302: web/src/index.ts, a.md, b.md, c.md".to_string()),
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
        // Unfolded every file is named.
        assert_eq!(links[4], ("c.md".into(), open("c.md")));
        // A link lies on its name only, inside the node's block and its collisions' fold.
        let block = scene
            .regions
            .iter()
            .find(|region| *region.action() == Act::Go(Go::FocusSpace("/w/303-every-button".into())))
            .unwrap()
            .area;
        let tree = scene
            .regions
            .iter()
            .find(|region| region.is_button() && matches!(region.action(), Act::OpenFile { .. }))
            .unwrap()
            .area;
        assert_eq!(tree.width, 8);
        assert!(tree.y >= block.y && tree.y < block.y + block.height);
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
        assert_eq!(lines[5], "   ○ :3002  ▶ stack down");
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
        assert_eq!(&lines[5].chars().skip(x).take(12).collect::<String>(), "▶ stack down");
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
        assert_eq!(lines[5], "   ○ :3002  ◐ starting …", "{lines:#?}");
        // Up after docker's last answer: `stack …` until docker tells, never `▶ stack down` again.
        actor.stacks.insert(
            dir.clone(),
            Run {
                done_at: Some(2_000),
                ..Run::default()
            },
        );
        let lines = at(&actor);
        assert_eq!(lines[5], "   ○ :3002  stack …", "{lines:#?}");
        actor.stacks.insert(
            dir.clone(),
            Run {
                done_at: Some(500),
                ..Run::default()
            },
        );
        assert!(at(&actor)[5].contains("▶ stack down"));
        actor.stacks.insert(
            dir.clone(),
            Run {
                error: Some("port 5434 is taken".into()),
                ..Run::default()
            },
        );
        let lines = at(&actor);
        assert!(lines[5].contains("▶ stack down"));
        assert_eq!(lines[6], "   ✗ the stack didn't start: port 5434 is taken");
        // The stack's line holds each of these: the lines before it stay as they were.
        let idle = text(&draw(&down(), Folds::default(), 100));
        assert_eq!(lines[3..5], idle[3..5]);
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
        assert!(lines.contains(&"   ⚠ the database doesn't answer: connection refused".to_string()));
        assert!(
            lines.contains(&"   ⚠ 4 migrations not applied: 0041_a, 0042_b, 0043_c +1 more  ▶ migrate".to_string()),
            "{lines:#?}"
        );
        assert!(buttons(&scene).contains(&("▶ migrate".into(), Act::Migrate("/w/305-refresh".into()))));
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
        assert!(lines.contains(&"   ◐ migrating …".to_string()), "{lines:#?}");
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
        let mut open = Folds::default();
        open.boxes.collides.insert(301);
        let scene = draw(&worktrees, open.clone(), 120);
        let lines = text(&scene);
        assert!(
            lines.contains(&"   ✗ a merge of develop conflicts in render.mjs".to_string()),
            "{lines:#?}"
        );
        // develop's line keeps the file that merges cleanly, not the conflicting one twice.
        assert!(lines.contains(&"     develop: tree.mjs".to_string()), "{lines:#?}");
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
        let lines = text(&draw(&worktrees, open, 120));
        assert!(
            lines.contains(&"     develop, merges cleanly: tree.mjs, render.mjs".to_string()),
            "{lines:#?}"
        );
        assert!(!lines.iter().any(|line| line.contains("✗")));
        // Only develop, all of it conflicting: no collision line, the conflict line alone.
        worktrees.tree.live[0].collides.truncate(1);
        worktrees.tree.live[0].conflicts = Some(worktrees.tree.live[0].collides[0].files.clone());
        let lines = text(&draw(&worktrees, Folds::default(), 120));
        assert!(!lines.iter().any(|line| line.contains("collides")));
        assert!(lines.contains(&"   ✗ a merge of develop conflicts in tree.mjs, render.mjs".to_string()));
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

    /// An MR of issue `number`, open or merged.
    fn mr(iid: i64, number: i64, branch: &str, is_merged: bool) -> MrRow {
        MrRow {
            number: Some(number),
            graph: if is_merged { String::new() } else { "●".into() },
            below: String::new(),
            mr: Some(MrFacts {
                iid,
                url: format!("https://gitlab/mr/{iid}"),
                pipeline: model::Span {
                    url: Some(format!("https://gitlab/pipelines/{iid}")),
                    ..model::Span::new("✓ passed", Paint::Green)
                },
                title: format!("feat(breadboard): the work of !{iid}"),
                branch: branch.into(),
                dir: None,
                is_merged,
                status: Some(if is_merged {
                    model::Span::dim("merged 2 h ago")
                } else {
                    model::Span::new("ready to merge", Paint::Green)
                }),
                conflict: (!is_merged).then_some(Conflict::None),
                conflicts: None,
                reasons: vec![],
                progress: (!is_merged).then(|| Progress {
                    ticked: 9,
                    total: 12,
                    work: (9, 9),
                    acceptance: (0, 3),
                    ..Default::default()
                }),
                ci: None,
                boxes: vec![],
                open_boxes: vec![],
                issue: None,
                spent: None,
            }),
        }
    }

    /// The worktrees with removed ones, and their MRs: !320 of #302 (the
    /// second worktree) first in the merge order, !330 of an issue without a
    /// worktree, a row of the graph alone, !299 merged of the first removed.
    fn joined() -> (Worktrees, MergeRequests) {
        let worktrees = worktrees(0, 2);
        let mut first = mr(320, 302, "feature/305-refresh", false);
        if let Some(facts) = first.mr.as_mut() {
            facts.status = None;
            facts.conflict = Some(Conflict::Button {
                behind: 3,
                is_refused: false,
            });
            facts.reasons = vec![
                Reason {
                    text: "2 acceptance boxes open".into(),
                    who: "agent".into(),
                    is_conflict: false,
                },
                Reason {
                    text: "waits for !330".into(),
                    who: "!330".into(),
                    is_conflict: false,
                },
            ];
        }
        let merge_requests = MergeRequests {
            open: vec![
                first,
                MrRow {
                    number: None,
                    graph: "├─┘".into(),
                    below: String::new(),
                    mr: None,
                },
                mr(330, 400, "feature/400-elsewhere", false),
            ],
            merged: vec![mr(299, 320, "feature/gone", true)],
            counts: "2 open · 1 merged · in merge order, side by side where independent".into(),
        };
        (worktrees, merge_requests)
    }

    #[test]
    fn gitlabs_conflict_is_said_red_and_its_agents() {
        let (worktrees, mut merge_requests) = joined();
        if let Some(facts) = merge_requests.open[0].mr.as_mut() {
            facts.conflicts = Some("conflicts with develop (README.md)".into());
            facts.reasons.clear();
        }
        let scene = draw_all(
            &worktrees,
            &merge_requests,
            Folds::default(),
            120,
            super::super::actor::idle(),
        );
        let lines: Vec<String> = text(&scene).iter().map(|line| line.replace('\u{a0}', " ")).collect();
        assert!(
            lines
                .iter()
                .any(|line| line
                    .contains("resolve merge conflicts  conflicts with develop (README.md) → agent · 3 behind")),
            "{lines:#?}"
        );
        let span = scene
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.starts_with("conflicts\u{a0}with"))
            .unwrap();
        assert_eq!(span.style.fg, Some(theme::Tone::Red.fg()));
    }

    fn draw_joined(folds: Folds, width: u16) -> Scene<Act> {
        let (worktrees, merge_requests) = joined();
        draw_all(&worktrees, &merge_requests, folds, width, super::super::actor::idle())
    }

    #[test]
    fn a_block_holds_its_worktree_and_its_mr_in_the_merge_order() {
        let scene = draw_joined(Folds::default(), 100);
        assert_eq!(
            scene.note.as_deref(),
            Some("2 wait for GitLab · 2 open · 1 merged · in merge order, side by side where independent")
        );
        let lines: Vec<String> = text(&scene).iter().map(|line| line.replace('\u{a0}', " ")).collect();
        let at = lines.iter().position(|line| line.contains("305-refresh")).unwrap();
        assert_eq!(
            lines[at..at + 9],
            [
                "●   2. !320  305-refresh                                                      waits for the pipeline",
                "       ● Mira Albrecht  (3 issues · 2 MRs done)",
                "       ↑1 · pipeline ✓ passed · ━━━━━━━━━━━━━━━━━━━━──────  9/12 boxes · work 9/9 · acceptance 0/3 ·",
                "        resolve merge conflicts  3 behind · 2 acceptance boxes open → agent",
                "       feat(breadboard): the work of !320",
                "├─┘",
                "●      !330  400-elsewhere                                                            ready to merge",
                "       pipeline ✓ passed · ━━━━━━━━━━━━━━━━━━━━━━━━────────  9/12 boxes · work 9/9 · acceptance 0/3",
                "       feat(breadboard): the work of !330",
            ],
            "{lines:#?}"
        );
        // Then the worktree without an MR, in the work order; the done ones fold.
        assert!(lines[at + 9].starts_with("    1.       303-every-button"), "{lines:#?}");
        assert!(
            lines.contains(&"▾ done (2) · 1 merged · 1 removed · click to fold".to_string()),
            "{lines:#?}"
        );
    }

    #[test]
    fn the_block_goes_to_its_space_the_number_to_the_mr_its_mark_to_the_pipeline() {
        let scene = draw_joined(Folds::default(), 100);
        let lines = text(&scene);
        let at = lines.iter().position(|line| line.contains("305-refresh")).unwrap() as u16;
        let on = |y: u16| -> Vec<(String, Act)> {
            scene
                .regions
                .iter()
                .filter(|region| region.area.y == y)
                .map(|region| match &region.target {
                    Target::Button { label, action, .. } => (label.clone(), action.clone()),
                    Target::Block(action) => ("block".into(), action.clone()),
                })
                .collect()
        };
        let show = |url: &str| {
            Act::Go(Go::Show {
                url: url.into(),
                dir: None,
            })
        };
        assert_eq!(
            on(at),
            [
                ("!320".into(), show("https://gitlab/mr/320")),
                ("block".into(), Act::Go(Go::FocusSpace("/w/305-refresh".into()))),
            ]
        );
        // The pipeline and the button that resolves its conflicts on the GitLab line.
        assert_eq!(on(at + 2), [("✓ passed".into(), show("https://gitlab/pipelines/320"))]);
        let mark = scene
            .regions
            .iter()
            .find(|region| region.area.y == at + 2)
            .unwrap()
            .area;
        let x = usize::from(mark.x);
        assert_eq!(
            &lines[at as usize + 2].chars().skip(x).take(8).collect::<String>(),
            "✓ passed"
        );
        assert!(on(at + 3).contains(&("resolve\u{a0}merge\u{a0}conflicts".into(), Act::Resolve(320))));
        // The whole block lights: its head down to its title.
        let block = scene
            .regions
            .iter()
            .find(|region| region.area.y == at && !region.is_button())
            .unwrap();
        assert_eq!(block.area.height, 5);
        // An MR without a worktree opens the MR.
        let other = lines.iter().position(|line| line.contains("400-elsewhere")).unwrap() as u16;
        assert!(on(other).contains(&("block".into(), show("https://gitlab/mr/330"))));
    }

    #[test]
    fn the_mark_of_an_mr_waiting_in_the_ci_queue_runs_it_now() {
        let (worktrees, mut merge_requests) = joined();
        if let Some(facts) = merge_requests.open[0].mr.as_mut() {
            facts.pipeline = model::Span::new("CI: next (2nd) ▸", Paint::Yellow);
            facts.ci = Some(breadboard_core::ci::Place::Next { at: 2 });
        }
        let scene = draw_all(
            &worktrees,
            &merge_requests,
            Folds::default(),
            120,
            super::super::actor::idle(),
        );
        let lines = text(&scene);
        assert!(
            lines.iter().any(|line| line.contains("pipeline CI: next (2nd) ▸")),
            "{lines:#?}"
        );
        assert!(
            scene
                .regions
                .iter()
                .any(|region| region.is_button() && matches!(region.action(), Act::RunNow(320)))
        );
    }

    #[test]
    fn merged_and_removed_fold_together_the_last_merged_first() {
        let lines = text(&draw_joined(Folds::default(), 100));
        let fold = lines.iter().position(|line| line.starts_with("▾ done (2)")).unwrap();
        // !299 merged, with its removed worktree #320 (place 20), then the other removed one.
        assert!(
            lines[fold + 1].starts_with("   20. !299  gone              ✓ passed"),
            "{lines:#?}"
        );
        assert!(
            lines[fold + 2].contains("the work of !299") && lines[fold + 2].ends_with("2 h ago"),
            "{lines:#?}"
        );
        // Two lines each: no facts, the removed one its issue's title.
        assert!(lines[fold + 3].starts_with("   21.       gone"), "{lines:#?}");
        assert_eq!(lines[fold + 4].trim(), "the title of #321", "{lines:#?}");
        assert!(!lines.iter().any(|line| line.contains("#320 open")), "{lines:#?}");
        // Folded: one line, both counted.
        let lines = text(&draw_joined(
            Folds {
                is_done_folded: true,
                ..Folds::default()
            },
            100,
        ));
        assert!(lines.contains(&"▸ done (2) · 1 merged · 1 removed · click to show".to_string()));
        assert!(!lines.iter().any(|line| line.contains("!299")));
    }

    #[test]
    fn everything_either_table_showed_is_in_the_block() {
        let (mut worktrees, merge_requests) = joined();
        let node = &mut worktrees.tree.live[1];
        node.app = Some(model::Span::new("● :3002", Paint::Green));
        node.stack = Some(model::Span::new("stack running", Paint::Green));
        node.app_port = Some(3002);
        node.collides = vec![Overlap {
            name: "develop".into(),
            files: vec!["a/tree.mjs".into()],
        }];
        let scene = draw_all(
            &worktrees,
            &merge_requests,
            Folds::default(),
            120,
            super::super::actor::idle(),
        );
        let all = text(&scene).join("\n").replace('\u{a0}', " ");
        for field in [
            // WORKTREES: the record, how reuse went, the node.
            "⇶ breadboard",
            ".claude/orchestrations/breadboard.json",
            "agents 2 · issues 3",
            "2. !320  305-refresh",
            "● :3002",
            "stack running",
            "waits for the pipeline",
            "Mira Albrecht  (3 issues · 2 MRs done)",
            "↑1 · pipeline",
            "⚠ collides with develop",
            // MERGE REQUESTS: the MR, its boxes, what it lacks with who, the graph, the merged.
            "!320",
            "✓ passed",
            "feat(breadboard): the work of !320",
            "9/12 boxes · work 9/9 · acceptance 0/3",
            "resolve merge conflicts",
            "3 behind",
            "2 acceptance boxes open → agent",
            "├─┘",
            "400-elsewhere",
            "ready to merge",
            "1 merged",
        ] {
            assert!(all.contains(field), "{field} missing:\n{all}");
        }
        assert!(
            scene
                .note
                .as_deref()
                .unwrap()
                .contains("side by side where independent")
        );
    }

    #[test]
    fn merging_moves_nothing_and_a_wide_title_never_runs_past_the_edge() {
        let (worktrees, mut merge_requests) = joined();
        if let Some(facts) = merge_requests.open[2].mr.as_mut() {
            facts.title = "fix(breadboard): the orchestrator no longer asks decisions answered in the overview".into();
        }
        for width in [24, 40, 60, 100] {
            let scene = draw_all(
                &worktrees,
                &merge_requests,
                Folds::default(),
                width,
                super::super::actor::idle(),
            );
            for line in &scene.lines {
                assert!(line.width() <= width as usize, "{width}: {line:?}");
            }
            let all = text(&scene).join(" ");
            let words: Vec<&str> = all.split_whitespace().collect();
            assert!(
                words
                    .join(" ")
                    .contains("the orchestrator no longer asks decisions answered in the overview"),
                "{width}: {all}"
            );
        }
    }

    /// The boxes of #346's fold: two sections, one ticked, one tried.
    fn boxes() -> Vec<breadboard_core::model::boxes::BoxSection> {
        use breadboard_core::model::boxes::{BoxLine, BoxSection, Check};
        let line = |text: &str, is_ticked: bool, check: Option<(&str, Option<&str>)>| BoxLine {
            is_ticked,
            text: text.into(),
            check: check.map(|(text, answer)| Check {
                text: text.into(),
                answer: answer.map(String::from),
            }),
        };
        vec![
            BoxSection {
                heading: "Scope".into(),
                boxes: vec![
                    line(
                        "A fold control on the MR row",
                        true,
                        Some(("test: boxes.rs › folds", None)),
                    ),
                    line("Folded by default", false, None),
                ],
            },
            BoxSection {
                heading: "Acceptance criteria".into(),
                boxes: vec![line(
                    "A click shows every box",
                    false,
                    Some(("try #12", Some("looks good"))),
                )],
            },
        ]
    }

    #[test]
    fn the_boxes_fold_under_the_bar_of_the_block_and_open_every_box() {
        let (mut worktrees, mut merge_requests) = joined();
        if let Some(facts) = merge_requests.open[0].mr.as_mut() {
            facts.boxes = boxes();
            facts.issue = Some(302);
        }
        let draw = |worktrees: &Worktrees, merge_requests: &MergeRequests, folds: Folds| {
            draw_all(worktrees, merge_requests, folds, 100, super::super::actor::idle())
        };
        let fold = Act::Fold(Fold::MrBoxes(320));
        let folded = draw(&worktrees, &merge_requests, Folds::default());
        let lines = text(&folded);
        // On the GitLab line, its fold mark in front of it.
        let bar = lines.iter().position(|line| line.contains("#302: 9/12 boxes")).unwrap();
        assert!(lines[bar].contains("pipeline ✓ passed · ▸ ━"), "{lines:#?}");
        assert!(!lines.iter().any(|line| line.contains("• A fold")), "{lines:#?}");
        let regions = |scene: &Scene<Act>| {
            scene
                .regions
                .iter()
                .filter(|region| *region.action() == fold)
                .map(|region| (region.area.y, region.area.height, region.is_button()))
                .collect::<Vec<_>>()
        };
        // The bar a button, the fold's block from its line down (Tab and Enter, #346): here its lacks too.
        assert_eq!(regions(&folded), [(bar as u16, 1, true), (bar as u16, 2, false)]);

        let mut folds = Folds::default();
        folds.boxes.mrs.insert(320);
        let open = draw(&worktrees, &merge_requests, folds.clone());
        let lines = text(&open);
        assert!(lines[bar].contains("▾ ━"), "{lines:#?}");
        let scope = lines.iter().position(|line| line.trim() == "Scope").unwrap();
        assert!(scope > bar, "{lines:#?}");
        assert!(
            lines[scope + 1].contains("• A fold control on the MR row"),
            "{lines:#?}"
        );
        // A plain bullet, its colour the state (#370): a ticked box green, an open one yellow; no ☑/☐.
        let bullet_of = |row: usize| {
            open.lines[row]
                .spans
                .iter()
                .find(|span| span.content.contains('•'))
                .and_then(|span| span.style.fg)
        };
        assert_eq!(bullet_of(scope + 1), Some(theme::Tone::Green.fg()));
        assert!(
            (scope..open.lines.len()).any(|row| bullet_of(row) == Some(theme::Tone::Yellow.fg())),
            "{lines:#?}"
        );
        assert!(!lines.iter().any(|line| line.contains(['☑', '☐'])), "{lines:#?}");
        assert!(
            lines.iter().any(|line| line.contains("try #12 looks good")),
            "{lines:#?}"
        );
        // The list is in the fold's block inside the block's: a click on it folds it again.
        let list = regions(&open);
        assert_eq!(list[0], (bar as u16, 1, true));
        let (y, height, is_button) = list[1];
        assert!(
            y as usize == bar && (y + height) as usize > scope + 2 && !is_button,
            "{list:?}"
        );

        // Open boxes that hold the merge show whole before any click (#354), in the block.
        if let Some(facts) = merge_requests.open[0].mr.as_mut() {
            facts.open_boxes = vec![boxes().remove(1)];
        }
        let first = draw_all(
            &worktrees,
            &merge_requests,
            Folds::default(),
            100,
            super::super::actor::idle(),
        );
        let lines = text(&first);
        assert!(lines[bar].contains("▾ ━"), "{lines:#?}");
        assert!(
            lines.iter().any(|line| line.trim() == "Acceptance criteria"),
            "{lines:#?}"
        );
        assert!(!lines.iter().any(|line| line.contains("Scope")), "{lines:#?}");

        // A worktree without an MR: the fold of its issue's boxes stands where the bar would.
        worktrees.tree.live[0].boxes = boxes();
        let lines = text(&draw(&worktrees, &merge_requests, Folds::default()));
        let head = lines.iter().position(|line| line.contains("303-every-button")).unwrap();
        assert_eq!(
            lines[head + 2].trim(),
            "#301 · no MR yet · ↑1 · ▸ 1/3 boxes · 2 open",
            "{lines:#?}"
        );
    }
}
