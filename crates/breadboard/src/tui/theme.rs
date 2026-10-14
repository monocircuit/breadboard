//! How the overview looks: the colours of the Node overview (lib/tty.mjs,
//! overview/boxes.mjs, lib/mouse.mjs), so the two read the same side by side.
//! Every section is an island, a dark grey a step above the terminal's black,
//! its title on a lighter grey; the title band uses that grey too.

use ratatui::style::{Color, Modifier, Style};

/// The ground of an island.
pub const ISLAND: Color = Color::Rgb(38, 38, 38);
/// The ground of an island's title line and of the title band: herdr's tab strip.
pub const HEAD: Color = Color::Rgb(58, 58, 58);
/// A clickable block under the pointer: a step lighter than the island.
pub const HOVER: Color = Color::Rgb(54, 54, 54);
/// The block picked with Tab (Enter runs it): a muted blue, as lazygit's
/// selected row, apart from the hover's grey, as dark, so dim text reads as on it.
pub const PICKED: Color = Color::Rgb(30, 52, 88);
/// A title line under the pointer: a step lighter than its own.
pub const HOVER_HEAD: Color = Color::Rgb(74, 74, 74);
/// The band's text: light on its grey, 9:1.
pub const BAND_TEXT: Color = Color::Rgb(235, 235, 235);
/// Status and keys in the band: quieter, still 4.9:1 on the grey.
pub const BAND_QUIET: Color = Color::Rgb(170, 170, 170);
/// A chip's ground: a step above the island's, so it never reads as a button.
const CHIP: Color = Color::Rgb(70, 70, 70);

/// The logo in half blocks, one string per line, five columns
/// each (overview/logo.mjs: drawn by hand, a rasterizer drops the triangle).
pub const LOGO: [&str; 3] = ["▀▀▀▀▀", "▀▀▀▀▀", "▀ 🭭 ▀"];

/// The tone of a button, its ground: cyan starts something, green confirms,
/// red deletes, yellow runs, gray can't be clicked yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Cyan,
    Green,
    Red,
    Yellow,
    Gray,
}

impl Tone {
    /// A button: bold dark text on the tone (red: light text, 4.5:1 needs it).
    pub fn button(self) -> Style {
        let (fg, bg) = match self {
            Tone::Cyan => (Color::Black, Color::Cyan),
            Tone::Green => (Color::Black, Color::Green),
            Tone::Red => (Color::Gray, Color::Red),
            Tone::Yellow => (Color::Black, Color::Yellow),
            Tone::Gray => (Color::Gray, Color::DarkGray),
        };
        let style = Style::new().fg(fg).bg(bg);
        if self == Tone::Gray {
            style.add_modifier(Modifier::DIM)
        } else {
            style.add_modifier(Modifier::BOLD)
        }
    }

    /// A button under the pointer: its tone's bright ground, its own highlight
    /// apart from the block's.
    pub fn lit(self) -> Style {
        let bg = match self {
            Tone::Cyan => Color::LightCyan,
            Tone::Green => Color::LightGreen,
            Tone::Red => Color::LightRed,
            Tone::Yellow => Color::LightYellow,
            Tone::Gray => return self.button(),
        };
        self.button().bg(bg)
    }

    /// The tone as a text colour: chips and marks.
    pub fn fg(self) -> Color {
        match self {
            Tone::Cyan => Color::Cyan,
            Tone::Green => Color::Green,
            Tone::Red => Color::Red,
            Tone::Yellow => Color::Yellow,
            Tone::Gray => Color::Gray,
        }
    }

    /// A chip: a label in the tone on a grey a step above the island, not bold.
    pub fn chip(self) -> Style {
        Style::new().fg(self.fg()).bg(CHIP)
    }
}

pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

pub fn tone(tone: Tone) -> Style {
    Style::new().fg(tone.fg())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lit_button_differs_from_the_button_and_the_block() {
        for tone in [Tone::Cyan, Tone::Green, Tone::Red, Tone::Yellow] {
            assert_ne!(tone.lit().bg, tone.button().bg);
            assert_ne!(tone.lit().bg, Some(HOVER));
        }
        assert_eq!(Tone::Gray.lit(), Tone::Gray.button());
    }

    #[test]
    fn the_logo_is_five_columns_wide() {
        for line in LOGO {
            assert_eq!(ratatui::text::Span::raw(line).width(), 5, "{line}");
        }
    }
}
