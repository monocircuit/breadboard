//! The boxes of an issue as a list under an MR's row or a worktree's node
//! (#346): folded behind `▸` by default, opened by a click or Enter on the
//! line that says how many there are. Grouped by section, each box `☑` or
//! `☐` with its whole text wrapped under it, then how it is verified
//! (#309's mark), a try with its answer.

use super::{Act, look};
use crate::tui::Scene;
use crate::tui::theme::{self, Tone};
use breadboard_core::model::boxes::{BoxSection, Check};
use ratatui::style::Style;
use ratatui::text::Span;

/// The fold's mark in front of what it folds: `▸` folded, `▾` open.
pub fn mark(is_open: bool) -> Span<'static> {
    look::dim(if is_open { "▾ " } else { "▸ " })
}

/// Every box of `sections` from column `at`, each section's heading over its
/// boxes; a box's lines wrap under its text, not under its `☐`.
pub fn list(scene: &mut Scene<Act>, sections: &[BoxSection], at: u16) {
    for section in sections {
        if !section.heading.is_empty() {
            scene.end_line();
            scene.pad_to(at);
            scene.indent(at);
            look::say(scene, &section.heading, theme::dim());
            scene.end_line();
        }
        for line in &section.boxes {
            scene.end_line();
            scene.indent(at);
            scene.pad_to(at);
            let (tick, text) = if line.is_ticked {
                (look::span("☑", theme::tone(Tone::Green)), theme::dim())
            } else {
                (look::span("☐", theme::tone(Tone::Yellow)), Style::new())
            };
            scene.atom_gap(vec![tick], 0);
            scene.indent(at + 2);
            // The text as words, its check one atom after a `·` that ends the
            // line before a break, never starts one.
            let mut words = look::words_of(&[look::span(&line.text, text)]);
            if let Some(check) = &line.check
                && let Some(last) = words.last_mut()
            {
                last.push(look::dim(" ·"));
                words.push(check_spans(check));
            }
            for word in words {
                scene.atom(word);
            }
            scene.indent(0);
            scene.end_line();
        }
    }
}

/// `test: a.rs › it works`, `try #12 looks good`: dim, a try's answer in
/// the colour of where it stands; one atom, broken only when wider than a line.
fn check_spans(check: &Check) -> Vec<Span<'static>> {
    let mut spans = vec![look::dim(&check.text)];
    if let Some(answer) = &check.answer {
        let tone = match answer.as_str() {
            "looks good" => Tone::Green,
            "broken" => Tone::Red,
            _ => Tone::Yellow,
        };
        spans.push(look::span(format!(" {answer}"), theme::tone(tone)));
    }
    spans
}
