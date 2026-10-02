//! How the view says what the actor does: each row where its act stands —
//! a merge armed or running, a try's terminal, a removal, the end, a
//! dismiss, the chat, a stack starting — in place of what the
//! model says; and the log under the band, what was done in the last
//! minutes, newest first, like lazygit's command log.

use super::spawn::Run;
use super::{Actor, auto, merge, tries};
use crate::tui::overview::{Act, Ctx, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Page, Scene, Section};
use breadboard_core::model::auto_actions::Removal;
use breadboard_core::model::orchestration::{NextBand, OrchestrationModel};
use breadboard_core::model::order::{Go, Item};
use breadboard_core::model::{Paint, Span, Tree};
use breadboard_core::record::ORCHESTRATOR;
use breadboard_core::time::hm;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// How long a thing done stays in the log under the band.
const SHOWN_MS: i64 = 60_000;

/// The actor's word on the rows of a model, by the item's key: what it says
/// in place of the model's text while its act is under way or done.
pub type Words = HashMap<String, Vec<Span>>;

/// The actor's words for the rows of a model: what it says in place of their text.
pub fn overlay(
    actor: &Actor,
    model: &OrchestrationModel,
    trees: &[Tree],
    removals: &HashMap<PathBuf, Removal>,
) -> Words {
    let mut words = Words::new();
    let mut each = |item: &Item| {
        if let Some(word) = word_of(actor, item, trees, removals, &model_name(item)) {
            words.insert(item.key.clone(), word);
        }
    };
    if let Ok(section) = &model.actions {
        section.actions.iter().chain(&section.up_next).for_each(&mut each);
    }
    if let Ok(next) = &model.next
        && let NextBand::Action(item) | NextBand::UpNext(item) = &next.band
    {
        each(item);
    }
    words
}

/// The orchestration of an `end:<name>` row.
fn model_name(item: &Item) -> String {
    item.key.strip_prefix("end:").unwrap_or_default().to_string()
}

/// What the actor says on a row instead of its text, None: the model's text.
fn word_of(
    actor: &Actor,
    item: &Item,
    trees: &[Tree],
    removals: &HashMap<PathBuf, Removal>,
    name: &str,
) -> Option<Vec<Span>> {
    if let Some(offer) = &item.merge {
        return merge::said(actor, offer, &item.text);
    }
    match &item.go {
        Go::Try { dir, .. } => {
            let action = item.action.as_ref()?;
            let tree = dir.as_ref().and_then(|dir| trees.iter().find(|tree| &tree.dir == dir));
            let spot = tries::spot(actor, action, tree)?;
            let mut word = vec![Span::plain(format!("{}  ", action.text))];
            word.extend(spot);
            Some(word)
        }
        Go::Remove(dir) => {
            let feature = item.who.clone();
            auto::removal_said(removals.get(dir)?, &feature)
        }
        Go::End => end_said(actor.end.as_ref()?, name),
        _ => None,
    }
}

/// The end's UP NEXT row while armed, running or failed (`endText`).
fn end_said(end: &Run, name: &str) -> Option<Vec<Span>> {
    if end.is_running {
        return Some(vec![Span::plain("ending … "), Span::dim(end.last_line())]);
    }
    if end.is_armed {
        return Some(vec![Span::new(
            format!("click again to end {ORCHESTRATOR} {name}"),
            Paint::Red,
        )]);
    }
    end.error.as_ref().map(|error| {
        vec![
            Span::plain("all merged, no worktree left: end the orchestration  "),
            Span::new(error.clone(), Paint::Red),
        ]
    })
}

/// No words: for a section drawn on its own (tests).
#[cfg(test)]
pub fn no_words() -> &'static Words {
    static NONE: std::sync::LazyLock<Words> = std::sync::LazyLock::new(Words::new);
    &NONE
}

/// The actor's word for a row, if it has one (the sections draw it in place of the text).
pub fn word<'a>(ctx: &'a Ctx, key: &str) -> Option<&'a [Span]> {
    ctx.words.get(key).map(Vec::as_slice)
}

/// Draws the actor's word for `key` in place of the row's text, its line
/// ended: true when it did.
pub fn said(scene: &mut Scene<Act>, ctx: &Ctx, key: &str) -> bool {
    match word(ctx, key) {
        Some(word) => {
            look::flow(scene, &look::spans(word));
            scene.end_line();
            true
        }
        None => false,
    }
}

/// The label of a free agent's dismiss: armed by a first click, it says so.
pub fn dismiss_label(ctx: &Ctx, id: &str, label: &str) -> String {
    match ctx.actor.dismissing.get(id) {
        Some(name) => format!("✕ click again to dismiss {name}"),
        None => label.to_string(),
    }
}

/// A button whose label and note say where its act stands (armed, running,
/// failed) when the actor has one under way; true when it drew its note,
/// then the section leaves out its own.
pub fn button(scene: &mut Scene<Act>, ctx: &Ctx, label: &str, tone: Tone, act: Act) -> bool {
    let actor = ctx.actor;
    let run = |scene: &mut Scene<Act>, run: &Run, running: &str, armed: &str, armed_note: &str| -> bool {
        if run.is_running {
            scene.button(running, Tone::Yellow, act.clone());
            look::say(scene, &run.last_line(), theme::dim());
            return true;
        }
        if run.is_armed {
            scene.button(armed, Tone::Red, act.clone());
            look::say(scene, armed_note, theme::dim());
            return true;
        }
        false
    };
    let error = |scene: &mut Scene<Act>, error: &str| {
        scene.button(label, tone, act.clone());
        look::say(scene, error, theme::tone(Tone::Red));
        true
    };
    match &act {
        Act::Go(Go::End) => {
            if let Some(end) = &actor.end {
                let name = match &actor.view {
                    breadboard_core::reader::View::Orchestration(name) => name.clone(),
                    _ => String::new(),
                };
                if run(
                    scene,
                    end,
                    "◐ ending …",
                    &format!("✕ click again to end {ORCHESTRATOR} {name}"),
                    "its space and the orchestrator's claude close, record and checkout go; the conversation stays in /resume",
                ) {
                    return true;
                }
                if let Some(why) = &end.error {
                    scene.button(label, tone, act.clone());
                    look::say(
                        scene,
                        "no worktree left: its space, record and checkout go",
                        theme::dim(),
                    );
                    look::say(scene, why, theme::tone(Tone::Red));
                    return true;
                }
            }
        }
        Act::Chat => {
            if let Some(said) = &actor.chat {
                scene.button(label, tone, act.clone());
                let style = if said.starts_with("the conversation") {
                    theme::tone(Tone::Red)
                } else {
                    theme::dim()
                };
                look::say(scene, said, style);
                return true;
            }
        }
        Act::StartStack(dir) => {
            if let Some(stack) = actor.stacks.get(dir) {
                if run(scene, stack, "◐ starting the stack …", label, "") {
                    return true;
                }
                if let Some(why) = &stack.error {
                    return error(scene, why);
                }
            }
        }
        _ => {}
    }
    scene.button(label, tone, act);
    false
}

/// The log in the band, right of the view's name: the newest thing done in
/// the last minute and how many more, one row, so the rows under the band
/// never move when it comes or goes (frame-scroll critic).
pub fn log(header: crate::tui::Header, ctx: &Ctx) -> crate::tui::Header {
    // `· N more` counts every other thing done in that minute: DONE lists them all.
    let mut shown = ctx.actor.did.iter().filter(|did| ctx.now - did.at < SHOWN_MS);
    let Some(newest) = shown.next() else { return header };
    let more: Vec<_> = shown.collect();
    let oldest = more.last().copied().unwrap_or(newest);
    let left = Duration::from_millis((SHOWN_MS - (ctx.now - oldest.at)).max(0) as u64 + 50);
    let (mark, tone) = match newest.paint {
        Paint::Green => ("✓", Some(Tone::Green)),
        Paint::Red => ("✗", Some(Tone::Red)),
        Paint::Yellow => ("◐", Some(Tone::Yellow)),
        _ => ("·", None),
    };
    let style = tone.map_or_else(theme::dim, theme::tone);
    // No time here, the room is short: DONE at the end has it.
    let entry = vec![
        look::span(format!("{mark} "), style),
        // A few words, so the band never cuts it: DONE at the end has it whole.
        look::span(short_of(&newest.text), ratatui::style::Style::new()),
    ];
    header.log(entry, more.len(), left)
}

/// A thing done in a few words: what comes before its `: `, a worktree
/// named by its issue (`#305`). Its `→ answer` and `of #305` are glued
/// (no-break spaces): the band, which drops the words that don't fit,
/// shows them whole or not at all, never `→ Rust +` (#304).
pub fn short_of(text: &str) -> String {
    let short = words_of(text);
    let (head, answer) = match short.split_once(" → ") {
        Some((head, answer)) => (head, Some(answer)),
        None => (short.as_str(), None),
    };
    let mut glued = head.replace(" of #", "\u{a0}of\u{a0}#");
    if let Some(answer) = answer {
        glued.push_str(&format!(" →\u{a0}{}", answer.replace(' ', "\u{a0}")));
    }
    glued
}

/// `short_of` before the glue.
fn words_of(text: &str) -> String {
    let text = text.split(" · ").next().unwrap_or(text);
    let mut parts = text.split(": ");
    let mut head = parts.next().unwrap_or(text).to_string();
    // `try #1` says too little: what happened to it too.
    if head.chars().count() < 12
        && let Some(next) = parts.next()
    {
        head = format!("{head}: {next}");
    }
    head.split(' ')
        .map(|word| {
            let digits = word.chars().take_while(char::is_ascii_digit).count();
            if digits > 0 && word[digits..].starts_with('-') && word.len() > digits + 1 {
                format!("#{}", &word[..digits])
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// DONE at the end of the page, lazygit's command log: everything this
/// overview did, newest first, each with its time and in full; at the end,
/// so it never moves a row above it. Nothing in a print.
pub fn done(page: &mut Page<Act>, ctx: &Ctx) {
    if ctx.actor.did.is_empty() {
        return;
    }
    page.section(Section::new("DONE"), |scene| {
        scene.note(format!(
            "{} · what this overview did, newest first",
            breadboard_core::model::counted(ctx.actor.did.len() as u64, "thing")
        ));
        for did in &ctx.actor.did {
            let (mark, tone) = match did.paint {
                Paint::Green => ("✓", Some(Tone::Green)),
                Paint::Red => ("✗", Some(Tone::Red)),
                Paint::Yellow => ("◐", Some(Tone::Yellow)),
                _ => ("·", None),
            };
            let style = tone.map_or_else(theme::dim, theme::tone);
            let seconds = (did.at / 1000).rem_euclid(60);
            scene.atom(vec![look::dim(format!("{}:{seconds:02}", hm(did.at)))]);
            scene.atom(vec![look::span(mark, style)]);
            scene.indent(11);
            scene.pad_to(11);
            let text = if tone.is_some() {
                ratatui::style::Style::new()
            } else {
                theme::dim()
            };
            look::flow(scene, &[look::span(&did.text, text)]);
            scene.indent(0);
            scene.end_line();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_a_thing_done_in_a_few_words() {
        assert_eq!(
            short_of("typed into the agent of 305-overview-gitlab-refresh: [overview] x: y"),
            "typed into the agent\u{a0}of\u{a0}#305"
        );
        assert_eq!(short_of("merged !316"), "merged !316");
        assert_eq!(
            short_of("removing the worktree 303-a-b: pnpm wt:remove 303-a-b"),
            "removing the worktree #303"
        );
    }

    #[test]
    fn glues_an_answer_so_the_band_never_cuts_it_in_the_middle() {
        let short = short_of("decide #2 of 307-x-y → Rust + ratatui · typed into its agent's pane");
        assert_eq!(short.replace('\u{a0}', " "), "decide #2 of #307 → Rust + ratatui");
        let words: Vec<&str> = short.split(' ').collect();
        assert_eq!(
            words,
            ["decide", "#2\u{a0}of\u{a0}#307", "→\u{a0}Rust\u{a0}+\u{a0}ratatui"]
        );
        let header = crate::tui::Header::new("⇶ @monocircuit/breadboard").log(
            vec![ratatui::text::Span::raw(format!("✓ {short}"))],
            2,
            Duration::from_secs(60),
        );
        for width in [80u16, 100, 120] {
            let band: String = crate::tui::band::lines(&header, "↓ 41 more", width)
                .iter()
                .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
                .collect::<String>()
                .replace('\u{a0}', " ");
            assert!(
                !band.contains('→') || band.contains("→ Rust + ratatui"),
                "{width}: {band}"
            );
            assert!(band.contains("✓ decide #2 of #307"), "{width}: {band}");
        }
    }

    #[test]
    fn says_where_the_end_stands() {
        let end = Run::armed();
        assert_eq!(
            breadboard_core::model::text_of(&end_said(&end, "b").unwrap()),
            "click again to end ⇶ b"
        );
    }
}
