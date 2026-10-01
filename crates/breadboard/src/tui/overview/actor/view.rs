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
const SHOWN_MS: i64 = 120_000;
/// How many things done the log shows.
const SHOWN: usize = 3;

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

/// The log under the band: the last things done, newest first, for two minutes.
pub fn log(page: &mut Page<Act>, ctx: &Ctx) {
    let shown: Vec<_> = ctx
        .actor
        .did
        .iter()
        .filter(|did| ctx.now - did.at < SHOWN_MS)
        .take(SHOWN)
        .collect();
    let Some(oldest) = shown.last() else { return };
    let left = SHOWN_MS - (ctx.now - oldest.at);
    page.section(Section::new("DONE").sticky().plain(), |scene| {
        scene.redraw_in(Duration::from_millis(left.max(0) as u64 + 50));
        for did in &shown {
            let (mark, tone) = match did.paint {
                Paint::Green => ("✓", Some(Tone::Green)),
                Paint::Red => ("✗", Some(Tone::Red)),
                Paint::Yellow => ("◐", Some(Tone::Yellow)),
                _ => ("·", None),
            };
            let style = tone.map_or_else(theme::dim, theme::tone);
            scene.pad_to(1);
            let seconds = (did.at / 1000).rem_euclid(60);
            scene.atom(vec![look::dim(format!("{}:{seconds:02}", hm(did.at)))]);
            scene.atom(vec![look::span(mark, style)]);
            scene.indent(12);
            let text = if tone.is_some() {
                look::span(&did.text, ratatui::style::Style::new())
            } else {
                look::dim(&did.text)
            };
            look::flow(scene, &[text]);
            scene.indent(0);
            scene.end_line();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_where_the_end_stands() {
        let end = Run::armed();
        assert_eq!(
            breadboard_core::model::text_of(&end_said(&end, "b").unwrap()),
            "click again to end ⇶ b"
        );
    }
}
