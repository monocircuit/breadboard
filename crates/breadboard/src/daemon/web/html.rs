//! The web UI's HTML: the views to pick, and a frame of the overview as the
//! terminal draws it, cell by cell, each run of one style a span.

use breadboard_core::reader::View;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// The views of every repo, the one shown marked; `read GitLab again` when the page may write.
pub fn nav(
    repos: &[PathBuf],
    shown: usize,
    view: &View,
    orchestrations_of: impl Fn(&Path) -> Vec<String>,
    is_writable: bool,
) -> String {
    let mut out = String::from("<nav>");
    for (index, main) in repos.iter().enumerate() {
        let name = main.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
        let _ = write!(out, "<span class=\"repo\">{}</span>", escaped(&name));
        let is_here = |other: &View| index == shown && other == view;
        link(&mut out, &format!("?repo={index}"), "everything", is_here(&View::Everything));
        for orchestration in orchestrations_of(main) {
            let href = format!("?repo={index}&orchestration={}", encoded(&orchestration));
            let is_shown = is_here(&View::Orchestration(orchestration.clone()));
            link(&mut out, &href, &format!("⇶ {orchestration}"), is_shown);
        }
    }
    if is_writable {
        out.push_str("<a class=\"write\" href=\"#\" data-write=\"refresh\">read GitLab again</a>");
    }
    out.push_str("</nav>");
    out
}

fn link(out: &mut String, href: &str, text: &str, is_shown: bool) {
    let class = if is_shown { " class=\"shown\"" } else { "" };
    let _ = write!(out, "<a href=\"{}\"{class}>{}</a>", escaped(href), escaped(text));
}

/// The frame: one line a row, a span a run of cells of one style.
pub fn frame(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut out = String::from("<pre class=\"frame\">");
    for y in area.top()..area.bottom() {
        let mut run: Option<(String, String)> = None;
        // The cells a wide one before covers: drawn by it.
        let mut covered = 0usize;
        for x in area.left()..area.right() {
            let cell = &buffer[(x, y)];
            if covered > 0 {
                covered -= 1;
                continue;
            }
            let symbol = cell.symbol();
            covered = ratatui::text::Span::raw(symbol).width().saturating_sub(1);
            let style = style_of(cell.fg, cell.bg, cell.modifier);
            match &mut run {
                Some((held, text)) if *held == style => text.push_str(symbol),
                _ => {
                    if let Some((held, text)) = run.take() {
                        span(&mut out, &held, &text);
                    }
                    run = Some((style, symbol.to_string()));
                }
            }
        }
        if let Some((held, text)) = run {
            // Its trailing blanks drawn only where a ground shows them.
            let text = if held.contains("background") {
                &text
            } else {
                text.trim_end_matches(' ')
            };
            span(&mut out, &held, text);
        }
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

fn span(out: &mut String, style: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    if style.is_empty() {
        out.push_str(&escaped(text));
    } else {
        let _ = write!(out, "<span style=\"{style}\">{}</span>", escaped(text));
    }
}

/// A cell's style as CSS: its colours (swapped when reversed), bold, dim, italic, lines.
fn style_of(fg: Color, bg: Color, modifier: Modifier) -> String {
    let (fg, bg) = if modifier.contains(Modifier::REVERSED) {
        (bg, fg)
    } else {
        (fg, bg)
    };
    let mut css = String::new();
    if let Some(fg) = css_of(fg) {
        let _ = write!(css, "color:{fg};");
    }
    if let Some(bg) = css_of(bg) {
        let _ = write!(css, "background:{bg};");
    }
    if modifier.contains(Modifier::BOLD) {
        css.push_str("font-weight:bold;");
    }
    if modifier.contains(Modifier::DIM) {
        css.push_str("opacity:.6;");
    }
    if modifier.contains(Modifier::ITALIC) {
        css.push_str("font-style:italic;");
    }
    match (
        modifier.contains(Modifier::UNDERLINED),
        modifier.contains(Modifier::CROSSED_OUT),
    ) {
        (true, true) => css.push_str("text-decoration:underline line-through;"),
        (true, false) => css.push_str("text-decoration:underline;"),
        (false, true) => css.push_str("text-decoration:line-through;"),
        (false, false) => {}
    }
    css
}

/// A terminal colour in CSS (a dark terminal's sixteen); None: the page's own.
fn css_of(color: Color) -> Option<String> {
    let named = |hex: &str| Some(hex.to_string());
    match color {
        Color::Reset => None,
        Color::Black => named("#1e1e1e"),
        Color::Red => named("#e06c75"),
        Color::Green => named("#98c379"),
        Color::Yellow => named("#e5c07b"),
        Color::Blue => named("#61afef"),
        Color::Magenta => named("#c678dd"),
        Color::Cyan => named("#56b6c2"),
        Color::Gray => named("#c8c8c8"),
        Color::DarkGray => named("#5c6370"),
        Color::LightRed => named("#ff7b86"),
        Color::LightGreen => named("#b5e890"),
        Color::LightYellow => named("#ffd98e"),
        Color::LightBlue => named("#82c4ff"),
        Color::LightMagenta => named("#de9cf0"),
        Color::LightCyan => named("#7fdbe6"),
        Color::White => named("#f0f0f0"),
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Indexed(index) => css_of(indexed(index)),
    }
}

/// xterm's 256 colours: the sixteen, a 6×6×6 cube, 24 greys.
fn indexed(index: u8) -> Color {
    const SIXTEEN: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    match index {
        0..=15 => SIXTEEN[index as usize],
        16..=231 => {
            let at = index - 16;
            let level = |step: u8| if step == 0 { 0 } else { 55 + step * 40 };
            Color::Rgb(level(at / 36), level(at / 6 % 6), level(at % 6))
        }
        _ => {
            let grey = 8 + (index - 232) * 10;
            Color::Rgb(grey, grey, grey)
        }
    }
}

/// Text for HTML: its five specials as entities.
pub fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch => out.push(ch),
        }
    }
    out
}

/// A query value, percent-encoded but for what needs none.
fn encoded(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use ratatui::text::{Line, Span};

    #[test]
    fn draws_each_run_of_one_style_as_a_span_and_escapes_its_text() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 2));
        buffer.set_line(
            0,
            0,
            &Line::from(vec![
                Span::styled("<b>", Style::new().fg(Color::Rgb(255, 0, 0))),
                Span::raw(" & "),
                Span::styled("ok", Style::new().bg(Color::Green).add_modifier(Modifier::BOLD)),
            ]),
            12,
        );
        buffer.set_line(0, 1, &Line::from("⇶ 漢x"), 12);
        let html = frame(&buffer);
        assert!(html.contains("<span style=\"color:#ff0000;\">&lt;b&gt;</span> &amp; "), "{html}");
        assert!(html.contains("<span style=\"background:#98c379;font-weight:bold;\">ok</span>"), "{html}");
        // A wide character covers the cell after it: drawn once, the row as wide as the terminal's.
        assert!(html.contains("\n⇶ 漢x\n"), "{html}");
        assert!(!html.contains("<script"));
    }

    #[test]
    fn names_every_view_and_marks_the_one_shown() {
        let repos = [PathBuf::from("/r/platform")];
        let view = View::Orchestration("a&b".into());
        let html = nav(&repos, 0, &view, |_| vec!["a&b".into(), "c".into()], false);
        assert!(html.contains("<span class=\"repo\">platform</span>"));
        assert!(html.contains("<a href=\"?repo=0\">everything</a>"), "{html}");
        assert!(
            html.contains("<a href=\"?repo=0&amp;orchestration=a%26b\" class=\"shown\">⇶ a&amp;b</a>"),
            "{html}"
        );
        assert!(!html.contains("read GitLab again"), "read-only: no write");
        assert!(nav(&repos, 0, &view, |_| Vec::new(), true).contains("data-write=\"refresh\""));
    }

    #[test]
    fn maps_xterm_colours() {
        assert_eq!(indexed(1), Color::Red);
        assert_eq!(indexed(16), Color::Rgb(0, 0, 0));
        assert_eq!(indexed(231), Color::Rgb(255, 255, 255));
        assert_eq!(indexed(232), Color::Rgb(8, 8, 8));
    }
}
