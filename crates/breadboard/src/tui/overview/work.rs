//! WORK of an orchestration (#347): what WORKTREES and MERGE REQUESTS
//! showed apart, one block per piece of work. Under the orchestration's
//! record (`⇶ <name>`, how reuse went) the blocks with an open MR come in
//! the merge order, the MR graph's lanes in front of them ("side by side
//! where independent"), then the worktrees without one in the work order;
//! the queue and the done ones (merged, closed, removed) fold, shown until
//! folded, as the merged MRs were.
//!
//! A block: its head line (number, feature, `!M`, pipeline, app and stack,
//! where it stands at the right), the MR's title with its status at the
//! right, the bar of its boxes (#294), what it lacks with who acts on it,
//! the worktree's facts with who works in it (#292), where it collides
//! (#332), its issue's title, the problem it works on and what its agent
//! is on now (#341). A click anywhere goes to the worktree's space
//! (its MR, else its issue, where it has none), `!M` opens the MR, its mark
//! the pipeline; hover lights the whole block (#296).

use super::merge_requests::{self as mrs};
use super::worktrees::{self as nodes, Cells, Piece};
use super::{Act, Ctx, Fold, look};
use crate::tui::links;
use crate::tui::scene::{Region, Target, width_of};
use crate::tui::theme;
use crate::tui::{Page, Scene};
use breadboard_core::model::orchestration::{MergeRequests, MrFacts, OrchestrationModel};
use breadboard_core::model::order::Go;
use breadboard_core::model::tree::{Node, Worktrees};
use breadboard_core::model::{self, Docker, Paint};
use ratatui::text::Span;

/// The section's name.
const WORK: &str = "WORK";
/// The widest the bar of the boxes gets: past it a line of dashes says nothing more.
const MAX_BAR: u16 = 72;
/// The fewest columns the bar keeps (merge_requests' `MIN_BAR`).
const MIN_BAR: u16 = 8;

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
/// graph's, the number's, the name's, `!M`'s; the app's and stack's after
/// the pipeline marks.
struct Layout {
    graph: u16,
    digits: usize,
    name: u16,
    iid: u16,
    bar: u16,
    cells: Cells,
}

impl Layout {
    fn of(parts: &Parts, width: u16, ctx: &Ctx) -> Layout {
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
        let room = width.saturating_sub(graph + digits as u16 + 2);
        let name = blocks
            .iter()
            .map(|block| Span::raw(name_of(block)).width() as u16)
            .max()
            .unwrap_or(0)
            + 2;
        let facts = || blocks.iter().filter_map(|block| block.mr);
        let iid = facts().map(|mr| Span::raw(format!("!{}", mr.iid)).width() as u16).max();
        let mark = facts().map(|mr| width_of(&mrs::mark_of(mr))).max().unwrap_or(0);
        let iid = iid.unwrap_or(0);
        let lead = name + if iid > 0 { iid + 2 + mark + 2 } else { 0 };
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
            name,
            iid,
            bar: room.min(MAX_BAR),
            cells: Cells::of(&live, lead, room, ctx),
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
    let layout = Layout::of(&parts, scene.width(), ctx);
    let tree = &worktrees.tree;
    // Why there is no app and no stack, or why a stack says `?`: said once, not on every block.
    let why = if !layout.cells.is_any_env && tree.live.iter().any(|node| node.has_tree) {
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
        look::gutter(scene, first, rest, |scene| {
            let mine = head(scene, block, layout, ctx);
            title(scene, block, ctx);
            if let Some(mr) = block.mr {
                if let Some(progress) = &mr.progress
                    && scene.width() >= MIN_BAR
                {
                    scene.atom_gap(mrs::bar(progress, layout.bar.min(scene.width())), 0);
                    scene.end_line();
                }
                if mrs::has_lacks(mr, ctx) {
                    mrs::lacks(scene, mr, ctx);
                    scene.end_line();
                }
            }
            match block.node {
                // A done one: its MR's title said what it was, else its issue's.
                Some(node) if block.is_done => {
                    if block.mr.is_none() {
                        look::say(scene, &node.title, theme::dim());
                        scene.end_line();
                    }
                }
                Some(node) => {
                    nodes::body(scene, node, &layout.cells, mine, ctx);
                    look::say(scene, &node.title, theme::dim());
                    scene.end_line();
                    nodes::said_lines(scene, node, ctx);
                }
                None => {}
            }
        });
    };
    match go_of(block) {
        Go::Nothing => draw(scene),
        go => scene.block(Act::Go(go), draw),
    }
}

/// The head line: the name, `!M` and its pipeline mark, the app and stack
/// in their column, where it stands at the right. Returns the cells that
/// stand in no column: they go beside the facts.
fn head(scene: &mut Scene<Act>, block: &Block, layout: &Layout, ctx: &Ctx) -> Option<Piece> {
    let style = match block.node {
        Some(node) if node.has_tree => look::pink().add_modifier(ratatui::style::Modifier::BOLD),
        _ => look::pink(),
    };
    scene.atom_gap(vec![look::span(name_of(block), style)], 0);
    if let Some(mr) = block.mr {
        scene.pad_to(layout.name);
        // `!M` is the MR's link, a click of its own (§11.3): a Ctrl-click in herdr opens its page too.
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
        scene.pad_to(layout.name + layout.iid + 2);
        for rect in scene.atom_gap(mrs::mark_of(mr), 0) {
            mrs::pipeline_region(scene, mr, rect);
        }
    }
    let mut mine = block.node.and_then(|node| layout.cells.of_node(node, ctx));
    if let Some(column) = layout.cells.column
        && let Some(piece) = mine.take()
    {
        scene.pad_to(column);
        let rects = scene.atom_gap(piece.spans.clone(), 0);
        piece.place(scene, &rects);
    }
    if let Some(node) = block.node {
        nodes::right(
            scene,
            vec![look::span(&node.state.text, look::style_of(&node.state.paint))],
        );
    }
    scene.end_line();
    mine
}

/// The MR's title with its status at the right (`ready to merge`, `merging
/// …`, a merged one's age), padded to the status before a merge: merging
/// never moves the block.
fn title(scene: &mut Scene<Act>, block: &Block, ctx: &Ctx) {
    let Some(mr) = block.mr else { return };
    look::flow(scene, &mrs::title_spans(&mr.title, mr.is_merged));
    let mut status = mrs::status_of(mr, mrs::merge_of(ctx, mr));
    let short = width_of(&mrs::status_of(mr, None)).saturating_sub(width_of(&status));
    if short > 0 {
        status.insert(0, Span::raw(" ".repeat(usize::from(short))));
    }
    if !status.is_empty() {
        scene.right(status);
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
    fn a_block_is_its_name_and_state_its_facts_and_agent_and_title_under_its_name() {
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
                "   #301 open · ↑1                  ● Mira Albrecht · 3 issues · 2 merged",
                "   the title of #301",
                "2. 305-refresh                                    waits for the pipeline",
                "   #302 open · ↑1                  ● Mira Albrecht · 3 issues · 2 merged",
                "   the title of #302",
            ]
        );
    }

    #[test]
    fn a_block_says_its_problem_and_what_its_agent_does_now_wrapped_under_their_text() {
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
                "   the title of #301",
                "   problem The overview's node names the topic of its",
                "           issue but not the problem it works on.",
                "   now     cause found, fix pushed ·\u{a0}4\u{a0}min\u{a0}ago",
            ]
        );
        let second = lines
            .iter()
            .position(|line| line.ends_with("the title of #302"))
            .unwrap();
        assert_eq!(
            lines[second..second + 2],
            ["   the title of #302", "   problem wt:new hangs."]
        );
        // The first line of the last answer, marked as said, without a time.
        worktrees.tree.live[1].now = Some(Now {
            text: "Pushed the fix.".into(),
            at: None,
            is_note: false,
        });
        let lines = text(&draw(&worktrees, Folds::default(), 56));
        assert!(lines.contains(&"   said    Pushed the fix.".to_string()), "{lines:#?}");
    }

    #[test]
    fn facts_that_do_not_fit_beside_the_agent_go_under_it_the_agent_stays_under_the_state() {
        let lines = text(&draw(&worktrees(0, 0), Folds::default(), 56));
        let at = first_node(&lines);
        assert_eq!(
            lines[at..at + 4],
            [
                "1. 303-every-button               waits for the pipeline",
                "   #301 open · ↑1  ● Mira Albrecht · 3 issues · 2 merged",
                "   the title of #301",
                "2. 305-refresh                    waits for the pipeline",
            ]
        );
        // Not even the first fact fits: the agent alone at the right, the facts under it.
        let lines = text(&draw(&worktrees(0, 0), Folds::default(), 46));
        let at = lines.iter().position(|line| line.contains("● Mira")).unwrap();
        assert_eq!(
            lines[at..at + 2],
            ["         ● Mira Albrecht · 3 issues · 2 merged", "   #301 open · ↑1"]
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
                (4, 3, 72, Act::Go(Go::FocusSpace("/w/303-every-button".into()))),
                (7, 3, 72, Act::Go(Go::FocusSpace("/w/305-refresh".into()))),
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
    fn app_and_stack_stand_in_a_column_after_the_names_where_it_collides_under_the_facts() {
        let scene = draw(&with_stacks(), Folds::default(), 100);
        assert_eq!(scene.note.as_deref(), Some("2 wait for GitLab"));
        assert_eq!(
            text(&scene)[3..10],
            [
                "1. 303-every-button  ● :3002  stack running                                   waits for the pipeline",
                "   #301 open · ↑1                                              ● Mira Albrecht · 3 issues · 2 merged",
                "   ⚠ collides with develop: tree.mjs · #302: web/src/index.ts, core/src/index.ts, a.md +1 more ·",
                "   #307, #294: c.md",
                "   the title of #301",
                "2. 305-refresh       ⇒ :3000  shared                                          waits for the pipeline",
                "   #302 open · ↑1                                              ● Mira Albrecht · 3 issues · 2 merged",
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
            lines[3..7],
            [
                "1. 303-every-button                         waits for the pipeline",
                "   #301 open · ↑1            ● Mira Albrecht · 3 issues · 2 merged",
                "                                           ● :3002  stack running",
                "   ⚠ collides with develop: tree.mjs · #302: web/src/index.ts,",
            ]
        );
        // A long state leaves no column: at the right of the agent's own line.
        let mut long = with_stacks();
        long.tree.live[1].state.text = "waits for you: try #1, decide #2, look #3, merge".into();
        let lines = text(&draw(&long, Folds::default(), 80));
        assert_eq!(
            lines[3..7],
            [
                "1. 303-every-button                                       waits for the pipeline",
                "   #301 open · ↑1                          ● Mira Albrecht · 3 issues · 2 merged",
                "                                                         ● :3002  stack running",
                "   ⚠ collides with develop: tree.mjs · #302: web/src/index.ts,",
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
        let scene = draw(&worktrees, Folds::default(), 100);
        let lines = text(&scene);
        assert!(
            lines.contains(
                &"   ⚠ collides with develop: tree.mjs · #302: web/src/index.ts, a.md, b.md +1 more".to_string()
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
            "1. 303-every-button  ○ :3002  ▶ stack down                                    waits for the pipeline"
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
        assert_eq!(lines[5], "   ✗ the stack didn't start: port 5434 is taken");
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
        assert!(lines.contains(&"   ⚠ the database doesn't answer: connection refused".to_string()));
        assert!(
            lines.contains(
                &"   ⚠ 4 migrations not applied: 0041_a, 0042_b, 0043_c +1 more  ▶ pnpm pg:migrate".to_string()
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
        let scene = draw(&worktrees, Folds::default(), 120);
        let lines = text(&scene);
        assert!(
            lines.contains(&"   ✗ a merge of develop conflicts in render.mjs".to_string()),
            "{lines:#?}"
        );
        // develop's line keeps the file that merges cleanly, not the conflicting one twice.
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("   ⚠ collides with develop: tree.mjs · #302:"))
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
                .any(|line| line.starts_with("   ⚠ collides with develop, merges cleanly: tree.mjs, render.mjs ·")),
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
                reasons: vec![],
                progress: (!is_merged).then(|| Progress {
                    ticked: 9,
                    total: 12,
                    work: (9, 9),
                    acceptance: (0, 3),
                    ..Default::default()
                }),
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
            lines[at..at + 10],
            [
                "●   2. 305-refresh       !320  ✓ passed                                       waits for the pipeline",
                "       feat(breadboard): the work of !320",
                "       ━━━━━━━━━━━━━━━━━━━━━━━━────────  9/12 boxes · work 9/9 · acceptance 0/3",
                "        resolve merge conflicts  3 behind · 2 acceptance boxes open → agent · waits for !330",
                "       #302 open · ↑1                                          ● Mira Albrecht · 3 issues · 2 merged",
                "       the title of #302",
                "├─┘",
                "●      400-elsewhere     !330  ✓ passed",
                "       feat(breadboard): the work of !330                                             ready to merge",
                "       ━━━━━━━━━━━━━━━━━━━━━━━━────────  9/12 boxes · work 9/9 · acceptance 0/3",
            ],
            "{lines:#?}"
        );
        // Then the worktree without an MR, in the work order; the done ones fold.
        assert!(lines[at + 10].starts_with("    1. 303-every-button"), "{lines:#?}");
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
                ("✓ passed".into(), show("https://gitlab/pipelines/320")),
                ("block".into(), Act::Go(Go::FocusSpace("/w/305-refresh".into()))),
            ]
        );
        assert!(on(at + 3).contains(&("resolve\u{a0}merge\u{a0}conflicts".into(), Act::Resolve(320))));
        // The whole block lights: its head down to its issue's title.
        let block = scene
            .regions
            .iter()
            .find(|region| region.area.y == at && !region.is_button())
            .unwrap();
        assert_eq!(block.area.height, 6);
        // An MR without a worktree opens the MR.
        let other = lines.iter().position(|line| line.contains("400-elsewhere")).unwrap() as u16;
        assert!(on(other).contains(&("block".into(), show("https://gitlab/mr/330"))));
    }

    #[test]
    fn merged_and_removed_fold_together_the_last_merged_first() {
        let lines = text(&draw_joined(Folds::default(), 100));
        let fold = lines.iter().position(|line| line.starts_with("▾ done (2)")).unwrap();
        // !299 merged, with its removed worktree #320 (place 20), then the other removed one.
        assert!(
            lines[fold + 1].starts_with("   20. gone              !299  ✓ passed"),
            "{lines:#?}"
        );
        assert!(
            lines[fold + 2].contains("the work of !299") && lines[fold + 2].ends_with("2 h ago"),
            "{lines:#?}"
        );
        // Two lines each: no facts, the removed one its issue's title.
        assert!(lines[fold + 3].starts_with("   21. gone"), "{lines:#?}");
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
            "2. 305-refresh",
            "● :3002",
            "stack running",
            "waits for the pipeline",
            "Mira Albrecht · 3 issues · 2 merged",
            "#302 open · ↑1",
            "⚠ collides with develop",
            "the title of #302",
            // MERGE REQUESTS: the MR, its boxes, what it lacks with who, the graph, the merged.
            "!320",
            "✓ passed",
            "feat(breadboard): the work of !320",
            "9/12 boxes · work 9/9 · acceptance 0/3",
            "resolve merge conflicts",
            "3 behind",
            "2 acceptance boxes open → agent",
            "waits for !330",
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
}
