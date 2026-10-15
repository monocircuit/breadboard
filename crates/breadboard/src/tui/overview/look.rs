//! How the sections paint core's text: core's paints as styles in herdr's
//! colours (herdr-tones.mjs, names.mjs), words that wrap whole, facts parted
//! by ` · `, a gutter in front of wrapped lines (the tree's stems, NEXT's
//! label, the graph's lanes), and nothing that reads as cut (#304).

use crate::tui::Scene;
use crate::tui::links;
use crate::tui::scene::width_of;
use crate::tui::theme::{self, Tone};
use breadboard_core::herdr;
use breadboard_core::model::try_state::TryTone;
use breadboard_core::model::{self, Fact, Paint};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

/// herdr's colours, read once: its sidebar's pink for names, its theme's for states.
struct Palette {
    pink: Color,
    yellow: Color,
    blue: Color,
    red: Color,
    teal: Color,
}

static PALETTE: OnceLock<Palette> = OnceLock::new();

/// Reads herdr's colours for the repo whose main checkout is `main`.
pub fn init(main: &Path) {
    let _ = PALETTE.set(palette_of(&herdr::config_text(main), &herdr::theme_text()));
}

fn palette_of(config: &str, theme: &str) -> Palette {
    let colours = herdr::colours_of(theme);
    let named = |name: &str, fallback: Color| colours.get(name).and_then(|hex| rgb(hex)).unwrap_or(fallback);
    Palette {
        pink: rgb(&herdr::branch_colour(config)).unwrap_or(Color::Magenta),
        yellow: named("yellow", Color::Yellow),
        blue: named("blue", Color::Blue),
        red: named("red", Color::Red),
        teal: named("teal", Color::Cyan),
    }
}

fn palette() -> &'static Palette {
    PALETTE.get_or_init(|| palette_of("", ""))
}

fn rgb(hex: &str) -> Option<Color> {
    let value = u32::from_str_radix(hex.strip_prefix('#')?.get(..6)?, 16).ok()?;
    Some(Color::Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8))
}

pub fn fg(colour: Color) -> Style {
    Style::new().fg(colour)
}

/// A worktree's name: the pink of herdr's sidebar.
pub fn pink() -> Style {
    fg(palette().pink)
}

/// Whom an agent waits for: work yellow, GitLab blue, you red, nothing teal.
pub fn wait(tone: model::Tone) -> Style {
    let palette = palette();
    fg(match tone {
        model::Tone::Work => palette.yellow,
        model::Tone::Gitlab => palette.blue,
        model::Tone::You => palette.red,
        model::Tone::Free => palette.teal,
    })
}

/// herdr's state of an agent in its colour: working yellow, idle and done teal, blocked red.
pub fn state(status: &str) -> Style {
    let palette = palette();
    match herdr::state_colour(status) {
        Some("yellow") => fg(palette.yellow),
        Some("teal") => fg(palette.teal),
        Some("red") => fg(palette.red),
        _ => theme::dim(),
    }
}

/// A try's state: ready as done, waiting as working, failed as blocked.
pub fn try_tone(tone: TryTone) -> Style {
    state(match tone {
        TryTone::Ready => "done",
        TryTone::Waits => "working",
        TryTone::Failed => "blocked",
    })
}

pub fn style_of(paint: &Paint) -> Style {
    match paint {
        Paint::Plain => Style::new(),
        Paint::Dim => theme::dim(),
        Paint::Bold => theme::bold(),
        Paint::Cyan => theme::tone(Tone::Cyan),
        Paint::Green => theme::tone(Tone::Green),
        Paint::Yellow => theme::tone(Tone::Yellow),
        Paint::Red => theme::tone(Tone::Red),
        Paint::Pink => pink(),
        Paint::Wait(tone) => wait(*tone),
        Paint::State(status) => state(status),
    }
}

/// `…` right after a word reads as text cut short (#304, the suite's CUT):
/// a mark of core's (`checking…`, `resumes 1a2b3c4d…`) gets a space before it.
pub fn uncut(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    let mut last: Option<char> = None;
    for ch in text.chars() {
        if ch == '…' && last.is_some_and(|last| last.is_ascii_alphanumeric() || "_()#".contains(last)) {
            out.push(' ');
        }
        out.push(ch);
        last = Some(ch);
    }
    out
}

/// A span of text in a style, never reading as cut.
pub fn span(text: impl AsRef<str>, style: Style) -> Span<'static> {
    Span::styled(uncut(text.as_ref()), style)
}

pub fn dim(text: impl AsRef<str>) -> Span<'static> {
    span(text, theme::dim())
}

pub fn bold(text: impl AsRef<str>) -> Span<'static> {
    span(text, theme::bold())
}

/// core's pieces of text as spans; a piece with a url opens it on a
/// Ctrl-click (OSC 8, the frame's `links`), as Node's `link`.
pub fn spans(pieces: &[model::Span]) -> Vec<Span<'static>> {
    pieces
        .iter()
        .map(|piece| span(&piece.text, links::style(style_of(&piece.paint), piece.url.as_deref())))
        .collect()
}

/// The words of `spans`, each with its styles: a word ends at a space, never
/// at a no-break one, and goes on across spans.
pub fn words_of(spans: &[Span<'static>]) -> Vec<Vec<Span<'static>>> {
    let mut words = Vec::new();
    let mut word: Vec<Span<'static>> = Vec::new();
    for span in spans {
        let mut text = String::new();
        for ch in span.content.chars() {
            if ch.is_whitespace() && ch != '\u{a0}' {
                if !text.is_empty() {
                    word.push(Span::styled(std::mem::take(&mut text), span.style));
                }
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            } else {
                text.push(ch);
            }
        }
        if !text.is_empty() {
            word.push(Span::styled(text, span.style));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// `spans` as words that wrap whole.
pub fn flow<A: Clone>(scene: &mut Scene<A>, spans: &[Span<'static>]) {
    for word in words_of(spans) {
        scene.atom(word);
    }
}

/// Text in one style as words that wrap whole; its lines stay lines.
pub fn say<A: Clone>(scene: &mut Scene<A>, text: &str, style: Style) {
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            scene.end_line();
        }
        flow(scene, &[span(line, style)]);
    }
}

/// The facts that say something, each as its spans: the parts of `whole`,
/// each whole on a line when it fits one (`untested: 1 commit` never splits).
pub fn parts(facts: &[Fact]) -> Vec<Vec<Span<'static>>> {
    facts
        .iter()
        .filter(|fact| !model::text_of(fact).trim().is_empty())
        .map(|fact| spans(fact))
        .collect()
}

/// Parts parted by a dim ` · `, each whole on a line when `room` holds it,
/// else at its words; the separator ends the line before a break, never starts one.
pub fn whole<A: Clone>(scene: &mut Scene<A>, parts: &[Vec<Span<'static>>], room: u16) {
    for (i, part) in parts.iter().enumerate() {
        let mut atom = part.clone();
        if i + 1 < parts.len() {
            atom.push(dim(" ·"));
        }
        if width_of(&atom) <= room {
            scene.atom(atom);
        } else {
            for word in words_of(&atom) {
                scene.atom(word);
            }
        }
    }
}

/// Plain texts parted by a dim ` · `, as `facts`.
pub fn parted<A: Clone>(scene: &mut Scene<A>, parts: &[Vec<Span<'static>>]) {
    let parts: Vec<&Vec<Span<'static>>> = parts.iter().filter(|part| !part.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let mut words = words_of(part);
        if i + 1 < parts.len()
            && let Some(last) = words.last_mut()
        {
            last.push(dim(" ·"));
        }
        for word in words {
            scene.atom(word);
        }
    }
}

/// What `build` writes, on lines that start with `first` (the first line)
/// and `rest` (the lines after): a tree's stems, NEXT's label, an MR's
/// lanes. The two are padded to one width; buttons and blocks keep working.
pub fn gutter<A: Clone>(
    scene: &mut Scene<A>,
    first: Vec<Span<'static>>,
    rest: Vec<Span<'static>>,
    build: impl FnOnce(&mut Scene<A>),
) {
    let width = width_of(&first).max(width_of(&rest));
    if width + 8 > scene.width() {
        // No room for a gutter: the text alone.
        scene.end_line();
        build(scene);
        scene.end_line();
        return;
    }
    let mut inner = Scene::new(scene.width() - width);
    build(&mut inner);
    inner.finish();
    scene.end_line();
    let top = scene.y();
    let pad = |mut spans: Vec<Span<'static>>, style: Style| {
        let short = width - width_of(&spans);
        if short > 0 {
            spans.push(Span::styled(" ".repeat(short as usize), style));
        }
        spans
    };
    for (i, line) in inner.lines.into_iter().enumerate() {
        let mut spans = if i == 0 {
            pad(first.clone(), Style::new())
        } else {
            pad(rest.clone(), Style::new())
        };
        spans.extend(line.spans);
        scene.lines.push(Line::from(spans));
    }
    for mut region in inner.regions {
        region.area.y += top;
        region.area.x += width;
        scene.regions.push(region);
    }
    if let Some(after) = inner.redraw_in {
        scene.redraw_in(after);
    }
}

/// `5 min ago` moves on: the page is drawn again at the next minute.
pub fn ticks<A: Clone>(scene: &mut Scene<A>) {
    scene.redraw_in(Duration::from_secs(60));
}

/// The tone of a kind's chip (action-block.mjs): merge green; starting
/// things cyan; what needs a thought yellow; what is broken red; the rest plain.
pub fn kind_tone(kind: &str) -> Tone {
    match kind {
        "merge" => Tone::Green,
        "try" | "browser-check" | "plan" | "hand over" | "start" | "clean up" => Tone::Cyan,
        "decide" | "waits" | "permission" => Tone::Yellow,
        "end" | "pipeline" | "push" => Tone::Red,
        _ => Tone::Gray,
    }
}

/// A kind as its chip says it: `browser-check` is a try.
pub fn kind_label(kind: &str) -> &str {
    if kind == "browser-check" { "try" } else { kind }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(scene: &Scene<u8>) -> Vec<String> {
        scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn a_mark_after_a_word_gets_a_space() {
        assert_eq!(uncut("checking for conflicts…"), "checking for conflicts …");
        assert_eq!(uncut("resumes 1a2b3c4d…"), "resumes 1a2b3c4d …");
        assert_eq!(uncut("… pending"), "… pending");
        assert_eq!(uncut("removing …"), "removing …");
    }

    #[test]
    fn facts_stay_whole_with_the_separator_at_the_end() {
        let mut scene = Scene::<u8>::new(20);
        let facts = parts(&[
            vec![model::Span::plain("#305"), model::Span::plain(" open")],
            vec![model::Span::plain("untested: 1 commit")],
            vec![model::Span::plain("waits for !316, !314, !318")],
        ]);
        whole(&mut scene, &facts, 20);
        scene.finish();
        // A fact wider than a line wraps at its words.
        assert_eq!(
            text(&scene),
            ["#305 open ·", "untested: 1 commit ·", "waits for !316,", "!314, !318"]
        );
    }

    #[test]
    fn a_gutter_starts_every_line_and_moves_the_buttons() {
        let mut scene = Scene::<u8>::new(20);
        scene.text("head", Style::new());
        gutter(&mut scene, vec![Span::raw("├─ ")], vec![Span::raw("│")], |scene| {
            scene.text("one two three four", Style::new());
            scene.button("go", Tone::Cyan, 7);
        });
        scene.finish();
        assert_eq!(text(&scene), ["head", "├─ one two three", "│  four  go "]);
        assert_eq!(scene.regions[0].area, ratatui::layout::Rect::new(8, 2, 4, 1));
    }
}
