//! The view on screen (overview/frame.mjs, scroll.mjs): the page is built
//! once per change of its data or width and kept; scrolling and hover only
//! cut a new window out of it and light what the pointer is on. A click
//! resolves against the frame the user sees, not the next one.

use super::App;
use super::band::{self, Header};
use super::clicks;
use super::page::{Ground, Page, Sheet, guarded};
use super::scene::Target;
use super::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use std::time::Instant;

/// What is lit: the block under the pointer and the button, by their region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Lit {
    block: Option<usize>,
    button: Option<usize>,
}

/// Where the page sits on screen.
#[derive(Clone, Copy, Debug)]
struct Window {
    /// The first row under the band.
    top: u16,
    /// Rows under the band.
    height: u16,
    /// Rows on top that don't scroll.
    sticky: u16,
    /// Rows the rest scrolls in.
    room: u16,
    rows: u16,
    max_scroll: u16,
}

impl Window {
    /// The page row on screen row `y`, scrolled by `scroll`.
    fn page_row(&self, y: u16, scroll: u16) -> Option<u16> {
        if y < self.top || y >= self.top + self.height {
            return None;
        }
        let at = y - self.top;
        let row = if at < self.sticky {
            at
        } else {
            self.sticky + scroll + (at - self.sticky)
        };
        (row < self.rows).then_some(row)
    }

    /// The screen row of page row `row`, scrolled by `scroll`, if shown.
    fn screen_y(&self, row: u16, scroll: u16) -> Option<u16> {
        if row < self.sticky {
            return Some(self.top + row);
        }
        let at = row.checked_sub(self.sticky + scroll)?;
        (at < self.room).then_some(self.top + self.sticky + at)
    }
}

pub struct View<A> {
    sheet: Option<Sheet<A>>,
    header: Header,
    band_height: u16,
    is_stale: bool,
    built_at: Instant,
    /// Where the next frame is scrolled to, and where the one on screen is.
    scroll: u16,
    shown_scroll: u16,
    /// The pointer, in screen cells.
    mouse: Option<(u16, u16)>,
    area: Rect,
    lit: Lit,
}

impl<A: Clone> Default for View<A> {
    fn default() -> Self {
        Self {
            sheet: None,
            header: Header::default(),
            band_height: 0,
            is_stale: true,
            built_at: Instant::now(),
            scroll: 0,
            shown_scroll: 0,
            mouse: None,
            area: Rect::default(),
            lit: Lit::default(),
        }
    }
}

impl<A: Clone> View<A> {
    /// The data changed: the next frame builds the page again.
    pub fn stale(&mut self) {
        self.is_stale = true;
    }

    /// When something on the page shows time and needs drawing again.
    pub fn redraw_at(&self) -> Option<Instant> {
        Some(self.built_at + self.sheet.as_ref()?.redraw_in?)
    }

    fn build<P: App<Action = A>>(&mut self, app: &P, width: u16) {
        self.header = guarded(|| app.header()).unwrap_or_else(|why| {
            Header::new("breadboard").status(vec![Span::styled(format!("⚠ {why}"), theme::tone(theme::Tone::Red))])
        });
        let mut page = Page::new(width);
        if let Err(why) = guarded(|| app.build(&mut page)) {
            page.failed("VIEW", &why);
        }
        self.sheet = Some(page.finish());
        self.band_height = band::lines(&self.header, "", width).len() as u16;
        self.built_at = Instant::now();
        self.is_stale = false;
    }

    fn window(&self) -> Window {
        let rows = self
            .sheet
            .as_ref()
            .map_or(0, |sheet| sheet.rows.len().min(u16::MAX as usize) as u16);
        let top = self.band_height.min(self.area.height);
        let height = self.area.height - top;
        let sticky = self.sheet.as_ref().map_or(0, |sheet| sheet.sticky);
        // Sticky only while it leaves half the pane to the rest.
        let sticky = if u32::from(sticky) * 2 <= u32::from(height) {
            sticky
        } else {
            0
        };
        let room = height - sticky;
        let max_scroll = rows.saturating_sub(sticky).saturating_sub(room);
        Window {
            top,
            height,
            sticky,
            room,
            rows,
            max_scroll,
        }
    }

    fn lit_at(&self, window: &Window, scroll: u16) -> Lit {
        let (Some(sheet), Some((x, y))) = (&self.sheet, self.mouse) else {
            return Lit::default();
        };
        let Some(row) = window.page_row(y, scroll) else {
            return Lit::default();
        };
        Lit {
            block: sheet.block_at(x, row).map(|(i, _)| i),
            button: sheet
                .hit(x, row)
                .filter(|(_, region)| region.is_button())
                .map(|(i, _)| i),
        }
    }

    /// Draws the frame: the page built again if stale or another width.
    pub fn draw<P: App<Action = A>>(&mut self, frame: &mut Frame, app: &P) {
        let area = frame.area();
        if self.is_stale || self.sheet.as_ref().is_none_or(|sheet| sheet.width != area.width) {
            self.build(app, area.width);
        }
        self.area = area;
        let window = self.window();
        self.scroll = self.scroll.min(window.max_scroll);
        let scroll = self.scroll;
        let lit = self.lit_at(&window, scroll);
        let Some(sheet) = &self.sheet else { return };
        let buffer = frame.buffer_mut();

        let mark = band::more_mark(scroll, window.max_scroll - scroll);
        let band_area = Rect::new(0, 0, area.width, window.top);
        buffer.set_style(band_area, Style::new().bg(theme::HEAD).fg(theme::BAND_TEXT));
        for (y, line) in band::lines(&self.header, &mark, area.width)
            .iter()
            .take(window.top as usize)
            .enumerate()
        {
            buffer.set_line(0, y as u16, line, area.width);
        }

        let lit_block = lit.block.map(|i| sheet.regions[i].area);
        for y in window.top..window.top + window.height {
            let Some(row) = window.page_row(y, scroll) else {
                continue;
            };
            let drawn = &sheet.rows[row as usize];
            let is_lit = lit_block.is_some_and(|block| row >= block.y && row < block.y + block.height);
            let ground = match (drawn.ground, is_lit) {
                (Ground::Plain, false) => None,
                (Ground::Island, false) => Some(theme::ISLAND),
                (Ground::Head, false) => Some(theme::HEAD),
                (Ground::Head, true) => Some(theme::HOVER_HEAD),
                (_, true) => Some(theme::HOVER),
            };
            if let Some(ground) = ground {
                buffer.set_style(Rect::new(0, y, area.width, 1), Style::new().bg(ground));
            }
            buffer.set_line(0, y, &drawn.line, area.width);
        }

        // The button under the pointer: its own, stronger light.
        if let Some(region) = lit.button.map(|i| &sheet.regions[i])
            && let Target::Button { tone, .. } = region.target
        {
            for row in region.area.y..region.area.y + region.area.height {
                if let Some(y) = window.screen_y(row, scroll) {
                    let cells = Rect::new(region.area.x, y, region.area.width, 1).intersection(buffer.area);
                    buffer.set_style(cells, tone.lit());
                }
            }
        }

        // Where the window is: a thumb in the island's right margin.
        if window.max_scroll > 0 && window.room > 0 && area.width > 0 {
            let (room, total) = (u32::from(window.room), u32::from(window.rows - window.sticky));
            let thumb = (room * room / total).clamp(1, room);
            let at = u32::from(scroll) * (room - thumb) / u32::from(window.max_scroll);
            for i in at..at + thumb {
                let cell = &mut buffer[(area.width - 1, window.top + window.sticky + i as u16)];
                cell.set_symbol("┃").set_fg(theme::BAND_QUIET);
            }
        }

        self.shown_scroll = scroll;
        self.lit = lit;
    }

    /// The pointer moved (None: it left). Whether what is lit changes: only
    /// then a frame is drawn, not on every move.
    pub fn hover(&mut self, at: Option<(u16, u16)>) -> bool {
        self.mouse = at;
        self.lit_at(&self.window(), self.shown_scroll) != self.lit
    }

    /// What a click at screen cell (x, y) runs, in the frame on screen.
    pub fn click(&self, x: u16, y: u16) -> Option<A> {
        let row = self.window().page_row(y, self.shown_scroll)?;
        let (_, region) = self.sheet.as_ref()?.hit(x, row)?;
        Some(region.action().clone())
    }

    /// The log's entry of a click at screen cell (x, y): what it ran, on which row.
    pub fn entry(&self, x: u16, y: u16) -> clicks::Entry {
        let mut entry = clicks::Entry {
            row: y,
            column: x,
            ran: "nothing".into(),
            ..clicks::Entry::default()
        };
        let (Some(sheet), Some(row)) = (&self.sheet, self.window().page_row(y, self.shown_scroll)) else {
            return entry;
        };
        entry.view_row = Some(row);
        entry.line = sheet.text().swap_remove(row as usize);
        if let Some((_, region)) = sheet.hit(x, row) {
            entry.ran = match &region.target {
                Target::Button { label, .. } => format!("button {label}"),
                Target::Block(_) => "line".into(),
            };
        }
        entry
    }

    /// Scrolls by `by` lines, within the page. Whether it moved.
    pub fn scroll_by(&mut self, by: i32) -> bool {
        let max = self.window().max_scroll;
        let to = (i32::from(self.scroll) + by).clamp(0, i32::from(max)) as u16;
        let has_moved = to != self.scroll;
        self.scroll = to;
        has_moved
    }

    /// Scrolls by `pages`: what the pane shows under the sticky rows, less one to keep the context.
    pub fn page_by(&mut self, pages: i32) -> bool {
        let page = i32::from(self.window().room.saturating_sub(1).max(1));
        self.scroll_by(pages * page)
    }

    pub fn scroll_top(&mut self) -> bool {
        self.scroll_by(-i32::from(self.scroll))
    }

    pub fn scroll_bottom(&mut self) -> bool {
        self.scroll_by(i32::from(u16::MAX))
    }

    #[cfg(test)]
    pub fn sheet(&self) -> &Sheet<A> {
        self.sheet.as_ref().expect("drawn")
    }

    #[cfg(test)]
    pub fn scroll(&self) -> u16 {
        self.scroll
    }

    /// The screen row of page row `row` in the frame on screen.
    #[cfg(test)]
    pub fn screen_y(&self, row: u16) -> Option<u16> {
        self.window().screen_y(row, self.shown_scroll)
    }

    #[cfg(test)]
    pub fn body(&self) -> (u16, u16) {
        let window = self.window();
        (window.top, window.height)
    }
}

/// The whole page as plain text at `width`, no terminal: `overview --once`.
pub fn once<P: App>(app: &P, width: u16) -> String {
    let mut view = View::<P::Action>::default();
    view.build(app, width);
    let mut lines: Vec<String> = band::lines(&view.header, "", width)
        .iter()
        .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect())
        .collect();
    lines.extend(view.sheet.as_ref().map(Sheet::text).unwrap_or_default());
    lines.iter().map(|line| line.trim_end().to_string() + "\n").collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::demo::{Action, Demo, demo};
    use crate::tui::input::{Command, Rejoin, command_of};
    use crate::tui::scene::Region;
    use crate::tui::{Flow, Input};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    const WIDTHS: [u16; 3] = [80, 120, 200];

    struct Screen {
        terminal: Terminal<TestBackend>,
        view: View<Action>,
        app: Demo,
    }

    impl Screen {
        fn new(width: u16, height: u16) -> Self {
            let terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut screen = Self {
                terminal,
                view: View::default(),
                app: demo(),
            };
            screen.draw();
            screen
        }

        fn draw(&mut self) {
            let Self { terminal, view, app } = self;
            terminal.draw(|frame| view.draw(frame, app)).unwrap();
        }

        fn bg(&self, x: u16, y: u16) -> Option<Color> {
            self.terminal.backend().buffer()[(x, y)].bg.into()
        }

        /// Scrolls so page row `row` is on screen, and where it is.
        fn show(&mut self, row: u16) -> u16 {
            self.view.scroll_top();
            let sticky = self.view.sheet().sticky;
            if row >= sticky {
                self.view.scroll_by(i32::from(row - sticky));
            }
            self.draw();
            self.view.screen_y(row).expect("row on screen")
        }

        fn regions(&self) -> Vec<Region<Action>> {
            self.view.sheet().regions.clone()
        }

        fn click(&mut self, action: Action) {
            if self.app.act(Input::Click(action)) == Flow::Redraw {
                self.view.stale();
            }
            self.draw();
        }
    }

    fn page_text(screen: &Screen) -> String {
        screen.view.sheet().text().join("\n")
    }

    #[test]
    fn nothing_is_cut_and_nothing_runs_past_the_edge() {
        for width in WIDTHS {
            let screen = Screen::new(width, 40);
            for row in &screen.view.sheet().rows {
                assert!(row.line.width() <= width as usize, "{width}: {:?}", row.line);
            }
            let text = page_text(&screen);
            let chars: Vec<char> = text.chars().collect();
            for (i, ch) in chars.iter().enumerate().skip(1) {
                let before = chars[i - 1];
                let is_cut = *ch == '…' && (before.is_alphanumeric() || "()#_".contains(before));
                assert!(!is_cut, "{width}: cut at {i}: {text}");
            }
            // Every word of every long line is on the page, whole.
            let words: Vec<&str> = text.split_whitespace().collect();
            for line in [
                "a GitLab refresh makes 312 calls and takes 29 s; read final things once, batch the rest",
                "the ratatui spike against live GitLab: every button works on every cell",
                "check:breadboard-gauntlet-rs",
            ] {
                for word in line.split_whitespace() {
                    assert!(words.contains(&word), "{width}: {word}");
                }
            }
        }
    }

    #[test]
    fn a_job_wraps_whole() {
        for width in WIDTHS {
            let screen = Screen::new(width, 40);
            let text = page_text(&screen);
            for job in [
                "✓ check:typecheck",
                "✗ check:breadboard-rs",
                "… check:breadboard-gauntlet-rs",
            ] {
                assert!(text.contains(job), "{width}: {job}");
            }
        }
    }

    #[test]
    fn every_cell_of_every_button_clicks_and_lights_the_whole_button() {
        for width in WIDTHS {
            let mut screen = Screen::new(width, 40);
            let regions = screen.regions();
            let buttons: Vec<_> = regions.iter().filter(|region| region.is_button()).collect();
            assert!(buttons.len() >= 17, "{width}: {}", buttons.len());
            for button in buttons {
                let Target::Button { tone, .. } = button.target else {
                    unreachable!()
                };
                let area = button.area;
                let y = screen.show(area.y);
                for x in area.x..area.x + area.width {
                    assert_eq!(
                        screen.view.click(x, y).as_ref(),
                        Some(button.action()),
                        "{width}: ({x}, {y})"
                    );
                    assert!(
                        screen.view.hover(Some((x, y))) || x > area.x,
                        "{width}: hover at ({x}, {y})"
                    );
                    screen.draw();
                    for cell in area.x..area.x + area.width {
                        assert_eq!(
                            screen.bg(cell, y),
                            tone.lit().bg,
                            "{width}: {:?} at {cell}",
                            button.target
                        );
                    }
                }
                // The cells around it are not the button.
                for x in [area.x.wrapping_sub(1), area.x + area.width] {
                    if x < width {
                        assert_ne!(
                            screen.view.click(x, y).as_ref(),
                            Some(button.action()),
                            "{width}: ({x}, {y})"
                        );
                    }
                }
                screen.view.hover(None);
            }
        }
    }

    #[test]
    fn a_click_split_after_its_escape_still_clicks() {
        let mut screen = Screen::new(120, 40);
        let regions = screen.regions();
        let button = regions.iter().find(|region| region.is_button()).unwrap();
        let y = screen.show(button.area.y);
        let x = button.area.x + button.area.width - 1;
        let mut rejoin = Rejoin::default();
        let mut events = rejoin.feed(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        for ch in format!("[<0;{};{}M", x + 1, y + 1).chars() {
            events.extend(rejoin.feed(Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(command_of(&events[0]), Some(Command::Click(x, y)));
        assert_eq!(screen.view.click(x, y).as_ref(), Some(button.action()));
    }

    #[test]
    fn hover_lights_the_whole_block_and_only_it() {
        for width in WIDTHS {
            let mut screen = Screen::new(width, 40);
            let regions = screen.regions();
            let blocks: Vec<_> = regions.iter().filter(|region| !region.is_button()).collect();
            assert!(blocks.len() >= 10, "{width}");
            let x = width - 3;
            for block in blocks {
                let (top, height) = screen.view.body();
                for row in block.area.y..block.area.y + block.area.height {
                    let y = screen.show(row);
                    screen.view.hover(Some((x, y)));
                    screen.draw();
                    for at in top..top + height {
                        let is_lit = matches!(screen.bg(x, at), Some(theme::HOVER | theme::HOVER_HEAD));
                        let shown = (block.area.y..block.area.y + block.area.height)
                            .any(|row| screen.view.screen_y(row) == Some(at));
                        assert_eq!(
                            is_lit, shown,
                            "{width}: block {:?}, pointer on row {row}, screen row {at}",
                            block.target
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn hover_follows_the_scroll() {
        let mut screen = Screen::new(120, 40);
        let block = screen.regions().into_iter().find(|region| !region.is_button()).unwrap();
        let y = screen.show(block.area.y);
        screen.view.hover(Some((60, y)));
        screen.draw();
        assert_eq!(screen.bg(60, y), Some(theme::HOVER));
        // The block moves up under the pointer: what is under it now lights.
        screen.view.scroll_by(1);
        screen.draw();
        let row = screen.view.window().page_row(y, screen.view.scroll()).unwrap();
        let under = screen.view.sheet().block_at(60, row).map(|(_, region)| region.area);
        assert_eq!(screen.bg(60, y) == Some(theme::HOVER), under.is_some(), "{under:?}");
    }

    #[test]
    fn a_panicking_section_draws_its_error_and_the_rest_still_works() {
        let mut screen = Screen::new(120, 40);
        screen.click(Action::Break);
        let text = page_text(&screen);
        assert!(text.contains("BROKEN failed to draw"), "{text}");
        assert!(
            text.contains("⚠ Cannot read properties of undefined (reading 'branch')"),
            "{text}"
        );
        assert!(text.contains("MERGE REQUESTS") && text.contains("WORKTREES"), "{text}");
        // The buttons after it still click; the band is drawn.
        let regions = screen.regions();
        let merge = regions.iter().rfind(|region| region.is_button()).unwrap();
        let y = screen.show(merge.area.y);
        assert_eq!(screen.view.click(merge.area.x, y).as_ref(), Some(merge.action()));
        let band: String = (0..120)
            .map(|x| screen.terminal.backend().buffer()[(x, 2)].symbol().to_string())
            .collect();
        assert!(band.contains("breadboard demo"), "{band}");
    }

    #[test]
    fn scrolling_stays_within_the_page() {
        let mut screen = Screen::new(120, 40);
        let window = screen.view.window();
        assert!(window.max_scroll > 0);
        assert!(!screen.view.scroll_by(-1), "above the top");
        assert!(!screen.view.page_by(-1));
        assert!(screen.view.page_by(1));
        assert_eq!(screen.view.scroll(), window.room - 1);
        assert!(screen.view.scroll_bottom());
        assert_eq!(screen.view.scroll(), window.max_scroll);
        assert!(!screen.view.scroll_by(3), "past the end");
        screen.draw();
        // The end of the page is the last row on screen.
        let last = window.rows - 1;
        assert_eq!(screen.view.screen_y(last), Some(39));
        assert!(screen.view.scroll_top());
        assert_eq!(screen.view.scroll(), 0);
    }

    #[test]
    fn a_page_that_got_shorter_pulls_the_scroll_up() {
        let mut screen = Screen::new(120, 40);
        screen.click(Action::Fold);
        screen.view.scroll_bottom();
        screen.draw();
        let was = screen.view.scroll();
        screen.click(Action::Fold);
        assert!(screen.view.scroll() < was);
        assert_eq!(screen.view.scroll(), screen.view.window().max_scroll);
    }

    #[test]
    fn the_band_marks_what_the_scroll_cuts_off() {
        let mut screen = Screen::new(120, 40);
        let line = |screen: &Screen| -> String {
            (0..120)
                .map(|x| screen.terminal.backend().buffer()[(x, 1)].symbol().to_string())
                .collect()
        };
        assert!(line(&screen).contains("↓ "), "{}", line(&screen));
        screen.view.scroll_bottom();
        screen.draw();
        let text = line(&screen);
        assert!(text.contains("↑ ") && !text.contains("↓ "), "{text}");
    }

    #[test]
    fn a_click_is_logged_with_what_it_ran() {
        let mut screen = Screen::new(120, 40);
        let regions = screen.regions();
        let button = regions.iter().find(|region| region.is_button()).unwrap();
        let y = screen.show(button.area.y);
        let entry = screen.view.entry(button.area.x, y);
        assert_eq!(entry.ran, "button 1. Rust + ratatui (Recommended)");
        assert_eq!(entry.view_row, Some(button.area.y));
        assert!(entry.line.contains("2. OpenTUI on Bun"), "{}", entry.line);
        assert_eq!(screen.view.entry(screen.view.sheet().width - 3, y).ran, "line");
        assert_eq!(screen.view.entry(0, 0).ran, "nothing");
        assert_eq!(screen.view.entry(0, 0).view_row, None);
    }

    #[test]
    fn a_click_runs_against_the_frame_on_screen() {
        let mut screen = Screen::new(120, 40);
        let regions = screen.regions();
        let button = regions.iter().find(|region| region.is_button()).unwrap();
        let y = screen.show(button.area.y);
        // The data changes and the view scrolls, but the next frame isn't drawn yet.
        screen.view.stale();
        screen.view.scroll_by(5);
        assert_eq!(screen.view.click(button.area.x, y).as_ref(), Some(button.action()));
    }

    #[test]
    fn only_a_change_of_what_is_lit_asks_for_a_frame() {
        let mut screen = Screen::new(120, 40);
        let block = screen.regions().into_iter().find(|region| !region.is_button()).unwrap();
        let y = screen.show(block.area.y);
        assert!(screen.view.hover(Some((50, y))));
        screen.draw();
        assert!(!screen.view.hover(Some((51, y))), "same block");
        assert!(screen.view.hover(None));
    }

    #[test]
    fn time_ticks_only_when_the_page_shows_it() {
        let screen = Screen::new(120, 40);
        // The demo shows `GitLab hh:mm · just now`: once a minute at most.
        let at = screen.view.redraw_at().expect("a clock");
        assert!(at <= Instant::now() + std::time::Duration::from_secs(60));
        assert!(at > Instant::now());
    }
}
