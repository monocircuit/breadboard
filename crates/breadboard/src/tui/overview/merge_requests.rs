//! The MR's pieces of a block in WORK (`work`, #347; render-orchestration.mjs,
//! mr-blockers.mjs, resolve.mjs before it): its lane in the graph of the
//! merge order, its pipeline mark (a click opens the pipeline), its status,
//! the bar of its boxes (#294) and what it still lacks with who acts on it,
//! its one button resolving conflicts (§11.3); the fold's counts of the
//! merged. The merge on two clicks is the posted merge action's (ACTIONS,
//! rule 6); a block only says when the overview merges it.

use super::actor::merge::{Merge, Phase};
use super::{Act, Ctx, look};
use crate::tui::Scene;
use crate::tui::scene::{Region, Target, width_of};
use crate::tui::theme::{self, Tone};
use breadboard_core::model::Paint;
use breadboard_core::model::mr_blockers::{Progress, Reason};
use breadboard_core::model::orchestration::{MergeRequests, MrFacts};
use breadboard_core::model::order::Go;
use breadboard_core::model::resolve::Conflict;
use breadboard_core::time::ago;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// The fewest columns the bar keeps before its counts go (#294 `MIN_BAR`).
const MIN_BAR: u16 = 8;
/// The status of an open MR nothing holds up.
const READY: &str = "ready to merge";
/// Core's status of an MR closed without a merge.
const CLOSED: &str = "closed";

/// The fold's words: `42 merged`, `41 merged · 1 closed` (closed without a
/// merge, core's `closed`): a closed MR is never counted as merged.
pub(super) fn done_counts(mrs: &MergeRequests) -> String {
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
pub(super) fn is_closed(mr: &MrFacts) -> bool {
    mr.status.as_ref().is_some_and(|status| status.text == CLOSED)
}

/// The merged, the last merged first (lazygit's and git log's order): what
/// just went in stands right under the fold. Those without a merge time
/// (the closed, a merge GitLab gave no time for) after.
pub(super) fn newest_first(mrs: &MergeRequests) -> impl Iterator<Item = &MrFacts> {
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
pub(super) fn show(mr: &MrFacts) -> Go {
    Go::Show {
        url: mr.url.clone(),
        dir: mr.dir.clone(),
    }
}

/// The lanes of an MR's first line, its `●` green when it is ready, red
/// when it is broken (its pipeline failed, conflicts its agent must resolve).
pub(super) fn node(graph: &str, mr: &MrFacts, width: usize) -> Vec<Span<'static>> {
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
pub(super) fn lanes(graph: &str, width: usize) -> Vec<Span<'static>> {
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
pub(super) fn mark_of(mr: &MrFacts) -> Vec<Span<'static>> {
    look::spans(std::slice::from_ref(&mr.pipeline))
}

/// The overview's merge of an MR: armed, running, merged or failed.
pub(super) fn merge_of<'a>(ctx: &'a Ctx, mr: &MrFacts) -> Option<&'a Merge> {
    ctx.actor.merges.of.get(&mr.iid)
}

/// Why the overview's last merge of an MR failed.
pub(super) fn failed_of<'a>(ctx: &'a Ctx, mr: &MrFacts) -> Option<&'a str> {
    merge_of(ctx, mr)
        .filter(|merge| merge.phase == Phase::Failed)
        .and_then(|merge| merge.error.as_deref())
}

/// What an MR's status says, at the right of its head: `2 h ago` under the
/// merged fold, or `ready to merge`; while the overview merges it (two clicks
/// on its merge action) `merging …`, then `merged`. Never a button: the row's
/// click opens the MR, as Node's (§11.3).
pub(super) fn status_of(mr: &MrFacts, merge: Option<&Merge>) -> Vec<Span<'static>> {
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

/// An MR's pipeline mark opens its pipeline (§3: `!iid` and the mark are
/// links), a merged one's too: a failed one's page is a click away from its
/// row; the rest of the row opens the MR.
pub(super) fn pipeline_region(scene: &mut Scene<Act>, mr: &MrFacts, area: Rect) {
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
pub(super) fn is_ready(mr: &MrFacts) -> bool {
    !mr.is_merged && mr.status.as_ref().is_some_and(|status| status.text == READY)
}

/// A title, its conventional-commit type and scope (`feat(breadboard): `)
/// quieter than what it says; a merged one all quiet.
pub(super) fn title_spans(title: &str, is_merged: bool) -> Vec<Span<'static>> {
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
pub(super) fn bar(progress: &Progress, width: u16) -> Vec<Span<'static>> {
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

/// Whether `lacks` draws anything: a ready MR with no conflicts (unsaid),
/// no reasons and no failed merge has no line, so its block ends at its bar.
pub(super) fn has_lacks(mr: &MrFacts, ctx: &Ctx) -> bool {
    !mr.reasons.is_empty()
        || failed_of(ctx, mr).is_some()
        || mr
            .conflict
            .as_ref()
            .is_some_and(|conflict| !matches!(conflict, Conflict::None))
}

/// Where its conflicts stand, then what it lacks, each `→ who` acts on it.
pub(super) fn lacks(scene: &mut Scene<Act>, mr: &MrFacts, ctx: &Ctx) {
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
    use breadboard_core::model::orchestration::MrRow;

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
        let wide = text(&bar(&progress, 50));
        assert!(wide.ends_with("  9/12 boxes · work 9/9 · acceptance 0/3"), "{wide}");
        assert_eq!(Span::raw(wide.as_str()).width(), 50);
        // Wider: the bar takes the room it is given, the counts at its end (#294).
        let wide = text(&bar(&progress, 80));
        assert_eq!(Span::raw(wide.as_str()).width(), 80);
        assert!(wide.starts_with(&"━".repeat(30)), "{wide}");
        assert!(text(&bar(&progress, 20)).ends_with("  9/12 boxes"));
        assert!(text(&bar(&progress, 15)).ends_with("  9/12"));
        assert_eq!(text(&bar(&progress, 8)), "━━━━━━──");
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
        let label = text(&bar(&progress, 80));
        assert!(label.ends_with("  0/9 boxes · acceptance 0/2 · scope 0/7"), "{label}");
        // Once they are ticked they go unnamed: the label accounts for the open boxes.
        let done = Progress {
            ticked: 7,
            other: (7, 7),
            ..progress
        };
        assert!(text(&bar(&done, 80)).ends_with("  7/9 boxes · acceptance 0/2"));
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
        assert_eq!(bar(&only_acceptance, 8)[0].style.fg, Some(Tone::Yellow.fg()));
        let work_open = Progress {
            work: (1, 2),
            acceptance: (1, 2),
            ..only_acceptance
        };
        assert_eq!(bar(&work_open, 8)[0].style.fg, Some(Tone::Green.fg()));
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
        }
    }
}
