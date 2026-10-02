//! The view on screen (overview/frame.mjs, scroll.mjs): the page is built
//! once per change of its data or width and kept; scrolling and hover only
//! cut a new window out of it and light what the pointer is on. A click
//! resolves against the frame the user sees, not the next one.
//!
//! Two things the Node overview didn't do, so a long view reads like a
//! lazygit panel: the title of the section under the scroll stays on top
//! (`Pin`), and a page built anew (new data, a resize) keeps the block the
//! reader was looking at on top (`Anchor`) instead of the row number.

use super::App;
use super::band::{self, Header};
use super::clicks;
use super::page::{self, Ground, Page, Row, Sheet, guarded};
use super::scene::Target;
use super::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::time::{Duration, Instant};

/// What is lit: the block under the pointer and the button, by their region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Lit {
    block: Option<usize>,
    button: Option<usize>,
}

/// A section's title kept on top of the rows that scroll while the section
/// is under them, as lazygit's panels never lose their title: page rows
/// `from..from + rows` drawn over the first scrolling rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pin {
    from: u16,
    rows: u16,
}

/// Where the page sits on screen, scrolled by `scroll`.
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
    scroll: u16,
    pin: Option<Pin>,
}

impl Window {
    /// The page row on screen row `y`.
    fn page_row(&self, y: u16) -> Option<u16> {
        if y < self.top || y >= self.top + self.height {
            return None;
        }
        let at = y - self.top;
        let row = match at.checked_sub(self.sticky) {
            None => at,
            Some(off) => match self.pin {
                Some(pin) if off < pin.rows => pin.from + off,
                _ => self.sticky + self.scroll + off,
            },
        };
        (row < self.rows).then_some(row)
    }

    /// The screen row of page row `row`, if shown.
    fn screen_y(&self, row: u16) -> Option<u16> {
        if row < self.sticky {
            return Some(self.top + row);
        }
        let pinned = self.pin.map_or(0, |pin| pin.rows);
        if let Some(pin) = self.pin
            && row >= pin.from
            && row < pin.from + pin.rows
        {
            return Some(self.top + self.sticky + row - pin.from);
        }
        let at = row.checked_sub(self.sticky + self.scroll)?;
        (at >= pinned && at < self.room).then_some(self.top + self.sticky + at)
    }

    /// The first page row the scroll puts on top, under the pin if there is one.
    fn first(&self) -> u16 {
        self.sticky + self.scroll
    }

    /// The first page row the reader sees under the sticky rows and the pin.
    fn first_seen(&self) -> u16 {
        self.first() + self.pin.map_or(0, |pin| pin.rows)
    }
}

/// The section whose title has scrolled off while its lines still show:
/// its title, pinned over the first scrolling rows. Only while a line of
/// the section with something on it shows under the pin, so a title never
/// sits over the blank after its section or the next section's title; not
/// in a pane too short to spare it.
fn pin_of<A>(sheet: &Sheet<A>, sticky: u16, room: u16, scroll: u16) -> Option<Pin> {
    let first = sticky + scroll;
    let placed = sheet
        .sections
        .iter()
        .find(|placed| placed.head > 0 && placed.from >= sticky && placed.from < first && first < placed.to)?;
    let under = first + placed.head;
    let is_shown = (under..placed.to.min(first + room)).any(|row| {
        sheet
            .rows
            .get(usize::from(row))
            .is_some_and(|row| row.line.spans.iter().any(|span| !span.content.trim().is_empty()))
    });
    (is_shown && room >= 4 * placed.head).then_some(Pin {
        from: placed.from,
        rows: placed.head,
    })
}

/// The rows of the sticky part that stay on top in a body `height` rows
/// tall: all of them while they leave half of it to the rest, else the
/// first half. The rows past the cut scroll with the rest, so nothing is
/// out of reach, and the cut depends on the pane alone, never on the scroll.
fn sticky_in(sticky: u16, height: u16) -> u16 {
    sticky.min(height / 2)
}

/// Row `into` of `was` rows, in the same thing now `now` rows long: as far
/// into it, in proportion (another width wraps it into more or fewer rows).
fn within(into: u16, was: u16, now: u16) -> u16 {
    let into = if was == now {
        into
    } else {
        (u32::from(into) * u32::from(now) / u32::from(was.max(1))) as u16
    };
    into.min(now.saturating_sub(1))
}

/// What the top of the scrolling rows shows, found again in the page built
/// anew (new data, another width) so the view stays where the reader is:
/// the block there and how far into it, else the section; the end stays
/// the end, the top the top.
#[derive(Clone, Debug)]
struct Anchor<A> {
    /// The scroll it was found at: kept when it still shows the same row.
    scroll: u16,
    is_end: bool,
    /// The block's action, which of the blocks with that action, rows into it, its height.
    block: Option<(A, usize, u16, u16)>,
    /// The section's name, which of that name, rows into it, its rows with the blank after it.
    section: Option<(String, usize, u16, u16)>,
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
    /// The block picked with Tab, as lazygit's selected row: lit like the
    /// pointer's, run by Enter. Its action and which of the blocks with
    /// that action, so it is found again in a page built anew.
    selected: Option<(A, usize)>,
    area: Rect,
    lit: Lit,
    /// The anchor the scroll was last placed by and the scroll it gave:
    /// while the reader hasn't scrolled since, the next build places by it
    /// again, so a resize there and back gives back the same frame.
    placed: Option<(Anchor<A>, u16)>,
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
            selected: None,
            area: Rect::default(),
            lit: Lit::default(),
            placed: None,
        }
    }
}

impl<A: Clone + PartialEq> View<A> {
    /// The data changed: the next frame builds the page again.
    pub fn stale(&mut self) {
        self.is_stale = true;
    }

    /// When something on the page or the band shows time and needs drawing again.
    pub fn redraw_at(&self) -> Option<Instant> {
        let page = self.sheet.as_ref().and_then(|sheet| sheet.redraw_in);
        let after = [page, self.header.redraw_in].into_iter().flatten().min()?;
        Some(self.built_at + after)
    }

    fn build<P: App<Action = A>>(&mut self, app: &P, width: u16) {
        self.header = guarded(|| app.header()).unwrap_or_else(|why| {
            Header::new("breadboard").status(vec![Span::styled(format!("⚠ {why}"), theme::tone(theme::Tone::Red))])
        });
        let mut page = Page::new(width);
        self.sheet = match guarded(|| app.build(&mut page)) {
            Ok(()) => {
                let mut sheet = page.finish();
                if self.area.height < band::TIGHT_BELOW {
                    sheet.tighten();
                }
                Some(sheet)
            }
            Err(why) => Some(failed(
                self.sheet.take().filter(|last| last.width == width),
                width,
                &why,
            )),
        };
        self.band_height = band::shown(&self.header, "", width, self.area.height).len() as u16;
        self.built_at = Instant::now();
        self.is_stale = false;
    }

    /// The window scrolled by `scroll`, as far as the page goes.
    fn window(&self, scroll: u16) -> Window {
        let sheet = self.sheet.as_ref();
        let rows = sheet.map_or(0, |sheet| sheet.rows.len().min(u16::MAX as usize) as u16);
        let top = self.band_height.min(self.area.height);
        let height = self.area.height - top;
        let sticky = sticky_in(sheet.map_or(0, |sheet| sheet.sticky), height);
        let room = height - sticky;
        let max_scroll = rows.saturating_sub(sticky).saturating_sub(room);
        let scroll = scroll.min(max_scroll);
        Window {
            top,
            height,
            sticky,
            room,
            rows,
            max_scroll,
            scroll,
            pin: sheet.and_then(|sheet| pin_of(sheet, sticky, room, scroll)),
        }
    }

    /// The window of the frame on screen.
    fn shown(&self) -> Window {
        self.window(self.shown_scroll)
    }

    fn lit_at(&self, window: &Window) -> Lit {
        if let Some(block) = self.selected_index() {
            return Lit {
                block: Some(block),
                button: None,
            };
        }
        let (Some(sheet), Some((x, y))) = (&self.sheet, self.mouse) else {
            return Lit::default();
        };
        let Some(row) = window.page_row(y) else {
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

    /// The region of the block picked with Tab, in the page on screen.
    fn selected_index(&self) -> Option<usize> {
        let (sheet, (action, nth)) = (self.sheet.as_ref()?, self.selected.as_ref()?);
        let (index, _) = sheet
            .regions
            .iter()
            .enumerate()
            .filter(|(_, region)| !region.is_button() && region.action() == action)
            .nth(*nth)?;
        Some(index)
    }

    /// What Enter runs: the block picked with Tab.
    pub fn selected(&self) -> Option<A> {
        let index = self.selected_index()?;
        Some(self.sheet.as_ref()?.regions[index].action().clone())
    }

    /// Picks the next block (`by` > 0, Tab) or the one before (Shift-Tab),
    /// as lazygit's j/k move the selection: the first one on screen when
    /// none is picked yet, no further than the first or the last; the
    /// scroll follows it. Whether anything changed.
    pub fn select_by(&mut self, by: i32) -> bool {
        let Some(sheet) = &self.sheet else { return false };
        let order = blocks(sheet);
        let Some(last) = order.len().checked_sub(1) else {
            return false;
        };
        let window = self.window(self.scroll);
        let at = match self
            .selected_index()
            .and_then(|index| order.iter().position(|&i| i == index))
        {
            Some(at) => (at as i64 + i64::from(by)).clamp(0, last as i64) as usize,
            None => {
                let is_shown = |i: &usize| window.screen_y(sheet.regions[*i].area.y).is_some();
                if by > 0 {
                    order.iter().position(is_shown).unwrap_or(0)
                } else {
                    order.iter().rposition(is_shown).unwrap_or(last)
                }
            }
        };
        let index = order[at];
        let region = &sheet.regions[index];
        let nth = sheet.regions[..index]
            .iter()
            .filter(|other| !other.is_button() && other.action() == region.action())
            .count();
        let picked = Some((region.action().clone(), nth));
        let area = region.area;
        let is_new = self.selected != picked;
        self.selected = picked;
        self.reveal(area) || is_new
    }

    /// Scrolls the least that shows the page rows of `area`, all of them,
    /// or its top when it is taller than the room. Whether it moved.
    fn reveal(&mut self, area: Rect) -> bool {
        let was = self.scroll;
        let window = self.window(was);
        if area.y < window.sticky || area.height == 0 {
            return false;
        }
        let last = area.bottom() - 1;
        let is_shown = |view: &Self, row| view.window(view.scroll).screen_y(row).is_some();
        if is_shown(self, area.y) && is_shown(self, last) {
            return false;
        }
        if area.y >= window.first_seen() {
            // Below: its last row the last on screen.
            self.scroll = (last + 1 - window.sticky)
                .saturating_sub(window.room)
                .min(window.max_scroll);
        }
        if area.y < window.first_seen() || !is_shown(self, area.y) {
            // Above, or taller than the room: its top on top, under the pin of its section.
            self.scroll = (area.y - window.sticky).min(window.max_scroll);
            while !is_shown(self, area.y) && self.scroll > 0 {
                self.scroll -= 1;
            }
        }
        self.scroll != was
    }

    /// Where the scroll is in the page on screen, to find it again in the next.
    fn anchor(&self) -> Option<Anchor<A>> {
        let sheet = self.sheet.as_ref()?;
        let window = self.window(self.scroll);
        if window.scroll == 0 {
            return None;
        }
        let row = window.first_seen();
        let mut anchor = Anchor {
            scroll: window.scroll,
            is_end: window.scroll >= window.max_scroll,
            block: None,
            section: None,
        };
        let blocks = || {
            sheet
                .regions
                .iter()
                .filter(|region| !region.is_button() && region.area.y >= window.sticky)
        };
        if let Some(block) = blocks()
            .filter(|region| region.area.y <= row && row < region.area.bottom())
            .min_by_key(|region| region.area.height)
        {
            let nth = blocks()
                .take_while(|region| !std::ptr::eq(*region, block))
                .filter(|region| region.action() == block.action())
                .count();
            anchor.block = Some((block.action().clone(), nth, row - block.area.y, block.area.height));
        }
        let mut seen = std::collections::HashMap::<&str, usize>::new();
        for placed in &sheet.sections {
            let nth = seen.entry(placed.name.as_str()).or_default();
            if placed.from <= row && row <= placed.to {
                anchor.section = Some((
                    placed.name.clone(),
                    *nth,
                    row - placed.from,
                    placed.to - placed.from + 1,
                ));
                break;
            }
            *nth += 1;
        }
        Some(anchor)
    }

    /// The scroll that shows `anchor` on top in the page built anew.
    fn place(&self, anchor: &Anchor<A>) -> Option<u16> {
        let sheet = self.sheet.as_ref()?;
        let window = self.window(0);
        if anchor.is_end {
            return Some(window.max_scroll);
        }
        let block = anchor.block.as_ref().and_then(|(action, nth, into, height)| {
            let region = sheet
                .regions
                .iter()
                .filter(|region| !region.is_button() && region.area.y >= window.sticky && region.action() == action)
                .nth(*nth)?;
            Some(region.area.y + within(*into, *height, region.area.height))
        });
        let section = || {
            let (name, nth, into, len) = anchor.section.as_ref()?;
            let placed = sheet.sections.iter().filter(|placed| &placed.name == name).nth(*nth)?;
            Some(placed.from + within(*into, *len, placed.to - placed.from + 1))
        };
        let row = block.or_else(section)?;
        // The row right under the pin of its section, if it gets one; the
        // scroll as it was when that still shows it, so nothing creeps.
        let scroll = row.saturating_sub(window.sticky);
        let candidates = [anchor.scroll, scroll.saturating_sub(1), scroll];
        let scroll = candidates
            .into_iter()
            .find(|&scroll| {
                let window = self.window(scroll);
                window.scroll == scroll && window.first_seen() == row
            })
            .unwrap_or(scroll);
        Some(scroll.min(window.max_scroll))
    }

    /// Draws the frame: the page built again if stale or another width, the
    /// scroll kept on what the reader was looking at.
    pub fn draw<P: App<Action = A>>(&mut self, frame: &mut Frame, app: &P) {
        let area = frame.area();
        // A new height too: a view may fit its lists to the pane (the issues of everything).
        if self.is_stale
            || self.area.height != area.height
            || self.sheet.as_ref().is_none_or(|sheet| sheet.width != area.width)
        {
            let anchor = match self.placed.take() {
                Some((anchor, at)) if at == self.scroll => Some(anchor),
                _ => self.anchor(),
            };
            self.area = area;
            self.build(app, area.width);
            if let Some(anchor) = anchor
                && let Some(scroll) = self.place(&anchor)
            {
                self.scroll = self.window(scroll).scroll;
                self.placed = Some((anchor, self.scroll));
            }
        }
        self.area = area;
        if self.selected.is_some() && self.selected_index().is_none() {
            // Its block is gone from the page built anew.
            self.selected = None;
        }
        let window = self.window(self.scroll);
        self.scroll = window.scroll;
        let scroll = window.scroll;
        let lit = self.lit_at(&window);
        let Some(sheet) = &self.sheet else { return };
        let buffer = frame.buffer_mut();

        let mut mark = band::more_mark(scroll, window.max_scroll - scroll);
        if let Some(index) = lit.block.filter(|_| self.selected.is_some()) {
            // Where the picked block is among all, as lazygit's `9 of 300`.
            let order = blocks(sheet);
            let at = order.iter().position(|&i| i == index).map_or(0, |at| at + 1);
            mark = [format!("{at} of {}", order.len()), mark]
                .join("  ")
                .trim_end()
                .to_string();
        }
        let band_area = Rect::new(0, 0, area.width, window.top);
        buffer.set_style(band_area, Style::new().bg(theme::HEAD).fg(theme::BAND_TEXT));
        for (y, line) in band::shown(&self.header, &mark, area.width, area.height)
            .iter()
            .take(window.top as usize)
            .enumerate()
        {
            buffer.set_line(0, y as u16, line, area.width);
        }

        let lit_block = lit.block.map(|i| sheet.regions[i].area);
        for y in window.top..window.top + window.height {
            let Some(row) = window.page_row(y) else {
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

        // The block picked with Tab: a bar in its left margin, as lazygit's
        // selected row stands out from the hovered one.
        if let Some(block) = lit_block.filter(|_| self.selected.is_some()) {
            for row in block.y..block.y + block.height {
                if let Some(y) = window.screen_y(row)
                    && y < area.height
                {
                    buffer[(0, y)].set_symbol("▌").set_fg(theme::Tone::Cyan.fg());
                }
            }
        }

        // The button under the pointer: its own, stronger light.
        if let Some(region) = lit.button.map(|i| &sheet.regions[i])
            && let Target::Button { tone, .. } = region.target
        {
            for row in region.area.y..region.area.y + region.area.height {
                if let Some(y) = window.screen_y(row) {
                    let cells = Rect::new(region.area.x, y, region.area.width, 1).intersection(buffer.area);
                    buffer.set_style(cells, tone.lit());
                }
            }
        }

        // Where the window is: a thumb in the island's right margin.
        // It runs beside the rows only, never on a pinned title.
        let pinned = window.pin.map_or(0, |pin| pin.rows);
        if window.max_scroll > 0 && window.room > pinned && area.width > 0 {
            let track = u32::from(window.room - pinned);
            let total = u32::from(window.rows - window.sticky);
            let thumb = (track * u32::from(window.room) / total).clamp(1, track);
            let at = u32::from(scroll) * (track - thumb) / u32::from(window.max_scroll);
            for i in at..at + thumb {
                let cell = &mut buffer[(area.width - 1, window.top + window.sticky + pinned + i as u16)];
                cell.set_symbol("┃").set_fg(theme::BAND_QUIET);
            }
        }

        self.shown_scroll = scroll;
        self.lit = lit;
    }

    /// The pointer moved (None: it left). Whether what is lit changes: only
    /// then a frame is drawn, not on every move.
    pub fn hover(&mut self, at: Option<(u16, u16)>) -> bool {
        // The pointer moved: it lights what it is on again, not the pick.
        let was_picked = at.is_some() && self.selected.take().is_some();
        self.mouse = at;
        was_picked || self.lit_at(&self.shown()) != self.lit
    }

    /// What a click at screen cell (x, y) runs, in the frame on screen.
    pub fn click(&self, x: u16, y: u16) -> Option<A> {
        let row = self.shown().page_row(y)?;
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
        let (Some(sheet), Some(row)) = (&self.sheet, self.shown().page_row(y)) else {
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
        let max = self.window(0).max_scroll;
        let to = (i32::from(self.scroll) + by).clamp(0, i32::from(max)) as u16;
        let has_moved = to != self.scroll;
        self.scroll = to;
        has_moved
    }

    /// Scrolls by `pages`: what the pane shows under the sticky rows, less
    /// one, so the last row seen stays on screen as the first (PgDn) or the
    /// first as the last (PgUp), under a pinned title too.
    ///
    /// It lands where the top row the reader sees starts something, as
    /// lazygit's pages start at an item: never inside a block (its name
    /// scrolled off) or on a section's line whose title is gone without a
    /// pin. For that it goes up to half a page less far, never further;
    /// the end and the top it reaches as they are.
    pub fn page_by(&mut self, pages: i32) -> bool {
        let mut has_moved = false;
        for _ in 0..pages.unsigned_abs() {
            let now = self.window(self.scroll);
            let step = now.room.saturating_sub(1);
            let step = if pages > 0 {
                // The pin of where it lands covers that window's first row.
                let then = self.window(now.scroll.saturating_add(step));
                step.saturating_sub(then.pin.map_or(0, |pin| pin.rows))
            } else {
                step.saturating_sub(now.pin.map_or(0, |pin| pin.rows))
            };
            let step = step.max(1);
            let less = step / 2;
            let target = if pages > 0 {
                now.scroll.saturating_add(step).min(now.max_scroll)
            } else {
                now.scroll.saturating_sub(step)
            };
            // The end and the top are where a page lands as they are.
            let is_edge = target == 0 || target == now.max_scroll;
            let landing = (0..=if is_edge { 0 } else { less })
                .map(|back| {
                    if pages > 0 {
                        target.saturating_sub(back).max(now.scroll)
                    } else {
                        target.saturating_add(back).min(now.scroll)
                    }
                })
                .take_while(|&scroll| scroll != now.scroll)
                .find(|&scroll| !self.is_cut(&self.window(scroll)))
                .unwrap_or(target);
            let has_moved_now = landing != self.scroll;
            self.scroll = landing;
            has_moved |= has_moved_now;
        }
        has_moved
    }

    /// Whether the top row the reader sees in `window` starts nothing: it
    /// is inside a block that fits half the room, below the block's first
    /// row, or a line of a section whose title scrolled off with no pin.
    fn is_cut(&self, window: &Window) -> bool {
        let Some(sheet) = &self.sheet else { return false };
        let row = window.first_seen();
        if row >= window.rows || window.scroll == 0 {
            return false;
        }
        let is_in_block = sheet.regions.iter().any(|region| {
            !region.is_button()
                && region.area.y >= window.sticky
                && region.area.y < row
                && row < region.area.bottom()
                && region.area.height <= window.room / 2
        });
        let is_orphan = window.pin.is_none()
            && sheet
                .sections
                .iter()
                .any(|placed| placed.head > 0 && placed.from >= window.sticky && placed.from < row && row < placed.to);
        is_in_block || is_orphan
    }

    /// Scrolls by `halves` of a page: Ctrl-D and Ctrl-U, as in less and vim.
    pub fn half_by(&mut self, halves: i32) -> bool {
        let half = i32::from((self.window(0).room / 2).max(1));
        self.scroll_by(halves * half)
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
        self.shown().screen_y(row)
    }

    /// The page row on screen row `y` in the frame on screen.
    #[cfg(test)]
    pub fn page_row(&self, y: u16) -> Option<u16> {
        self.shown().page_row(y)
    }

    #[cfg(test)]
    pub fn body(&self) -> (u16, u16) {
        let window = self.window(0);
        (window.top, window.height)
    }
}

/// The blocks of a page in reading order: from the top, an outer block
/// before the ones inside it.
fn blocks<A>(sheet: &Sheet<A>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..sheet.regions.len())
        .filter(|&i| !sheet.regions[i].is_button())
        .collect();
    order.sort_by_key(|&i| (sheet.regions[i].area.y, std::cmp::Reverse(sheet.regions[i].area.height)));
    order
}

/// How often a view that failed to draw is built again (Node's local read).
const TRY_AGAIN: Duration = Duration::from_secs(2);

/// The view failed to draw as a whole (guard.mjs `guarded`): the last good
/// page with its first line replaced, not pushed down, by
/// ` the overview failed to draw: <why> · trace in <log> · trying again`,
/// so every other row keeps its place and its click; built again in a while.
fn failed<A: Clone>(last: Option<Sheet<A>>, width: u16, why: &str) -> Sheet<A> {
    let log = page::log_error(why);
    let text = format!(
        " the overview failed to draw: {} · trace in {} · trying again",
        page::message_of(why),
        log.display()
    );
    let row = Row {
        line: Line::from(Span::styled(text, theme::tone(theme::Tone::Red))),
        ground: Ground::Plain,
    };
    let mut sheet = last.unwrap_or_else(|| Page::new(width).finish());
    match sheet.rows.first_mut() {
        Some(first) => *first = row,
        None => sheet.rows.push(row),
    }
    sheet.regions.retain(|region| region.area.y > 0);
    sheet.redraw_in = Some(sheet.redraw_in.map_or(TRY_AGAIN, |was| was.min(TRY_AGAIN)));
    sheet
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
    use crate::tui::{Flow, Input, Section};
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

        /// Scrolls so page row `row` is on screen, as high as it goes (a
        /// pinned title may cover the top row), and where it is.
        fn show(&mut self, row: u16) -> u16 {
            self.view.scroll_top();
            let sticky = self.view.sheet().sticky;
            if row >= sticky {
                self.view.scroll_by(i32::from(row - sticky));
            }
            self.draw();
            while self.view.screen_y(row).is_none() && self.view.scroll_by(-1) {
                self.draw();
            }
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
        let row = screen.view.page_row(y).unwrap();
        let under = screen.view.sheet().block_at(60, row).map(|(_, region)| region.area);
        assert_eq!(screen.bg(60, y) == Some(theme::HOVER), under.is_some(), "{under:?}");
    }

    #[test]
    fn a_panicking_section_draws_its_error_and_the_rest_still_works() {
        let mut screen = Screen::new(120, 40);
        screen.click(Action::Break);
        let text = page_text(&screen);
        let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            text.contains("\n  BROKEN failed to draw: ")
                && words.contains(
                    "BROKEN failed to draw: Cannot read properties of undefined (reading 'branch') · trace in "
                ),
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
        let window = screen.view.window(0);
        assert!(window.max_scroll > 0);
        assert!(!screen.view.scroll_by(-1), "above the top");
        assert!(!screen.view.page_by(-1));
        assert!(screen.view.page_by(1));
        assert!(screen.view.scroll() >= window.room - 2 && screen.view.scroll() < window.room);
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
        assert_eq!(screen.view.scroll(), screen.view.window(0).max_scroll);
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

    /// The block whose rows start the scrolling part (under a pinned title).
    fn block_on_top(screen: &Screen) -> Option<Action> {
        let window = screen.view.shown();
        let first = window.first() + window.pin.map_or(0, |pin| pin.rows);
        screen
            .view
            .sheet()
            .block_at(0, first)
            .map(|(_, region)| region.action().clone())
    }

    #[test]
    fn a_title_scrolled_off_stays_on_top_while_its_section_shows() {
        let mut screen = Screen::new(120, 40);
        let placed = screen
            .view
            .sheet()
            .sections
            .iter()
            .find(|placed| placed.name == "WORKTREES")
            .cloned()
            .unwrap();
        let sticky = screen.view.shown().sticky;
        screen.view.scroll_by(i32::from(placed.from + placed.head + 3 - sticky));
        screen.draw();
        let (top, _) = screen.view.body();
        let y = top + sticky;
        let row: String = (0..120)
            .map(|x| screen.terminal.backend().buffer()[(x, y)].symbol().to_string())
            .collect();
        assert!(row.starts_with(" WORKTREES 6 wait for GitLab"), "{row}");
        assert_eq!(screen.bg(60, y), Some(theme::HEAD));
        // The title does nothing on a click and doesn't light.
        assert_eq!(screen.view.click(60, y), None);
        screen.view.hover(Some((60, y)));
        screen.draw();
        assert_eq!(screen.bg(60, y), Some(theme::HEAD));
        // The row it covers is not on screen; the one after it is, right under it.
        let first = screen.view.shown().first();
        assert_eq!(screen.view.screen_y(first), None);
        assert_eq!(screen.view.screen_y(first + 1), Some(y + 1));
        // Past the section's end the next title takes over: none while a title shows by itself.
        screen.view.scroll_by(i32::from(placed.to - first));
        screen.draw();
        assert!(screen.view.shown().pin.is_none());
        // At the top nothing is pinned.
        screen.view.scroll_top();
        screen.draw();
        assert!(screen.view.shown().pin.is_none());
    }

    #[test]
    fn a_title_is_pinned_only_while_a_line_of_its_section_shows_under_it() {
        for (width, height) in [(80, 24), (120, 40), (200, 30)] {
            let mut screen = Screen::new(width, height);
            let max = screen.view.window(0).max_scroll;
            for scroll in 0..=max {
                screen.view.scroll_top();
                screen.view.scroll_by(i32::from(scroll));
                screen.draw();
                let window = screen.view.shown();
                let Some(pin) = window.pin else { continue };
                let sheet = screen.view.sheet();
                let placed = sheet.sections.iter().find(|placed| placed.from == pin.from).unwrap();
                // Under the pin: a line of its own section with text on it.
                let under = window.first_seen();
                assert!(under < placed.to, "{width}x{height} at {scroll}: {placed:?}");
                let text = sheet.text();
                assert!(
                    (under..placed.to.min(window.first() + window.room))
                        .any(|row| !text[row as usize].trim().is_empty()),
                    "{width}x{height} at {scroll}: {placed:?}"
                );
            }
            // A section scrolled up to its last line: no pin over that line.
            for placed in screen.view.sheet().sections.clone() {
                let sticky = screen.view.shown().sticky;
                if placed.head == 0 || placed.from < sticky || placed.to < placed.from + placed.head + 1 {
                    continue;
                }
                screen.view.scroll_top();
                if !screen.view.scroll_by(i32::from(placed.to - 1 - sticky)) {
                    continue;
                }
                screen.draw();
                let window = screen.view.shown();
                if window.first() == placed.to - 1 {
                    assert!(window.pin.is_none(), "{width}x{height}: {placed:?}");
                    assert_eq!(screen.view.screen_y(placed.to - 1), Some(window.top + window.sticky));
                }
            }
        }
    }

    #[test]
    fn another_width_keeps_the_block_on_top() {
        let mut screen = Screen::new(120, 40);
        let block = screen
            .regions()
            .into_iter()
            .find(|region| region.target == Target::Block(Action::Open("#296".into())))
            .unwrap();
        screen.show(block.area.y + 1);
        assert_eq!(block_on_top(&screen), Some(Action::Open("#296".into())));
        for width in [80, 200, 120] {
            screen.terminal.backend_mut().resize(width, 40);
            screen.draw();
            assert_eq!(block_on_top(&screen), Some(Action::Open("#296".into())), "{width}");
        }
    }

    /// A list of 60 blocks of two lines under `above` lines that come and go.
    struct List {
        above: u16,
    }

    impl App for List {
        type Action = u16;
        type Model = ();
        fn header(&self) -> Header {
            Header::new("list")
        }
        fn build(&self, page: &mut Page<u16>) {
            page.section("ABOVE", |scene| {
                for i in 0..self.above {
                    scene.text(&format!("line {i}"), Style::new());
                    scene.newline();
                }
            });
            page.section("LIST", |scene| {
                for i in 0..60 {
                    scene.block(i, |scene| {
                        scene.text(&format!("item {i}"), Style::new());
                        scene.newline();
                        scene.text("its second line", Style::new());
                    });
                }
            });
        }
        fn update(&mut self, _: ()) {}
        fn act(&mut self, _: Input<u16>) -> Flow {
            Flow::Same
        }
    }

    #[test]
    fn rows_above_that_come_and_go_keep_the_reader_where_they_were() {
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        let mut view = View::<u16>::default();
        let mut app = List { above: 2 };
        let mut draw = |view: &mut View<u16>, app: &List| {
            terminal.draw(|frame| view.draw(frame, app)).unwrap();
        };
        draw(&mut view, &app);
        let on_top = |view: &View<u16>| {
            let first = view.shown().first_seen();
            view.sheet().block_at(0, first).map(|(_, region)| *region.action())
        };
        view.scroll_by(30);
        draw(&mut view, &app);
        let was = on_top(&view).expect("a block on top");
        let scroll = view.scroll();
        for above in [40, 0, 2] {
            app.above = above;
            view.stale();
            draw(&mut view, &app);
            assert_eq!(on_top(&view), Some(was), "{above} above");
        }
        assert_eq!(view.scroll(), scroll);
        // The end stays the end, the top the top.
        view.scroll_bottom();
        draw(&mut view, &app);
        app.above = 40;
        view.stale();
        draw(&mut view, &app);
        assert_eq!(view.scroll(), view.window(0).max_scroll);
        view.scroll_top();
        draw(&mut view, &app);
        app.above = 2;
        view.stale();
        draw(&mut view, &app);
        assert_eq!(view.scroll(), 0);
    }

    #[test]
    fn a_rebuild_of_the_same_page_never_moves_the_scroll() {
        for width in [80, 120] {
            let mut screen = Screen::new(width, 30);
            let max = screen.view.window(0).max_scroll;
            for scroll in 0..=max {
                screen.view.scroll_top();
                screen.view.scroll_by(i32::from(scroll));
                screen.draw();
                screen.view.stale();
                screen.draw();
                assert_eq!(screen.view.scroll(), scroll, "{width}");
            }
        }
    }

    /// The page rows the reader sees in the scrolling part, under the pin.
    fn seen(screen: &Screen) -> std::ops::Range<u16> {
        let window = screen.view.shown();
        window.first_seen()..(window.first() + window.room).min(window.rows)
    }

    #[test]
    fn a_page_keeps_one_row_seen_and_lands_on_the_head_of_a_block() {
        for (width, height) in [(120, 40), (80, 24), (200, 30)] {
            let mut screen = Screen::new(width, height);
            let (mut pins, mut pages) = (0, 0);
            loop {
                let was = seen(&screen);
                if !screen.view.page_by(1) {
                    break;
                }
                screen.draw();
                let now = seen(&screen);
                pins += usize::from(screen.view.shown().pin.is_some());
                pages += 1;
                assert!(now.start < was.end, "{width}x{height}: {was:?} then {now:?}");
                let window = screen.view.shown();
                let is_end = screen.view.scroll() == window.max_scroll;
                // At least half a page on, never a block cut at the top.
                assert!(
                    is_end || now.start + window.room / 2 >= was.end,
                    "{width}x{height}: {was:?} then {now:?}"
                );
                assert!(is_end || !screen.view.is_cut(&window), "{width}x{height}: {now:?}");
            }
            assert!(pages > 1, "{width}x{height}: {pages} pages");
            assert!(pins > 0, "{width}x{height}: no pin");
            loop {
                let was = seen(&screen);
                if !screen.view.page_by(-1) {
                    break;
                }
                screen.draw();
                let now = seen(&screen);
                assert!(now.end > was.start, "{width}x{height}: {was:?} then {now:?}");
                let window = screen.view.shown();
                let is_top = screen.view.scroll() == 0;
                assert!(
                    is_top || now.end <= was.start + window.room / 2,
                    "{width}x{height}: {was:?} then {now:?}"
                );
                assert!(!screen.view.is_cut(&window), "{width}x{height}: {now:?}");
            }
            assert_eq!(screen.view.scroll(), 0);
        }
    }

    #[test]
    fn a_resize_there_and_back_gives_back_the_same_frame() {
        let text = |screen: &Screen| -> Vec<String> {
            let buffer = screen.terminal.backend().buffer();
            (0..buffer.area.height)
                .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect())
                .collect()
        };
        let mut screen = Screen::new(120, 40);
        let max = screen.view.window(0).max_scroll;
        for scroll in (1..max).step_by(3) {
            screen.view.scroll_top();
            screen.view.scroll_by(i32::from(scroll));
            screen.draw();
            let before = text(&screen);
            for (width, height) in [(80, 24), (200, 30), (60, 50), (120, 40)] {
                screen.terminal.backend_mut().resize(width, height);
                screen.draw();
            }
            assert_eq!(text(&screen), before, "scroll {scroll}");
        }
    }

    #[test]
    fn the_thumb_runs_beside_the_rows_never_on_a_pinned_title() {
        for (width, height) in [(120, 40), (80, 24)] {
            let mut screen = Screen::new(width, height);
            let max = screen.view.window(0).max_scroll;
            let mut pins = 0;
            for _ in 0..=max {
                screen.draw();
                let window = screen.view.shown();
                if let Some(pin) = window.pin {
                    pins += 1;
                    let top = window.top + window.sticky;
                    for y in top..top + pin.rows {
                        let cell = &screen.terminal.backend().buffer()[(width - 1, y)];
                        assert_ne!(cell.symbol(), "┃", "{width}x{height} at {}", window.scroll);
                    }
                }
                screen.view.scroll_by(1);
            }
            assert!(pins > 0, "{width}x{height}");
        }
    }

    /// Sticky rows over a list, as many as `sticky`.
    struct Tall {
        sticky: u16,
    }

    impl App for Tall {
        type Action = u16;
        type Model = ();
        fn header(&self) -> Header {
            Header::new("tall")
        }
        fn build(&self, page: &mut Page<u16>) {
            page.section(Section::new("NEXT").sticky().plain(), |scene| {
                for i in 0..self.sticky {
                    scene.text(&format!("next {i}"), Style::new());
                    scene.newline();
                }
            });
            page.section("LIST", |scene| {
                for i in 0..60 {
                    scene.block(i, |scene| scene.text(&format!("item {i}"), Style::new()));
                }
            });
        }
        fn update(&mut self, _: ()) {}
        fn act(&mut self, _: Input<u16>) -> Flow {
            Flow::Same
        }
    }

    #[test]
    fn a_sticky_part_taller_than_half_the_pane_is_cut_and_its_rest_scrolls() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut view = View::<u16>::default();
        let app = Tall { sticky: 14 };
        terminal.draw(|frame| view.draw(frame, &app)).unwrap();
        let (_, height) = view.body();
        let window = view.shown();
        // The top half stays, never none of it, whatever the scroll.
        assert_eq!(window.sticky, height / 2);
        for _ in 0..30 {
            view.scroll_by(1);
            terminal.draw(|frame| view.draw(frame, &app)).unwrap();
            assert_eq!(view.shown().sticky, height / 2);
            assert_eq!(view.page_row(window.top), Some(0));
        }
        // The rows of NEXT past the cut can be read: they scroll.
        view.scroll_top();
        terminal.draw(|frame| view.draw(frame, &app)).unwrap();
        assert!(view.screen_y(13).is_some());
    }

    #[test]
    fn half_a_page_and_back() {
        let mut screen = Screen::new(120, 40);
        let room = screen.view.shown().room;
        assert!(screen.view.half_by(1));
        assert_eq!(screen.view.scroll(), room / 2);
        assert!(screen.view.half_by(-1));
        assert_eq!(screen.view.scroll(), 0);
    }

    #[test]
    fn time_ticks_only_when_the_page_shows_it() {
        let screen = Screen::new(120, 40);
        // The demo shows `GitLab hh:mm · just now`: once a minute at most.
        let at = screen.view.redraw_at().expect("a clock");
        assert!(at <= Instant::now() + std::time::Duration::from_secs(60));
        assert!(at > Instant::now());
    }

    #[test]
    fn tab_picks_the_blocks_in_order_the_scroll_follows_and_enter_runs_the_pick() {
        let mut screen = Screen::new(120, 24);
        let order = blocks(screen.view.sheet());
        assert!(order.len() > 5);
        assert_eq!(screen.view.selected(), None);
        let mut seen = vec![];
        for _ in 0..order.len() + 2 {
            screen.view.select_by(1);
            screen.draw();
            let picked = screen.view.selected().expect("picked");
            let index = screen.view.selected_index().unwrap();
            let area = screen.view.sheet().regions[index].area;
            // Its rows are on screen (its top at least, when it is taller than the room), lit, with the bar.
            let y = screen.view.screen_y(area.y).expect("its top on screen");
            assert_eq!(screen.bg(60, y), Some(theme::HOVER), "{picked:?}");
            assert_eq!(screen.terminal.backend().buffer()[(0, y)].symbol(), "▌");
            if area.height < screen.view.shown().room {
                assert!(screen.view.screen_y(area.bottom() - 1).is_some(), "{picked:?}");
            }
            seen.push(index);
        }
        // Every block once, in reading order, then it stays on the last.
        let mut expected = order.clone();
        let first_on_screen = expected[0];
        expected.extend([*order.last().unwrap(), *order.last().unwrap()]);
        assert_eq!(seen[0], first_on_screen);
        assert_eq!(seen, expected);
        let band: String = (0..120)
            .map(|x| screen.terminal.backend().buffer()[(x, 0)].symbol().to_string())
            .collect();
        assert!(band.contains(&format!("{} of {}", order.len(), order.len())), "{band}");
        // Back up to the first: the scroll follows to the top.
        for _ in 0..order.len() {
            screen.view.select_by(-1);
        }
        screen.draw();
        assert_eq!(screen.view.selected_index(), Some(order[0]));
        // Enter runs it: the run loop sends the pick as a click.
        let picked = screen.view.selected().unwrap();
        assert_eq!(screen.view.sheet().regions[order[0]].action(), &picked);
        // The pointer moving takes the light back.
        assert!(screen.view.hover(Some((60, 10))));
        assert_eq!(screen.view.selected(), None);
    }

    #[test]
    fn a_short_pane_has_the_band_without_its_blank_lines() {
        let tall = Screen::new(80, 40);
        let short = Screen::new(80, 24);
        assert_eq!(tall.view.body().0, 5);
        assert_eq!(short.view.body().0, 3);
        let row = |screen: &Screen, y| -> String {
            (0..80)
                .map(|x| screen.terminal.backend().buffer()[(x, y)].symbol().to_string())
                .collect()
        };
        assert!(row(&short, 1).contains("breadboard demo"), "{}", row(&short, 1));
    }

    #[test]
    fn a_short_pane_drops_the_blank_after_the_sticky_part_and_clicks_still_land() {
        let app = Tall { sticky: 3 };
        for (height, sticky) in [(40, 4), (24, 3)] {
            let mut terminal = Terminal::new(TestBackend::new(80, height)).unwrap();
            let mut view = View::<u16>::default();
            terminal.draw(|frame| view.draw(frame, &app)).unwrap();
            assert_eq!(view.sheet().sticky, sticky, "{height}");
            let (top, _) = view.body();
            // LIST's title right under the sticky part, then its first item.
            let list = view
                .sheet()
                .sections
                .iter()
                .find(|placed| placed.name == "LIST")
                .unwrap();
            assert_eq!(list.from, sticky, "{height}");
            assert_eq!(view.click(2, top + sticky + 1), Some(0), "{height}");
            assert_eq!(view.click(2, top + sticky + 2), Some(1), "{height}");
        }
    }

    #[test]
    fn a_view_that_fails_keeps_its_last_page_with_the_error_on_its_first_line() {
        let mut page = Page::<u8>::new(60);
        page.section("FIRST", |scene| scene.block(1, |scene| scene.text("one", Style::new())));
        page.section("SECOND", |scene| {
            scene.block(2, |scene| scene.text("two", Style::new()))
        });
        let last = page.finish();
        let was = last.text();
        let sheet = failed(Some(last), 60, "boom (at src/x.rs:1:1)");
        let text = sheet.text();
        assert!(
            text[0].starts_with(" the overview failed to draw: boom · trace in ")
                && text[0].ends_with(" · trying again"),
            "{text:?}"
        );
        assert_eq!(text[1..], was[1..], "every other row keeps its place");
        assert!(sheet.regions.iter().any(|region| *region.action() == 2));
        assert_eq!(sheet.redraw_in, Some(TRY_AGAIN));
        // Nothing before it: the line alone.
        assert_eq!(failed::<u8>(None, 60, "boom").text().len(), 1);
    }
}
