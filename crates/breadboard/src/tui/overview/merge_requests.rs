//! MERGE REQUESTS of an orchestration (render-orchestration.mjs,
//! mr-blockers.mjs, resolve.mjs): the open MRs as a graph in the merge
//! order, each a block of its head (`!iid`, pipeline, title, status), the bar
//! of its boxes (#294) and what it still lacks with who acts on it; the
//! merged below a fold. Titles wrap in their column, reasons under the
//! title's, nothing is cut (#304). Hover lights a whole row (#296); a click
//! opens the MR, its buttons resolve conflicts or merge on two clicks.

use super::actor::merge::{Merge, Phase};
use super::{Act, Ctx, Fold, look};
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
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// The fewest columns a title gets beside its status (mr-blockers.mjs `MIN_TITLE`).
const MIN_TITLE: u16 = 24;
/// The fewest columns the bar keeps before its counts go (#294 `MIN_BAR`).
const MIN_BAR: u16 = 8;
/// The status of an open MR nothing holds up, and its button.
const READY: &str = "ready to merge";
const MERGE: &str = "merge";
/// The button once a first click armed it: as wide as `ready to merge` and its
/// button, so nothing on the row moves (merge.mjs `mergeText`'s armed words).
const ARMED: &str = "click again to merge";
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
    for row in &mrs.open {
        open_row(scene, row, &columns, ctx);
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
    let fold = format!("{mark} {} merged{hint}", mrs.merged.len());
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
    for row in &mrs.merged {
        let Some(mr) = &row.mr else { continue };
        let lanes = vec![Span::raw(" ".repeat(columns.graph as usize))];
        scene.block(Act::Go(show(mr)), |scene| {
            look::gutter(scene, lanes.clone(), lanes, |scene| head(scene, mr, &columns, None));
        });
    }
}

/// A click on an MR's row: its page, in the tab group of its worktree.
fn show(mr: &MrFacts) -> Go {
    Go::Show {
        url: mr.url.clone(),
        dir: mr.dir.clone(),
    }
}

/// An open MR: its lane, head, bar and what it lacks below.
fn open_row(scene: &mut Scene<Act>, row: &MrRow, columns: &Columns, ctx: &Ctx) {
    let width = columns.graph as usize;
    let Some(mr) = &row.mr else {
        // A line of the graph alone: no MR, no click.
        scene.end_line();
        scene.atom(vec![look::dim(format!("{:<width$}", row.graph))]);
        scene.end_line();
        return;
    };
    // The MR's own node in the colour of where it stands, its lanes dim.
    let first = node(&row.graph, mr, width);
    let rest = vec![look::dim(format!("{:<width$}", row.below))];
    scene.block(Act::Go(show(mr)), |scene| {
        look::gutter(scene, first, rest, |scene| {
            head(scene, mr, columns, merge_of(ctx, mr));
            let at = columns.title_at();
            if let Some(progress) = mr.progress {
                let beside = if columns.status > 0 { columns.status + 2 } else { 0 };
                let room = scene.width().saturating_sub(at);
                let size = (columns.title_in(scene.width()) + beside).min(room);
                if room >= MIN_BAR {
                    scene.pad_to(at);
                    scene.atom_gap(bar(progress, size), 0);
                    scene.end_line();
                }
            }
            if mr.conflict.is_some() || !mr.reasons.is_empty() || failed_of(ctx, mr).is_some() {
                let at = if scene.width() >= at + MIN_COLUMNS { at } else { 2 };
                scene.indent(at);
                scene.pad_to(at);
                lacks(scene, mr, ctx);
                scene.indent(0);
                scene.end_line();
            }
        });
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
    let padded = format!("{graph:<width$}");
    let Some(at) = padded.find('●') else {
        return vec![look::dim(padded)];
    };
    let (before, after) = padded.split_at(at);
    let after = &after['●'.len_utf8()..];
    let style = tone.map_or_else(theme::dim, |tone| Style::new().fg(tone.fg()));
    vec![look::dim(before), Span::styled("●", style), look::dim(after)]
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

/// What an MR's status says, at the right of its head: `merged 2 h ago`, or
/// `ready to merge` and the button that merges on two clicks; once the first
/// click armed it, the button says so in red, then `merging …`, `merged`.
fn status_of(mr: &MrFacts, merge: Option<&Merge>) -> Vec<Span<'static>> {
    let Some(status) = mr.status.as_ref() else {
        return vec![];
    };
    let mut spans = look::spans(std::slice::from_ref(status));
    if !is_ready(mr) {
        return spans;
    }
    match merge.map(|merge| merge.phase) {
        Some(Phase::Armed) => return vec![Span::styled(format!(" {ARMED} "), Tone::Red.button())],
        Some(Phase::Running) => return vec![look::span("merging …", theme::tone(Tone::Yellow))],
        Some(Phase::Merged) => return vec![look::span("merged", theme::tone(Tone::Green))],
        Some(Phase::Failed) | None => {}
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(format!(" {MERGE} "), Tone::Green.button()));
    spans
}

/// `!iid  mark  title  status`: the title wraps in its column, the status at
/// the right of its first line; a narrow row is one flow of words. The
/// layout is the one of the status before any click: arming, merging never
/// move the row.
fn head(scene: &mut Scene<Act>, mr: &MrFacts, columns: &Columns, merge: Option<&Merge>) {
    let iid = if mr.is_merged {
        look::dim(format!("!{}", mr.iid))
    } else {
        look::bold(format!("!{}", mr.iid))
    };
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
            let x = scene.x();
            let rects = scene.atom(status.clone());
            merge_region(
                scene,
                mr,
                merge,
                &status,
                rects.first().map(|rect| rect.x).unwrap_or(x),
                scene.y(),
            );
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
    look::flow(&mut left, &title);
    left.finish();
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
            merge_region(scene, mr, merge, &status, x, top);
        }
        scene.lines.push(Line::from(spans));
    }
    if !status.is_empty() && !is_beside {
        // Too narrow to stand beside the title: its own line, at the right.
        let x = width.saturating_sub(status_width);
        let mut spans = Vec::new();
        pad(&mut spans, x);
        spans.extend(status.iter().cloned());
        merge_region(scene, mr, merge, &status, x, scene.y());
        scene.lines.push(Line::from(spans));
    }
}

/// An MR's pipeline mark opens its pipeline (§3: `!iid` and the mark are
/// links), a merged one's too: a failed one's page is a click away from its
/// row; the rest of the row opens the MR.
fn pipeline_region(scene: &mut Scene<Act>, mr: &MrFacts, area: Rect) {
    let Some(url) = mr.pipeline.url.as_ref() else {
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
            action: Act::Go(Go::Show {
                url: url.clone(),
                dir: mr.dir.clone(),
            }),
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

/// The region of the merge button in a status drawn at `x`, line `y`: two
/// clicks merge (merge.mjs `clickMerge`), the actor arms and merges; none
/// while it merges.
fn merge_region(scene: &mut Scene<Act>, mr: &MrFacts, merge: Option<&Merge>, status: &[Span<'static>], x: u16, y: u16) {
    if !is_ready(mr) {
        return;
    }
    let (label, tone) = match merge.map(|merge| merge.phase) {
        Some(Phase::Armed) => (ARMED, Tone::Red),
        Some(Phase::Running | Phase::Merged) => return,
        Some(Phase::Failed) | None => (MERGE, Tone::Green),
    };
    let width = Span::raw(format!(" {label} ")).width() as u16;
    let at = x + width_of(status) - width;
    scene.regions.push(Region {
        area: Rect::new(at, y, width, 1),
        target: Target::Button {
            label: label.into(),
            tone,
            action: Act::Go(Go::Merge {
                branch: mr.branch.clone(),
                action: None,
            }),
        },
    });
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
/// section on a narrow width, never `…`.
fn bar(progress: Progress, width: u16) -> Vec<Span<'static>> {
    let Progress {
        ticked,
        total,
        work,
        acceptance,
    } = progress;
    let mut full = vec![format!("{ticked}/{total} boxes")];
    for (name, (done, all)) in [("work", work), ("acceptance", acceptance)] {
        if all > 0 {
            full.push(format!("{name} {done}/{all}"));
        }
    }
    let candidates = [
        full.join(" · "),
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

/// Where its conflicts stand, then what it lacks, each `→ who` acts on it.
fn lacks(scene: &mut Scene<Act>, mr: &MrFacts, ctx: &Ctx) {
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
        Some(Conflict::Checking) => parts.push(vec![look::dim("checking for conflicts…")]),
        Some(Conflict::None) => parts.push(vec![look::dim("no merge conflicts")]),
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
            parts.push(vec![
                look::span(format!("◐ {text}"), theme::tone(Tone::Yellow)),
                look::dim(format!(" {}", ago(*at, ctx.now))),
            ]);
            look::ticks(scene);
        }
    }
    parts.extend(reasons_of(&mr.reasons));
    look::parted(scene, &parts);
}

/// The reasons as parts, `text → who`; the MRs it waits for said once,
/// `waits for !311, !303` (the `→ !311` would only name them again).
fn reasons_of(reasons: &[Reason]) -> Vec<Vec<Span<'static>>> {
    let glue = |text: &str| text.replace(' ', "\u{a0}");
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
                parts.push(vec![look::span(
                    format!("waits for {}", waits.join(", ")),
                    theme::tone(Tone::Yellow),
                )]);
            }
            continue;
        }
        parts.push(vec![
            look::span(&reason.text, theme::tone(Tone::Yellow)),
            look::dim(format!("\u{a0}→\u{a0}{}", glue(&reason.who))),
        ]);
    }
    parts
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
        };
        let wide = text(&bar(progress, 50));
        assert!(wide.ends_with("  9/12 boxes · work 9/9 · acceptance 0/3"), "{wide}");
        assert_eq!(Span::raw(wide.as_str()).width(), 50);
        // Wider: the bar takes the room it is given, the counts at its end (#294).
        let wide = text(&bar(progress, 80));
        assert_eq!(Span::raw(wide.as_str()).width(), 80);
        assert!(wide.starts_with(&"━".repeat(30)), "{wide}");
        assert!(text(&bar(progress, 20)).ends_with("  9/12 boxes"));
        assert!(text(&bar(progress, 15)).ends_with("  9/12"));
        assert_eq!(text(&bar(progress, 8)), "━━━━━━──");
    }

    #[test]
    fn the_bar_is_yellow_once_only_acceptance_boxes_are_open() {
        let only_acceptance = Progress {
            ticked: 2,
            total: 4,
            work: (2, 2),
            acceptance: (0, 2),
        };
        assert_eq!(bar(only_acceptance, 8)[0].style.fg, Some(Tone::Yellow.fg()));
        let work_open = Progress {
            work: (1, 2),
            acceptance: (1, 2),
            ..only_acceptance
        };
        assert_eq!(bar(work_open, 8)[0].style.fg, Some(Tone::Green.fg()));
    }

    #[test]
    fn the_mrs_it_waits_for_are_named_once() {
        let parts = reasons_of(&[
            reason("pipeline pending", "GitLab"),
            reason("waits for !311", "!311"),
            reason("after !303 (shares a.rs)", "!303"),
            reason("waits for !304", "!304"),
        ]);
        let parts: Vec<String> = parts.iter().map(|part| text(part)).collect();
        assert_eq!(
            parts,
            [
                "pipeline pending\u{a0}→\u{a0}GitLab",
                "waits for !311, !304",
                "after !303 (shares a.rs)\u{a0}→\u{a0}!303",
            ]
        );
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
        }
    }

    fn lines(scene: &Scene<Act>) -> Vec<String> {
        scene.lines.iter().map(|line| text(&line.spans)).collect()
    }

    #[test]
    fn a_ready_mr_offers_its_merge_beside_the_title() {
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
        let mut scene = Scene::<Act>::new(70);
        head(&mut scene, mrs.open[0].mr.as_ref().unwrap(), &columns, None);
        scene.finish();
        let lines = lines(&scene);
        assert!(lines[0].starts_with("!311  ✓ passed  feat(x): one two"), "{lines:?}");
        assert!(lines[0].ends_with("ready to merge  merge "), "{lines:?}");
        assert!(lines[1].trim_end().ends_with("six seven"), "{lines:?}");
        let button = scene
            .regions
            .iter()
            .find(|region| region.is_button())
            .expect("the merge button");
        assert_eq!(button.area, Rect::new(63, 0, 7, 1));
        // Too narrow beside the title: the status on a line of its own, the button with it.
        let mut scene = Scene::<Act>::new(60);
        head(&mut scene, mrs.open[0].mr.as_ref().unwrap(), &columns, None);
        scene.finish();
        let lines = self::lines(&scene);
        assert!(
            lines.iter().all(|line| Span::raw(line.as_str()).width() <= 60),
            "{lines:?}"
        );
        assert!(lines.last().unwrap().ends_with("ready to merge  merge "), "{lines:?}");
        let button = scene
            .regions
            .iter()
            .find(|region| region.is_button())
            .expect("the merge button");
        assert_eq!(button.area, Rect::new(53, lines.len() as u16 - 1, 7, 1));
        assert!(matches!(button.action(), Act::Go(Go::Merge { branch, .. }) if branch == "feature/302-x"));
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
        // Its pipeline as an open one's, `merged <ago>` dim at the right (§11.3).
        assert!(
            lines[0].starts_with("!311  ✓ passed  fix(breadboard): the orchestrator"),
            "{lines:?}"
        );
        assert!(lines[0].ends_with("  merged 25 h ago"), "{lines:?}");
        assert!(lines.len() > 1 && lines[1].starts_with(&" ".repeat(columns.title_at() as usize)));
        assert!(lines.iter().all(|line| Span::raw(line.as_str()).width() <= 80));
        let title: Vec<&str> = lines
            .iter()
            .map(|line| {
                line.trim_end()
                    .trim_end_matches("merged 25 h ago")
                    .trim_start_matches("!311  ✓ passed")
            })
            .flat_map(str::split_whitespace)
            .collect();
        assert!(title.join(" ").contains(long), "{lines:?}");
        assert!(scene.regions.is_empty());
    }

    #[test]
    fn an_armed_merge_says_so_on_its_button_and_moves_nothing() {
        assert_eq!(
            Span::raw(format!("{READY}  {MERGE} ")).width(),
            Span::raw(format!(" {ARMED} ")).width()
        );
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
        let (before, armed, running) = (draw(None), draw(Some(Phase::Armed)), draw(Some(Phase::Running)));
        let armed_lines = lines(&armed);
        assert!(armed_lines[0].ends_with(" click again to merge "), "{armed_lines:?}");
        // The title wraps where it did: only the status changed.
        for scene in [&armed, &running] {
            let lines = lines(scene);
            assert_eq!(lines.len(), self::lines(&before).len());
            assert_eq!(lines[1..], self::lines(&before)[1..]);
            assert_eq!(lines[0][..40], self::lines(&before)[0][..40]);
        }
        let button = armed.regions.iter().find(|region| region.is_button()).expect("armed");
        assert_eq!(button.area, Rect::new(48, 0, 22, 1));
        assert!(matches!(&button.target, Target::Button { label, tone: Tone::Red, .. } if label == ARMED));
        assert!(running.regions.iter().all(|region| !region.is_button()));
        assert!(lines(&running)[0].trim_end().ends_with("merging …"));
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
}
