//! The printed overview as lines of text: the title band, sections in rounded
//! boxes, rows wrapped at words (never cut with `…`, #304), colour only when
//! asked. A section is drawn on its own: one that breaks on bad data costs its
//! box, not the view (#306).

use ratatui::text::Span;
use std::panic::{self, AssertUnwindSafe};

/// How a row reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    /// Secondary: details under a row, hints.
    Dim,
    /// What needs the reader: an open action, a failure.
    Strong,
}

/// A row of a section: `text` from `indent`, wrapped lines from `hang`;
/// `right` at the right edge when it fits next to the text, else on a line of
/// its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub indent: usize,
    pub hang: usize,
    pub text: String,
    pub right: Option<String>,
    pub tone: Tone,
}

impl Row {
    pub fn new(indent: usize, text: impl Into<String>) -> Row {
        Row {
            indent,
            hang: indent,
            text: text.into(),
            right: None,
            tone: Tone::Plain,
        }
    }
    pub fn dim(indent: usize, text: impl Into<String>) -> Row {
        Row {
            tone: Tone::Dim,
            ..Row::new(indent, text)
        }
    }
    pub fn strong(mut self) -> Row {
        self.tone = Tone::Strong;
        self
    }
    pub fn hang(mut self, hang: usize) -> Row {
        self.hang = hang;
        self
    }
    pub fn right(mut self, right: impl Into<String>) -> Row {
        let right = right.into();
        self.right = (!right.is_empty()).then_some(right);
        self
    }
}

/// The text being drawn, `width` columns wide.
pub struct Frame {
    width: usize,
    is_colour: bool,
    lines: Vec<String>,
}

/// Columns a text takes on screen.
pub fn width_of(text: &str) -> usize {
    Span::raw(text).width()
}

impl Frame {
    pub fn new(width: usize, is_colour: bool) -> Frame {
        Frame {
            width: width.max(40),
            is_colour,
            lines: Vec::new(),
        }
    }

    pub fn blank(&mut self) {
        self.lines.push(String::new());
    }

    /// A row outside the boxes, wrapped to the whole width.
    pub fn line(&mut self, row: Row) {
        for line in lay_out(&row, self.width) {
            let painted = self.paint(&line, row.tone);
            self.lines.push(painted);
        }
    }

    /// The title band: the view's name, bold.
    pub fn band(&mut self, title: &str) {
        self.blank();
        let row = Row::new(1, title).strong();
        self.line(row);
        self.blank();
    }

    /// A section in its box: `name` bold in the top border, `summary` after it.
    /// `rows` runs guarded: a panic in it draws one line that says so.
    pub fn section(&mut self, name: &str, summary: &str, rows: impl FnOnce() -> Vec<Row>) {
        let rows = match guarded(rows) {
            Ok(rows) => rows,
            Err(message) => vec![Row::new(0, format!("{name} could not be shown: {message}")).strong()],
        };
        let inner = self.width - 4;
        let head = if summary.is_empty() {
            name.to_string()
        } else {
            format!("{name} {summary}")
        };
        let (top_text, spill) = if width_of(&head) + 5 <= self.width {
            (head, None)
        } else {
            (name.to_string(), Some(summary.to_string()))
        };
        let fill = self.width.saturating_sub(width_of(&top_text) + 5);
        let top = format!(
            "{}{}{}",
            self.paint("╭─ ", Tone::Dim),
            self.paint(&top_text, Tone::Strong),
            self.paint(&format!(" {}╮", "─".repeat(fill)), Tone::Dim)
        );
        self.lines.push(top);
        let side = self.paint("│", Tone::Dim);
        for row in spill.map(|summary| Row::dim(0, summary)).into_iter().chain(rows) {
            for line in lay_out(&row, inner) {
                let gap = " ".repeat(inner.saturating_sub(width_of(&line)));
                let painted = self.paint(&line, row.tone);
                self.lines.push(format!("{side} {painted}{gap} {side}"));
            }
        }
        self.lines
            .push(self.paint(&format!("╰{}╯", "─".repeat(self.width - 2)), Tone::Dim));
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(|line| format!("{line}\n")).collect()
    }

    fn paint(&self, text: &str, tone: Tone) -> String {
        match (self.is_colour, tone) {
            (false, _) | (_, Tone::Plain) => text.to_string(),
            (true, Tone::Dim) => format!("\x1b[2m{text}\x1b[0m"),
            (true, Tone::Strong) => format!("\x1b[1m{text}\x1b[0m"),
        }
    }
}

/// `run`, with a panic turned into its message (and kept off stderr).
fn guarded<T>(run: impl FnOnce() -> T) -> Result<T, String> {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let result = panic::catch_unwind(AssertUnwindSafe(run));
    panic::set_hook(hook);
    result.map_err(|payload| {
        payload
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "it broke on its data".into())
    })
}

/// The lines of a row `width` wide: its words wrapped (the spaces between
/// them kept, dropped where a line breaks), a word longer than a line broken
/// at the edge, `right` flush right.
pub fn lay_out(row: &Row, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = " ".repeat(row.indent);
    let mut is_fresh = true;
    for (gap, word) in words_of(&row.text) {
        let room = width.saturating_sub(width_of(&line));
        if !is_fresh && gap + width_of(word) > room {
            lines.push(std::mem::replace(&mut line, " ".repeat(row.hang)));
            is_fresh = true;
        }
        if !is_fresh || lines.is_empty() {
            line += &" ".repeat(gap);
        }
        // A word wider than a line of its own: broken where the line ends.
        let mut rest = word;
        while width_of(&line) + width_of(rest) > width && width_of(&line) < width {
            let cut = cut_at(rest, width - width_of(&line));
            if cut == 0 {
                break;
            }
            line.push_str(&rest[..cut]);
            lines.push(std::mem::replace(&mut line, " ".repeat(row.hang)));
            rest = &rest[cut..];
        }
        line.push_str(rest);
        is_fresh = false;
    }
    if let Some(right) = &row.right {
        let used = width_of(&line);
        if used + 2 + width_of(right) <= width {
            line += &" ".repeat(width - used - width_of(right));
            line += right;
        } else {
            lines.push(line);
            line = format!("{}{right}", " ".repeat(width.saturating_sub(width_of(right))));
        }
    }
    lines.push(line);
    lines.into_iter().map(|line| line.trim_end().to_string()).collect()
}

/// The words of a text, each with the spaces before it.
fn words_of(text: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut gap = 0;
    for part in text.split(' ') {
        if part.is_empty() {
            gap += 1;
        } else {
            words.push((gap, part));
            gap = 1;
        }
    }
    words
}

/// Rows of cells as a table: every column but the last padded to its widest,
/// two spaces between; and where the last column starts, for wrapped lines.
pub fn table(rows: &[Vec<String>]) -> (Vec<String>, usize) {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| width_of(cell))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let lines = rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(column, cell)| {
                    let is_last = column + 1 == row.len();
                    if is_last {
                        cell.clone()
                    } else {
                        format!("{cell}{}", " ".repeat(widths[column] - width_of(cell)))
                    }
                })
                .collect();
            cells.join("  ")
        })
        .collect();
    let hang = widths
        .iter()
        .take(columns.saturating_sub(1))
        .map(|width| width + 2)
        .sum();
    (lines, hang)
}

/// The byte index where `text` fills `columns`.
fn cut_at(text: &str, columns: usize) -> usize {
    let mut used = 0;
    for (at, character) in text.char_indices() {
        used += width_of(&character.to_string());
        if used > columns {
            return at;
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_words_with_its_hang_and_never_cuts() {
        let row = Row::new(1, "one two three four five").hang(3);
        assert_eq!(lay_out(&row, 10), vec![" one two", "   three", "   four", "   five"]);
        let spaced = Row::new(0, "│  #3  ·  a b").hang(4);
        assert_eq!(lay_out(&spaced, 9), vec!["│  #3  ·", "    a b"]);
        let long = Row::new(0, "abcdefghijkl");
        assert_eq!(lay_out(&long, 5), vec!["abcde", "fghij", "kl"]);
    }

    #[test]
    fn lines_cells_up_in_columns() {
        let rows = vec![
            vec!["!3".to_string(), "draft".into(), "a".into()],
            vec!["!311".into(), "ready".into(), "b".into()],
        ];
        assert_eq!(
            table(&rows),
            (vec!["!3    draft  a".to_string(), "!311  ready  b".into()], 13)
        );
    }

    #[test]
    fn puts_the_right_text_on_the_edge_or_below() {
        let row = Row::new(0, "left").right("right");
        assert_eq!(lay_out(&row, 12), vec!["left   right"]);
        assert_eq!(lay_out(&row, 9), vec!["left", "    right"]);
    }

    #[test]
    fn a_section_that_panics_costs_its_box_only() {
        let mut frame = Frame::new(40, false);
        frame.section("ACTIONS", "", || panic!("no id"));
        frame.section("WORKTREES", "2", || vec![Row::new(0, "a")]);
        let text = frame.text();
        assert!(text.contains("│ ACTIONS could not be shown: no id"), "{text}");
        assert!(text.contains("╭─ WORKTREES 2 ─"), "{text}");
        assert!(text.lines().all(|line| width_of(line) <= 40), "{text}");
        assert!(
            text.lines()
                .filter(|line| line.starts_with('│'))
                .all(|line| width_of(line) == 40),
            "{text}"
        );
    }

    #[test]
    fn colours_only_when_asked() {
        let mut frame = Frame::new(40, true);
        frame.line(Row::new(0, "x").strong());
        assert_eq!(frame.text(), "\x1b[1mx\x1b[0m\n");
    }
}
