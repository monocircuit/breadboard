//! Links a Ctrl-click opens in the terminal (OSC 8, lib/tty.mjs `link`):
//! `#305`, `!318`, a pipeline's mark.
//!
//! ratatui's spans carry no url, so a linked span carries it in its style:
//! an underline colour that names the url (`style`). Every part of the
//! frame keeps a style as it wraps, splits and lights text, so the mark
//! arrives in the frame's buffer wherever the text landed. Nothing else
//! sets an underline colour. Before the buffer reaches the terminal the
//! marks are taken out (`Links::take`); after it, the linked cells are
//! written once more inside `ESC ]8;;url ESC \` … `ESC ]8;; ESC \`, a run
//! per line, so a link that wraps is closed at the line's end and opened
//! again on the next (`Links::write`).

use ratatui::backend::Backend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

/// The urls a style can name, by their number; a url keeps its number.
#[derive(Default)]
struct Urls {
    ids: HashMap<String, u32>,
    urls: Vec<String>,
}

fn urls() -> &'static Mutex<Urls> {
    static URLS: OnceLock<Mutex<Urls>> = OnceLock::new();
    URLS.get_or_init(Mutex::default)
}

/// `style` naming `url`, the page a Ctrl-click on its text opens; `style`
/// as it is without one. A url a terminal could read as more than a url
/// (a control character) links nothing.
pub fn style(style: Style, url: Option<&str>) -> Style {
    let Some(url) = url.filter(|url| !url.is_empty() && !url.chars().any(char::is_control)) else {
        return style;
    };
    let Ok(mut urls) = urls().lock() else { return style };
    let next = urls.urls.len() as u32 + 1;
    if next >= 1 << 24 {
        return style;
    }
    let id = *urls.ids.entry(url.to_string()).or_insert(next);
    if id == next {
        urls.urls.push(url.to_string());
    }
    style.underline_color(Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8))
}

/// The url a cell's mark names.
fn url_of(cell: &Cell) -> Option<String> {
    let Color::Rgb(r, g, b) = cell.underline_color else {
        return None;
    };
    let id = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
    let urls = urls().lock().ok()?;
    urls.urls.get((id as usize).checked_sub(1)?).cloned()
}

/// Cells of one line under one url (none: cells that were a link and are no more).
#[derive(Clone, Debug, PartialEq)]
struct Run {
    x: u16,
    y: u16,
    url: Option<String>,
    cells: Vec<Cell>,
}

impl Run {
    fn area(&self) -> Rect {
        Rect::new(self.x, self.y, self.cells.len() as u16, 1)
    }
}

/// The links of the frame on screen and what is still to be written.
#[derive(Default)]
pub struct Links {
    area: Rect,
    shown: Vec<Run>,
    to_write: Vec<Run>,
}

impl Links {
    /// The frame drawn into `buffer`, before ratatui writes it: its marks
    /// taken out, its linked runs noted. A run on screen as it was needs no
    /// writing (ratatui leaves its cells alone); a link that is gone is
    /// written plain again, so its cells stop opening it.
    pub fn take(&mut self, buffer: &mut Buffer) {
        let area = buffer.area;
        let mut runs: Vec<Run> = Vec::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let cell = &mut buffer[(x, y)];
                let Some(url) = url_of(cell) else { continue };
                cell.underline_color = Color::Reset;
                let cell = cell.clone();
                match runs.last_mut() {
                    Some(run)
                        if run.y == y && run.x + run.cells.len() as u16 == x && run.url.as_ref() == Some(&url) =>
                    {
                        run.cells.push(cell);
                    }
                    _ => runs.push(Run {
                        x,
                        y,
                        url: Some(url),
                        cells: vec![cell],
                    }),
                }
            }
        }
        // ratatui clears the screen when the size changes: every link is written again.
        let is_same_area = area == self.area;
        let was = std::mem::take(&mut self.shown);
        let is_linked = |x: u16, y: u16| runs.iter().any(|run| run.area().contains((x, y).into()));
        let mut to_write: Vec<Run> = Vec::new();
        for gone in was.iter().filter(|_| is_same_area) {
            let row = gone.area().intersection(area);
            for x in row.left()..row.right() {
                if is_linked(x, gone.y) {
                    continue;
                }
                let cell = buffer[(x, gone.y)].clone();
                match to_write.last_mut() {
                    Some(run) if run.y == gone.y && run.x + run.cells.len() as u16 == x => run.cells.push(cell),
                    _ => to_write.push(Run {
                        x,
                        y: gone.y,
                        url: None,
                        cells: vec![cell],
                    }),
                }
            }
        }
        to_write.extend(runs.iter().filter(|run| !is_same_area || !was.contains(run)).cloned());
        self.area = area;
        self.shown = runs;
        self.to_write = to_write;
    }

    /// The runs `take` noted, written after ratatui's frame: the linked
    /// ones inside OSC 8, the ones that lost their link without it.
    pub fn write<B: Backend<Error = io::Error> + Write>(&mut self, backend: &mut B) -> io::Result<()> {
        if self.to_write.is_empty() {
            return Ok(());
        }
        for run in std::mem::take(&mut self.to_write) {
            if let Some(url) = &run.url {
                write!(backend, "\x1b]8;;{url}\x1b\\")?;
            }
            let mut skip = 0;
            let mut cells = Vec::with_capacity(run.cells.len());
            for (i, cell) in run.cells.iter().enumerate() {
                // The cells a wide glyph covers are its own: ratatui skips them too.
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                skip = unicode_width(cell.symbol()).saturating_sub(1);
                cells.push((run.x + i as u16, run.y, cell));
            }
            Backend::draw(backend, cells.into_iter())?;
            if run.url.is_some() {
                write!(backend, "\x1b]8;;\x1b\\")?;
            }
        }
        Backend::flush(backend)
    }
}

fn unicode_width(symbol: &str) -> usize {
    ratatui::text::Span::raw(symbol).width()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::CrosstermBackend;
    use ratatui::text::{Line, Span};

    /// A frame of one line: `#305 open · !318`, the two numbers linked as `linked` says.
    fn frame(is_linked: bool) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 2));
        let url = |url: &'static str| is_linked.then_some(url);
        let line = Line::from(vec![
            Span::raw(" "),
            Span::styled("#305", style(Style::new(), url("https://x.org/issues/305"))),
            Span::raw(" open · "),
            Span::styled("!318", style(Style::new(), url("https://x.org/merge_requests/318"))),
        ]);
        buffer.set_line(0, 1, &line, 20);
        buffer
    }

    /// What a backend wrote, kept after it is gone.
    #[derive(Clone, Default)]
    struct Tape(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl Write for Tape {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn written(links: &mut Links) -> String {
        let tape = Tape::default();
        links.write(&mut CrosstermBackend::new(tape.clone())).unwrap();
        String::from_utf8(tape.0.take()).unwrap()
    }

    /// What was written with its styles and moves left out.
    fn plain(out: &str) -> String {
        let mut text = String::new();
        let mut chars = out.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '\x1b' {
                text.push(ch);
                continue;
            }
            match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ch.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    let mut osc = String::from("<");
                    while let Some(ch) = chars.next() {
                        if ch == '\x1b' {
                            chars.next();
                            break;
                        }
                        osc.push(ch);
                    }
                    text.push_str(&(osc + ">"));
                }
                _ => {}
            }
        }
        text
    }

    #[test]
    fn a_linked_span_is_written_inside_osc_8_and_its_mark_never_reaches_the_terminal() {
        let mut links = Links::default();
        let mut buffer = frame(true);
        links.take(&mut buffer);
        assert!(buffer.content.iter().all(|cell| cell.underline_color == Color::Reset));
        assert_eq!(
            plain(&written(&mut links)),
            "<8;;https://x.org/issues/305>#305<8;;><8;;https://x.org/merge_requests/318>!318<8;;>"
        );
        // The next frame the same: ratatui leaves the cells alone, so do the links.
        let mut buffer = frame(true);
        links.take(&mut buffer);
        assert_eq!(written(&mut links), "");
    }

    #[test]
    fn a_link_that_is_gone_is_written_plain_and_a_resize_writes_every_link_again() {
        let mut links = Links::default();
        links.take(&mut frame(true));
        written(&mut links);
        links.take(&mut frame(false));
        assert_eq!(plain(&written(&mut links)), "#305!318");
        links.take(&mut frame(true));
        written(&mut links);
        let mut wider = Buffer::empty(Rect::new(0, 0, 30, 2));
        wider.set_line(
            0,
            1,
            &Line::from(
                frame(true).content[20..]
                    .iter()
                    .map(|cell| Span::styled(cell.symbol().to_string(), cell.style()))
                    .collect::<Vec<_>>(),
            ),
            30,
        );
        links.take(&mut wider);
        assert!(plain(&written(&mut links)).contains("<8;;https://x.org/issues/305>#305<8;;>"));
    }

    #[test]
    fn a_url_with_a_control_character_links_nothing() {
        assert_eq!(style(Style::new(), Some("https://x.org/\x1b]8;;evil")), Style::new());
        assert_eq!(style(Style::new(), Some("")), Style::new());
    }
}
