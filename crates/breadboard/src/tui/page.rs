//! The page: every section of a view, each built on its own. A section that
//! panics draws its error in its own place while the others draw (#306);
//! the sticky ones stay on top while the rest scrolls under them.

use super::scene::{self, Region, Scene, Target};
use super::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

thread_local! {
    /// Set while a section builds: its panic is caught and drawn in its place.
    static IN_SECTION: Cell<bool> = const { Cell::new(false) };
    /// Where the last section's panic happened, for its error line.
    static PANIC_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Whether the panic happening now is a section's, caught by `Page::section`.
pub fn is_in_section() -> bool {
    IN_SECTION.with(Cell::get)
}

/// `run` with its panic caught, as the text it panicked with and where:
/// the panic hook leaves the terminal alone meanwhile. Nests.
pub fn guarded<R>(run: impl FnOnce() -> R) -> Result<R, String> {
    let was = IN_SECTION.with(|flag| flag.replace(true));
    let result = catch_unwind(AssertUnwindSafe(run));
    IN_SECTION.with(|flag| flag.set(was));
    result.map_err(|panic| {
        let why = panic_text(&*panic);
        match PANIC_AT.with(|cell| cell.borrow_mut().take()) {
            Some(at) => format!("{why} (at {at})"),
            None => why,
        }
    })
}

/// The panic hook keeps where a section panicked: the payload doesn't say.
pub fn note_panic_at(at: String) {
    PANIC_AT.with(|cell| *cell.borrow_mut() = Some(at));
}

/// How a section sits on the page.
#[derive(Clone, Debug)]
pub struct Section {
    name: String,
    is_sticky: bool,
    is_plain: bool,
}

impl Section {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_sticky: false,
            is_plain: false,
        }
    }

    /// Stays on top while the rest scrolls (the orchestration's NEXT); only
    /// while it leaves at least half the pane to the rest.
    pub fn sticky(mut self) -> Self {
        self.is_sticky = true;
        self
    }

    /// No island and no title line: its lines alone (NEXT, the chat line).
    pub fn plain(mut self) -> Self {
        self.is_plain = true;
        self
    }
}

impl From<&str> for Section {
    fn from(name: &str) -> Self {
        Section::new(name)
    }
}

/// What a row of the page is drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    /// The terminal's own, between islands and under plain sections.
    Plain,
    Island,
    Head,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub line: Line<'static>,
    pub ground: Ground,
}

/// Where a section sits on the page: its title line(s) from `from`, its
/// lines up to `to` (the blank line after it not counted).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub name: String,
    pub from: u16,
    /// Rows of its title line: 0 for a plain section.
    pub head: u16,
    pub to: u16,
}

struct Part<A> {
    rows: Vec<Row>,
    regions: Vec<Region<A>>,
    sections: Vec<Placed>,
}

impl<A> Part<A> {
    fn new() -> Self {
        Self {
            rows: vec![],
            regions: vec![],
            sections: vec![],
        }
    }
}

/// The page while an app adds its sections, in the order they are shown.
pub struct Page<A> {
    width: u16,
    top: Part<A>,
    rest: Part<A>,
    redraw_in: Option<Duration>,
}

/// The page laid out: rows and their regions in page coordinates, the first
/// `sticky` rows the ones that stay on top.
pub struct Sheet<A> {
    pub width: u16,
    pub rows: Vec<Row>,
    pub regions: Vec<Region<A>>,
    pub sections: Vec<Placed>,
    pub sticky: u16,
    pub redraw_in: Option<Duration>,
}

impl<A: Clone> Page<A> {
    pub fn new(width: u16) -> Self {
        Self {
            width,
            top: Part::new(),
            rest: Part::new(),
            redraw_in: None,
        }
    }

    /// The columns inside an island: a column of margin on both sides.
    pub fn inner(&self) -> u16 {
        self.width.saturating_sub(2).max(1)
    }

    /// A section: `build` writes its lines into a scene as wide as the inside
    /// of its island. A panic in it is caught: the section shows its name and
    /// why, the others draw as usual.
    pub fn section(&mut self, section: impl Into<Section>, build: impl FnOnce(&mut Scene<A>)) {
        let section = section.into();
        let mut scene = Scene::new(self.inner());
        match guarded(|| build(&mut scene)) {
            Ok(()) => {
                scene.finish();
                if let Some(after) = scene.redraw_in {
                    self.redraw_in = Some(self.redraw_in.map_or(after, |was| was.min(after)));
                }
                self.add(&section, scene, section.is_plain);
            }
            Err(why) => self.failed(section, &why),
        }
    }

    /// A section that failed to draw: its name and why, in its own place.
    pub fn failed(&mut self, section: impl Into<Section>, why: &str) {
        let section = section.into();
        let mut failed = Scene::new(self.inner());
        failed.note("failed to draw");
        failed.indent(1);
        failed.text(&format!("⚠ {why}"), theme::tone(theme::Tone::Red));
        failed.finish();
        self.add(&section, failed, false);
    }

    /// The scene of a section onto the page: its title line, its lines on
    /// the island, a blank line after it.
    fn add(&mut self, section: &Section, scene: Scene<A>, is_plain: bool) {
        let width = self.width;
        let inner = self.inner();
        let part = if section.is_sticky {
            &mut self.top
        } else {
            &mut self.rest
        };
        let from = part.rows.len() as u16;
        if !is_plain {
            let mut head = Scene::<A>::new(inner);
            scene::heading(&mut head, &section.name, scene.note.as_deref());
            head.finish();
            part.rows
                .extend(head.lines.into_iter().map(|line| margin(line, Ground::Head)));
        }
        let top = part.rows.len() as u16;
        let ground = if is_plain { Ground::Plain } else { Ground::Island };
        part.rows
            .extend(scene.lines.into_iter().map(|line| margin(line, ground)));
        part.regions.extend(scene.regions.into_iter().map(|mut region| {
            region.area.y += top;
            match region.target {
                // A block lights the whole row, margins too.
                Target::Block(_) => region.area = Rect::new(0, region.area.y, width, region.area.height),
                Target::Button { .. } => region.area.x += 1,
            }
            region
        }));
        part.sections.push(Placed {
            name: section.name.clone(),
            from,
            head: top - from,
            to: part.rows.len() as u16,
        });
        part.rows.push(Row {
            line: Line::default(),
            ground: Ground::Plain,
        });
    }

    pub fn finish(self) -> Sheet<A> {
        let sticky = self.top.rows.len() as u16;
        let mut rows = self.top.rows;
        let mut regions = self.top.regions;
        let mut sections = self.top.sections;
        rows.extend(self.rest.rows);
        regions.extend(self.rest.regions.into_iter().map(|mut region| {
            region.area.y += sticky;
            region
        }));
        sections.extend(self.rest.sections.into_iter().map(|mut placed| {
            placed.from += sticky;
            placed.to += sticky;
            placed
        }));
        Sheet {
            width: self.width,
            rows,
            regions,
            sections,
            sticky,
            redraw_in: self.redraw_in,
        }
    }
}

impl<A> Sheet<A> {
    /// What a mouse at page position (x, row) hits: a button before its
    /// block, the innermost block of nested ones.
    pub fn hit(&self, x: u16, row: u16) -> Option<(usize, &Region<A>)> {
        let mut inside = self
            .regions
            .iter()
            .enumerate()
            .filter(|(_, region)| region.contains(x, row));
        let button = inside.clone().find(|(_, region)| region.is_button());
        button.or_else(|| inside.by_ref().min_by_key(|(_, region)| region.area.height))
    }

    /// The block under (x, row), the innermost.
    pub fn block_at(&self, x: u16, row: u16) -> Option<(usize, &Region<A>)> {
        self.regions
            .iter()
            .enumerate()
            .filter(|(_, region)| !region.is_button() && region.contains(x, row))
            .min_by_key(|(_, region)| region.area.height)
    }

    /// The page as plain text, one string per row (`--once`, the tests).
    pub fn text(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.line.spans.iter().map(|span| span.content.as_ref()).collect())
            .collect()
    }
}

/// A line with the island's column of margin before it.
fn margin(line: Line<'static>, ground: Ground) -> Row {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(line.spans);
    Row {
        line: Line::from(spans).style(Style::new()),
        ground,
    }
}

pub fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|text| text.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panicked".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::Tone;

    #[test]
    fn a_panicking_section_draws_its_error_in_its_place() {
        let mut page = Page::<u8>::new(60);
        page.section("FIRST", |scene| scene.text("before", Style::new()));
        page.section("BROKEN", |_| {
            let issue: Option<&str> = std::hint::black_box(None);
            issue.expect("Cannot read properties of undefined (reading 'branch')");
        });
        page.section("LAST", |scene| scene.button("after", Tone::Cyan, 3));
        let sheet = page.finish();
        let text = sheet.text().join("\n");
        assert!(text.contains(" FIRST\n before\n"), "{text}");
        assert!(
            text.contains(" BROKEN failed to draw\n  ⚠ Cannot read properties of undefined"),
            "{text}"
        );
        assert!(text.contains(" LAST\n  after "), "{text}");
        assert!(!is_in_section());
        // The section after the broken one still clicks.
        let button = sheet.regions.iter().find(|r| r.is_button()).unwrap();
        assert_eq!(
            sheet.hit(button.area.x, button.area.y).map(|(_, r)| *r.action()),
            Some(3)
        );
    }

    #[test]
    fn sticky_sections_go_first() {
        let mut page = Page::<u8>::new(40);
        page.section("REST", |scene| scene.block(1, |scene| scene.text("rest", Style::new())));
        page.section(Section::new("NEXT").sticky().plain(), |scene| {
            scene.text("next", Style::new())
        });
        let sheet = page.finish();
        assert_eq!(sheet.text()[..4], [" next", "", " REST", " rest"]);
        assert_eq!(sheet.sticky, 2);
        assert_eq!(sheet.regions[0].area, Rect::new(0, 3, 40, 1));
        let placed = |name: &str, from, head, to| Placed {
            name: name.into(),
            from,
            head,
            to,
        };
        assert_eq!(sheet.sections, [placed("NEXT", 0, 0, 1), placed("REST", 2, 1, 4)]);
    }

    #[test]
    fn a_button_wins_over_its_block_and_an_inner_block_over_the_outer() {
        let mut page = Page::<u8>::new(40);
        page.section("S", |scene| {
            scene.block(1, |scene| {
                scene.text("outer", Style::new());
                scene.block(2, |scene| {
                    scene.text("inner", Style::new());
                    scene.button("go", Tone::Cyan, 3);
                });
            });
        });
        let sheet = page.finish();
        let action = |x, y| sheet.hit(x, y).map(|(_, region)| *region.action());
        assert_eq!(action(30, 1), Some(1));
        assert_eq!(action(30, 2), Some(2));
        assert_eq!(action(8, 2), Some(3));
        assert_eq!(action(0, 3), None);
    }
}
