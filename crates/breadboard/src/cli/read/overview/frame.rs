//! The printed overview as lines (boxes.mjs): the title band with the logo,
//! sections in boxes, without colour rounded borders, with colour islands on
//! a dark grey as the live view draws them. Rows are wrapped at words, never
//! cut with `…` (#304). A section is drawn on its own: one that breaks on bad
//! data costs its box, not the view (#306).

use super::paint::{self, RESET, bold, dim, is_colour, pad, red, visible, wrap};
use std::panic::{self, AssertUnwindSafe};

/// The logo of the title band (logo.mjs), five columns by three lines.
pub const LOGO: [&str; 3] = ["▀▀▀▀▀", "▀▀▀▀▀", "▀ 🭭 ▀"];
/// The background of an island: a dark grey a step above the terminal's black.
const ISLAND: &str = "\x1b[48;2;38;38;38m";
/// The background of an island's title line and of the band: herdr's tab strip.
const ISLAND_HEAD: &str = "\x1b[48;2;58;58;58m";
/// The band's text, light on its grey; its time quieter.
const BAND_TEXT: &str = "\x1b[38;2;235;235;235m";
const BAND_QUIET: &str = "\x1b[38;2;170;170;170m";

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
}

/// The text being drawn, `width` columns wide.
pub struct Frame {
    width: usize,
    lines: Vec<String>,
}

impl Frame {
    pub fn new(width: usize) -> Frame {
        Frame {
            width: width.max(40),
            lines: Vec::new(),
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// How wide a line inside a box is: the island's margin, or the border and a blank.
    pub fn inner(&self) -> usize {
        self.width - if is_colour() { 2 } else { 4 }
    }

    pub fn blank(&mut self) {
        self.lines.push(String::new());
    }

    /// A painted line as it is; one wider than the frame wraps.
    pub fn push(&mut self, line: impl Into<String>) {
        let line = line.into();
        if visible(&line) <= self.width {
            self.lines.push(line);
        } else {
            self.lines.extend(wrap(&line, self.width));
        }
    }

    /// A painted text outside the boxes from column `indent`, wrapped there.
    pub fn text_at(&mut self, indent: usize, text: &str) {
        let pad = " ".repeat(indent);
        for line in wrap(text, self.width.saturating_sub(indent).max(1)) {
            self.lines.push(format!("{pad}{line}"));
        }
    }

    /// A row outside the boxes, wrapped to the whole width.
    pub fn line(&mut self, row: Row) {
        let lines = lay_out(&row, self.width);
        self.lines.extend(lines);
    }

    /// The title band (boxes.mjs `titleLines`): the logo's height and a line
    /// above and below it, the logo at the right end, the name on the left of
    /// its middle line, `right` (the time) on the right of its first. Without
    /// room for name and logo it drops the logo.
    pub fn band(&mut self, left: &str, right: &str) {
        let art_width = visible(LOGO[0]);
        let logo_room = art_width + 3;
        let name = paint::strip(left);
        let has_logo = self.width.saturating_sub(2 + logo_room) >= visible(&name);
        let inner = self.width - 2 - if has_logo { logo_room } else { 0 };
        let rows = [("", ""), ("", right), (name.as_str(), ""), ("", "")];
        for at in 0..LOGO.len() + 2 {
            let (left, right) = rows.get(at).copied().unwrap_or(("", ""));
            let (left, gap, right) = spread(left, right, inner);
            let mark = if has_logo {
                let art = at
                    .checked_sub(1)
                    .and_then(|at| LOGO.get(at))
                    .map(|line| line.to_string())
                    .unwrap_or_else(|| " ".repeat(art_width));
                format!("  {art} ")
            } else {
                String::new()
            };
            let line = if is_colour() {
                format!(
                    "{ISLAND_HEAD}{BAND_TEXT} \x1b[1m{left}\x1b[22m{gap}{BAND_QUIET}{right}{BAND_TEXT}{mark} {RESET}"
                )
            } else {
                format!(" {left}{gap}{right}{mark} ")
            };
            self.lines.push(line);
        }
    }

    /// A section in its box: `title` (painted) in its head, the lines `body`
    /// gives for the inner width below it. `body` runs guarded: a panic in it
    /// draws one line in the box that says so, named `name`.
    pub fn boxed(&mut self, name: &str, title: &str, body: impl FnOnce(usize) -> Vec<String>) {
        let inner = self.inner();
        let lines = match guarded(|| body(inner)) {
            Ok(lines) => lines,
            Err(message) => vec![red(&format!("{name} could not be shown: {message}"))],
        };
        let lines: Vec<String> = lines
            .into_iter()
            .flat_map(|line| {
                if visible(&line) <= inner {
                    vec![line]
                } else {
                    wrap(&line, inner)
                }
            })
            .collect();
        let heads = title_lines(title, inner, if is_colour() { inner } else { self.width - 5 });
        if is_colour() {
            for head in &heads {
                self.lines
                    .push(on_island(&format!(" {} ", pad(head, inner)), ISLAND_HEAD));
            }
            for line in lines {
                self.lines.push(on_island(&format!(" {} ", pad(&line, inner)), ISLAND));
            }
            self.lines.push(String::new());
            return;
        }
        let (first, spill) = heads
            .split_first()
            .map(|(first, rest)| (first.clone(), rest.to_vec()))
            .unwrap_or_default();
        let head = format!(" {first} ");
        self.lines.push(format!(
            "{}{head}{}",
            dim("╭─"),
            dim(&format!(
                "{}╮",
                "─".repeat(self.width.saturating_sub(3 + visible(&head)))
            ))
        ));
        let side = dim("│");
        for line in spill.into_iter().chain(lines) {
            self.lines.push(format!("{side} {} {side}", pad(&line, inner)));
        }
        self.lines.push(dim(&format!("╰{}╯", "─".repeat(self.width - 2))));
    }

    /// A section of rows (the views of a worktree and of everything): `name`
    /// bold, `summary` dim after it.
    pub fn section(&mut self, name: &str, summary: &str, rows: impl FnOnce() -> Vec<Row>) {
        let title = if summary.is_empty() {
            bold(name)
        } else {
            format!("{} {}", bold(name), dim(summary))
        };
        self.boxed(name, &title, |inner| {
            rows().iter().flat_map(|row| lay_out(row, inner)).collect()
        });
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(|line| format!("{line}\n")).collect()
    }
}

/// A line on an island's ground, kept after every reset inside it.
fn on_island(line: &str, ground: &str) -> String {
    format!("{ground}{}{RESET}", line.replace(RESET, &format!("{RESET}{ground}")))
}

/// The lines of a box's title: as many of its ` · ` parts as fit `first`
/// columns on the first, the rest wrapped at `rest` below (never cut).
fn title_lines(title: &str, rest: usize, first: usize) -> Vec<String> {
    if visible(title) <= first {
        return vec![title.to_string()];
    }
    let mut lines = wrap(title, first);
    if lines.len() > 1 {
        let tail: Vec<String> = lines.split_off(1);
        lines.extend(wrap(&tail.join(" "), rest));
    }
    lines
}

/// `left` on the left and `right` on the right of `width` columns, plain;
/// what doesn't fit is cut from the right one first (boxes.mjs `spread`).
fn spread(left: &str, right: &str, width: usize) -> (String, String, String) {
    let lefts: Vec<char> = left.chars().collect();
    let rights: Vec<char> = right.chars().collect();
    let room = width.saturating_sub(lefts.len() + if !left.is_empty() && !right.is_empty() { 2 } else { 0 });
    let shown_right: String = rights.iter().take(room).collect();
    let right_count = shown_right.chars().count();
    let shown_left: String = lefts.iter().take(width.saturating_sub(right_count)).collect();
    let gap = " ".repeat(width.saturating_sub(shown_left.chars().count() + right_count));
    (shown_left, gap, shown_right)
}

/// `run`, with a panic turned into its message (and kept off stderr).
pub fn guarded<T>(run: impl FnOnce() -> T) -> Result<T, String> {
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

/// The lines of a row `width` wide: its words wrapped, the lines after the
/// first from `hang`, `right` flush right or on a line of its own.
pub fn lay_out(row: &Row, width: usize) -> Vec<String> {
    let text = match row.tone {
        Tone::Plain => row.text.clone(),
        Tone::Dim => dim(&row.text),
        Tone::Strong => bold(&row.text),
    };
    let mut lines = paint::hanging(&" ".repeat(row.indent), &text, width, row.hang);
    if let Some(right) = &row.right {
        let last = lines.pop().unwrap_or_default();
        let used = visible(&last);
        if used + 2 + visible(right) <= width {
            lines.push(format!("{last}{}{right}", " ".repeat(width - used - visible(right))));
        } else {
            lines.push(last);
            lines.push(format!("{}{right}", " ".repeat(width.saturating_sub(visible(right)))));
        }
    }
    lines.into_iter().map(|line| line.trim_end().to_string()).collect()
}

/// Rows of cells as a table: every column but the last padded to its widest,
/// two spaces between; and where the last column starts, for wrapped lines.
pub fn table(rows: &[Vec<String>]) -> (Vec<String>, usize) {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| visible(cell))
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
                    if column + 1 == row.len() {
                        cell.clone()
                    } else {
                        pad(cell, widths[column])
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

#[cfg(test)]
mod tests {
    use super::super::paint::tests::coloured;
    use super::*;

    #[test]
    fn wraps_at_words_with_its_hang_and_never_cuts() {
        let row = Row::new(1, "one two three four five").hang(3);
        assert_eq!(lay_out(&row, 10), vec![" one two", "   three", "   four", "   five"]);
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
        let row = Row {
            right: Some("right".into()),
            ..Row::new(0, "left")
        };
        assert_eq!(lay_out(&row, 12), vec!["left   right"]);
        assert_eq!(lay_out(&row, 9), vec!["left", "    right"]);
    }

    #[test]
    fn a_section_that_panics_costs_its_box_only() {
        let mut frame = Frame::new(40);
        frame.section("ACTIONS", "", || panic!("no id"));
        frame.section("WORKTREES", "2", || vec![Row::new(0, "a")]);
        let text = frame.text();
        assert!(text.contains("│ ACTIONS could not be shown: no id"), "{text}");
        assert!(text.contains("╭─ WORKTREES 2 ─"), "{text}");
        assert!(text.lines().all(|line| visible(line) == 40), "{text}");
    }

    #[test]
    fn a_long_title_goes_on_inside_the_box() {
        let mut frame = Frame::new(40);
        frame.boxed(
            "X",
            "WORKTREES 7 wait for GitLab · 1 will be removed · 41 removed",
            |_| vec![],
        );
        let text = frame.text();
        assert!(text.contains("╭─ WORKTREES 7 wait for GitLab · 1 ────╮"), "{text}");
        assert!(text.contains("│ will be removed · 41 removed         │"), "{text}");
    }

    #[test]
    fn the_band_has_the_time_name_and_logo() {
        let mut frame = Frame::new(40);
        frame.band("⇶ x", "10:33:14 PM");
        let lines: Vec<String> = frame.text().lines().map(String::from).collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[1], format!(" {}10:33:14 PM  ▀▀▀▀▀  ", " ".repeat(19)));
        assert!(lines[2].starts_with(" ⇶ x "), "{lines:?}");
        assert!(lines[3].ends_with("  ▀ 🭭 ▀  "), "{lines:?}");
        assert!(lines.iter().all(|line| visible(line) == 40), "{lines:?}");
    }

    #[test]
    fn with_colour_a_box_is_an_island() {
        coloured(|| {
            let mut frame = Frame::new(40);
            frame.boxed("A", &bold("A"), |_| vec![green_line()]);
            let lines: Vec<String> = frame.text().lines().map(String::from).collect();
            assert!(lines[0].starts_with(ISLAND_HEAD), "{lines:?}");
            assert!(lines[1].starts_with(ISLAND), "{lines:?}");
            assert_eq!(visible(&lines[1]), 40);
            assert_eq!(lines[2], "");
        });
    }

    fn green_line() -> String {
        paint::green("ok")
    }
}
