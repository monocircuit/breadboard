//! MERGE REQUESTS of an orchestration (render-orchestration.mjs,
//! mr-blockers.mjs, resolve.mjs): the open MRs as a graph in the merge
//! order, each a block of its head (`!iid`, pipeline, title, status), the bar
//! of its boxes (#294) and what it still lacks with who acts on it; the
//! merged below a fold. Titles wrap in their column, reasons under the
//! title's, nothing is cut (#304). Hover lights a whole row (#296); a click
//! opens the MR, its pipeline mark the pipeline, its one button resolves
//! conflicts (§11.3). The merge on two clicks is the posted merge action's
//! (ACTIONS, rule 6); a row only says when the overview merges it.

use super::actor::merge::{Merge, Phase};
use super::{Act, Ctx, Fold, boxes, look};
use crate::tui::links;
use crate::tui::scene::{Region, Target, width_of};
use crate::tui::theme::{self, Tone};
use crate::tui::{Page, Scene};
use breadboard_core::model::Paint;
use breadboard_core::model::mr_blockers::{Progress, Reason};
use breadboard_core::model::orchestration::{MergeRequests, MrFacts, MrRow, OrchestrationModel};
use breadboard_core::model::order::Go;
use breadboard_core::model::resolve::Conflict;
use breadboard_core::time::ago;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// The fewest columns a title gets beside its status (mr-blockers.mjs `MIN_TITLE`).
const MIN_TITLE: u16 = 24;
/// The fewest columns the bar keeps before its counts go (#294 `MIN_BAR`).
const MIN_BAR: u16 = 8;
/// The status of an open MR nothing holds up.
const READY: &str = "ready to merge";
/// Core's status of an MR closed without a merge.
const CLOSED: &str = "closed";
/// Below this many columns for the title the row is one flow of words, no columns.
const MIN_COLUMNS: u16 = 16;

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    let mrs = match &model.merge_requests {
        Ok(mrs) => mrs,
        Err(why) => return page.failed("MERGE REQUESTS", why),
    };
    page.section("MERGE REQUESTS", |scene| {
        if !model.has_remote {
            let text = if ctx.gitlab.error.is_some() {
                "GitLab does not answer yet".to_string()
            } else {
                ctx.gitlab.progress.clone()
            };
            scene.pad_to(1);
            look::say(scene, &text, theme::dim());
            return;
        }
        if !mrs.counts.is_empty() {
            scene.note(&mrs.counts);
        }
        if mrs.open.is_empty() && mrs.merged.is_empty() {
            scene.pad_to(1);
            scene.text("none yet", theme::dim());
            return;
        }
        rows(scene, mrs, ctx);
        scene.end_line();
        scene.pad_to(1);
        look::say(scene, &ctx.gitlab.fresh(), theme::dim());
    });
}

/// The columns every row shares, so open and merged line up across the fold:
/// the graph's, the iid's, the pipeline mark's, the status's.
struct Columns {
    graph: u16,
    iid: u16,
    mark: u16,
    status: u16,
    /// The longest title.
    title: u16,
}

impl Columns {
    fn of(mrs: &MergeRequests) -> Self {
        let width = |text: &str| Span::raw(text).width() as u16;
        let facts = || mrs.open.iter().chain(&mrs.merged).filter_map(|row| row.mr.as_ref());
        let lanes = mrs
            .open
            .iter()
            .map(|row| width(&row.graph).max(width(&row.below)))
            .max()
            .unwrap_or(0);
        Columns {
            // Two columns between the lanes and the iids, as the Node overview.
            graph: if lanes > 0 { lanes + 2 } else { 0 },
            iid: facts().map(|mr| width(&format!("!{}", mr.iid))).max().unwrap_or(0),
            mark: facts().map(|mr| width_of(&mark_of(mr))).max().unwrap_or(0),
            status: facts().map(|mr| width_of(&status_of(mr, None))).max().unwrap_or(0),
            title: facts().map(|mr| width(&mr.title)).max().unwrap_or(0),
        }
    }

    /// Where titles, bars and reasons start, after `!iid  mark  `.
    fn title_at(&self) -> u16 {
        self.iid + 2 + self.mark + 2
    }

    /// The title's column in `width` (the row's, after its lanes): as wide as
    /// the longest title, the statuses' after it, at least MIN_TITLE.
    fn title_in(&self, width: u16) -> u16 {
        let beside = if self.status > 0 { self.status + 2 } else { 0 };
        let room = width.saturating_sub(self.title_at() + beside).max(MIN_TITLE);
        self.title.min(room).min(width.saturating_sub(self.title_at()))
    }
}

fn rows(scene: &mut Scene<Act>, mrs: &MergeRequests, ctx: &Ctx) {
    let columns = Columns::of(mrs);
    for (i, row) in mrs.open.iter().enumerate() {
        let is_parted = mrs.open.get(i + 1).is_some_and(|next| next.mr.is_some());
        open_row(scene, row, &columns, ctx, is_parted);
    }
    if mrs.merged.is_empty() {
        return;
    }
    let is_folded = ctx.folds.is_merged_folded && !ctx.is_once;
    let hint = match (ctx.is_once, is_folded) {
        (true, _) => "",
        (false, true) => " · click to show",
        (false, false) => " · click to fold",
    };
    let mark = if is_folded { "▸" } else { "▾" };
    let fold = format!("{mark} {}{hint}", done_counts(mrs));
    if ctx.is_once {
        scene.end_line();
        look::say(scene, &fold, theme::dim());
        scene.end_line();
    } else {
        scene.block(Act::Fold(Fold::Merged), |scene| look::say(scene, &fold, theme::dim()));
    }
    if is_folded {
        return;
    }
    for mr in newest_first(mrs) {
        let lanes = vec![Span::raw(" ".repeat(columns.graph as usize))];
        scene.block(Act::Go(show(mr)), |scene| {
            look::gutter(scene, lanes.clone(), lanes, |scene| head(scene, mr, &columns, None));
        });
    }
}

/// The fold's words: `42 merged`, `41 merged · 1 closed` (closed without a
/// merge, core's `closed`): a closed MR is never counted as merged.
fn done_counts(mrs: &MergeRequests) -> String {
    let rows = mrs.merged.iter().filter_map(|row| row.mr.as_ref());
    let closed = rows.filter(|mr| is_closed(mr)).count();
    let merged = mrs.merged.len() - closed;
    match (merged, closed) {
        (_, 0) => format!("{merged} merged"),
        (0, _) => format!("{closed} closed"),
        _ => format!("{merged} merged · {closed} closed"),
    }
}

/// Closed without a merge.
fn is_closed(mr: &MrFacts) -> bool {
    mr.status.as_ref().is_some_and(|status| status.text == CLOSED)
}

/// The merged, the last merged first (lazygit's and git log's order): what
/// just went in stands right under the fold. Those without a merge time
/// (the closed, a merge GitLab gave no time for) after.
fn newest_first(mrs: &MergeRequests) -> impl Iterator<Item = &MrFacts> {
    let rows = || mrs.merged.iter().filter_map(|row| row.mr.as_ref());
    let is_untimed = |mr: &&MrFacts| {
        mr.status
            .as_ref()
            .is_none_or(|status| status.text == "merged" || status.text == CLOSED)
    };
    rows()
        .filter(|mr| !is_untimed(mr))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .chain(rows().filter(is_untimed))
}

/// A click on an MR's row: its page, in the tab group of its worktree.
fn show(mr: &MrFacts) -> Go {
    Go::Show {
        url: mr.url.clone(),
        dir: mr.dir.clone(),
    }
}

/// An open MR: its lane, head, bar and what it lacks below; `is_parted`:
/// an empty row ends its block, with only the lanes that go on through it
/// (git log --graph's line between two commits), when the next row is an MR.
/// What an MR lacks, its block's last line, never stands right above the next
/// MR's `●` and reads as that one's; the row is the block's (it lights and
/// clicks with it, #296). A row of the graph alone (`├─┘`) parts two blocks
/// already. Not a `│` under every `●`: in this graph a lane under an MR says
/// another needs it.
fn open_row(scene: &mut Scene<Act>, row: &MrRow, columns: &Columns, ctx: &Ctx, is_parted: bool) {
    let width = columns.graph as usize;
    let Some(mr) = &row.mr else {
        // A line of the graph alone: no MR, no click.
        scene.end_line();
        scene.atom(lanes(&row.graph, width));
        scene.end_line();
        return;
    };
    // The MR's own node in the colour of where it stands, its lanes dim.
    let first = node(&row.graph, mr, width);
    let rest = lanes(&row.below, width);
    scene.block(Act::Go(show(mr)), |scene| {
        look::gutter(scene, first, rest, |scene| {
            head(scene, mr, columns, merge_of(ctx, mr));
            let at = columns.title_at();
            if let Some(progress) = &mr.progress {
                bar_line(scene, mr, progress, columns, ctx);
            }
            if has_lacks(mr, ctx) {
                let at = if scene.width() >= at + MIN_COLUMNS { at } else { 2 };
                scene.indent(at);
                scene.pad_to(at);
                lacks(scene, mr, ctx);
                scene.indent(0);
                scene.end_line();
            }
            if is_parted {
                scene.blank();
            }
        });
    });
}

/// The bar of an MR's boxes under its head, the fold of their list (#346):
/// `▸ ━━──  2/6 boxes · …`, a click or Enter on it opens every box under
/// it, one on the line or the list folds it again. `--once` has no fold.
fn bar_line(scene: &mut Scene<Act>, mr: &MrFacts, progress: &Progress, columns: &Columns, ctx: &Ctx) {
    let at = columns.title_at();
    let beside = if columns.status > 0 { columns.status + 2 } else { 0 };
    let room = scene.width().saturating_sub(at);
    let size = (columns.title_in(scene.width()) + beside).min(room);
    if ctx.is_once || mr.boxes.is_empty() {
        if room >= MIN_BAR {
            scene.pad_to(at);
            scene.atom_gap(bar(progress, mr.issue, size), 0);
            scene.end_line();
        }
        return;
    }
    let is_open = ctx.folds.boxes.mrs.contains(&mr.iid);
    scene.block(Act::Fold(Fold::MrBoxes(mr.iid)), |scene| {
        scene.pad_to(at);
        let mut spans = vec![boxes::mark(is_open)];
        if room >= MIN_BAR + 2 {
            spans.extend(bar(progress, mr.issue, size - 2));
        } else {
            spans.push(look::dim(format!("{}/{}", progress.ticked, progress.total)));
        }
        scene.atom_gap(spans, 0);
        scene.end_line();
        if is_open {
            let at = if scene.width() >= at + MIN_COLUMNS { at } else { 2 };
            boxes::list(scene, &mr.boxes, at + 2);
        }
    });
}

/// The lanes of an MR's first line, its `●` green when it is ready, red
/// when it is broken (its pipeline failed, conflicts its agent must resolve).
fn node(graph: &str, mr: &MrFacts, width: usize) -> Vec<Span<'static>> {
    let tone = if is_ready(mr) {
        Some(Tone::Green)
    } else if mr.pipeline.text.starts_with('✗')
        || matches!(mr.conflict, Some(Conflict::Button { .. } | Conflict::LeftAfterPush))
    {
        Some(Tone::Red)
    } else {
        None
    };
    let mut spans = lanes(graph, width);
    if let Some(tone) = tone
        && let Some(node) = spans.iter_mut().find(|span| span.content == "●")
    {
        node.style = Style::new().fg(tone.fg());
    }
    spans
}

/// The colours of the lanes, one per lane in turn (lazygit's graph): none
/// of the colours that say where an MR stands (green, red, yellow).
const LANES: [Color; 3] = [Color::Cyan, Color::Blue, Color::Magenta];

/// A line of the graph padded to `width`, each lane in its colour: a lane is
/// 2 columns, a `─` belongs to the lane it joins (the one at its right), so
/// `├─┘` shows which lane goes into which.
fn lanes(graph: &str, width: usize) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, ch) in format!("{graph:<width$}").chars().enumerate() {
        let lane = if ch == '─' { i.div_ceil(2) } else { i / 2 };
        let style = if ch == ' ' {
            Style::new()
        } else {
            Style::new().fg(LANES[lane % LANES.len()])
        };
        match spans.last_mut() {
            Some(last) if last.style == style && !last.content.contains('●') && ch != '●' => {
                last.content.to_mut().push(ch);
            }
            _ => spans.push(Span::styled(ch.to_string(), style)),
        }
    }
    spans
}

/// The column after `!iid`: the MR's pipeline mark, a merged one's too
/// (§11.3): GitLab merges on a red pipeline as well, and that must still
/// show under `N merged`.
fn mark_of(mr: &MrFacts) -> Vec<Span<'static>> {
    look::spans(std::slice::from_ref(&mr.pipeline))
}

/// The overview's merge of an MR: armed, running, merged or failed.
fn merge_of<'a>(ctx: &'a Ctx, mr: &MrFacts) -> Option<&'a Merge> {
    ctx.actor.merges.of.get(&mr.iid)
}

/// Why the overview's last merge of an MR failed.
fn failed_of<'a>(ctx: &'a Ctx, mr: &MrFacts) -> Option<&'a str> {
    merge_of(ctx, mr)
        .filter(|merge| merge.phase == Phase::Failed)
        .and_then(|merge| merge.error.as_deref())
}

/// What an MR's status says, at the right of its head: `2 h ago` under the
/// merged fold, or `ready to merge`; while the overview merges it (two clicks
/// on its merge action) `merging …`, then `merged`. Never a button: the row's
/// click opens the MR, as Node's (§11.3).
fn status_of(mr: &MrFacts, merge: Option<&Merge>) -> Vec<Span<'static>> {
    let Some(status) = mr.status.as_ref() else {
        return vec![];
    };
    if mr.is_merged {
        // Under `▾ 42 merged` its age alone, as git log's dates: `merged`
        // on each row said the fold's word again and cost the title 7 columns.
        let text = status.text.strip_prefix("merged ").unwrap_or(&status.text);
        return vec![look::dim(text)];
    }
    match merge.map(|merge| merge.phase) {
        Some(Phase::Running) if is_ready(mr) => vec![look::span("merging …", theme::tone(Tone::Yellow))],
        Some(Phase::Merged) if is_ready(mr) => vec![look::span("merged", theme::tone(Tone::Green))],
        _ => look::spans(std::slice::from_ref(status)),
    }
}

/// `!iid  mark  title  status`: the title wraps in its column, the status at
/// the right of its first line; a narrow row is one flow of words. The
/// layout is the one of the status before a merge: merging never moves the row.
fn head(scene: &mut Scene<Act>, mr: &MrFacts, columns: &Columns, merge: Option<&Merge>) {
    // `!iid` is the MR's link, open and merged alike (§11.3): a Ctrl-click
    // in herdr opens its page, as on the worktree's node.
    let iid = if mr.is_merged {
        look::dim(format!("!{}", mr.iid))
    } else {
        look::bold(format!("!{}", mr.iid))
    };
    let iid = Span::styled(iid.content, links::style(iid.style, Some(&mr.url)));
    let title = title_spans(&mr.title, mr.is_merged);
    let mark = mark_of(mr);
    let status = {
        let mut status = status_of(mr, merge);
        let short = width_of(&status_of(mr, None)).saturating_sub(width_of(&status));
        if short > 0 {
            status.insert(0, Span::raw(" ".repeat(short as usize)));
        }
        status
    };
    if mr.is_merged {
        look::ticks(scene);
    }
    let width = scene.width();
    let at = columns.title_at();
    let column = columns.title_in(width);
    if column < MIN_COLUMNS || width < at + column {
        // No room for columns: the row as words, the status after them.
        scene.end_line();
        scene.indent(2);
        scene.atom(vec![iid]);
        for rect in scene.atom(mark) {
            pipeline_region(scene, mr, rect);
        }
        look::flow(scene, &title);
        if !status.is_empty() {
            scene.atom(status);
        }
        scene.indent(0);
        scene.end_line();
        return;
    }
    // The title alone in its column; a row without a status takes the status's room too.
    let beside = if columns.status > 0 { columns.status + 2 } else { 0 };
    // A shorter status than the widest leaves its title the difference: statuses stand at the right edge.
    let own = width_of(&status);
    let own_beside = if own > 0 { own + 2 } else { 0 };
    let span = (column + beside - own_beside).min(width - at);
    let mut left = Scene::<Act>::new(at + span);
    left.atom(vec![iid]);
    left.pad_to(columns.iid + 2);
    let mark_width = width_of(&mark);
    left.atom_gap(mark, 0);
    left.pad_to(at);
    left.indent(at);
    // The status stands beside the first line only: the words that do not
    // fit there wrap under the title's column across the whole row, so a
    // long title takes one line less (lazygit's subjects use the whole width).
    let words = look::words_of(&title);
    let first = words
        .iter()
        .scan(at, |x, word| {
            let gap = if *x > at { 1 } else { 0 };
            *x += gap + width_of(word);
            (*x <= at + span).then_some(())
        })
        .count()
        .max(1);
    for word in &words[..first.min(words.len())] {
        left.atom(word.clone());
    }
    left.finish();
    if words.len() > first {
        let mut rest = Scene::<Act>::new(width);
        rest.indent(at);
        rest.pad_to(at);
        for word in &words[first..] {
            rest.atom(word.clone());
        }
        rest.finish();
        left.lines.extend(rest.lines);
    }
    scene.end_line();
    let top = scene.y();
    pipeline_region(scene, mr, Rect::new(columns.iid + 2, top, mark_width, 1));
    let status_width = width_of(&status);
    let is_beside = !status.is_empty() && at + span + 2 + status_width <= width;
    for (i, line) in left.lines.into_iter().enumerate() {
        let mut spans = line.spans;
        if i == 0 && is_beside {
            let x = width - status_width;
            pad(&mut spans, x);
            spans.extend(status.iter().cloned());
        }
        scene.lines.push(Line::from(spans));
    }
    if !status.is_empty() && !is_beside {
        // Too narrow to stand beside the title: its own line, at the right.
        let x = width.saturating_sub(status_width);
        let mut spans = Vec::new();
        pad(&mut spans, x);
        spans.extend(status.iter().cloned());
        scene.lines.push(Line::from(spans));
    }
}

/// An MR's pipeline mark opens its pipeline (§3: `!iid` and the mark are
/// links), a merged one's too: a failed one's page is a click away from its
/// row; the rest of the row opens the MR. One waiting in the CI queue runs now.
fn pipeline_region(scene: &mut Scene<Act>, mr: &MrFacts, area: Rect) {
    // Waiting in the CI queue (#357): a click moves it to the front.
    let action = if mr.ci.is_some() {
        Act::RunNow(mr.iid)
    } else if let Some(url) = mr.pipeline.url.as_ref() {
        Act::Go(Go::Show {
            url: url.clone(),
            dir: mr.dir.clone(),
        })
    } else {
        return;
    };
    let tone = match mr.pipeline.paint {
        Paint::Green => Tone::Green,
        Paint::Red => Tone::Red,
        Paint::Yellow => Tone::Yellow,
        _ => Tone::Gray,
    };
    scene.regions.push(Region {
        area,
        target: Target::Button {
            label: mr.pipeline.text.clone(),
            tone,
            action,
        },
    });
}

/// An open MR nothing holds up: its status offers the merge.
fn is_ready(mr: &MrFacts) -> bool {
    !mr.is_merged && mr.status.as_ref().is_some_and(|status| status.text == READY)
}

/// Spaces up to column `x` at the end of a line.
fn pad(spans: &mut Vec<Span<'static>>, x: u16) {
    let short = x.saturating_sub(width_of(spans));
    if short > 0 {
        spans.push(Span::raw(" ".repeat(short as usize)));
    }
}

/// A title, its conventional-commit type and scope (`feat(breadboard): `)
/// quieter than what it says; a merged one all quiet.
fn title_spans(title: &str, is_merged: bool) -> Vec<Span<'static>> {
    if is_merged {
        return vec![look::dim(title)];
    }
    let prefix = title.find(": ").filter(|&at| {
        at <= 24
            && title[..at]
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "()-_!/,".contains(ch))
    });
    match prefix {
        Some(at) => vec![look::dim(&title[..at + 1]), look::span(&title[at + 1..], Style::new())],
        None => vec![look::span(title, Style::new())],
    }
}

/// The bar of an MR's boxes in at most `width` columns (#294, `progressBar`): the
/// ticked part a thick line, green (yellow once only acceptance boxes are
/// open), the rest a thin dim one; behind it the counts, dropped section by
/// section on a narrow width, never `…`. The counts are `issue`'s boxes and
/// say so (`#335: 8/10 boxes`, #346): the MR's own description has its own.
fn bar(progress: &Progress, issue: Option<i64>, width: u16) -> Vec<Span<'static>> {
    let &Progress {
        ticked,
        total,
        work,
        acceptance,
        other,
        ..
    } = progress;
    // The boxes under neither Work nor Acceptance (an issue's `## Scope`): no
    // reason names them (they hold no merge up), so the bar's label does by
    // their section while one is open, or `0/9 boxes` would count 7 the row
    // says nothing of.
    let mut full = vec![format!("{ticked}/{total} boxes")];
    let sections = [("work", work, true), ("acceptance", acceptance, true)];
    let other = (progress.other_name.as_str(), other, other.0 < other.1);
    for (name, (done, all), is_said) in sections.into_iter().chain([other]) {
        if all > 0 && is_said {
            full.push(format!("{name} {done}/{all}"));
        }
    }
    let source = issue.map(|issue| format!("#{issue}: ")).unwrap_or_default();
    let candidates = [
        format!("{source}{}", full.join(" · ")),
        format!("{source}{ticked}/{total} boxes"),
        format!("{ticked}/{total} boxes"),
        format!("{ticked}/{total}"),
    ];
    let label = candidates
        .into_iter()
        .find(|label| width >= Span::raw(label.as_str()).width() as u16 + 2 + MIN_BAR);
    let size = match &label {
        Some(label) => width - Span::raw(label.as_str()).width() as u16 - 2,
        None => width,
    } as usize;
    let filled = (size * ticked + total / 2).checked_div(total).unwrap_or(0);
    let open = total - ticked;
    let is_only_acceptance = open > 0 && acceptance.1 - acceptance.0 == open;
    let tone = if is_only_acceptance { Tone::Yellow } else { Tone::Green };
    let mut spans = Vec::new();
    if filled > 0 {
        spans.push(Span::styled("━".repeat(filled), Style::new().fg(tone.fg())));
    }
    if size > filled {
        spans.push(look::dim("─".repeat(size - filled)));
    }
    if let Some(label) = label {
        spans.push(look::dim(format!("  {label}")));
    }
    spans
}

/// Whether `lacks` draws anything: a ready MR with no conflicts (unsaid),
/// no reasons and no failed merge has no line, so its block ends at its bar.
fn has_lacks(mr: &MrFacts, ctx: &Ctx) -> bool {
    !mr.reasons.is_empty()
        || failed_of(ctx, mr).is_some()
        || mr
            .conflict
            .as_ref()
            .is_some_and(|conflict| !matches!(conflict, Conflict::None))
}

/// Where its conflicts stand, then what it lacks, each `→ who` acts on it.
fn lacks(scene: &mut Scene<Act>, mr: &MrFacts, ctx: &Ctx) {
    // A short reason stays on one line: `try #2 broken → you`, not `try #2`
    // and `broken → you` on two; one longer than half the line wraps at its
    // words, or it would leave the line before it half empty. The scene
    // starts at its indent here.
    let room = scene.width().saturating_sub(scene.x()) / 2;
    let mut parts: Vec<Vec<Span<'static>>> = Vec::new();
    if let Some(error) = failed_of(ctx, mr) {
        parts.push(vec![look::span(
            format!("merge failed: {error}"),
            theme::tone(Tone::Red),
        )]);
    }
    let button = "resolve\u{a0}merge\u{a0}conflicts";
    match &mr.conflict {
        None => {}
        Some(Conflict::Checking) => parts.push(vec![look::dim("checking for conflicts …")]),
        // No conflicts goes unsaid, as lazygit leaves a clean file unmarked:
        // an open MR always has a conflict state, so the row without one is clean.
        Some(Conflict::None) => {}
        Some(Conflict::Resolved) => parts.push(vec![look::span("✓ conflicts resolved", theme::tone(Tone::Green))]),
        Some(Conflict::Button { behind, is_refused }) => {
            scene.button(button, Tone::Yellow, Act::Resolve(mr.iid));
            let mut part = Vec::new();
            if *behind > 0 {
                part.push(look::span(format!("{behind} behind"), theme::tone(Tone::Yellow)));
            }
            if *is_refused {
                part.push(look::span(" didn't get in to its agent", theme::tone(Tone::Red)));
            }
            parts.push(part);
        }
        Some(Conflict::LeftAfterPush) => {
            scene.button(button, Tone::Yellow, Act::Resolve(mr.iid));
            parts.push(vec![look::dim("conflicts left after its push")]);
        }
        Some(Conflict::Grey { why }) => {
            scene.inactive(button);
            parts.push(vec![look::dim(why)]);
        }
        Some(Conflict::Sent { text, at }) => {
            // Core's text starts with its own `◐` (resolve.rs `progress_of`).
            parts.push(vec![
                look::span(text.clone(), theme::tone(Tone::Yellow)),
                look::dim(format!(" {}", ago(*at, ctx.now))),
            ]);
            look::ticks(scene);
        }
    }
    parts.extend(reasons_of(&mr.reasons, room));
    look::parted(scene, &parts);
}

/// The reasons as parts, `text → who`; the MRs it waits for said once,
/// `waits for !311, !303` (the `→ !311` would only name them again), and a
/// reason that names the MR acting on it has no arrow either: `after !304
/// (needs #294)`, not `… → !304`. A part
/// at most `room` wide is glued into one word, so it wraps whole; the arrow
/// and its who always are.
fn reasons_of(reasons: &[Reason], room: u16) -> Vec<Vec<Span<'static>>> {
    let glue = |part: Vec<Span<'static>>| {
        if width_of(&part) > room {
            return part;
        }
        part.into_iter()
            .map(|span| Span::styled(span.content.replace(' ', "\u{a0}"), span.style))
            .collect()
    };
    let waits: Vec<&str> = reasons
        .iter()
        .filter(|reason| is_wait(reason))
        .map(|reason| reason.who.as_str())
        .collect();
    let mut parts = Vec::new();
    let mut is_waits_said = false;
    for reason in reasons {
        if is_wait(reason) {
            if !is_waits_said {
                is_waits_said = true;
                parts.push(glue(vec![look::span(
                    format!("waits for {}", waits.join(", ")),
                    theme::tone(Tone::Yellow),
                )]));
            }
            continue;
        }
        // A box list's own `…` is never left alone with the arrow on a line.
        let text = match reason.text.strip_suffix(" …") {
            Some(head) => format!("{head}\u{a0}…"),
            None => reason.text.clone(),
        };
        if names_its_who(reason) {
            parts.push(glue(vec![look::span(&text, theme::tone(Tone::Yellow))]));
            continue;
        }
        // What waits for you stands out of what others do (lazygit's status
        // colours): `→ you` bold, every other who dim.
        let who = format!("\u{a0}→\u{a0}{}", reason.who.replace(' ', "\u{a0}"));
        let who = if reason.who == "you" {
            look::bold(who)
        } else {
            look::dim(who)
        };
        parts.push(glue(vec![look::span(&text, theme::tone(Tone::Yellow)), who]));
    }
    parts
}

/// A reason on another MR whose text names that MR (`after !304 (needs #294)`).
fn names_its_who(reason: &Reason) -> bool {
    reason.who.starts_with('!')
        && reason
            .text
            .match_indices(reason.who.as_str())
            .any(|(at, _)| !reason.text[at + reason.who.len()..].starts_with(|ch: char| ch.is_ascii_digit()))
}

/// `waits for !N → !N`: a wait on another MR, named by its who alone.
fn is_wait(reason: &Reason) -> bool {
    reason.who.starts_with('!') && reason.text == format!("waits for {}", reason.who)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn reason(text: &str, who: &str) -> Reason {
        Reason {
            text: text.into(),
            who: who.into(),
            is_conflict: false,
        }
    }

    #[test]
    fn the_bar_drops_its_counts_section_by_section() {
        let progress = Progress {
            ticked: 9,
            total: 12,
            work: (9, 9),
            acceptance: (0, 3),
            ..Default::default()
        };
        let wide = text(&bar(&progress, None, 50));
        assert!(wide.ends_with("  9/12 boxes · work 9/9 · acceptance 0/3"), "{wide}");
        assert_eq!(Span::raw(wide.as_str()).width(), 50);
        // Wider: the bar takes the room it is given, the counts at its end (#294).
        let wide = text(&bar(&progress, None, 80));
        assert_eq!(Span::raw(wide.as_str()).width(), 80);
        assert!(wide.starts_with(&"━".repeat(30)), "{wide}");
        assert!(text(&bar(&progress, None, 20)).ends_with("  9/12 boxes"));
        assert!(text(&bar(&progress, None, 15)).ends_with("  9/12"));
        assert_eq!(text(&bar(&progress, None, 8)), "━━━━━━──");
    }

    #[test]
    fn the_bar_names_the_boxes_no_reason_names() {
        // 9 boxes, 7 under `## Scope`: the reasons name the 2 acceptance boxes only.
        let progress = Progress {
            ticked: 0,
            total: 9,
            work: (0, 0),
            acceptance: (0, 2),
            other: (0, 7),
            other_name: "scope".into(),
        };
        let label = text(&bar(&progress, None, 80));
        assert!(label.ends_with("  0/9 boxes · acceptance 0/2 · scope 0/7"), "{label}");
        // Once they are ticked they go unnamed: the label accounts for the open boxes.
        let done = Progress {
            ticked: 7,
            other: (7, 7),
            ..progress
        };
        assert!(text(&bar(&done, None, 80)).ends_with("  7/9 boxes · acceptance 0/2"));
    }

    #[test]
    fn the_bar_is_yellow_once_only_acceptance_boxes_are_open() {
        let only_acceptance = Progress {
            ticked: 2,
            total: 4,
            work: (2, 2),
            acceptance: (0, 2),
            ..Default::default()
        };
        assert_eq!(bar(&only_acceptance, None, 8)[0].style.fg, Some(Tone::Yellow.fg()));
        let work_open = Progress {
            work: (1, 2),
            acceptance: (1, 2),
            ..only_acceptance
        };
        assert_eq!(bar(&work_open, None, 8)[0].style.fg, Some(Tone::Green.fg()));
    }

    #[test]
    fn the_mrs_it_waits_for_are_named_once() {
        let parts = reasons_of(
            &[
                reason("pipeline pending", "GitLab"),
                reason("waits for !311", "!311"),
                reason("after !303 (shares a.rs)", "!303"),
                reason("waits for !304", "!304"),
                reason("after !3030 (needs #1)", "!303"),
            ],
            80,
        );
        let parts: Vec<String> = parts.iter().map(|part| text(part).replace('\u{a0}', " ")).collect();
        // `after !303 …` names its MR: no `→ !303` after it, as `waits for`.
        assert_eq!(
            parts,
            [
                "pipeline pending → GitLab",
                "waits for !311, !304",
                "after !303 (shares a.rs)",
                "after !3030 (needs #1) → !303"
            ]
        );
    }

    #[test]
    fn a_reason_that_fits_wraps_whole() {
        let reasons = [
            reason("try #2 broken", "you"),
            reason("2 work boxes open: one two three", "agent"),
        ];
        let parts = reasons_of(&reasons, 30);
        let words = |part: &Vec<Span<'static>>| look::words_of(part).len();
        // `try #2 broken → you` is one word; the longer one wraps at its spaces, the arrow glued.
        assert_eq!(words(&parts[0]), 1);
        assert_eq!(text(&parts[0]), "try\u{a0}#2\u{a0}broken\u{a0}→\u{a0}you");
        assert!(words(&parts[1]) > 1);
        assert!(
            text(&parts[1]).ends_with("three\u{a0}→\u{a0}agent"),
            "{:?}",
            text(&parts[1])
        );
    }

    #[test]
    fn a_box_lists_dots_go_with_the_arrow() {
        let parts = reasons_of(
            &[reason(
                "2 acceptance boxes open: The user can run …, The record says …",
                "agent",
            )],
            20,
        );
        let words = look::words_of(&parts[0]);
        assert_eq!(text(words.last().unwrap()), "says\u{a0}…\u{a0}→\u{a0}agent");
    }

    /// The lines of an open row at `width`, drawn as the section draws it.
    fn drawn(mr: MrFacts, width: u16) -> Vec<String> {
        let gitlab = super::super::GitlabState::default();
        let ctx = Ctx {
            is_once: false,
            folds: Default::default(),
            gitlab: &gitlab,
            now: 60_000,
            main_name: "platform",
            actor: super::super::actor::idle(),
            words: super::super::actor::view::no_words(),
        };
        let mrs = MergeRequests {
            open: vec![MrRow {
                number: Some(mr.iid),
                graph: "●".into(),
                below: String::new(),
                mr: Some(mr),
            }],
            ..Default::default()
        };
        let mut scene = Scene::<Act>::new(width);
        rows(&mut scene, &mrs, &ctx);
        scene.finish();
        lines(&scene)
    }

    /// An open row with its boxes at `width`, their fold as `folds` has it, `--once` or not.
    fn drawn_boxes(folds: super::super::Folds, is_once: bool, width: u16) -> Scene<Act> {
        use breadboard_core::model::boxes::{BoxLine, BoxSection, Check};
        let line = |text: &str, is_ticked: bool, check: Option<(&str, Option<&str>)>| BoxLine {
            is_ticked,
            text: text.into(),
            check: check.map(|(text, answer)| Check {
                text: text.into(),
                answer: answer.map(String::from),
            }),
        };
        let mr = MrFacts {
            progress: Some(Progress {
                ticked: 1,
                total: 3,
                acceptance: (0, 1),
                other: (1, 2),
                other_name: "scope".into(),
                ..Default::default()
            }),
            boxes: vec![
                BoxSection {
                    heading: "Scope".into(),
                    boxes: vec![
                        line(
                            "A fold control on the MR row",
                            true,
                            Some(("test: boxes.rs › folds", None)),
                        ),
                        line(
                            "Folded by default; the state per MR survives refreshes and a restart",
                            false,
                            None,
                        ),
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
            ],
            reasons: vec![reason("1 acceptance box open", "agent")],
            issue: Some(346),
            ..facts("feat(breadboard): an MR's boxes fold out", None)
        };
        let gitlab = super::super::GitlabState::default();
        let ctx = Ctx {
            is_once,
            folds,
            gitlab: &gitlab,
            now: 60_000,
            main_name: "platform",
            actor: super::super::actor::idle(),
            words: super::super::actor::view::no_words(),
        };
        let mrs = MergeRequests {
            open: vec![MrRow {
                number: Some(346),
                graph: "●".into(),
                below: String::new(),
                mr: Some(mr),
            }],
            ..Default::default()
        };
        let mut scene = Scene::<Act>::new(width);
        rows(&mut scene, &mrs, &ctx);
        scene.finish();
        scene
    }

    #[test]
    fn the_boxes_fold_under_the_bar_and_open_every_box_whole() {
        let folded = drawn_boxes(Default::default(), false, 60);
        let lines = super::tests::lines(&folded);
        assert!(lines[1].trim_start().starts_with("▸ ━"), "{lines:?}");
        // The counts name the issue they are of: the MR's description has boxes of its own.
        assert!(lines[1].ends_with("  #346: 1/3 boxes"), "{lines:?}");
        assert!(!lines.iter().any(|line| line.contains("☐")), "{lines:?}");
        // The bar's line is its own block in the row's, its click the fold.
        let fold = Act::Fold(Fold::MrBoxes(311));
        let region = folded
            .regions
            .iter()
            .find(|region| *region.action() == fold && !region.is_button())
            .expect("the fold's block");
        assert_eq!((region.area.y, region.area.height), (1, 1));

        let mut folds = super::super::Folds::default();
        folds.boxes.mrs.insert(311);
        let open = drawn_boxes(folds, false, 60);
        let lines = super::tests::lines(&open);
        assert!(lines[1].trim_start().starts_with("▾ ━"), "{lines:?}");
        assert_eq!(
            lines[2..10],
            [
                "                     Scope",
                "                     ☑ A fold control on the MR row ·",
                "                       test: boxes.rs › folds",
                "                     ☐ Folded by default; the state per MR",
                "                       survives refreshes and a restart",
                "                     Acceptance criteria",
                "                     ☐ A click shows every box ·",
                "                       try #12 looks good",
            ],
            "{lines:?}"
        );
        // The list is the fold's block: a click on it folds it again, the reasons stay under it.
        let region = open
            .regions
            .iter()
            .find(|region| *region.action() == fold)
            .expect("the fold's block");
        assert_eq!((region.area.y, region.area.height), (1, 9), "{lines:?}");
        assert!(lines[10].contains("1 acceptance box open"), "{lines:?}");

        // `--once` draws the bar alone: nothing to click.
        let once = drawn_boxes(Default::default(), true, 60);
        assert!(super::tests::lines(&once)[1].trim_start().starts_with('━'));
        assert!(!once.regions.iter().any(|region| *region.action() == fold));
    }

    #[test]
    fn the_iid_is_the_mrs_link_open_and_merged_wide_and_narrow() {
        // §11.3: `!iid` linked on every row, so a Ctrl-click opens the MR.
        let linked = links::style(Style::default(), Some("https://gitlab/mr/311")).underline_color;
        assert!(linked.is_some());
        for is_merged in [false, true] {
            for width in [100, 24] {
                let mr = MrFacts {
                    is_merged,
                    ..facts("feat(x): one two three four five six seven", None)
                };
                let mrs = MergeRequests {
                    open: vec![MrRow {
                        number: Some(311),
                        graph: "●".into(),
                        below: String::new(),
                        mr: Some(mr.clone()),
                    }],
                    ..Default::default()
                };
                let mut scene = Scene::<Act>::new(width);
                head(&mut scene, &mr, &Columns::of(&mrs), None);
                scene.finish();
                let iid = scene
                    .lines
                    .iter()
                    .flat_map(|line| &line.spans)
                    .find(|span| span.content == "!311")
                    .unwrap_or_else(|| panic!("no !311 at {width}: {:?}", lines(&scene)));
                assert_eq!(iid.style.underline_color, linked, "merged {is_merged}, width {width}");
            }
        }
    }

    #[test]
    fn a_sent_resolve_shows_its_mark_once() {
        let mr = MrFacts {
            conflict: Some(Conflict::Sent {
                text: "◐ sent to its agent".into(),
                at: 60_000,
            }),
            reasons: vec![reason("pipeline pending", "GitLab")],
            ..facts("fix(x): y", None)
        };
        let lines = drawn(mr, 100);
        let all = lines.join("\n").replace('\u{a0}', " ");
        assert_eq!(all.matches('◐').count(), 1, "{lines:?}");
        assert!(
            all.contains("◐ sent to its agent just now · pipeline pending → GitLab"),
            "{lines:?}"
        );
    }

    #[test]
    fn each_row_says_gitlab_checks_its_conflicts() {
        // Node's cell on every checked row (render-orchestration.mjs): the
        // row says where it stands even with no other reason (#296 bar).
        let mr = MrFacts {
            conflict: Some(Conflict::Checking),
            ..facts("fix(x): y", None)
        };
        let lines = drawn(mr, 80);
        assert!(
            lines.iter().any(|line| line.contains("checking for conflicts …")),
            "{lines:?}"
        );
    }

    #[test]
    fn no_conflicts_goes_unsaid() {
        let mr = MrFacts {
            conflict: Some(Conflict::None),
            reasons: vec![reason("pipeline pending", "GitLab")],
            ..facts("fix(x): y", None)
        };
        let lines = drawn(mr, 80);
        let all = lines.join("\n").replace('\u{a0}', " ");
        assert!(!all.contains("conflicts"), "{lines:?}");
        assert!(
            all.lines()
                .nth(1)
                .unwrap()
                .trim_start()
                .starts_with("pipeline pending → GitLab"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_ready_mr_with_nothing_lacking_ends_at_its_bar_with_one_empty_row() {
        // GitLab settled on `no merge conflicts`, no reasons: no reasons line
        // (it would be empty), the parting row alone before the next `●`.
        let row = |iid: i64, conflict: Conflict, reasons: Vec<Reason>| MrRow {
            number: Some(iid),
            graph: "●".into(),
            below: String::new(),
            mr: Some(MrFacts {
                iid,
                conflict: Some(conflict),
                reasons,
                progress: Some(Progress {
                    ticked: 9,
                    total: 9,
                    work: (7, 7),
                    acceptance: (2, 2),
                    ..Default::default()
                }),
                ..facts("feat(breadboard): an open MR shows a progress bar", None)
            }),
        };
        let gitlab = super::super::GitlabState::default();
        let ctx = Ctx {
            is_once: false,
            folds: Default::default(),
            gitlab: &gitlab,
            now: 60_000,
            main_name: "platform",
            actor: super::super::actor::idle(),
            words: super::super::actor::view::no_words(),
        };
        for width in [80, 120] {
            let mrs = MergeRequests {
                open: vec![
                    row(316, Conflict::None, vec![]),
                    row(311, Conflict::None, vec![reason("draft: mark ready", "you")]),
                    row(318, Conflict::None, vec![]),
                ],
                ..Default::default()
            };
            let mut scene = Scene::<Act>::new(width);
            rows(&mut scene, &mrs, &ctx);
            scene.finish();
            let lines = lines(&scene);
            let nodes: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].starts_with('●')).collect();
            assert_eq!(nodes.len(), 3, "{lines:?}");
            // !316: title, bar, one empty row, then !311's `●`.
            assert!(lines[nodes[1] - 2].contains("9/9 boxes"), "{width}: {lines:?}");
            assert_eq!(lines[nodes[1] - 1].trim(), "", "{width}: {lines:?}");
            // !311 keeps its reasons line, then the empty row.
            assert!(lines[nodes[2] - 2].contains("→\u{a0}you"), "{width}: {lines:?}");
            assert_eq!(lines[nodes[2] - 1].trim(), "", "{width}: {lines:?}");
            // The last block ends at its bar.
            assert!(lines.last().unwrap().contains("9/9 boxes"), "{width}: {lines:?}");
        }
    }

    #[test]
    fn what_an_mr_lacks_never_stands_on_the_next_mrs_node() {
        // The breadboard world: every MR in one lane, each a block of title, bar
        // and reasons. The line right above an `●` is never a reasons line.
        let row = |iid: i64, who: &str| MrRow {
            number: Some(iid),
            graph: "●".into(),
            below: String::new(),
            mr: Some(MrFacts {
                iid,
                conflict: Some(Conflict::Checking),
                reasons: vec![reason("draft: mark ready", who)],
                progress: Some(Progress {
                    ticked: 8,
                    total: 8,
                    work: (6, 6),
                    acceptance: (2, 2),
                    ..Default::default()
                }),
                ..facts("feat(breadboard): an open MR shows a progress bar", None)
            }),
        };
        let graph_alone = MrRow {
            number: None,
            graph: "├─┘".into(),
            below: String::new(),
            mr: None,
        };
        let gitlab = super::super::GitlabState::default();
        let ctx = Ctx {
            is_once: false,
            folds: Default::default(),
            gitlab: &gitlab,
            now: 60_000,
            main_name: "platform",
            actor: super::super::actor::idle(),
            words: super::super::actor::view::no_words(),
        };
        for width in [80, 120] {
            let mrs = MergeRequests {
                open: vec![
                    row(308, "you"),
                    row(314, "agent"),
                    graph_alone.clone(),
                    row(318, "agent"),
                ],
                ..Default::default()
            };
            let mut scene = Scene::<Act>::new(width);
            rows(&mut scene, &mrs, &ctx);
            scene.finish();
            let lines = lines(&scene);
            let nodes: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].starts_with('●')).collect();
            assert_eq!(nodes.len(), 3, "{lines:?}");
            for &at in &nodes[1..] {
                let above = lines[at - 1].trim();
                assert!(
                    !above.contains('→') && !above.contains("checking"),
                    "{width}: {lines:?}"
                );
            }
            // Each reasons line belongs to the `●` above it: `→ you` is !308's.
            let you = lines.iter().position(|line| line.contains("→\u{a0}you")).unwrap();
            assert!(you > nodes[0] && you < nodes[1], "{lines:?}");
            // One empty row between two blocks; the graph's own row parts the others.
            assert_eq!(lines[nodes[1] - 1].trim(), "", "{lines:?}");
            assert_eq!(lines[nodes[2] - 1].trim(), "├─┘", "{lines:?}");
            // The empty row is the block's above: it lights and clicks with it (#296).
            let blocks: Vec<Rect> = scene
                .regions
                .iter()
                .filter(|region| matches!(region.target, Target::Block(_)))
                .map(|region| region.area)
                .collect();
            assert_eq!(blocks[0].y as usize, nodes[0], "{blocks:?}");
            assert_eq!((blocks[0].y + blocks[0].height) as usize, nodes[1], "{blocks:?}");
            assert_eq!((blocks[1].y + blocks[1].height) as usize, nodes[2] - 1, "{blocks:?}");
        }
    }

    #[test]
    fn each_lane_has_its_colour_a_join_the_colour_of_the_lane_it_joins() {
        let style_at = |spans: &[Span<'static>], at: usize| {
            let mut x = 0;
            for span in spans {
                let n = span.content.chars().count();
                if at < x + n {
                    return span.style.fg;
                }
                x += n;
            }
            None
        };
        let join = lanes("├─┘", 5);
        assert_eq!(text(&join), "├─┘  ");
        assert_eq!(style_at(&join, 0), Some(LANES[0]));
        assert_eq!(style_at(&join, 1), Some(LANES[1]));
        assert_eq!(style_at(&join, 2), Some(LANES[1]));
        // The MR's node: its own span, green once it is ready.
        let ready = breadboard_core::model::Span::new(READY, breadboard_core::model::Paint::Green);
        let spans = node("│ ●", &facts("x", Some(ready)), 5);
        assert_eq!(style_at(&spans, 0), Some(LANES[0]));
        assert_eq!(style_at(&spans, 2), Some(Tone::Green.fg()));
        assert_eq!(style_at(&node("│ ●", &facts("x", None), 5), 2), Some(LANES[1]));
    }

    #[test]
    fn the_last_merged_comes_first_the_closed_after() {
        let done = |iid: i64, status: &str| MrRow {
            number: Some(iid),
            graph: String::new(),
            below: String::new(),
            mr: Some(MrFacts {
                iid,
                is_merged: true,
                ..facts("x", Some(breadboard_core::model::Span::dim(status)))
            }),
        };
        let mrs = MergeRequests {
            merged: vec![done(1, "merged 3 h ago"), done(2, "merged 1 h ago"), done(3, "merged")],
            ..Default::default()
        };
        assert_eq!(newest_first(&mrs).map(|mr| mr.iid).collect::<Vec<_>>(), [2, 1, 3]);
    }

    #[test]
    fn the_fold_counts_the_closed_apart() {
        let done = |iid: i64, status: &str| MrRow {
            number: Some(iid),
            graph: String::new(),
            below: String::new(),
            mr: Some(MrFacts {
                iid,
                is_merged: true,
                ..facts("x", Some(breadboard_core::model::Span::dim(status)))
            }),
        };
        let mut mrs = MergeRequests {
            merged: vec![done(1, "merged 3 h ago"), done(2, "closed")],
            ..Default::default()
        };
        assert_eq!(done_counts(&mrs), "1 merged · 1 closed");
        mrs.merged.remove(1);
        assert_eq!(done_counts(&mrs), "1 merged");
        // A closed row says so; a merged one its age alone.
        let closed = done(2, "closed");
        assert_eq!(text(&status_of(closed.mr.as_ref().unwrap(), None)), "closed");
        assert_eq!(text(&status_of(mrs.merged[0].mr.as_ref().unwrap(), None)), "3 h ago");
    }

    #[test]
    fn what_waits_for_you_stands_out() {
        let parts = reasons_of(
            &[reason("try #2 broken", "you"), reason("pipeline pending", "GitLab")],
            80,
        );
        let who = |part: &Vec<Span<'static>>| part.last().unwrap().style;
        assert!(who(&parts[0]).add_modifier.contains(ratatui::style::Modifier::BOLD));
        assert!(who(&parts[1]).add_modifier.contains(ratatui::style::Modifier::DIM));
    }

    #[test]
    fn a_title_says_its_type_quieter() {
        let spans = title_spans("feat(breadboard): a bar", false);
        assert_eq!(text(&spans), "feat(breadboard): a bar");
        assert_eq!(spans[0].content, "feat(breadboard):");
        assert_eq!(
            text(&title_spans("Just words: here and there", false)),
            "Just words: here and there"
        );
    }

    fn facts(title: &str, status: Option<breadboard_core::model::Span>) -> MrFacts {
        MrFacts {
            iid: 311,
            url: "https://gitlab/mr/311".into(),
            pipeline: breadboard_core::model::Span::new("✓ passed", breadboard_core::model::Paint::Green),
            title: title.into(),
            branch: "feature/302-x".into(),
            dir: None,
            is_merged: false,
            status,
            conflict: None,
            reasons: vec![],
            progress: None,
            ci: None,
            boxes: vec![],
            issue: None,
        }
    }

    fn lines(scene: &Scene<Act>) -> Vec<String> {
        scene.lines.iter().map(|line| text(&line.spans)).collect()
    }

    #[test]
    fn a_ready_mr_says_so_beside_the_title_and_offers_no_merge() {
        let ready = breadboard_core::model::Span::new(READY, breadboard_core::model::Paint::Green);
        let mrs = MergeRequests {
            open: vec![MrRow {
                number: Some(302),
                graph: "●".into(),
                below: String::new(),
                mr: Some(facts("feat(x): one two three four five six seven", Some(ready))),
            }],
            ..Default::default()
        };
        let columns = Columns::of(&mrs);
        for width in [70, 50] {
            let mut scene = Scene::<Act>::new(width);
            head(&mut scene, mrs.open[0].mr.as_ref().unwrap(), &columns, None);
            scene.finish();
            let lines = lines(&scene);
            assert!(lines[0].starts_with("!311  ✓ passed  feat(x): one two"), "{lines:?}");
            assert!(lines.iter().any(|line| line.ends_with("  ready to merge")), "{lines:?}");
            assert!(
                lines
                    .iter()
                    .all(|line| Span::raw(line.as_str()).width() <= width as usize)
            );
            // The merge is the posted action's two clicks (rule 6): the row only opens the MR.
            assert!(scene.regions.iter().all(|region| !region.is_button()), "{width}");
        }
    }

    #[test]
    fn a_title_wraps_in_its_column_never_cut() {
        let long = "fix(breadboard): the orchestrator no longer asks decisions answered in the overview";
        let mut merged = facts(long, Some(breadboard_core::model::Span::dim("merged 25 h ago")));
        merged.is_merged = true;
        let mrs = MergeRequests {
            merged: vec![MrRow {
                number: Some(254),
                graph: String::new(),
                below: String::new(),
                mr: Some(merged),
            }],
            ..Default::default()
        };
        let columns = Columns::of(&mrs);
        let mut scene = Scene::<Act>::new(80);
        head(&mut scene, mrs.merged[0].mr.as_ref().unwrap(), &columns, None);
        scene.finish();
        let lines = lines(&scene);
        // Its pipeline as an open one's, its age dim at the right (§11.3), `merged` said by the fold.
        assert!(
            lines[0].starts_with("!311  ✓ passed  fix(breadboard): the orchestrator no"),
            "{lines:?}"
        );
        assert!(lines[0].ends_with("  25 h ago"), "{lines:?}");
        // The rest of the title runs under the status's column too: two lines, not three.
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines.len() > 1 && lines[1].starts_with(&" ".repeat(columns.title_at() as usize)));
        assert!(lines.iter().all(|line| Span::raw(line.as_str()).width() <= 80));
        let title: Vec<&str> = lines
            .iter()
            .map(|line| {
                line.trim_end()
                    .trim_end_matches("25 h ago")
                    .trim_start_matches("!311  ✓ passed")
            })
            .flat_map(str::split_whitespace)
            .collect();
        assert!(title.join(" ").contains(long), "{lines:?}");
        assert!(scene.regions.is_empty());
    }

    #[test]
    fn the_overviews_merge_shows_on_its_row_and_moves_nothing() {
        let ready = breadboard_core::model::Span::new(READY, breadboard_core::model::Paint::Green);
        let mrs = MergeRequests {
            open: vec![MrRow {
                number: Some(302),
                graph: "●".into(),
                below: String::new(),
                mr: Some(facts("feat(x): one two three four five six seven", Some(ready))),
            }],
            ..Default::default()
        };
        let columns = Columns::of(&mrs);
        let mr = mrs.open[0].mr.as_ref().unwrap();
        let draw = |phase: Option<Phase>| {
            let merge = phase.map(|phase| Merge {
                phase,
                error: None,
                seq: 0,
            });
            let mut scene = Scene::<Act>::new(70);
            head(&mut scene, mr, &columns, merge.as_ref());
            scene.finish();
            scene
        };
        let before = lines(&draw(None));
        // Armed on its action: the row is as before, it has no button to arm.
        assert_eq!(lines(&draw(Some(Phase::Armed))), before);
        for (phase, words) in [(Phase::Running, "merging …"), (Phase::Merged, "merged")] {
            let scene = draw(Some(phase));
            let lines = lines(&scene);
            assert!(lines[0].trim_end().ends_with(words), "{lines:?}");
            // The title wraps where it did: only the status changed.
            assert_eq!(lines.len(), before.len());
            assert_eq!(lines[1..], before[1..]);
            assert_eq!(lines[0][..40], before[0][..40]);
            assert!(scene.regions.iter().all(|region| !region.is_button()));
        }
    }

    #[test]
    fn the_pipeline_mark_opens_its_pipeline() {
        // A merged MR's too: GitLab merges on a red pipeline as well.
        for is_merged in [false, true] {
            let mut mr = facts(
                "fix(x): y",
                is_merged.then(|| breadboard_core::model::Span::dim("merged 2 h ago")),
            );
            mr.is_merged = is_merged;
            mr.pipeline.url = Some("https://gitlab/pipelines/9".into());
            let mrs = MergeRequests {
                open: vec![MrRow {
                    number: Some(302),
                    graph: String::new(),
                    below: String::new(),
                    mr: Some(mr),
                }],
                ..Default::default()
            };
            let columns = Columns::of(&mrs);
            for width in [70, 20] {
                let mut scene = Scene::<Act>::new(width);
                head(&mut scene, mrs.open[0].mr.as_ref().unwrap(), &columns, None);
                scene.finish();
                let found = scene
                    .regions
                    .iter()
                    .find(|region| region.is_button())
                    .expect("its mark");
                let line = &lines(&scene)[found.area.y as usize];
                let x = line.chars().take_while(|ch| *ch != '✓').count() as u16;
                assert_eq!((found.area.x, found.area.width), (x, 8), "{width}: {line}");
                assert!(
                    matches!(found.action(), Act::Go(Go::Show { url, .. }) if url.ends_with("/pipelines/9")),
                    "{width}"
                );
            }
        }
    }

    #[test]
    fn the_mark_of_an_mr_waiting_in_the_ci_queue_runs_it_now() {
        let mut mr = facts("fix(x): y", None);
        mr.pipeline = breadboard_core::model::Span::new("CI: next (2nd) ▸", breadboard_core::model::Paint::Yellow);
        mr.pipeline.url = Some("https://gitlab/pipelines/9".into());
        mr.ci = Some(breadboard_core::ci::Place::Next { at: 2 });
        let columns = Columns::of(&MergeRequests {
            open: vec![MrRow {
                number: Some(302),
                graph: String::new(),
                below: String::new(),
                mr: Some(mr.clone()),
            }],
            ..Default::default()
        });
        let mut scene = Scene::<Act>::new(90);
        head(&mut scene, &mr, &columns, None);
        scene.finish();
        let found = scene
            .regions
            .iter()
            .find(|region| region.is_button())
            .expect("its mark");
        assert!(matches!(found.action(), Act::RunNow(311)));
        assert!(lines(&scene)[0].contains("CI: next (2nd) ▸"), "{:?}", lines(&scene));
    }
}
