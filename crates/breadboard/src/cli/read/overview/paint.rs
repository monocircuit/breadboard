//! Painted text for the printed views (lib/tty.mjs): colours and OSC 8 links
//! only when colour is on, the columns a painted line takes, and wrapping at
//! words that carries a colour or a link over the break, so every line stands
//! on its own. Nothing is cut with `…` (#304): a text too long for its line
//! goes on over the next ones.

use ratatui::text::Span;
use std::cell::Cell;

thread_local! {
    static IS_COLOUR: Cell<bool> = const { Cell::new(false) };
}

/// Colour on or off for what this thread paints (the views print from one thread).
pub fn set_colour(is_colour: bool) {
    IS_COLOUR.with(|cell| cell.set(is_colour));
}

pub fn is_colour() -> bool {
    IS_COLOUR.with(Cell::get)
}

pub const RESET: &str = "\x1b[0m";
/// The end of an OSC 8 link.
const LINK_END: &str = "\x1b]8;;\x1b\\";

fn paint(code: &str, text: &str) -> String {
    if is_colour() {
        format!("\x1b[{code}m{text}{RESET}")
    } else {
        text.to_string()
    }
}

pub fn bold(text: &str) -> String {
    paint("1", text)
}
pub fn dim(text: &str) -> String {
    paint("2", text)
}
pub fn red(text: &str) -> String {
    paint("31", text)
}
pub fn green(text: &str) -> String {
    paint("32", text)
}
pub fn yellow(text: &str) -> String {
    paint("33", text)
}
pub fn blue(text: &str) -> String {
    paint("34", text)
}
pub fn cyan(text: &str) -> String {
    paint("36", text)
}

/// `text` in a colour of herdr's config, `#FF79C6`: 24 bits, as herdr paints its sidebar.
pub fn hex(colour: &str, text: &str) -> String {
    let part = |at: usize| u8::from_str_radix(colour.get(at..at + 2).unwrap_or("ff"), 16).unwrap_or(255);
    paint(&format!("38;2;{};{};{}", part(1), part(3), part(5)), text)
}

/// An OSC 8 hyperlink: Ctrl-click opens it, in herdr and most terminals.
pub fn link(url: Option<&str>, text: &str) -> String {
    match url.filter(|url| !url.is_empty()) {
        Some(url) if is_colour() => format!("\x1b]8;;{url}\x1b\\{text}{LINK_END}"),
        _ => text.to_string(),
    }
}

/// A button: a coloured chip, `[ label ]` without colours, `( label )` for a
/// grey one (it can't be clicked yet). `tone` is the ground: cyan starts
/// something, red deletes, yellow runs, green finishes, gray can't yet.
pub fn button(label: &str, tone: &str) -> String {
    let code = match tone {
        "green" => "1;30;42",
        "red" => "1;37;41",
        "yellow" => "1;30;43",
        "gray" => "2;37;100",
        _ => "1;30;46",
    };
    if is_colour() {
        format!("\x1b[{code}m {label} {RESET}")
    } else if tone == "gray" {
        format!("( {label} )")
    } else {
        format!("[ {label} ]")
    }
}

/// A chip: a label on a grey a step above the island's, in its tone's
/// colour, never read as a button; the label alone without colours.
pub fn chip(label: &str, tone: &str) -> String {
    let code = match tone {
        "cyan" => 36,
        "green" => 32,
        "red" => 31,
        "yellow" => 33,
        _ => 37,
    };
    if is_colour() {
        format!("\x1b[{code};48;2;70;70;70m {label} {RESET}")
    } else {
        label.to_string()
    }
}

/// The pieces of a painted text: text, or an escape (a colour or a link).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Piece<'a> {
    Text(&'a str),
    Escape(&'a str),
}

/// A painted text cut at its escapes: `ESC [ … m` and `ESC ] 8 ; ; … ESC \`.
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('\x1b') {
        if at > 0 {
            pieces.push(Piece::Text(&rest[..at]));
        }
        let tail = &rest[at..];
        let length = if tail.starts_with("\x1b[") {
            tail.find('m').map(|end| end + 1)
        } else if tail.starts_with("\x1b]") {
            tail.find("\x1b\\").map(|end| end + 2)
        } else {
            None
        };
        match length {
            Some(length) => {
                pieces.push(Piece::Escape(&tail[..length]));
                rest = &tail[length..];
            }
            None => {
                pieces.push(Piece::Text(&tail[..1]));
                rest = &tail[1..];
            }
        }
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    pieces
}

/// The text without its colours and links.
pub fn strip(text: &str) -> String {
    pieces(text)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Text(text) => Some(text),
            Piece::Escape(_) => None,
        })
        .collect()
}

/// Columns a character takes on screen.
fn char_width(character: char) -> usize {
    if character.is_ascii() {
        return usize::from(!character.is_ascii_control());
    }
    let mut buffer = [0; 4];
    Span::raw(&*character.encode_utf8(&mut buffer)).width()
}

/// Columns a painted text takes on screen.
pub fn visible(text: &str) -> usize {
    strip(text).chars().map(char_width).sum()
}

/// `text` padded with spaces to `width` columns.
pub fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(visible(text))))
}

/// Where the lines of `plain` start and end, in characters, at `width`
/// columns: a break at the last space that fits, a hard cut in a word longer
/// than a line; spaces at a break are dropped (tty.mjs `breaks`).
fn breaks(plain: &[char], width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let mut ranges = Vec::new();
    let mut from = 0;
    loop {
        let mut used = 0;
        let mut end = from;
        while end < plain.len() && used + char_width(plain[end]) <= width {
            used += char_width(plain[end]);
            end += 1;
        }
        if end >= plain.len() {
            ranges.push((from, plain.len()));
            return ranges;
        }
        // A space right at the edge breaks there too.
        let space = (from + 1..=end).rev().find(|at| plain[*at] == ' ');
        let to = space.unwrap_or_else(|| end.max(from + 1));
        ranges.push((from, to));
        from = to;
        while from < plain.len() && plain[from] == ' ' {
            from += 1;
        }
        if from >= plain.len() {
            return ranges;
        }
    }
}

/// A painted text as lines of at most `width` columns, words kept whole
/// where they fit. A colour or link that spans a break is closed at the end
/// of the line and opened again at the start of the next (tty.mjs `wrap`,
/// without its cut: every word is shown, #304).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let plain: Vec<char> = strip(text).chars().collect();
    let width = width.max(1);
    if plain.iter().copied().map(char_width).sum::<usize>() <= width {
        return vec![text.to_string()];
    }
    let ranges = breaks(&plain, width);
    let mut lines = vec![String::new()];
    let mut colours: Vec<&str> = Vec::new();
    let mut open_link = "";
    let mut shown = 0;
    let mut at = 0;
    for piece in pieces(text) {
        match piece {
            Piece::Escape(escape) if escape.starts_with("\x1b[") => {
                if escape == RESET || escape == "\x1b[m" {
                    colours.clear();
                } else {
                    colours.push(escape);
                }
                lines[at].push_str(escape);
            }
            Piece::Escape(escape) => {
                open_link = if escape == LINK_END { "" } else { escape };
                lines[at].push_str(escape);
            }
            Piece::Text(text) => {
                for character in text.chars() {
                    if at + 1 < ranges.len() && shown >= ranges[at + 1].0 {
                        if !colours.is_empty() {
                            lines[at].push_str(RESET);
                        }
                        if !open_link.is_empty() {
                            lines[at].push_str(LINK_END);
                        }
                        lines.push(format!("{}{open_link}", colours.concat()));
                        at += 1;
                    }
                    if shown < ranges[at].1 {
                        lines[at].push(character);
                    }
                    shown += 1;
                }
            }
        }
    }
    lines
}

/// Below this many columns left for it, the last cell of a table row goes on its own lines.
const MIN_ROOM: usize = 20;

/// Columns of painted cells, each as wide as its widest cell, two spaces
/// between; the last cell of a row wraps under where it starts when `width`
/// leaves it too little room (tty.mjs `tableRows`, never cut).
pub fn table_rows(rows: &[Vec<String>], width: usize) -> Vec<Vec<String>> {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            if widths.len() <= column {
                widths.push(0);
            }
            widths[column] = widths[column].max(visible(cell));
        }
    }
    rows.iter()
        .map(|row| {
            let Some((last, head)) = row.split_last() else {
                return vec![String::new()];
            };
            let head: String = head
                .iter()
                .enumerate()
                .map(|(column, cell)| format!("{}  ", pad(cell, widths[column])))
                .collect();
            let indent = visible(&head);
            let room = width.saturating_sub(indent);
            if room < MIN_ROOM && visible(last) > room {
                // Too little room after the columns: the cell on its own lines
                // below, under the second column.
                let under = widths.first().map_or(0, |first| first + 2);
                let mut lines = vec![head.trim_end().to_string()];
                lines.extend(
                    wrap(last, width.saturating_sub(under).max(1))
                        .into_iter()
                        .map(|line| format!("{}{line}", " ".repeat(under))),
                );
                return lines;
            }
            wrap(last, room.max(1))
                .into_iter()
                .enumerate()
                .map(|(at, line)| {
                    if at == 0 {
                        format!("{head}{line}")
                    } else {
                        format!("{}{line}", " ".repeat(indent))
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// `run` with colour on, off again after it.
    pub fn coloured<T>(run: impl FnOnce() -> T) -> T {
        set_colour(true);
        let result = run();
        set_colour(false);
        result
    }

    #[test]
    fn paints_only_when_colour_is_on() {
        assert_eq!(bold("x"), "x");
        assert_eq!(link(Some("https://a"), "!3"), "!3");
        assert_eq!(button("✓", "green"), "[ ✓ ]");
        assert_eq!(button("✕ end", "gray"), "( ✕ end )");
        coloured(|| {
            assert_eq!(bold("x"), "\x1b[1mx\x1b[0m");
            assert_eq!(link(Some("https://a"), "!3"), "\x1b]8;;https://a\x1b\\!3\x1b]8;;\x1b\\");
            assert_eq!(link(None, "!3"), "!3");
            assert_eq!(hex("#FF79C6", "a"), "\x1b[38;2;255;121;198ma\x1b[0m");
        });
    }

    #[test]
    fn counts_the_columns_a_painted_line_takes() {
        coloured(|| {
            let line = format!("{} {}", green("✓ passed"), link(Some("u"), "!3"));
            assert_eq!(visible(&line), 11);
            assert_eq!(strip(&line), "✓ passed !3");
        });
        assert_eq!(visible("▀ 🭭 ▀"), 5);
        assert_eq!(pad("ab", 4), "ab  ");
    }

    #[test]
    fn wraps_at_words_and_breaks_a_long_word() {
        assert_eq!(wrap("one two three four", 9), vec!["one two", "three", "four"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("short", 10), vec!["short"]);
        // A glued space never breaks: who acts stays with its reason.
        assert_eq!(wrap("a b\u{a0}→\u{a0}c", 5), vec!["a", "b\u{a0}→\u{a0}c"]);
    }

    #[test]
    fn a_colour_or_link_goes_on_over_the_break() {
        coloured(|| {
            let lines = wrap(&yellow("one two"), 4);
            assert_eq!(lines, vec!["\x1b[33mone\x1b[0m", "\x1b[33mtwo\x1b[0m"]);
            let lines = wrap(&link(Some("u"), "ab cd"), 3);
            assert_eq!(lines[0], "\x1b]8;;u\x1b\\ab\x1b]8;;\x1b\\");
            assert_eq!(lines[1], "\x1b]8;;u\x1b\\cd\x1b]8;;\x1b\\");
        });
    }

    #[test]
    fn lines_cells_up_and_wraps_the_last() {
        let rows = vec![
            vec!["!3".to_string(), "a long title here".into()],
            vec!["!311".into(), "b".into()],
        ];
        let lines = table_rows(&rows, 30);
        assert_eq!(lines[0], vec!["!3    a long title here"]);
        assert_eq!(lines[1], vec!["!311  b"]);
        let narrow = table_rows(&rows, 30);
        assert_eq!(narrow[1], vec!["!311  b"]);
        let cramped = table_rows(&rows, 12);
        assert_eq!(cramped[0], vec!["!3", "      a long", "      title", "      here"]);
    }
}
