//! ACTIONS and UP NEXT of an orchestration (render-orchestration.mjs,
//! action-block.mjs, answers.mjs): what the agents ask of the user, by what
//! blocks most, each a block between rules; a button or an option answers,
//! the rest of the block does the action. UP NEXT: the automatic follow-ups,
//! a click each.
//!
//! An answer is stored in the actions file on the click and typed into the
//! agent's pane on the actor's thread (`herdr agent prompt`), claimed in the
//! file first so the Stop hook or a second overview never delivers it twice.
//! A try's `broken` and a plan's `different` are drafts: written into the
//! agent's input unsent, its tab in front, for the user to complete. The
//! block shows the answer from the click on, before a read has it (#290,
//! #303); the log under the band says where it went.

use super::actor::{self, Actor, glab, herdr};
use super::{Act, Ctx, Overview, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Flow, KeyEvent, Page, Scene};
use breadboard_core::actions::{self as stored, Action};
use breadboard_core::herdr::Pane;
use breadboard_core::model::Paint;
use breadboard_core::model::next::once_text;
use breadboard_core::model::orchestration::{ActionsSection, NextBand, OrchestrationModel};
use breadboard_core::model::order::{Go, Item, MergeOffer, Target};
use breadboard_core::reader::{Model, RemoteOf};
use breadboard_core::time::{ago, parse_iso};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    let section = match &model.actions {
        Ok(section) => section,
        Err(why) => return page.failed("ACTIONS", why),
    };
    let in_next = in_next(model).map(|item| item.key.as_str());
    let pick = pick_of(model, page.picked());
    page.section("ACTIONS", |scene| actions(scene, section, in_next, &pick, ctx));
    forget_done(&section.up_next);
    let up_next: Vec<&Item> = section.up_next.iter().filter(|item| !is_done(item)).collect();
    if !up_next.is_empty() {
        page.section("UP NEXT", |scene| {
            scene.note(format!(
                "{} automatic · a click does it, each goes by itself",
                up_next.len()
            ));
            for item in up_next {
                up_next_row(scene, item, ctx);
            }
        });
    }
}

/// The rows of UP NEXT whose click did all they do (the plan written):
/// gone from the click on, as an answer shows at once, until a read has
/// them gone, at most CLICKED_FOR.
static DONE: LazyLock<Mutex<Vec<(Go, Instant)>>> = LazyLock::new(Mutex::default);

/// The row that does `go` did it: it goes at once (`act`, the plan written).
pub fn did(go: &Go) {
    locked(&DONE).push((go.clone(), Instant::now()));
}

/// Whether `item`'s click did it and a read hasn't caught up.
pub fn is_done(item: &Item) -> bool {
    locked(&DONE)
        .iter()
        .any(|(go, at)| *go == item.go && at.elapsed() <= CLICKED_FOR)
}

/// A read without the rows: their record goes, so the same row read again later shows.
fn forget_done(up_next: &[Item]) {
    locked(&DONE).retain(|(go, at)| at.elapsed() <= CLICKED_FOR && up_next.iter().any(|item| item.go == *go));
}

/// What Tab picked, as the digit keys see it (lazygit: keys act on the
/// focused menu and its highlighted row, never on another).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Pick {
    /// Nothing: the digits do nothing. A stray keystroke (`2 more tests
    /// please⏎` typed into the wrong pane) never answers NEXT's action.
    #[default]
    Nothing,
    /// An action's block, or one of a decide's options (`option`, its answer):
    /// the digits answer that action, Enter does the block or picks the option.
    Action { key: String, option: Option<String> },
    /// Anything else (a worktree, an UP NEXT row, a merge request): the digits do nothing.
    Other,
}

impl Pick {
    /// Whether the digits answer `item`: only when it is picked with Tab.
    pub fn keys(&self, item: &Item) -> bool {
        match self {
            Pick::Action { key, .. } => *key == item.key,
            Pick::Nothing | Pick::Other => false,
        }
    }

    /// The option of `item` picked, if one of its options is.
    fn option_of(&self, item: &Item) -> Option<&str> {
        match self {
            Pick::Action { key, option } if *key == item.key => option.as_deref(),
            _ => None,
        }
    }
}

/// What the block `picked` with Tab is, among the actions of `model`.
pub fn pick_of(model: &OrchestrationModel, picked: Option<&Act>) -> Pick {
    let items = model
        .actions
        .as_ref()
        .map(|section| section.actions.as_slice())
        .unwrap_or_default();
    pick_in(items, picked)
}

/// What the block `picked` with Tab is, among `items`.
fn pick_in(items: &[Item], picked: Option<&Act>) -> Pick {
    let Some(picked) = picked else { return Pick::Nothing };
    let found = match picked {
        Act::Action { key, .. } => items.iter().find(|item| item.key == *key).map(|item| (item, None)),
        Act::Answer { file, id, answer } => items
            .iter()
            .find(|item| item.file.as_ref() == Some(file) && item.action.as_ref().is_some_and(|a| a.id == *id))
            .map(|item| (item, Some(answer.clone()))),
        _ => None,
    };
    match found {
        Some((item, option)) => Pick::Action {
            key: item.key.clone(),
            option,
        },
        None => Pick::Other,
    }
}

/// The action NEXT shows as its block, if it shows one.
fn in_next(model: &OrchestrationModel) -> Option<&Item> {
    match &model.next.as_ref().ok()?.band {
        NextBand::Action(item) => Some(item),
        _ => None,
    }
}

/// ACTIONS: each action a block between rules; the one NEXT shows (always on
/// screen, above) only by its head and where it is, not twice in full. The
/// block picked with Tab carries its keys, and under it what each does.
fn actions(scene: &mut Scene<Act>, section: &ActionsSection, in_next: Option<&str>, pick: &Pick, ctx: &Ctx) {
    let items = &section.actions;
    if items.is_empty() {
        scene.note("nothing waits for you");
        scene.pad_to(1);
        scene.text("none: an agent asks with pnpm wt:action <kind> <text>", theme::dim());
        return;
    }
    scene.note(note_of(items));
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            scene.rule();
        }
        if in_next == Some(item.key.as_str()) {
            shown_in_next(scene, item, ctx);
        } else if pick.keys(item) {
            action_block(scene, item, ctx, true);
            let hint = picked_hint(item, pick.option_of(item));
            if !hint.is_empty() {
                let room = scene.width();
                look::whole(scene, &hint, room);
            }
        } else {
            action_block(scene, item, ctx, false);
        }
    }
}

/// The action NEXT shows, in ACTIONS: its head and a pointer up; a click does the action.
fn shown_in_next(scene: &mut Scene<Act>, item: &Item, ctx: &Ctx) {
    let draw = |scene: &mut Scene<Act>| {
        head(scene, item, ctx, true);
        look::say(scene, &pointer_of(item), theme::dim());
        scene.end_line();
    };
    match click_of(item) {
        Some(act) => scene.block(act, draw),
        None => draw(scene),
    }
}

/// The pointer up of the action NEXT shows: where its answers are, or, for a
/// kind without answers (a merge, a question without options), what a click
/// here does; it never names an answer there is none of.
fn pointer_of(item: &Item) -> String {
    if !keyed(item).is_empty() {
        return "↑ in NEXT: answer it there".into();
    }
    match does_of(item) {
        Some(does) if !item.is_inert() => format!("↑ in NEXT · a click here {does}{}", there_of(item)),
        _ => "↑ in NEXT".into(),
    }
}

/// Where a question without options is answered: in its agent's tab. A
/// `waits` is one too: its agent asked it in its own words.
fn there_of(item: &Item) -> &'static str {
    if matches!(item.kind.as_str(), "decide" | "waits") {
        ": answer it there"
    } else {
        ""
    }
}

/// ACTIONS' note: what waits for the user, the answers on their way and the
/// tries that can't be tried yet apart (they wait for their agent or a merge,
/// not for the user), what a click does.
fn note_of(items: &[Item]) -> String {
    let shown: Vec<_> = items.iter().map(shown_answer).collect();
    // A ✓ goes to no one: ticked off, not on its way.
    let ticked = shown.iter().flatten().filter(|(_, waits)| waits == TICKED).count();
    let answered = shown.iter().flatten().count() - ticked;
    let waiting = items
        .iter()
        .zip(&shown)
        .filter(|(item, shown)| item.is_waiting() && shown.is_none())
        .count();
    let open = items.len() - answered - ticked - waiting;
    let mut parts = vec![match open {
        0 => "nothing waits for you".to_string(),
        n => format!("{n} for you"),
    }];
    match answered {
        0 => {}
        1 => parts.push("1 answered, on its way".into()),
        n => parts.push(format!("{n} answered, on their way")),
    }
    if ticked > 0 {
        parts.push(format!("{ticked} ticked off"));
    }
    if waiting > 0 {
        parts.push(format!("{waiting} waiting"));
    }
    // A ✓ alone tells no one: the words only where an answer goes to an agent.
    if items.iter().any(|item| !keyed(item).is_empty()) {
        parts.push("buttons and options answer its agent".into());
    }
    let rest = rest_does(items);
    if !rest.is_empty() {
        parts.push(format!("a click on the rest {rest}"));
    }
    // Node's `auto ones go by themselves` read as "they run by themselves",
    // wrong next to a merge that takes two clicks: an auto row leaves once its cause is gone.
    if items.iter().any(|item| item.is_auto) {
        parts.push("auto ones leave once done".into());
    }
    parts.join(" · ")
}

/// What a click on the body of `item` does, in words. A merge (merge.rs
/// `click`) opens its MR while anything blocks it, else arms it for a second
/// click; without its offer in the view it does nothing.
fn does_of(item: &Item) -> Option<&'static str> {
    Some(match &item.go {
        Go::FocusSpace(_) => "opens its space",
        Go::FocusAgent(_) => "opens its agent's tab",
        Go::Try { .. } => "runs its try",
        Go::Merge { .. } => match &item.merge {
            Some(offer) if offer.blockers.is_empty() => "merges on a second click",
            Some(offer) if !offer.url.is_empty() => "opens its MR in GitLab",
            _ => return None,
        },
        Go::Show { .. } => "opens its page",
        _ => return None,
    })
}

/// NEXT's hint under a merge that can't merge yet: what a click on it does
/// instead (opens the MR) and when two clicks merge it. None once two clicks
/// merge (its text says so) or when nothing is left to wait for.
fn merge_hint(offer: &MergeOffer) -> Option<String> {
    if offer.blockers.is_empty() || offer.url.is_empty() {
        return None;
    }
    let opens = format!("a click on the rest opens !{} in GitLab", offer.iid);
    // Merged or closed: nothing clears.
    if offer.blockers.iter().any(|blocker| blocker.starts_with("it is ")) {
        return Some(opens);
    }
    Some(format!(
        "{opens}: two clicks merge it once {}",
        cleared(&offer.blockers)
    ))
}

/// What `blockers` turn into once they clear (next.rs' `once_text`, each):
/// `its pipeline passes and GitLab has checked it`; one it has no words for
/// is named by where it stands, the line above.
fn cleared(blockers: &[String]) -> String {
    let mut once: Vec<String> = Vec::new();
    let mut is_unnamed = false;
    for blocker in blockers {
        let text = match blocker.as_str() {
            "GitLab is checking" => "GitLab has checked it".to_string(),
            // GitLab's own `draft_status` on an MR no longer marked draft (after
            // its agent posted the merge and it was marked ready): not the agent's to do.
            "draft" => "GitLab no longer calls it a draft".to_string(),
            _ => once_text(std::slice::from_ref(blocker)),
        };
        if text == *blocker {
            is_unnamed = true;
        } else if !once.contains(&text) {
            once.push(text);
        }
    }
    if is_unnamed {
        once.push(if once.is_empty() {
            "what the line above names clears".into()
        } else {
            "the rest above clears".into()
        });
    }
    match once.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// What a click on the rest of the blocks of `items` does, in words, by
/// their first appearance: `opens its space or runs its try`; "" for none.
fn rest_does(items: &[Item]) -> String {
    let mut does: Vec<&str> = Vec::new();
    for item in items.iter().filter(|item| !item.is_inert()) {
        let Some(what) = does_of(item) else { continue };
        if !does.contains(&what) {
            does.push(what);
        }
    }
    match does.split_last() {
        None => String::new(),
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    }
}

/// What a click on the body of an item does: nothing for an inert one (#290).
/// Named by its key: two actions of one agent are two picks.
fn click_of(item: &Item) -> Option<Act> {
    (!item.is_inert()).then(|| Act::Action {
        key: item.key.clone(),
        go: item.go.clone(),
    })
}

/// `kind` chip, who in bold, how long ago, a try's state: the head of a block
/// and of an UP NEXT row. A try that waits is grey but for its state.
fn head(scene: &mut Scene<Act>, item: &Item, ctx: &Ctx, is_chip_auto: bool) {
    let label = look::kind_label(&item.kind);
    let label = if item.is_auto && is_chip_auto {
        format!("{label} · auto")
    } else {
        label.to_string()
    };
    let is_grey = item.is_waiting();
    let tone = if is_grey {
        Tone::Gray
    } else {
        look::kind_tone(&item.kind)
    };
    scene.chip(&label, tone);
    let who = if is_grey { theme::dim() } else { theme::bold() };
    scene.atom_gap(vec![look::span(&item.who, who)], 2);
    if let Some(at) = parse_iso(&item.at) {
        scene.atom_gap(vec![look::dim(ago(at, ctx.now))], 2);
        look::ticks(scene);
    }
    if let Some(state) = &item.try_state {
        scene.atom_gap(vec![look::span(&state.label, look::try_tone(state.tone))], 2);
    }
    scene.end_line();
}

/// An action as its block (action-block.mjs): the head, its text, its
/// answers. `is_keyed`: the one whose answers the digit keys give too
/// (`key`): the block picked with Tab. A decide's options sit under the
/// block, not in it: each is a block of its own that answers, so hover on
/// the question never lights them as if they did the block's click.
pub fn action_block(scene: &mut Scene<Act>, item: &Item, ctx: &Ctx, is_keyed: bool) {
    let is_menu = !menu_of(item).is_empty();
    let draw = |scene: &mut Scene<Act>| {
        head(scene, item, ctx, true);
        text(scene, item, ctx);
        if !is_menu {
            answers(scene, item, is_keyed);
        }
    };
    match click_of(item) {
        Some(act) => scene.block(act, draw),
        None => draw(scene),
    }
    if is_menu {
        answers(scene, item, is_keyed);
    }
}

/// The options a decide is drawn with as a menu: none once answered, for an
/// automatic item, or without a file to store the answer in.
fn menu_of(item: &Item) -> Vec<stored::DecideOption> {
    match (&item.action, &item.file) {
        (Some(action), Some(_)) if action.kind == "decide" && !item.is_auto && !item.is_waiting() => {
            if shown_answer(item).is_some() {
                return vec![];
            }
            stored::options_of(action)
        }
        _ => vec![],
    }
}

/// A try's text and what a click on it opens, apart: core joins them with two spaces.
fn spot_of(item: &Item) -> (&str, &str) {
    match &item.action {
        Some(action) if action.is_try() => match item.text.strip_prefix(action.text.as_str()) {
            Some(spot) => (&action.text, spot.trim()),
            None => (&item.text, ""),
        },
        _ => (&item.text, ""),
    }
}

/// Words after the last atom, the first `gap` columns after it.
fn flow_after(scene: &mut Scene<Act>, spans: &[Span<'static>], gap: u16) {
    let mut words = look::words_of(spans).into_iter();
    if let Some(first) = words.next() {
        scene.atom_gap(first, gap);
    }
    for word in words {
        scene.atom(word);
    }
}

/// The text: a decide's question in bold, the rest dim; a try's spot dim
/// after it; a merge's `!iid` and what its two clicks need; a waiting try all
/// grey. Where a merge, a try, a removal or the end stands once clicked, the
/// actor's word takes its place.
fn text(scene: &mut Scene<Act>, item: &Item, ctx: &Ctx) {
    if item.kind == "decide" {
        let (question, rest) = match item.text.find('?') {
            Some(at) => item.text.split_at(at + 1),
            None => (item.text.as_str(), ""),
        };
        lines(scene, question, theme::bold());
        if !rest.trim().is_empty() {
            scene.end_line();
            lines(scene, rest.trim(), theme::dim());
        }
    } else if let Some(word) = actor::view::word(ctx, &item.key) {
        flow_spaced(scene, &look::spans(word), 1);
    } else if let Some(offer) = &item.merge {
        scene.atom(vec![look::span(format!("!{}", offer.iid), Style::new())]);
        merge_text(scene, &item.text, offer);
    } else {
        let (text, spot) = spot_of(item);
        let style = if item.is_waiting() { theme::dim() } else { Style::new() };
        lines(scene, text, style);
        if !spot.is_empty() {
            flow_after(scene, &[look::dim(spot)], 2);
        }
    }
    scene.end_line();
}

/// Words that wrap whole, where two spaces part them kept two apart (a
/// try's text and its spot, `trySpot` after `  `): flowed, they'd read as one
/// sentence. The first word `gap` columns after the last atom.
fn flow_spaced(scene: &mut Scene<Act>, spans: &[Span<'static>], gap: u16) {
    let mut parts: Vec<Vec<Span<'static>>> = vec![vec![]];
    for span in spans {
        for (i, piece) in span.content.split("  ").enumerate() {
            if i > 0 && parts.last().is_some_and(|part| !part.is_empty()) {
                parts.push(vec![]);
            }
            if let Some(part) = parts.last_mut().filter(|_| !piece.is_empty()) {
                part.push(Span::styled(piece.to_string(), span.style));
            }
        }
    }
    for (i, part) in parts.iter().filter(|part| !part.is_empty()).enumerate() {
        flow_after(scene, part, if i == 0 { gap } else { 2 });
    }
}

/// Text whose lines stay lines and whose empty lines stay empty (a plan's paragraphs).
fn lines(scene: &mut Scene<Act>, text: &str, style: Style) {
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            scene.end_line();
        }
        if line.trim().is_empty() {
            scene.newline();
        } else {
            look::say(scene, line, style);
        }
    }
}

/// What a merge's two clicks need (merge.mjs `mergeText`): its blockers, dim, or the clicks.
fn merge_text(scene: &mut Scene<Act>, text: &str, offer: &MergeOffer) {
    let verb = if offer.is_draft {
        "mark ready and merge"
    } else {
        "merge"
    };
    if !offer.blockers.is_empty() {
        let mut parts = vec![text.to_string()];
        if offer.is_draft && offer.can_mark_ready {
            parts.push("draft".into());
        }
        parts.extend(offer.blockers.iter().cloned());
        look::say(scene, &parts.join(" · "), theme::dim());
        return;
    }
    look::say(scene, text, Style::new());
    let after = if offer.to_check > 0 {
        format!(" · {} to check after merge", offer.to_check)
    } else {
        String::new()
    };
    look::say(scene, &format!("· two clicks {verb}{after}"), theme::dim());
}

/// Whether `option` is the one recommended: `wt:action` stores its label,
/// a number given resolved (action-block.mjs: the stored option, exactly).
fn is_recommended(action: &Action, option: &stored::DecideOption) -> bool {
    action.recommend.as_deref() == Some(option.answer.as_str())
}

/// The answer area: none for an automatic item or a waiting try; the answer
/// on its way; a decide's options, numbered like a menu, each answering on
/// all of its lines; else a button per answer, a green ✓ alone. Keyed (in
/// NEXT): the numbers are keys, cyan, and each button carries its own.
fn answers(scene: &mut Scene<Act>, item: &Item, is_keyed: bool) {
    let (Some(action), Some(file)) = (&item.action, &item.file) else {
        return;
    };
    if item.is_auto || item.is_waiting() {
        return;
    }
    if let Some((answer, waits)) = shown_answer(item) {
        scene.atom(vec![look::span(format!("→ {answer}"), theme::tone(Tone::Yellow))]);
        // Apart from the answer: `→ Rust + ratatui its agent …` reads as one answer without colour.
        let mut words = look::words_of(&[look::dim(&waits)]).into_iter();
        if let Some(mut first) = words.next() {
            first.insert(0, look::dim("· "));
            scene.atom(first);
        }
        for word in words {
            scene.atom(word);
        }
        scene.end_line();
        return;
    }
    let answer = |answer: &str| Act::Answer {
        file: file.clone(),
        id: action.id,
        answer: answer.to_string(),
    };
    let options = stored::options_of(action);
    if action.kind == "decide" && !options.is_empty() {
        for (at, option) in options.iter().enumerate() {
            scene.block(answer(&option.answer), |scene| {
                let number = format!("{}.", at + 1);
                let style = if is_keyed {
                    theme::tone(Tone::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                };
                scene.atom(vec![look::span(&number, style)]);
                // The label and its description hang under the label's start.
                scene.indent(number.len() as u16 + 1);
                look::say(scene, &option.label, theme::bold());
                if is_recommended(action, option) {
                    look::say(scene, "(Recommended)", theme::tone(Tone::Green));
                }
                if !option.description.is_empty() {
                    scene.end_line();
                    look::say(scene, &option.description, theme::dim());
                }
                scene.indent(0);
            });
        }
        return;
    }
    let labels = stored::answers_of(action);
    if labels.is_empty() {
        // Says what it does: no key, nothing typed (answers.mjs: `doneAction` alone).
        scene.button(TICK, Tone::Green, answer("✓"));
        look::say(scene, NOT_TOLD, theme::dim());
    }
    for (at, label) in labels.iter().enumerate() {
        let shown = if is_keyed {
            format!("{} {label}", at + 1)
        } else {
            label.clone()
        };
        scene.button(&shown, Tone::Cyan, answer(label));
    }
    scene.end_line();
}

/// The ✓ of a kind without answers, and what it does not do.
const TICK: &str = "✓ tick off";
/// What a ✓ clicked here shows until a read has it gone: no agent hears of it.
const TICKED: &str = "ticked off";
const NOT_TOLD: &str = "takes it off the list · its agent is not told";

/// The answers the digit keys give for `item` in NEXT, key 1 first: its
/// options or its buttons as drawn, each typed to its agent. None for a ✓
/// (it tells no one, so no single key does it), once it is answered, for an
/// automatic item or a waiting try.
pub fn keyed(item: &Item) -> Vec<String> {
    let Some(action) = &item.action else { return vec![] };
    if item.file.is_none() || item.is_auto || item.is_waiting() || shown_answer(item).is_some() {
        return vec![];
    }
    stored::answers_of(action)
}

/// Whether `item` is drawn with the ✓ of a kind without answers.
fn has_tick(item: &Item) -> bool {
    let Some(action) = &item.action else { return false };
    item.file.is_some()
        && !item.is_auto
        && !item.is_waiting()
        && shown_answer(item).is_none()
        && stored::answers_of(action).is_empty()
}

/// What NEXT says under its block, in parts, as lazygit's options bar names
/// each key and what it does: each digit (cyan, as on its button) and where
/// its answer goes; for a ✓ (no key: it tells no one) what a click on the
/// rest does, the way to its agent; for an automatic item (a `waits`, which
/// has no buttons) what a click on it does. Enter is not one: it runs only
/// the row picked with Tab, which is lit. None for the rest.
pub fn hint(item: &Item) -> Vec<Vec<Span<'static>>> {
    let key = key_span;
    let answers = keyed(item);
    if answers.is_empty() {
        if item.is_inert() || shown_answer(item).is_some() {
            return vec![];
        }
        // A merge's own text says what two clicks do; a blocked one's click opens its MR instead.
        if let Some(offer) = &item.merge {
            return merge_hint(offer)
                .map(|hint| vec![vec![look::dim(hint)]])
                .unwrap_or_default();
        }
        let on = if has_tick(item) {
            "the rest"
        } else if item.is_auto || item.action.is_none() {
            "it"
        } else {
            return vec![];
        };
        return does_of(item)
            .map(|does| vec![vec![look::dim(format!("a click on {on} {does}{}", there_of(item)))]])
            .unwrap_or_default();
    }
    let Some(action) = item.action.as_ref() else {
        return vec![];
    };
    if action.kind == "decide" {
        let keys = match answers.len() {
            1 => "1".to_string(),
            n => format!("1-{n}"),
        };
        return vec![vec![key(keys), look::dim(" picks that option: typed to its agent")]];
    }
    answers
        .iter()
        .enumerate()
        .map(|(at, answer)| {
            let goes = if stored::is_draft(&Action {
                answer: Some(answer.clone()),
                ..action.clone()
            }) {
                "a draft in its agent's input for you to finish"
            } else {
                "typed to its agent"
            };
            vec![key((at + 1).to_string()), look::dim(format!(" {answer}: {goes}"))]
        })
        .collect()
}

/// NEXT's hint while its block is not picked: how its digits come to act
/// (`tab to it, then 1-2 picks that option: …`), so none acts on a stray
/// keystroke; a ✓'s or an automatic item's own words as `hint`.
pub fn unpicked_hint(item: &Item) -> Vec<Vec<Span<'static>>> {
    let mut parts = hint(item);
    if !keyed(item).is_empty()
        && let Some(first) = parts.first_mut()
    {
        first.splice(0..0, [key_span("tab"), look::dim(" to it, then ")]);
    }
    parts
}

/// A digit (`act::key`): the answer it gives the action picked with Tab,
/// through the click's path. Nothing picked, a pick that is no action, or a
/// digit its action has no answer for: nothing. A digit never answers a
/// block the user didn't light first, so a keystroke meant for an agent's
/// pane never writes into one (rule 6). Any other key is left to the others
/// (Enter runs the frame's pick).
pub fn key(app: &mut Overview, key: KeyEvent, picked: Option<&Act>) -> Option<Flow> {
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
        return None;
    }
    let KeyCode::Char(digit @ '1'..='9') = key.code else {
        return None;
    };
    let model = app.orchestration()?;
    let item = match pick_of(model, picked) {
        Pick::Action { key, .. } => item_of_key(model, &key)?.clone(),
        Pick::Nothing | Pick::Other => return Some(Flow::Same),
    };
    let answers = keyed(&item);
    let Some(answer_of) = answers.get(digit as usize - '1' as usize) else {
        return Some(Flow::Same);
    };
    let (file, id) = (item.file.clone()?, item.action.as_ref()?.id);
    Some(answer(app, &file, id, answer_of))
}

/// The action of `model` named `key`.
fn item_of_key<'a>(model: &'a OrchestrationModel, key: &str) -> Option<&'a Item> {
    model.actions.as_ref().ok()?.actions.iter().find(|item| item.key == key)
}

/// What the keys do on a block picked with Tab, as lazygit names a menu's
/// keys: its digits (as `hint`), then what Enter does there: the option
/// picked is given, else the block's click. A ✓'s or an automatic item's own
/// words (a click) when there are no keys at all.
pub fn picked_hint(item: &Item, option: Option<&str>) -> Vec<Vec<Span<'static>>> {
    let is_keyed = !keyed(item).is_empty();
    let mut parts = if is_keyed { hint(item) } else { vec![] };
    let enter = match option {
        Some(label) if is_keyed => Some(format!(" picks \u{201c}{label}\u{201d}: typed to its agent")),
        Some(_) => None,
        None if item.is_inert() => None,
        // Where a question without options is answered: only where no key answers it.
        None if is_keyed => does_of(item).map(|does| format!(" {does}")),
        None => does_of(item).map(|does| format!(" {does}{}", there_of(item))),
    };
    match enter {
        Some(does) => parts.push(vec![key_span("enter"), look::dim(does)]),
        None if parts.is_empty() => return hint(item),
        None => {}
    }
    parts
}

/// A key as the hints name it: cyan and bold, as on its button.
fn key_span(text: impl Into<String>) -> Span<'static> {
    look::span(text.into(), theme::tone(Tone::Cyan).add_modifier(Modifier::BOLD))
}

/// A row of UP NEXT in NEXT (next.mjs): kind and who, its text right after
/// them, whole; a click anywhere on it does it.
pub fn next_row(scene: &mut Scene<Act>, item: &Item, ctx: &Ctx) {
    let draw = |scene: &mut Scene<Act>| {
        scene.chip(look::kind_label(&item.kind), look::kind_tone(&item.kind));
        scene.atom(vec![look::bold(&item.who)]);
        match actor::view::word(ctx, &item.key) {
            Some(word) => flow_spaced(scene, &look::spans(word), 2),
            None => flow_after(scene, &[look::span(&item.text, Style::new())], 2),
        }
        scene.end_line();
    };
    match click_of(item) {
        Some(act) => scene.block(act, draw),
        None => draw(scene),
    }
}

/// A row of UP NEXT: kind, who, how long ago; its text below, whole (#273).
pub fn up_next_row(scene: &mut Scene<Act>, item: &Item, ctx: &Ctx) {
    let draw = |scene: &mut Scene<Act>| {
        head(scene, item, ctx, false);
        scene.indent(2);
        scene.pad_to(2);
        if !actor::view::said(scene, ctx, &item.key) {
            look::say(scene, &item.text, Style::new());
        }
        scene.indent(0);
        scene.end_line();
    };
    match click_of(item) {
        Some(act) => scene.block(act, draw),
        None => draw(scene),
    }
}

// ---------------------------------------------------------------------------
// Answering (answers.mjs)

/// An answer clicked here: shown until a read has it, at most CLICKED_FOR.
struct Clicked {
    answer: String,
    waits: String,
    at: Instant,
}

/// How long a clicked answer stands in for one a read doesn't show (a read takes seconds).
const CLICKED_FOR: Duration = Duration::from_secs(60);

/// What the block says between the click and the delivery.
const TYPING: &str = "typing it into its agent's pane …";

/// `file#id` → the answer clicked for it in this overview.
static CLICKED: LazyLock<Mutex<HashMap<(PathBuf, i64), Clicked>>> = LazyLock::new(Mutex::default);

/// actions file → the state of its agent's pane at the last delivery.
static TRIED: LazyLock<Mutex<HashMap<PathBuf, Option<String>>>> = LazyLock::new(Mutex::default);

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The answer an item shows and why it waits: its own on its way, else one
/// clicked here that a read hasn't caught up with (answers.mjs `shownAction`).
fn shown_answer(item: &Item) -> Option<(String, String)> {
    let action = item.action.as_ref()?;
    let key = (item.file.clone()?, action.id);
    let mut clicked = locked(&CLICKED);
    if let Some(answer) = action.answer.as_deref().filter(|answer| !answer.is_empty()) {
        // Still typing: the read came between the store and the delivery.
        if let Some(own) = clicked.get(&key).filter(|own| own.waits == TYPING) {
            return Some((own.answer.clone(), own.waits.clone()));
        }
        clicked.remove(&key);
        return Some((answer.to_string(), waits_of(&key.0, action.waits.clone())));
    }
    let own = clicked.get(&key)?;
    if own.at.elapsed() > CLICKED_FOR {
        clicked.remove(&key);
        return None;
    }
    Some((own.answer.clone(), own.waits.clone()))
}

/// Why an answer not delivered waits: its own reason (a draft's), else
/// where its agent is: at a question (its pane was `blocked` at the last
/// try), or on to its next stop (answers.mjs says the latter for both).
fn waits_of(file: &Path, waits: Option<String>) -> String {
    waits.unwrap_or_else(|| {
        let is_blocked = locked(&TRIED)
            .get(file)
            .is_some_and(|state| state.as_deref() == Some("blocked"));
        if is_blocked {
            "its agent is at a question: it goes in once that is answered".into()
        } else {
            "at its next stop".into()
        }
    })
}

/// An answer in the log under the band: quoted, so a cut there never reads
/// as a shorter answer (`→ "OpenTUI on` is visibly cut).
fn logged(what: &str, answer: &str, waits: &str) -> String {
    format!("{what} → \u{201c}{answer}\u{201d} · {waits}")
}

fn remember(file: &Path, id: i64, answer: &str, waits: &str) {
    locked(&CLICKED).insert(
        (file.to_path_buf(), id),
        Clicked {
            answer: answer.into(),
            waits: waits.into(),
            at: Instant::now(),
        },
    );
}

/// The item of ACTIONS the answer is for.
fn item_of(app: &Overview, file: &Path, id: i64) -> Option<Item> {
    let Some(Model::Orchestration(model)) = &app.model else {
        return None;
    };
    let section = model.actions.as_ref().ok()?;
    section
        .actions
        .iter()
        .find(|item| item.file.as_deref() == Some(file) && item.action.as_ref().is_some_and(|a| a.id == id))
        .cloned()
}

/// What an answer is about, for the log: `decide #2 of Lucía Nkemelu`, the
/// agent by the name the overview shows it by (#292), as `told …` does.
fn what_of(actor: &Actor, item: &Item, id: i64) -> String {
    format!("{} #{id} of {}", look::kind_label(&item.kind), actor.shown(&item.who))
}

/// The line an answer is typed or written as, word for word, for the log.
fn line_of(action: &Action) -> String {
    if stored::is_draft(action) {
        stored::draft_line(action)
    } else {
        stored::answer_line(action)
    }
}

/// A click on an answer (answers.mjs `answerWith`): ✓ ticks a kind without
/// answers off; any other is stored at once and shown on its block, then
/// typed into its agent's pane on the actor's thread (a draft written into
/// its input, its tab in front). A try found `broken` puts its ready MR back
/// to draft (#280). The log under the band says where it went.
pub fn answer(app: &mut Overview, file: &Path, id: i64, answer: &str) -> Flow {
    let Some(item) = item_of(app, file, id) else {
        return Flow::Same;
    };
    let Some(action) = item.action.clone() else {
        return Flow::Same;
    };
    let what = what_of(&app.actor, &item, id);
    if stored::answers_of(&action).is_empty() {
        if stored::done_action(file, id).unwrap_or(false) {
            remember(file, id, "✓", TICKED);
            app.actor
                .log(Paint::Green, format!("{what} ticked off · its agent is not told"));
        }
        return Flow::Redraw;
    }
    if !stored::answer_action(file, id, answer).unwrap_or(false) {
        return Flow::Same;
    }
    remember(file, id, answer, TYPING);
    let is_draft = stored::is_draft(&Action {
        answer: Some(answer.to_string()),
        ..action.clone()
    });
    let redraft = redraft_of(app, &item, &action, answer);
    let line = line_of(&Action {
        answer: Some(answer.to_string()),
        ..action.clone()
    });
    let main = app.actor.main.clone();
    let (file, to, answer) = (file.to_path_buf(), item.to.clone(), answer.to_string());
    app.actor.queue(move || {
        if let Some(to) = &to {
            if is_draft {
                focus_agent(&main, to);
            }
            let _ = deliver_answers(&main, &file, to);
        }
        let left = stored::actions_of(&file).into_iter().find(|left| left.id == id);
        let is_in = left.is_none();
        let waits = match left {
            Some(left) => waits_of(&file, left.waits),
            None if is_draft => DRAFT_WRITTEN.into(),
            None => TYPED_IN.into(),
        };
        let redrafted = redraft.map(|mr| redraft_mr(&main, &file, mr));
        Some(Box::new(move |app: &mut Overview| {
            remember(&file, id, &answer, &waits);
            let paint = if is_in { Paint::Green } else { Paint::Yellow };
            app.actor.log_with(paint, logged(&what, &answer, &waits), Some(line));
            match redrafted {
                Some(Ok(iid)) => {
                    app.actor
                        .log(Paint::Green, format!("!{iid} back to draft until the fix is tried"));
                    app.refresh_gitlab();
                }
                Some(Err(why)) => app.actor.log(Paint::Red, format!("the MR stays ready: {why}")),
                None => {}
            }
            app.redraw();
        }))
    });
    Flow::Redraw
}

/// The MR a try answered `broken` puts back to draft (merge.mjs
/// `shouldRedraft`): not one on the test instance, only while its MR is
/// open and ready. Its iid and title.
fn redraft_of(app: &Overview, item: &Item, action: &Action, answer: &str) -> Option<(i64, String)> {
    if !action.is_try() || action.on.as_deref() == Some("test") || answer != "broken" {
        return None;
    }
    let (Some(branch), Some(RemoteOf::Orchestration(remote))) = (&item.branch, &app.remote) else {
        return None;
    };
    let mr = remote.of(branch).mr.as_ref()?;
    (mr.state == "opened" && !mr.is_draft).then(|| (mr.iid, mr.title.clone()))
}

/// The MR back to draft (merge.mjs `redraftOnBroken`): the merges its agent
/// posted withdrawn from `file`, the title `Draft: …`. A failure only costs
/// the step: its agent puts it back to draft too (the brief).
fn redraft_mr(main: &Path, file: &Path, (iid, title): (i64, String)) -> Result<i64, String> {
    for merge in stored::actions_of(file).iter().filter(|other| other.kind == "merge") {
        let _ = stored::done_action(file, merge.id);
    }
    let title = format!("Draft: {}", glab::ready_title(&title));
    glab::write(main, "PUT", &glab::mr_endpoint(iid), &[("title", title)]).map(|_| iid)
}

/// The prompt mark of Claude Code at the start of its input line.
const PROMPT: char = '❯';

/// What `screen` (the last lines of an agent's pane) has in Claude Code's
/// input: the text after the last `❯` up to the rule below it; "" when
/// empty, None without an input line.
pub fn input_in(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.split('\n').collect();
    let start = lines.iter().rposition(|line| line.trim_start().starts_with(PROMPT))?;
    let is_rule = |line: &str| line.trim_start().starts_with("───");
    let end = (start + 1..lines.len())
        .find(|at| is_rule(lines[*at]))
        .unwrap_or(lines.len());
    let text = lines[start..end].join("\n");
    let text = text.trim_start();
    Some(text.strip_prefix(PROMPT).unwrap_or(text).trim().to_string())
}

/// Whether the user typed into the input of `pane` (an unreadable one counts as empty).
fn has_input(pane: &Pane) -> bool {
    let read = ["pane", "read", &pane.id, "--source", "visible", "--lines", "30"];
    let Ok(output) = Command::new("herdr")
        .args(read)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    output.status.success() && input_in(&String::from_utf8_lossy(&output.stdout)).is_some_and(|input| !input.is_empty())
}

/// The agent's tab `to` addresses in front, its space first (answers.mjs `focusAgent`).
fn focus_agent(main: &Path, to: &Target) -> bool {
    herdr::agent_pane_of(main, to).is_some_and(|pane| {
        herdr::did(&["workspace", "focus", &pane.workspace]) && herdr::did(&["tab", "focus", &pane.tab])
    })
}

/// The draft of `action` written into the input of `pane`, unsent. One that
/// waited for a blocked pane is dropped when the user typed meanwhile: they
/// said it themselves. True when it is done with.
fn write_draft(pane: &Pane, action: &Action) -> bool {
    (action.waits.is_some() && has_input(pane))
        || herdr::did(&["pane", "send-text", &pane.id, &stored::draft_line(action)])
}

/// Why a draft waits, on its row.
fn draft_waits_for(pane: Option<&Pane>) -> &'static str {
    match pane {
        Some(_) => "the agent is at a question: the draft follows once it's answered",
        None => "no agent in its tab: the draft follows once there is one",
    }
}

/// The answers of `file` delivered to `to` (answers.mjs `deliverAnswers`):
/// each claimed in the file before it is typed and taken out once it got in.
/// A draft is written, not submitted. A pane at a question gets nothing: the
/// drafts say why they wait. The ones that got in.
fn deliver_answers(main: &Path, file: &Path, to: &Target) -> Vec<Action> {
    let pane = herdr::agent_pane_of(main, to);
    locked(&TRIED).insert(file.to_path_buf(), pane.as_ref().and_then(|pane| pane.status.clone()));
    let answered: Vec<Action> = stored::actions_of(file)
        .into_iter()
        .filter(|action| action.answer.as_deref().is_some_and(|answer| !answer.is_empty()))
        .collect();
    let pane = match pane {
        Some(pane) if herdr::can_type(Some(&pane)) => pane,
        pane => {
            // Text written now would land in the question's dialog.
            let waits = draft_waits_for(pane.as_ref());
            for action in answered.iter().filter(|action| stored::is_draft(action)) {
                if action.waits.as_deref() != Some(waits) {
                    let _ = stored::draft_waits(file, action.id, Some(waits));
                }
            }
            return Vec::new();
        }
    };
    let mut delivered = Vec::new();
    for id in answered.iter().map(|action| action.id) {
        let Ok(Some(action)) = stored::claim_answer(file, id) else {
            continue;
        };
        let is_in = if stored::is_draft(&action) {
            write_draft(&pane, &action)
        } else {
            herdr::type_into(Some(&pane), &stored::answer_line(&action))
        };
        let _ = if is_in {
            stored::done_action(file, id).map(|_| ())
        } else {
            stored::release_answer(file, id)
        };
        if is_in {
            delivered.push(action);
        }
    }
    delivered
}

/// Where a delivered answer went, for its block and the log.
fn went_to(action: &Action) -> &'static str {
    if stored::is_draft(action) {
        DRAFT_WRITTEN
    } else {
        TYPED_IN
    }
}

/// Where an answer went, on its block and in the log.
const TYPED_IN: &str = "typed into its agent's pane";
const DRAFT_WRITTEN: &str = "written into its agent's input: say what's wrong there and send it";

/// After a read: answers that didn't get in are tried again on the actor's
/// thread once their agent's pane changed state, not on every read; after a
/// start once (answers.mjs `retryAnswers`). Only the overview that delivers
/// calls it (the actor's pid file), never a print. Each that gets in says so
/// on its block and in the log, green, after the yellow of its wait.
pub fn retry_answers(actor: &Actor, model: &OrchestrationModel) {
    let Ok(section) = &model.actions else { return };
    let mut targets: Vec<(PathBuf, Target)> = Vec::new();
    let mut whats: HashMap<(PathBuf, i64), String> = HashMap::new();
    for item in &section.actions {
        if let (Some(file), Some(to)) = (&item.file, &item.to)
            && item.answer().is_some()
        {
            if let Some(action) = &item.action {
                whats.insert((file.clone(), action.id), what_of(actor, item, action.id));
            }
            if !targets.iter().any(|(known, _)| known == file) {
                targets.push((file.clone(), to.clone()));
            }
        }
    }
    if targets.is_empty() {
        locked(&TRIED).clear();
        return;
    }
    let main = actor.main.clone();
    actor.queue(move || {
        let mut delivered: Vec<(PathBuf, Action)> = Vec::new();
        for (file, to) in targets {
            let state = herdr::agent_pane_of(&main, &to).and_then(|pane| pane.status);
            if locked(&TRIED).get(&file) == Some(&state) {
                continue;
            }
            delivered.extend(
                deliver_answers(&main, &file, &to)
                    .into_iter()
                    .map(|action| (file.clone(), action)),
            );
        }
        if delivered.is_empty() {
            return None;
        }
        Some(Box::new(move |app: &mut Overview| {
            for (file, action) in delivered {
                let answer = action.answer.clone().unwrap_or_default();
                let waits = went_to(&action);
                remember(&file, action.id, &answer, waits);
                let what = whats
                    .remove(&(file, action.id))
                    .unwrap_or_else(|| format!("#{}", action.id));
                app.actor
                    .log_with(Paint::Green, logged(&what, &answer, waits), Some(line_of(&action)));
            }
            app.redraw();
        }) as actor::Effect)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::model::try_state::{TryState, TryTone};

    fn decide() -> Action {
        Action {
            id: 2,
            kind: "decide".into(),
            text: "Which stack? The numbers favour one".into(),
            options: vec!["Rust + ratatui".into(), "Bun: flexbox".into()],
            descriptions: vec!["one binary".into()],
            recommend: Some("Rust + ratatui".into()),
            ..Default::default()
        }
    }

    fn item_with(action: Action) -> Item {
        Item {
            key: format!("k#{}", action.id),
            kind: action.kind.clone(),
            text: action.text.clone(),
            who: "307-x".into(),
            file: Some(PathBuf::from(format!("/nowhere/{}/actions.json", action.id))),
            action: Some(action),
            ..Default::default()
        }
    }

    fn drawn(item: &Item, width: u16) -> (Vec<String>, Scene<Act>) {
        let gitlab = super::super::GitlabState::default();
        let ctx = Ctx {
            is_once: false,
            folds: Default::default(),
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor: actor::idle(),
            words: actor::view::no_words(),
        };
        let mut scene = Scene::new(width);
        action_block(&mut scene, item, &ctx, false);
        scene.finish();
        let text = scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .collect();
        (text, scene)
    }

    fn answered_at(scene: &Scene<Act>, x: u16, y: u16) -> Option<String> {
        scene
            .regions
            .iter()
            .filter(|region| region.contains(x, y))
            .min_by_key(|region| (!region.is_button(), region.area.height))
            .and_then(|region| match region.action() {
                Act::Answer { answer, .. } => Some(answer.clone()),
                _ => None,
            })
    }

    #[test]
    fn a_decide_is_a_menu_whose_every_line_answers_its_option() {
        let (text, scene) = drawn(&item_with(decide()), 40);
        assert_eq!(
            text[1..],
            [
                "Which stack?",
                "The numbers favour one",
                "1. Rust + ratatui (Recommended)",
                "   one binary",
                "2. Bun",
                "   flexbox",
            ]
        );
        assert_eq!(answered_at(&scene, 30, 3).as_deref(), Some("Rust + ratatui"));
        assert_eq!(answered_at(&scene, 39, 4).as_deref(), Some("Rust + ratatui"));
        // The stored option is the answer, not the label shown.
        assert_eq!(answered_at(&scene, 0, 6).as_deref(), Some("Bun: flexbox"));
        assert_eq!(answered_at(&scene, 0, 1), None);
    }

    #[test]
    fn hover_on_a_decides_question_lights_no_option() {
        let mut item = item_with(decide());
        item.go = Go::FocusSpace(PathBuf::from("/w"));
        let (_, scene) = drawn(&item, 40);
        let body: Vec<_> = scene
            .regions
            .iter()
            .filter(|region| matches!(region.action(), Act::Action { .. }))
            .collect();
        assert_eq!(body.len(), 1);
        // Head, question, the rest of the text: the options below are blocks of their own.
        assert_eq!((body[0].area.y, body[0].area.bottom()), (0, 3));
        assert_eq!(answered_at(&scene, 30, 3).as_deref(), Some("Rust + ratatui"));
    }

    #[test]
    fn a_spot_stays_two_spaces_after_its_text() {
        // Across spans too: the actor's word may split the text from its spot.
        let spans = vec![Span::raw("the page opened"), look::dim("  :3000/x")];
        let mut scene: Scene<Act> = Scene::new(60);
        flow_spaced(&mut scene, &spans, 1);
        scene.end_line();
        scene.finish();
        let line: String = scene.lines[0].spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(line.trim_end(), "the page opened  :3000/x");
        // Too narrow for both: the spot wraps whole to the next line.
        let mut scene: Scene<Act> = Scene::new(20);
        flow_spaced(&mut scene, &spans, 1);
        scene.end_line();
        scene.finish();
        let lines: Vec<String> = scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .collect();
        assert_eq!(lines[0].trim_end(), "the page opened");
        assert_eq!(lines[1].trim_end(), ":3000/x");
    }

    #[test]
    fn an_up_next_row_its_click_did_goes_at_once_until_a_read_drops_it() {
        let go = Go::FocusSpace(PathBuf::from("/nowhere/done-by-click"));
        let row = Item {
            key: "order".into(),
            go: go.clone(),
            ..Default::default()
        };
        assert!(!is_done(&row));
        did(&go);
        assert!(is_done(&row), "gone from the click on");
        // A read that still has it keeps it gone; one without it forgets the click.
        forget_done(std::slice::from_ref(&row));
        assert!(is_done(&row));
        forget_done(&[]);
        assert!(!is_done(&row), "the same row read again later shows");
    }

    #[test]
    fn a_try_has_its_buttons_and_its_spot_dim_after_its_text() {
        let action = Action {
            id: 1,
            kind: "try".into(),
            text: "the spike".into(),
            run: Some("cargo run".into()),
            ..Default::default()
        };
        let mut item = item_with(action);
        item.text = "the spike  opens a terminal: cargo run".into();
        let (text, scene) = drawn(&item, 60);
        assert_eq!(
            text[1..],
            ["the spike  opens a terminal: cargo run", " looks good   broken "]
        );
        assert_eq!(answered_at(&scene, 1, 2).as_deref(), Some("looks good"));
        assert_eq!(answered_at(&scene, 18, 2).as_deref(), Some("broken"));
    }

    #[test]
    fn a_kind_without_answers_gets_a_tick_and_a_waiting_try_none() {
        let merge = Action {
            id: 3,
            kind: "merge".into(),
            text: "ready".into(),
            ..Default::default()
        };
        let (text, scene) = drawn(&item_with(merge), 80);
        assert_eq!(
            text.last().map(String::as_str),
            Some(" ✓ tick off  takes it off the list · its agent is not told")
        );
        assert_eq!(answered_at(&scene, 1, 2).as_deref(), Some("✓"));
        assert_eq!(answered_at(&scene, 20, 2), None, "the words beside it are no button");
        let mut waiting = item_with(Action {
            id: 4,
            kind: "try".into(),
            text: "later".into(),
            ..Default::default()
        });
        waiting.try_state = Some(TryState {
            label: "not pushed".into(),
            tone: TryTone::Waits,
            is_ready: false,
            url: None,
        });
        let (text, scene) = drawn(&waiting, 40);
        assert_eq!(text.len(), 2);
        assert!(scene.regions.is_empty(), "inert: no click, no answer");
    }

    #[test]
    fn a_clicked_answer_shows_until_a_read_has_it() {
        let mut action = decide();
        action.id = 77;
        let item = item_with(action.clone());
        let file = item.file.clone().unwrap();
        remember(&file, 77, "Rust + ratatui", TYPING);
        let typing = "→ Rust + ratatui · typing it into its agent's pane …";
        let (text, scene) = drawn(&item, 60);
        assert_eq!(text.last().unwrap(), typing);
        assert!(
            scene
                .regions
                .iter()
                .all(|region| !matches!(region.action(), Act::Answer { .. }))
        );
        // A read between the store and the delivery keeps saying it types.
        action.answer = Some("Rust + ratatui".into());
        let (text, _) = drawn(&item_with(action.clone()), 60);
        assert_eq!(text.last().unwrap(), typing);
        // Once the delivery is done with, a read that has the answer takes over, with why it waits.
        remember(&file, 77, "Rust + ratatui", "at its next stop");
        let (text, _) = drawn(&item_with(action), 60);
        assert_eq!(text.last().unwrap(), "→ Rust + ratatui · at its next stop");
        assert!(!locked(&CLICKED).contains_key(&(file, 77)));
    }

    #[test]
    fn in_next_the_digits_answer_only_what_is_typed_to_the_agent() {
        let item = item_with(decide());
        assert_eq!(keyed(&item), ["Rust + ratatui", "Bun: flexbox"]);
        assert_eq!(hint_text(&item), ["1-2 picks that option: typed to its agent"]);
        let try_item = item_with(Action {
            id: 5,
            kind: "try".into(),
            text: "the spike".into(),
            ..Default::default()
        });
        assert_eq!(keyed(&try_item), ["looks good", "broken"]);
        assert_eq!(
            hint_text(&try_item),
            [
                "1 looks good: typed to its agent",
                "2 broken: a draft in its agent's input for you to finish"
            ]
        );
        // The digit is a key, as on its button.
        assert_eq!(hint(&try_item)[0][0].style.fg, theme::tone(Tone::Cyan).fg);
        let look_item = item_with(Action {
            id: 9,
            kind: "look".into(),
            text: "read it".into(),
            ..Default::default()
        });
        assert_eq!(hint_text(&look_item), ["1 done: typed to its agent"]);
        let gitlab = super::super::GitlabState::default();
        let mut scene = Scene::new(60);
        let ctx = Ctx {
            is_once: false,
            folds: Default::default(),
            gitlab: &gitlab,
            now: 0,
            main_name: "platform",
            actor: actor::idle(),
            words: actor::view::no_words(),
        };
        action_block(&mut scene, &try_item, &ctx, true);
        scene.finish();
        let last: String = scene.lines[2].spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(last, " 1 looks good   2 broken ");
        assert_eq!(answered_at(&scene, 1, 2).as_deref(), Some("looks good"));
        // A ✓ tells its agent nothing: no key gives it, nothing says a key answers.
        let merge = item_with(Action {
            id: 6,
            kind: "merge".into(),
            text: "ready".into(),
            ..Default::default()
        });
        assert!(keyed(&merge).is_empty() && hint(&merge).is_empty());
        let open_question = item_with(Action {
            id: 10,
            kind: "decide".into(),
            text: "Keep Node running?".into(),
            ..Default::default()
        });
        assert!(keyed(&open_question).is_empty() && hint(&open_question).is_empty());
        // Its way to the agent instead: what a click on the rest does.
        let mut in_space = open_question.clone();
        in_space.go = Go::FocusSpace(PathBuf::from("/w"));
        assert_eq!(
            hint_text(&in_space),
            ["a click on the rest opens its space: answer it there"]
        );
        let mut auto = item_with(decide());
        auto.is_auto = true;
        assert!(keyed(&auto).is_empty() && hint(&auto).is_empty());
        // An automatic `waits` has no buttons: the hint says what a click on it does.
        let waits = Item {
            key: "waits:307-x".into(),
            kind: "waits".into(),
            text: "Shall I merge it?".into(),
            who: "307-x".into(),
            is_auto: true,
            go: Go::FocusAgent(PathBuf::from("/w")),
            ..Default::default()
        };
        assert!(keyed(&waits).is_empty());
        assert_eq!(
            hint_text(&waits),
            ["a click on it opens its agent's tab: answer it there"]
        );
        assert_eq!(
            pointer_of(&waits),
            "↑ in NEXT · a click here opens its agent's tab: answer it there"
        );
    }

    fn hint_text(item: &Item) -> Vec<String> {
        hint(item)
            .iter()
            .map(|part| part.iter().map(|span| span.content.as_ref()).collect())
            .collect()
    }

    fn texts_of(parts: &[Vec<Span<'static>>]) -> Vec<String> {
        parts
            .iter()
            .map(|part| part.iter().map(|span| span.content.as_ref()).collect())
            .collect()
    }

    fn try_ready() -> Item {
        let mut item = item_with(Action {
            id: 1,
            kind: "try".into(),
            text: "the spike".into(),
            ..Default::default()
        });
        item.go = Go::Try {
            dir: Some(PathBuf::from("/w")),
            id: 1,
        };
        item.try_state = Some(TryState {
            is_ready: true,
            label: "ready".into(),
            tone: TryTone::Ready,
            url: None,
        });
        item
    }

    #[test]
    fn a_digit_answers_only_the_block_picked_with_tab() {
        let (decide, try_item) = (item_with(decide()), try_ready());
        let items = [decide.clone(), try_item.clone()];
        // Nothing picked: no block is keyed, NEXT's neither (a stray `2` answers nothing).
        let pick = pick_in(&items, None);
        assert_eq!(pick, Pick::Nothing);
        assert!(!pick.keys(&decide) && !pick.keys(&try_item));
        // The try's block picked: its digits, never NEXT's decide.
        let picked = Act::Action {
            key: try_item.key.clone(),
            go: try_item.go.clone(),
        };
        let pick = pick_in(&items, Some(&picked));
        assert!(pick.keys(&try_item) && !pick.keys(&decide));
        // One of the decide's options: the decide's digits, Enter that option.
        let option = Act::Answer {
            file: decide.file.clone().unwrap(),
            id: 2,
            answer: "Bun: flexbox".into(),
        };
        let pick = pick_in(&items, Some(&option));
        assert!(pick.keys(&decide) && !pick.keys(&try_item));
        assert_eq!(pick.option_of(&decide), Some("Bun: flexbox"));
        // Anything else picked (a worktree): no block's digits.
        let pick = pick_in(&items, Some(&Act::Go(Go::FocusSpace(PathBuf::from("/w")))));
        assert_eq!(pick, Pick::Other);
        assert!(!pick.keys(&decide) && !pick.keys(&try_item));
    }

    #[test]
    fn unpicked_the_hint_says_tab_first() {
        assert_eq!(
            texts_of(&unpicked_hint(&item_with(decide()))),
            ["tab to it, then 1-2 picks that option: typed to its agent"]
        );
        assert_eq!(
            texts_of(&unpicked_hint(&try_ready())),
            [
                "tab to it, then 1 looks good: typed to its agent",
                "2 broken: a draft in its agent's input for you to finish"
            ]
        );
        // Without keys (a ✓, an automatic item): its own words, no tab.
        let mut open = item_with(Action {
            id: 12,
            kind: "decide".into(),
            text: "Which one?".into(),
            ..Default::default()
        });
        open.go = Go::FocusAgent(PathBuf::from("/w"));
        assert_eq!(
            texts_of(&unpicked_hint(&open)),
            ["a click on the rest opens its agent's tab: answer it there"]
        );
    }

    #[test]
    fn a_picked_block_names_its_digits_and_what_enter_does() {
        assert_eq!(
            texts_of(&picked_hint(&try_ready(), None)),
            [
                "1 looks good: typed to its agent",
                "2 broken: a draft in its agent's input for you to finish",
                "enter runs its try",
            ]
        );
        let mut decide = item_with(decide());
        decide.go = Go::FocusSpace(PathBuf::from("/w"));
        assert_eq!(
            texts_of(&picked_hint(&decide, Some("Bun: flexbox"))),
            [
                "1-2 picks that option: typed to its agent",
                "enter picks \u{201c}Bun: flexbox\u{201d}: typed to its agent"
            ]
        );
        // Answered by keys: not `answer it there`.
        assert_eq!(texts_of(&picked_hint(&decide, None))[1], "enter opens its space");
        let mut open = item_with(Action {
            id: 5,
            kind: "decide".into(),
            text: "Which one?".into(),
            ..Default::default()
        });
        open.go = Go::FocusAgent(PathBuf::from("/w"));
        assert_eq!(
            texts_of(&picked_hint(&open, None)),
            ["enter opens its agent's tab: answer it there"]
        );
    }

    #[test]
    fn a_tick_off_is_not_counted_on_its_way() {
        let mut item = item_with(Action {
            id: 41,
            kind: "decide".into(),
            text: "Which one?".into(),
            ..Default::default()
        });
        item.go = Go::FocusAgent(PathBuf::from("/w"));
        let file = item.file.clone().unwrap();
        remember(&file, 41, "✓", TICKED);
        assert_eq!(
            note_of(std::slice::from_ref(&item)),
            "nothing waits for you · 1 ticked off · a click on the rest opens its agent's tab"
        );
        locked(&CLICKED).remove(&(file, 41));
    }

    #[test]
    fn the_note_counts_apart_what_is_answered_and_on_its_way() {
        let mut first = item_with(decide());
        first.go = Go::FocusSpace(PathBuf::from("/w"));
        let mut second = item_with(Action {
            id: 31,
            kind: "look".into(),
            text: "read it".into(),
            answer: Some("done".into()),
            ..Default::default()
        });
        second.go = Go::FocusSpace(PathBuf::from("/w"));
        assert_eq!(
            note_of(&[first.clone(), second.clone()]),
            "1 for you · 1 answered, on its way · buttons and options answer its agent · \
             a click on the rest opens its space"
        );
        let mut third = second.clone();
        third.action.as_mut().unwrap().id = 32;
        assert_eq!(
            note_of(&[second, third]),
            "nothing waits for you · 2 answered, on their way · a click on the rest opens its space"
        );
        assert_eq!(
            note_of(&[first]),
            "1 for you · buttons and options answer its agent · a click on the rest opens its space"
        );
    }

    #[test]
    fn the_note_says_what_a_click_on_the_rest_of_a_block_does() {
        let mut space = item_with(decide());
        space.go = Go::FocusSpace(PathBuf::from("/w"));
        let mut tried = item_with(Action {
            id: 5,
            kind: "try".into(),
            ..Default::default()
        });
        tried.go = Go::Try { dir: None, id: 5 };
        let other = space.clone();
        assert_eq!(rest_does(&[space.clone()]), "opens its space");
        assert_eq!(rest_does(&[space, tried, other]), "opens its space or runs its try");
        assert_eq!(rest_does(&[]), "");
    }

    fn merge_item(blockers: &[&str]) -> Item {
        let mut item = item_with(Action {
            id: 2,
            kind: "merge".into(),
            text: "ready from 305".into(),
            ..Default::default()
        });
        item.go = Go::Merge {
            branch: "feature/305-x".into(),
            action: None,
        };
        item.merge = Some(MergeOffer {
            iid: 318,
            url: "https://gitlab.invalid/mr/318".into(),
            is_draft: true,
            can_mark_ready: true,
            blockers: blockers.iter().map(|blocker| blocker.to_string()).collect(),
            ..Default::default()
        });
        item
    }

    #[test]
    fn a_blocked_merge_says_its_click_opens_the_mr_not_that_it_merges() {
        let blocked = merge_item(&["pipeline pending", "GitLab is checking"]);
        let mut waiting = item_with(Action {
            id: 6,
            kind: "try".into(),
            text: "the deploy on test".into(),
            ..Default::default()
        });
        waiting.try_state = Some(TryState {
            label: "waits for merge !316".into(),
            tone: TryTone::Waits,
            is_ready: false,
            url: None,
        });
        assert_eq!(
            note_of(&[blocked.clone(), waiting]),
            "1 for you · 1 waiting · a click on the rest opens its MR in GitLab"
        );
        assert_eq!(does_of(&blocked), Some("opens its MR in GitLab"));
        assert_eq!(
            note_of(std::slice::from_ref(&blocked)),
            "1 for you · a click on the rest opens its MR in GitLab"
        );
        assert_eq!(
            hint_text(&blocked),
            [
                "a click on the rest opens !318 in GitLab: two clicks merge it once its pipeline passes \
              and GitLab has checked it"
            ]
        );
        assert_eq!(pointer_of(&blocked), "↑ in NEXT · a click here opens its MR in GitLab");
        let ready = merge_item(&[]);
        assert_eq!(does_of(&ready), Some("merges on a second click"));
        assert!(hint(&ready).is_empty(), "its text says two clicks merge");
        assert_eq!(pointer_of(&ready), "↑ in NEXT · a click here merges on a second click");
        // Marked ready after its agent posted the merge, GitLab still says draft: not the agent's.
        let drafted = merge_item(&["draft"]);
        assert_eq!(
            hint_text(&drafted),
            ["a click on the rest opens !318 in GitLab: two clicks merge it once GitLab no longer calls it a draft"]
        );
        let other = merge_item(&["needs an approval"]);
        assert_eq!(
            hint_text(&other),
            ["a click on the rest opens !318 in GitLab: two clicks merge it once what the line above names clears"]
        );
        let merged = merge_item(&["it is merged"]);
        assert_eq!(hint_text(&merged), ["a click on the rest opens !318 in GitLab"]);
        let mut gone = merge_item(&[]);
        gone.merge = None;
        assert_eq!(does_of(&gone), None, "a merge gone from the view does nothing");
        assert_eq!(
            cleared(&[
                "1 work box open: x".into(),
                "pipeline failed".into(),
                "open discussions".into()
            ]),
            "its work boxes are ticked, its pipeline is fixed and the rest above clears"
        );
    }

    #[test]
    fn the_pointer_up_names_the_answers_only_where_there_are_some() {
        assert_eq!(pointer_of(&item_with(decide())), "↑ in NEXT: answer it there");
        let mut open_question = item_with(Action {
            id: 10,
            kind: "decide".into(),
            text: "Keep Node running?".into(),
            ..Default::default()
        });
        assert_eq!(pointer_of(&open_question), "↑ in NEXT");
        open_question.go = Go::FocusAgent(PathBuf::from("/w"));
        assert_eq!(
            pointer_of(&open_question),
            "↑ in NEXT · a click here opens its agent's tab: answer it there"
        );
    }

    #[test]
    fn an_answer_waiting_on_a_blocked_agent_says_so() {
        let file = PathBuf::from("/nowhere/blocked/actions.json");
        locked(&TRIED).insert(file.clone(), Some("blocked".into()));
        assert_eq!(
            waits_of(&file, None),
            "its agent is at a question: it goes in once that is answered"
        );
        assert_eq!(waits_of(&file, Some("own".into())), "own");
        locked(&TRIED).insert(file.clone(), Some("idle".into()));
        assert_eq!(waits_of(&file, None), "at its next stop");
        assert_eq!(
            logged("decide #2 of 307-x", "OpenTUI on Bun", "typed"),
            "decide #2 of 307-x → \u{201c}OpenTUI on Bun\u{201d} · typed"
        );
    }

    #[test]
    fn a_plan_keeps_its_empty_lines() {
        let plan = item_with(Action {
            id: 8,
            kind: "plan".into(),
            text: "core first\n\nsee docs/rewrite-plan.md".into(),
            ..Default::default()
        });
        let (text, _) = drawn(&plan, 60);
        assert_eq!(text[1..4], ["core first", "", "see docs/rewrite-plan.md"]);
    }

    #[test]
    fn reads_what_the_user_typed_into_claude_codes_input() {
        let screen = "output\n────\n❯ half a sentence\n────\n  ? for shortcuts";
        assert_eq!(input_in(screen).as_deref(), Some("half a sentence"));
        assert_eq!(input_in("────\n❯ \n────").as_deref(), Some(""));
        assert_eq!(input_in("no prompt here"), None);
    }
}
