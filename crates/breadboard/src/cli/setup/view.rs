//! The changelogs in the terminal (changelog-view.mjs, lib/tty.mjs): the
//! Markdown painted in herdr's theme colours, links clickable (OSC 8), and
//! `--watch`, the live view of the tab `changelog`: the title standing, the
//! rest scrolling (wheel j k ↑↓ PgUp PgDn space g G), its `##` sections
//! foldable (a click on the heading, z, Z) and jumpable (] [ 1–9), the
//! heading under the pointer lit, drawn again whenever the file changes.
//! Plain ANSI through crossterm, no widget library: the view is a pager, and
//! its links must stay OSC 8 (rewrite rule 4).

use super::proc::outln;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::{execute, terminal};
use regex::Regex;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

/// A colour of the theme as 24 bits.
type Rgb = (u8, u8, u8);

/// The painters of the theme: title, day (`##`), group (`###`), link.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    title: Rgb,
    day: Rgb,
    group: Rgb,
    link: Rgb,
}

fn hex(text: &str) -> Option<Rgb> {
    let text = text.strip_prefix('#')?;
    let channel = |at: usize| u8::from_str_radix(text.get(at..at + 2)?, 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// The hex of `colour` in the `[theme.custom]` of herdr's config text, else the fallback.
fn colour_in(base: &str, colour: &str, fallback: &str) -> Rgb {
    let section = base
        .split_inclusive('\n')
        .fold((false, String::new()), |(is_in, mut text), line| {
            let is_in = if line.starts_with('[') {
                line.starts_with("[theme.custom]")
            } else {
                is_in
            };
            if is_in {
                text.push_str(line);
            }
            (is_in, text)
        });
    let pattern = Regex::new(&format!(
        r#"(?m)^{}\s*=\s*"(#[0-9A-Fa-f]{{6}})""#,
        regex::escape(colour)
    ))
    .ok();
    pattern
        .and_then(|pattern| pattern.captures(&section.1).and_then(|found| hex(&found[1])))
        .or_else(|| hex(fallback))
        .unwrap_or((255, 255, 255))
}

/// The config herdr runs with: the one it was started with, the repo's, your own.
fn herdr_config_text() -> String {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    [
        std::env::var_os("HERDR_CONFIG_PATH").map(std::path::PathBuf::from),
        Some(super::repo::main_dir().join(".herdr").join("config.toml")),
        Some(home.join(".config").join("herdr").join("config.toml")),
    ]
    .into_iter()
    .flatten()
    .find(|file| file.exists())
    .and_then(|file| fs::read_to_string(file).ok())
    .unwrap_or_default()
}

pub fn theme() -> Theme {
    let base = herdr_config_text();
    Theme {
        title: colour_in(&base, "pink", "#FF79C6"),
        day: colour_in(&base, "blue", "#89B4FA"),
        group: colour_in(&base, "mauve", "#CBA6F7"),
        link: colour_in(&base, "teal", "#94E2D5"),
    }
}

/// Whether output gets colours: a terminal (or FORCE_COLOR), and no NO_COLOR.
pub fn is_color() -> bool {
    (std::io::stdout().is_terminal() || std::env::var_os("FORCE_COLOR").is_some())
        && std::env::var_os("NO_COLOR").is_none()
}

/// A run of text with one look.
#[derive(Debug, Clone, Default, PartialEq)]
struct Span {
    text: String,
    fg: Option<Rgb>,
    is_bold: bool,
    is_dim: bool,
    link: Option<String>,
}

/// A painted line: its spans.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Line {
    spans: Vec<Span>,
}

impl Line {
    fn plain(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    fn width(&self) -> usize {
        self.spans.iter().map(|span| span.text.chars().count()).sum()
    }

    /// The characters `from..to` of the line, their looks kept.
    fn slice(&self, from: usize, to: usize) -> Line {
        let mut spans = Vec::new();
        let mut at = 0;
        for span in &self.spans {
            let length = span.text.chars().count();
            let (start, end) = (from.max(at), to.min(at + length));
            if start < end {
                let text: String = span.text.chars().skip(start - at).take(end - start).collect();
                spans.push(Span { text, ..span.clone() });
            }
            at += length;
        }
        Line { spans }
    }

    fn prefixed(mut self, prefix: &str) -> Line {
        if !prefix.is_empty() {
            self.spans.insert(
                0,
                Span {
                    text: prefix.to_string(),
                    ..Span::default()
                },
            );
        }
        self
    }

    /// The line as ANSI (with colours) or as plain text, on `ground` when lit.
    fn ansi(&self, is_color: bool, ground: Option<(usize, Rgb)>) -> String {
        if !is_color {
            return self.plain();
        }
        let mut out = String::new();
        let open_ground = |out: &mut String| {
            if let Some((_, (r, g, b))) = ground {
                out.push_str(&format!("\x1b[48;2;{r};{g};{b}m"));
            }
        };
        open_ground(&mut out);
        for span in &self.spans {
            let mut codes = Vec::new();
            if span.is_bold {
                codes.push("1".to_string());
            }
            if span.is_dim {
                codes.push("2".to_string());
            }
            if let Some((r, g, b)) = span.fg {
                codes.push(format!("38;2;{r};{g};{b}"));
            }
            if let Some(url) = &span.link {
                out.push_str(&format!("\x1b]8;;{url}\x1b\\"));
            }
            if codes.is_empty() {
                out.push_str(&span.text);
            } else {
                out.push_str(&format!("\x1b[{}m{}\x1b[0m", codes.join(";"), span.text));
                open_ground(&mut out);
            }
            if span.link.is_some() {
                out.push_str("\x1b]8;;\x1b\\");
            }
        }
        if let Some((width, _)) = ground {
            out.push_str(&" ".repeat(width.saturating_sub(self.width())));
            out.push_str("\x1b[0m");
        }
        out
    }
}

/// Where the lines of `plain` start and end at `width` (lib/tty.mjs `breaks`).
fn breaks(plain: &[char], width: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut from = 0;
    while plain.len() - from > width {
        let last = (from + width).min(plain.len() - 1);
        let space = (from..=last).rev().find(|at| plain[*at] == ' ');
        let to = match space {
            Some(space) if space > from => space,
            _ => from + width,
        };
        ranges.push((from, to));
        from = to;
        while plain.get(from) == Some(&' ') {
            from += 1;
        }
    }
    ranges.push((from, plain.len()));
    ranges
}

/// A painted line as lines of at most `width` characters, words kept whole where they fit.
fn wrap(line: &Line, width: usize) -> Vec<Line> {
    let plain: Vec<char> = line.plain().chars().collect();
    if plain.len() <= width {
        return vec![line.clone()];
    }
    breaks(&plain, width)
        .into_iter()
        .map(|(from, to)| line.slice(from, to))
        .collect()
}

static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)\s]+)\)").unwrap());
static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());
static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*([^*]+)\*\*").unwrap());
static TIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" · (\d\d:\d\d)$").unwrap());

/// `[text](url)` as a link, `` `code` `` dimmed, `**text**` bold (an MR's
/// summary has it), a trailing ` · HH:MM` dimmed.
fn inline(text: &str, theme: &Theme) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let (body, time) = match TIME.captures(text) {
        Some(found) => (
            &text[..found.get(0).map_or(text.len(), |all| all.start())],
            Some(found[1].to_string()),
        ),
        None => (text, None),
    };
    let mut rest = body;
    while !rest.is_empty() {
        // The first of a link, code and bold; at the same place the earlier listed.
        let found = [&*LINK, &*CODE, &*BOLD]
            .into_iter()
            .enumerate()
            .filter_map(|(kind, pattern)| pattern.captures(rest).map(|found| (kind, found)))
            .min_by_key(|(kind, found)| (found.get(0).map_or(usize::MAX, |all| all.start()), *kind));
        let Some((kind, found)) = found else {
            spans.push(Span {
                text: rest.to_string(),
                ..Span::default()
            });
            break;
        };
        let all = found.get(0).expect("a match has its whole");
        if all.start() > 0 {
            spans.push(Span {
                text: rest[..all.start()].to_string(),
                ..Span::default()
            });
        }
        spans.push(match kind {
            0 => Span {
                text: found[1].to_string(),
                fg: Some(theme.link),
                link: Some(found[2].to_string()),
                ..Span::default()
            },
            1 => Span {
                text: found[1].to_string(),
                is_dim: true,
                ..Span::default()
            },
            _ => Span {
                text: found[1].to_string(),
                is_bold: true,
                ..Span::default()
            },
        });
        rest = &rest[all.end()..];
    }
    if let Some(time) = time {
        spans.push(Span {
            text: " ".into(),
            ..Span::default()
        });
        spans.push(Span {
            text: format!("· {time}"),
            is_dim: true,
            ..Span::default()
        });
    }
    spans
}

/// The Markdown of a changelog as painted lines of at most `width` columns.
pub fn render_markdown(text: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let wrapped = |line: Line, indent: &str| -> Vec<Line> {
        let room = width.saturating_sub(indent.chars().count()).max(10);
        wrap(&line, room)
            .into_iter()
            .enumerate()
            .map(|(i, part)| if i == 0 { part } else { part.prefixed(indent) })
            .collect()
    };
    for raw in text.replace('\r', "").split('\n') {
        let hashes = raw.chars().take_while(|c| *c == '#').count();
        if (1..=4).contains(&hashes) && raw[hashes..].starts_with(char::is_whitespace) {
            let title = raw[hashes..].trim_start().to_string();
            let span = match hashes {
                1 => Span {
                    text: title,
                    fg: Some(theme.title),
                    is_bold: true,
                    ..Span::default()
                },
                2 => Span {
                    text: title,
                    fg: Some(theme.day),
                    is_bold: true,
                    ..Span::default()
                },
                3 => Span {
                    text: title,
                    fg: Some(theme.group),
                    ..Span::default()
                },
                _ => Span {
                    text: title,
                    fg: Some(theme.group),
                    is_dim: true,
                    ..Span::default()
                },
            };
            lines.extend(wrapped(Line { spans: vec![span] }, ""));
            continue;
        }
        if let Some(item) = raw.strip_prefix("- ") {
            let mut spans = vec![Span {
                text: "• ".into(),
                ..Span::default()
            }];
            spans.extend(inline(item, theme));
            lines.extend(wrapped(Line { spans }, "  "));
            continue;
        }
        if let Some(under) = raw.strip_prefix("  ") {
            let mut spans = vec![Span {
                text: "  ".into(),
                ..Span::default()
            }];
            spans.extend(inline(under, theme));
            lines.extend(wrapped(Line { spans }, "  "));
            continue;
        }
        if raw.is_empty() {
            lines.push(Line::default());
        } else {
            lines.extend(wrapped(
                Line {
                    spans: inline(raw, theme),
                },
                "",
            ));
        }
    }
    while lines.last().is_some_and(|line| line.width() == 0) {
        lines.pop();
    }
    lines
}

/// `text` on stdout: painted on a terminal, else as Markdown.
pub fn print(text: &str) {
    if !is_color() {
        outln!("{text}");
        return;
    }
    let width = terminal::size().map(|(columns, _)| usize::from(columns)).unwrap_or(100);
    let painted: Vec<String> = render_markdown(text, width, &theme())
        .iter()
        .map(|line| line.ansi(true, None))
        .collect();
    outln!("{}", painted.join("\n"));
}

const OPEN: &str = "▾";
const FOLDED: &str = "▸";

/// A line of the view: the `##` section it belongs to, and whether it is that heading.
#[derive(Debug, Clone)]
pub struct ViewLine {
    line: Line,
    section: Option<String>,
    is_heading: bool,
}

/// The lines of the view: the text before the first `##`, then each section, folded ones as their heading alone.
pub fn view_lines(text: &str, width: usize, folded: &[String], theme: &Theme) -> Vec<ViewLine> {
    let text = text.replace('\r', "");
    let mut parts: Vec<String> = Vec::new();
    for line in text.split('\n') {
        if line.starts_with("## ") || parts.is_empty() {
            parts.push(line.to_string());
        } else if let Some(last) = parts.last_mut() {
            last.push('\n');
            last.push_str(line);
        }
    }
    let head = if parts.first().is_some_and(|first| !first.starts_with("## ")) {
        parts.remove(0)
    } else {
        String::new()
    };
    let mut lines: Vec<ViewLine> = render_markdown(&head, width, theme)
        .into_iter()
        .map(|line| ViewLine {
            line,
            section: None,
            is_heading: false,
        })
        .collect();
    for part in parts {
        let (first, body) = part.split_once('\n').unwrap_or((&part, ""));
        let section = first[3..].trim().to_string();
        let is_folded = folded.contains(&section);
        let source = if is_folded {
            format!("## {FOLDED} {section}")
        } else {
            format!("## {OPEN} {section}\n{body}")
        };
        if lines.last().is_some_and(|last| last.line.width() != 0) {
            lines.push(ViewLine {
                line: Line::default(),
                section: None,
                is_heading: false,
            });
        }
        for (i, line) in render_markdown(&source, width, theme).into_iter().enumerate() {
            lines.push(ViewLine {
                line,
                section: Some(section.clone()),
                is_heading: i == 0,
            });
        }
    }
    lines
}

/// The ground of a heading under the pointer.
const HOVER: Rgb = (54, 54, 54);
const WHEEL_LINES: i64 = 3;

struct Watch<'a> {
    file: &'a Path,
    empty: &'a str,
    theme: Theme,
    offset: i64,
    folded: Vec<String>,
    shown: Vec<ViewLine>,
    pointer: Option<u16>,
    lit: Option<String>,
    text: String,
}

impl Watch<'_> {
    fn size() -> (usize, usize) {
        terminal::size()
            .map(|(columns, rows)| (usize::from(columns), usize::from(rows)))
            .unwrap_or((100, 40))
    }

    fn read(&self) -> String {
        fs::read_to_string(self.file).unwrap_or_else(|_| self.empty.to_string())
    }

    fn lines(&self) -> Vec<ViewLine> {
        view_lines(&self.text, Self::size().0, &self.folded, &self.theme)
    }

    fn heading_on(&self, row: Option<u16>) -> Option<String> {
        let row = usize::from(row?);
        let line = self.shown.get(row.checked_sub(2)?)?;
        line.is_heading.then(|| line.section.clone()).flatten()
    }

    fn draw(&mut self) {
        let (columns, rows) = Self::size();
        let mut all = self.lines();
        let title = if all.is_empty() {
            ViewLine {
                line: Line::default(),
                section: None,
                is_heading: false,
            }
        } else {
            all.remove(0)
        };
        let room = rows.saturating_sub(1);
        let max = all.len().saturating_sub(room) as i64;
        self.offset = self.offset.clamp(0, max);
        let sections = all.iter().filter(|line| line.is_heading).count();
        let mut mark = Vec::new();
        if self.offset > 0 {
            mark.push(format!("↑ {}", self.offset));
        }
        if max - self.offset > 0 {
            mark.push(format!("↓ {}", max - self.offset));
        }
        if sections > 1 {
            mark.push("] [ 1–9 jump · z Z fold".into());
        }
        let start = self.offset as usize;
        self.shown = all[start.min(all.len())..(start + room).min(all.len())].to_vec();
        self.lit = self.heading_on(self.pointer);
        let mut head = title.line.ansi(true, None);
        if !mark.is_empty() {
            head.push_str(&format!("  \x1b[2m{}\x1b[0m", mark.join("  ")));
        }
        let mut out = format!("\x1b[H\x1b[2J{head}\x1b[K");
        for line in &self.shown {
            let is_lit = line.is_heading && line.section.is_some() && line.section == self.lit;
            out.push_str("\r\n");
            out.push_str(&line.line.ansi(true, is_lit.then_some((columns, HOVER))));
            out.push_str("\x1b[K");
        }
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }

    fn headings(&self) -> Vec<(usize, String)> {
        self.lines()
            .into_iter()
            .skip(1)
            .enumerate()
            .filter(|(_, line)| line.is_heading)
            .filter_map(|(at, line)| line.section.map(|section| (at, section)))
            .collect()
    }

    fn jump_to(&mut self, heading: Option<(usize, String)>) {
        if let Some((at, _)) = heading {
            self.offset = at as i64;
            self.draw();
        }
    }

    fn toggle(&mut self, section: Option<String>) {
        let Some(section) = section else { return };
        if let Some(at) = self.folded.iter().position(|known| *known == section) {
            self.folded.remove(at);
        } else {
            self.folded.push(section.clone());
        }
        let heading = self.headings().into_iter().find(|(_, known)| *known == section);
        self.jump_to(heading);
    }

    fn scroll(&mut self, by: i64) {
        self.offset = self.offset.saturating_add(by).max(0);
        self.draw();
    }

    fn hover(&mut self, row: Option<u16>) {
        self.pointer = row;
        if self.heading_on(row) != self.lit {
            self.draw();
        }
    }

    /// Handles a key; false to leave.
    fn key(&mut self, key: KeyEvent) -> bool {
        let page = (Self::size().1 as i64 - 2).max(1);
        match key.code {
            KeyCode::Char('q') => return false,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Char('j') | KeyCode::Down => self.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll(page),
            KeyCode::PageUp => self.scroll(-page),
            KeyCode::Char('g') => self.scroll(i64::MIN / 2),
            KeyCode::Char('G') => self.scroll(i64::MAX / 2),
            KeyCode::Char(']') => {
                let next = self.headings().into_iter().find(|(at, _)| *at as i64 > self.offset);
                self.jump_to(next);
            }
            KeyCode::Char('[') => {
                let before = self
                    .headings()
                    .into_iter()
                    .rev()
                    .find(|(at, _)| (*at as i64) < self.offset);
                self.jump_to(before);
            }
            KeyCode::Char(digit @ '1'..='9') => {
                let nth = self.headings().into_iter().nth(digit as usize - '1' as usize);
                self.jump_to(nth);
            }
            KeyCode::Char('z') | KeyCode::Enter => {
                let top = self
                    .shown
                    .iter()
                    .find_map(|line| line.section.clone())
                    .or_else(|| self.headings().first().map(|(_, s)| s.clone()));
                self.toggle(top);
            }
            KeyCode::Char('Z') => {
                let all: Vec<String> = self.headings().into_iter().map(|(_, section)| section).collect();
                let is_all_folded = all.iter().all(|section| self.folded.contains(section));
                if is_all_folded {
                    self.folded.retain(|section| !all.contains(section));
                } else {
                    for section in all {
                        if !self.folded.contains(&section) {
                            self.folded.push(section);
                        }
                    }
                }
                self.offset = 0;
                self.draw();
            }
            _ => {}
        }
        true
    }
}

/// The live view of `file`; `empty` while there is none.
pub fn watch(file: &Path, empty: &str) -> std::process::ExitCode {
    let mut stdout = std::io::stdout();
    let _ = terminal::enable_raw_mode();
    let _ = write!(stdout, "\x1b[?1049h\x1b[?25l\x1b[?7l");
    let _ = execute!(stdout, event::EnableMouseCapture, event::EnableFocusChange);
    let restore = || {
        let mut stdout = std::io::stdout();
        let _ = execute!(stdout, event::DisableMouseCapture, event::DisableFocusChange);
        let _ = write!(stdout, "\x1b[?7h\x1b[?25h\x1b[?1049l");
        let _ = stdout.flush();
        let _ = terminal::disable_raw_mode();
    };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut stdout = std::io::stdout();
        let _ = execute!(stdout, event::DisableMouseCapture, event::DisableFocusChange);
        let _ = write!(stdout, "\x1b[?7h\x1b[?25h\x1b[?1049l");
        let _ = terminal::disable_raw_mode();
        previous(info);
    }));
    let mut view = Watch {
        file,
        empty,
        theme: theme(),
        offset: 0,
        folded: Vec::new(),
        shown: Vec::new(),
        pointer: None,
        lit: None,
        text: String::new(),
    };
    view.text = view.read();
    view.draw();
    loop {
        match event::poll(Duration::from_secs(1)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                    if !view.key(key) {
                        break;
                    }
                }
                Ok(Event::Mouse(mouse)) => {
                    let row = mouse.row + 1;
                    match mouse.kind {
                        MouseEventKind::Moved | MouseEventKind::Drag(_) => view.hover(Some(row)),
                        MouseEventKind::Down(MouseButton::Left) if mouse.modifiers.is_empty() => {
                            let heading = view.heading_on(Some(row));
                            view.toggle(heading);
                        }
                        MouseEventKind::ScrollDown => view.scroll(WHEEL_LINES),
                        MouseEventKind::ScrollUp => view.scroll(-WHEEL_LINES),
                        _ => {}
                    }
                }
                Ok(Event::FocusLost) => view.hover(None),
                Ok(Event::Resize(..)) => view.draw(),
                Ok(_) => {}
                Err(_) => break,
            },
            Ok(false) => {
                let now = view.read();
                if now != view.text {
                    view.text = now;
                    view.draw();
                }
            }
            Err(_) => break,
        }
    }
    restore();
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: Theme = Theme {
        title: (1, 1, 1),
        day: (2, 2, 2),
        group: (3, 3, 3),
        link: (4, 4, 4),
    };

    #[test]
    fn renders_items_with_a_hanging_indent() {
        let lines = render_markdown(
            "# T\n\n- [!1](u) a long entry that wraps · 10:00\n  under\n",
            20,
            &THEME,
        );
        let plain: Vec<String> = lines.iter().map(Line::plain).collect();
        assert_eq!(
            plain,
            vec!["T", "", "• !1 a long entry", "  that wraps · 10:00", "  under"]
        );
        assert!(lines[2].ansi(true, None).contains("\x1b]8;;u\x1b\\"));
    }

    #[test]
    fn paints_bold_code_and_links_in_order() {
        let spans = inline("a **b** `c` [d](u) **e", &THEME);
        let looks: Vec<(&str, bool, bool, bool)> = spans
            .iter()
            .map(|span| (span.text.as_str(), span.is_bold, span.is_dim, span.link.is_some()))
            .collect();
        assert_eq!(
            looks,
            vec![
                ("a ", false, false, false),
                ("b", true, false, false),
                (" ", false, false, false),
                ("c", false, true, false),
                (" ", false, false, false),
                ("d", false, false, true),
                (" **e", false, false, false),
            ]
        );
    }

    #[test]
    fn folds_a_section_to_its_heading() {
        let text = "# T\n\n## a\n\n- x\n\n## b\n\n- y\n";
        let lines = view_lines(text, 40, &["a".to_string()], &THEME);
        let plain: Vec<String> = lines.iter().map(|line| line.line.plain()).collect();
        assert_eq!(plain, vec!["T", "", "▸ a", "", "▾ b", "", "• y"]);
        assert!(lines[2].is_heading);
    }

    #[test]
    fn reads_the_theme() {
        let base = "[theme]\nname = \"x\"\n[theme.custom]\npink = \"#010203\"\n[other]\nblue = \"#FFFFFF\"\n";
        assert_eq!(colour_in(base, "pink", "#000000"), (1, 2, 3));
        assert_eq!(colour_in(base, "blue", "#89B4FA"), (0x89, 0xB4, 0xFA));
    }

    #[test]
    fn breaks_at_the_last_space() {
        let plain: Vec<char> = "aaa bbb ccc".chars().collect();
        assert_eq!(breaks(&plain, 7), vec![(0, 7), (8, 11)]);
        let word: Vec<char> = "abcdefghij".chars().collect();
        assert_eq!(breaks(&word, 4), vec![(0, 4), (4, 8), (8, 10)]);
    }
}
