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

/// `text` cut after `columns` visible columns, no `…`: every colour and link
/// it opens is closed as in the whole text (its escapes are all kept).
pub fn cut(text: &str, columns: usize) -> String {
    let mut left = columns;
    let mut out = String::new();
    for piece in pieces(text) {
        match piece {
            Piece::Escape(escape) => out.push_str(escape),
            Piece::Text(text) => {
                for character in text.chars() {
                    let width = char_width(character);
                    if width > left {
                        left = 0;
                        break;
                    }
                    left -= width;
                    out.push(character);
                }
            }
        }
    }
    out
}

/// `text` without its ` · ` parts from the first that doesn't fit `width`
/// whole (tty.mjs `fitParts`): a heading keeps its counts and drops what
/// follows them. None when even its first part is too wide.
pub fn fit_parts(text: &str, width: usize) -> Option<String> {
    if visible(text) <= width {
        return Some(text.to_string());
    }
    let plain = strip(text);
    let end = plain
        .match_indices(" · ")
        .map(|(at, _)| visible(&plain[..at]))
        .filter(|columns| *columns > 0 && *columns <= width)
        .last()?;
    Some(cut(text, end))
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

/// Which characters of a painted text hold together: a button or a chip,
/// painted on a ground of its own (`[ label ]` / `( label )` without colours),
/// is never split over two lines while it fits on one.
fn glue_of(text: &str) -> Vec<bool> {
    let mut glued = Vec::new();
    let mut is_on_ground = false;
    for piece in pieces(text) {
        match piece {
            Piece::Escape(escape) if escape.starts_with("\x1b[") => {
                let params: Vec<u16> = escape[2..escape.len() - 1]
                    .split(';')
                    .map(|param| param.parse().unwrap_or(0))
                    .collect();
                if params.iter().any(|param| matches!(param, 40..=48 | 100..=107)) {
                    is_on_ground = true;
                } else if params.iter().all(|param| *param == 0) || params.contains(&49) {
                    is_on_ground = false;
                }
            }
            Piece::Escape(_) => {}
            Piece::Text(text) => glued.extend(text.chars().map(|_| is_on_ground)),
        }
    }
    // Without colours: from `[ ` or `( ` to the next ` ]` or ` )`.
    let plain: Vec<char> = strip(text).chars().collect();
    let mut at = 0;
    while at + 1 < plain.len() {
        let close = match (plain[at], plain[at + 1]) {
            ('[', ' ') => ']',
            ('(', ' ') => ')',
            _ => {
                at += 1;
                continue;
            }
        };
        let end = (at + 2..plain.len()).find(|end| plain[*end] == close && plain[*end - 1] == ' ');
        match end {
            Some(end) => {
                glued[at..=end].iter_mut().for_each(|is_glued| *is_glued = true);
                at = end + 1;
            }
            None => at += 1,
        }
    }
    glued
}

/// Where the lines of `plain` start and end, in characters, at `width`
/// columns: a break at the last space that fits outside a button, else after
/// the last `-` or `/` of a word longer than a line (a branch, a path, a
/// URL), else at
/// any space, else a hard cut; spaces at a break are dropped (tty.mjs
/// `breaks`, which cut buttons and names anywhere).
fn breaks(plain: &[char], glued: &[bool], width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let is_glued = |at: usize| glued.get(at).copied().unwrap_or(false);
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
        let spaces = || (from + 1..=end).rev().filter(|at| plain[*at] == ' ');
        let to = spaces()
            .find(|at| !is_glued(*at))
            .or_else(|| {
                (from + 1..=end)
                    .rev()
                    .find(|at| matches!(plain[*at - 1], '-' | '/') && plain[*at] != ' ' && !is_glued(*at))
            })
            .or_else(|| spaces().next())
            .unwrap_or_else(|| end.max(from + 1));
        ranges.push((from, to));
        // A button's own padding stays with it, unless it breaks inside, too wide for a line.
        let is_inside = is_glued(to) && is_glued(to - 1);
        from = to;
        while from < plain.len() && plain[from] == ' ' && (is_inside || !is_glued(from)) {
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
    let ranges = breaks(&plain, &glue_of(text), width);
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
/// between; the last cell of a row wraps under where it starts (tty.mjs
/// `tableRows`, never cut). Where `width` leaves it too little room the
/// row is stacked instead: its first two cells on the first line, the rest
/// on the lines below, a step in, so a row still reads as one and nothing
/// wraps back to the edge.
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
    // Under the second column where that leaves room, else a step in from the edge.
    let first = widths.first().map_or(0, |first| first + 2);
    let under = if width.saturating_sub(first) >= MIN_ROOM.min(width / 2) {
        first
    } else {
        2
    };
    let below = |text: &str, indent: usize| -> Vec<String> {
        wrap(text, width.saturating_sub(indent).max(1))
            .into_iter()
            .map(|line| format!("{}{line}", " ".repeat(indent)))
            .collect()
    };
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
            if row.len() > 2 && indent > width {
                // The columns alone are wider than the line: the row stacked.
                let mut lines = match row.first() {
                    Some(mark) if row.len() > 1 => {
                        let mut lines = wrap(&row[1], width.saturating_sub(first).max(1));
                        for (at, line) in lines.iter_mut().enumerate() {
                            let lead = if at == 0 { pad(mark, first) } else { " ".repeat(first) };
                            *line = format!("{lead}{line}");
                        }
                        lines
                    }
                    _ => Vec::new(),
                };
                let rest: Vec<&str> = row[2..]
                    .iter()
                    .map(String::as_str)
                    .filter(|cell| !strip(cell).trim().is_empty())
                    .collect();
                if !rest.is_empty() {
                    lines.extend(below(&rest.join("  "), (first + 2).min(width / 2)));
                }
                return lines;
            }
            if room < MIN_ROOM && visible(last) > room {
                // Too little room after the columns: the cell on its own lines below.
                let mut lines = vec![head.trim_end().to_string()];
                lines.extend(below(last, under));
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

    #[test]
    fn a_heading_drops_the_parts_that_do_not_fit() {
        let title = "WORKTREES 1 at work · 41 removed · 9.3 GB free (shared stack below 1.5 GB)";
        assert_eq!(fit_parts(title, 200).as_deref(), Some(title));
        assert_eq!(
            fit_parts(title, 40).as_deref(),
            Some("WORKTREES 1 at work · 41 removed")
        );
        assert_eq!(fit_parts(title, 20).as_deref(), Some("WORKTREES 1 at work"));
        assert_eq!(fit_parts(title, 10), None);
        assert_eq!(cut("ab\x1b[2mcd\x1b[22m", 3), "ab\x1b[2mc\x1b[22m");
    }

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
        // A name longer than a line breaks after a `-`.
        assert_eq!(
            wrap("from 306-overview-merge-requests-crash-row (!315)", 19),
            vec!["from", "306-overview-merge-", "requests-crash-row", "(!315)"]
        );
        assert_eq!(
            wrap("https://gitlab.com/monocircuit/platform/-/merge_requests/316", 45),
            vec!["https://gitlab.com/monocircuit/platform/-/", "merge_requests/316"]
        );
    }

    #[test]
    fn a_button_moves_to_the_next_line_whole() {
        let line = format!("1 issue {}", button("✕ dismiss", "red"));
        assert_eq!(wrap(&line, 15), vec!["1 issue", "[ ✕ dismiss ]"]);
        assert_eq!(wrap("( ✕ end ) a", 6), vec!["( ✕", "end )", "a"]);
        coloured(|| {
            let line = format!("1 issue {}", button("✕ dismiss", "red"));
            let lines = wrap(&line, 15);
            assert_eq!(lines.len(), 2);
            assert_eq!(strip(&lines[1]), " ✕ dismiss ");
        });
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
        // A wide first column: the cell a step in from the edge, not under it.
        let rows = vec![vec![
            "free 298-breadboard-no-push".to_string(),
            "0 % · 1 issue [ ✕ dismiss ]".into(),
        ]];
        assert_eq!(
            table_rows(&rows, 40)[0],
            vec!["free 298-breadboard-no-push", "  0 % · 1 issue [ ✕ dismiss ]"]
        );
    }

    #[test]
    fn stacks_a_row_whose_columns_are_wider_than_the_line() {
        let rows = vec![vec![
            "→".to_string(),
            "292-overview-every-agent-has-name ⇶ breadboard".into(),
            "—".into(),
            "no .env.local".into(),
            "1 ahead".into(),
            String::new(),
        ]];
        assert_eq!(
            table_rows(&rows, 40)[0],
            vec![
                "→  292-overview-every-agent-has-name ⇶",
                "   breadboard",
                "     —  no .env.local  1 ahead",
            ]
        );
    }
}
