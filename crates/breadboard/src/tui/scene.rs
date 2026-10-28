//! A scene: the lines of one section and, from the same code that places
//! their text, the regions a mouse can hit (#303, #296). Text is laid out in
//! atoms that wrap whole (#304): a word, a job, a button. Only an atom wider
//! than a whole line is broken over lines; nothing is cut, nothing runs past
//! the edge.

use super::theme::{self, Tone};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::time::Duration;

/// What a mouse can hit.
#[derive(Clone, Debug, PartialEq)]
pub enum Target<A> {
    /// A block of lines: hover lights all of them, a click runs its action.
    Block(A),
    /// A button inside a block: its own stronger highlight, its own action.
    Button { label: String, tone: Tone, action: A },
}

/// A target and where it is drawn: in scene coordinates (y counts lines).
#[derive(Clone, Debug, PartialEq)]
pub struct Region<A> {
    pub area: Rect,
    pub target: Target<A>,
}

impl<A> Region<A> {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        let a = self.area;
        x >= a.x && x < a.x + a.width && y >= a.y && y < a.y + a.height
    }

    pub fn is_button(&self) -> bool {
        matches!(self.target, Target::Button { .. })
    }

    pub fn action(&self) -> &A {
        match &self.target {
            Target::Block(action) | Target::Button { action, .. } => action,
        }
    }
}

/// The lines of a section as a section builder writes them. `x` and `y` are
/// where the next atom goes; `indent` where a wrapped line starts.
pub struct Scene<A> {
    width: u16,
    pub(super) lines: Vec<Line<'static>>,
    pub(super) regions: Vec<Region<A>>,
    current: Vec<Span<'static>>,
    x: u16,
    indent: u16,
    /// The lines that go on with the line before them (a wrap, an atom
    /// moved down): a page never lands on one, its start would be above.
    pub(super) continued: Vec<u16>,
    /// The lines on the island's lighter grey (#490): every second block of WORK.
    pub(super) striped: Vec<u16>,
    pub(super) note: Option<String>,
    pub(super) redraw_in: Option<Duration>,
}

impl<A: Clone> Scene<A> {
    pub fn new(width: u16) -> Self {
        Self {
            width,
            lines: vec![],
            regions: vec![],
            current: vec![],
            x: 0,
            indent: 0,
            continued: vec![],
            striped: vec![],
            note: None,
            redraw_in: None,
        }
    }

    /// How many columns a line has.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// The line the next atom lands on.
    pub fn y(&self) -> u16 {
        self.lines.len() as u16
    }

    /// The column the next atom lands on, before its gap.
    pub fn x(&self) -> u16 {
        self.x
    }

    /// Where lines start from now on, wrapped ones too; an atom on a line
    /// that hasn't reached it yet goes there.
    pub fn indent(&mut self, indent: u16) {
        self.indent = indent;
    }

    /// What the section's title line says after its name: `3 open · 1 draft`.
    pub fn note(&mut self, note: impl Into<String>) {
        self.note = Some(note.into());
    }

    /// The section shows time (`2 min ago`): it is drawn again after `after`.
    /// Without this nothing ticks, idle stays idle.
    pub fn redraw_in(&mut self, after: Duration) {
        self.redraw_in = Some(self.redraw_in.map_or(after, |was| was.min(after)));
    }

    pub fn newline(&mut self) {
        self.lines.push(Line::from(std::mem::take(&mut self.current)));
        self.x = 0;
    }

    /// Ends the current line if it has anything on it.
    pub fn end_line(&mut self) {
        if self.x > 0 {
            self.newline();
        }
    }

    /// An empty line, after the current one.
    pub fn blank(&mut self) {
        self.end_line();
        self.newline();
    }

    /// Where a line starts: the indent, unless it leaves no room at all.
    fn start(&self) -> u16 {
        if self.indent < self.width { self.indent } else { 0 }
    }

    fn wrap(&mut self) {
        self.newline();
        self.continued.push(self.y());
        self.pad_to(self.start());
    }

    /// Spaces up to column `x` on the current line, never past the edge.
    pub fn pad_to(&mut self, x: u16) {
        let x = x.min(self.width);
        if x > self.x {
            self.current.push(Span::raw(" ".repeat((x - self.x) as usize)));
            self.x = x;
        }
    }

    /// One atom: on this line one column after the last if it fits, else on
    /// the next. Returns where it was drawn: one rect, or one per line for an
    /// atom wider than a line.
    pub fn atom(&mut self, spans: Vec<Span<'static>>) -> Vec<Rect> {
        self.atom_gap(spans, 1)
    }

    /// `atom` with `gap` columns before it instead of one.
    pub fn atom_gap(&mut self, spans: Vec<Span<'static>>, gap: u16) -> Vec<Rect> {
        if self.width == 0 {
            return vec![];
        }
        let width = width_of(&spans);
        let start = self.start();
        if self.x <= start {
            self.pad_to(start);
        } else if self.x + gap + width <= self.width {
            self.pad_to(self.x + gap);
        } else {
            self.wrap();
        }
        if self.x + width <= self.width {
            return vec![self.put(spans, width)];
        }
        // Wider than what a line has: broken over lines, never cut (#304).
        let mut rects = vec![];
        for (piece, width) in split(spans, self.width - self.x, self.width - start) {
            if self.x + width > self.width {
                self.wrap();
            }
            rects.push(self.put(piece, width));
        }
        rects
    }

    /// An atom at the right edge of this line, two columns after the last;
    /// on the next line when it doesn't fit (`waits for the pipeline`).
    pub fn right(&mut self, spans: Vec<Span<'static>>) -> Vec<Rect> {
        let width = width_of(&spans);
        if width > self.width {
            return self.atom(spans);
        }
        if self.x > 0 && self.x + 2 + width > self.width {
            self.newline();
            self.continued.push(self.y());
        }
        self.pad_to(self.width - width);
        vec![self.put(spans, width)]
    }

    fn put(&mut self, spans: Vec<Span<'static>>, width: u16) -> Rect {
        let area = Rect::new(self.x, self.y(), width, 1);
        self.current.extend(spans);
        self.x += width;
        area
    }

    /// Words that wrap as words. A no-break space (U+00A0) glues two words
    /// into one atom: `→ who`, `resolve merge conflicts`.
    pub fn text(&mut self, text: &str, style: Style) {
        for word in text
            .split(|ch: char| ch.is_whitespace() && ch != '\u{a0}')
            .filter(|word| !word.is_empty())
        {
            self.atom(vec![Span::styled(word.to_string(), style)]);
        }
    }

    /// A dim line across the whole width between two blocks.
    pub fn rule(&mut self) {
        self.end_line();
        let line = "─".repeat(self.width as usize);
        self.current.push(Span::styled(line, theme::dim()));
        self.x = self.width;
        self.newline();
    }

    /// Parts that wrap whole, `a · b · c`, the separator ending the line
    /// before a break. A part wider than a line wraps as words.
    pub fn parts(&mut self, parts: &[&str], style: Style) {
        let room = self.width.saturating_sub(self.start());
        for (i, part) in parts.iter().enumerate() {
            let text = if i + 1 < parts.len() {
                format!("{part} ·")
            } else {
                part.to_string()
            };
            if Span::raw(text.as_str()).width() > room as usize {
                self.text(&text, style);
            } else {
                self.atom(vec![Span::styled(text, style)]);
            }
        }
    }

    /// A button: ` label ` on its tone; a click on any of its cells runs `action`.
    pub fn button(&mut self, label: &str, tone: Tone, action: A) {
        for area in self.atom(vec![Span::styled(format!(" {label} "), tone.button())]) {
            let target = Target::Button {
                label: label.into(),
                tone,
                action: action.clone(),
            };
            self.regions.push(Region { area, target });
        }
    }

    /// A button that can't be clicked yet: gray, no region.
    pub fn inactive(&mut self, label: &str) {
        self.atom(vec![Span::styled(format!(" {label} "), Tone::Gray.button())]);
    }

    /// A chip: a label on a grey a step above the island, no button.
    pub fn chip(&mut self, label: &str, tone: Tone) {
        self.atom(vec![Span::styled(format!(" {label} "), tone.chip())]);
    }

    /// The lines `build` writes, as one block, edge to edge: hover lights all
    /// of them, a click anywhere off its buttons runs `action`. Blocks nest:
    /// the inner one wins.
    pub fn block(&mut self, action: A, build: impl FnOnce(&mut Self)) {
        self.end_line();
        let from = self.y();
        build(self);
        self.end_line();
        let to = self.y();
        if to > from {
            let area = Rect::new(0, from, self.width, to - from);
            self.regions.push(Region {
                area,
                target: Target::Block(action),
            });
        }
    }

    /// The lines `build` writes on the island's lighter grey (#490), so a
    /// block stands apart from the one above it.
    pub fn stripe(&mut self, build: impl FnOnce(&mut Self)) {
        self.end_line();
        let from = self.y();
        build(self);
        self.end_line();
        self.striped.extend(from..self.y());
    }

    /// The scene as it ends: the current line ended.
    pub(super) fn finish(&mut self) {
        self.end_line();
    }
}

/// A heading: the name bold, the note's parts dim after it, on one line as
/// Node's `fitParts`: the parts after the last ` · ` that fits are left out,
/// so a title never wraps or ends on a dangling `·`. Only a first part
/// wider than the line wraps, as words.
pub(super) fn heading<A: Clone>(scene: &mut Scene<A>, name: &str, note: Option<&str>) {
    scene.atom(vec![Span::styled(name.to_string(), theme::bold())]);
    let Some(note) = note else { return };
    let mut shown = String::new();
    for part in note.split(" · ") {
        let next = if shown.is_empty() {
            part.to_string()
        } else {
            format!("{shown} · {part}")
        };
        if !shown.is_empty() && scene.x() + 1 + Span::raw(next.as_str()).width() as u16 > scene.width() {
            break;
        }
        shown = next;
    }
    if scene.x() + 1 + Span::raw(shown.as_str()).width() as u16 <= scene.width() {
        scene.atom(vec![Span::styled(shown, theme::dim())]);
    } else {
        scene.text(&shown, theme::dim());
    }
}

pub(super) fn width_of(spans: &[Span]) -> u16 {
    spans.iter().map(|span| span.width() as u16).sum()
}

/// `spans` in pieces of at most `first` columns, then at most `rest`: each
/// piece ends after the last `-`, `/`, `_`, `:`, `.` or space that fits
/// (`303-overview-every-drawn-` / `button-works`), else where the room
/// ends; every piece holds at least one character, so a line too narrow
/// still advances.
fn split(spans: Vec<Span<'static>>, first: u16, rest: u16) -> Vec<(Vec<Span<'static>>, u16)> {
    let cells: Vec<(char, Style, u16)> = spans
        .iter()
        .flat_map(|span| {
            span.content.chars().map(move |ch| {
                let mut buffer = [0u8; 4];
                (ch, span.style, Span::raw(&*ch.encode_utf8(&mut buffer)).width() as u16)
            })
        })
        .collect();
    let mut pieces = vec![];
    let mut at = 0;
    let mut room = first.max(1);
    while at < cells.len() {
        // As many cells as fit, at least one.
        let mut end = at;
        let mut width = 0u16;
        while end < cells.len() && (end == at || width + cells[end].2 <= room) {
            width += cells[end].2;
            end += 1;
        }
        // Back to a break after a separator, if the piece has one past its start.
        let is_break =
            |i: usize| "-/_:. ".contains(cells[i - 1].0) && cells[at..i].iter().any(|cell| !cell.0.is_whitespace());
        if end < cells.len()
            && let Some(cut) = (at + 1..=end).rev().find(|&i| is_break(i))
        {
            end = cut;
        }
        let mut piece: Vec<Span<'static>> = vec![];
        let mut width = 0;
        for &(ch, style, w) in &cells[at..end] {
            width += w;
            match piece.last_mut() {
                Some(last) if last.style == style => last.content.to_mut().push(ch),
                _ => piece.push(Span::styled(ch.to_string(), style)),
            }
        }
        pieces.push((piece, width));
        at = end;
        room = rest.max(1);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(scene: &Scene<u8>) -> Vec<String> {
        scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn atoms_wrap_whole_and_keep_the_indent() {
        let mut scene = Scene::<u8>::new(20);
        scene.indent(2);
        scene.text("one two three four five six", Style::new());
        scene.finish();
        assert_eq!(text(&scene), ["  one two three four", "  five six"]);
    }

    #[test]
    fn an_atom_wider_than_a_line_is_broken_never_cut() {
        let mut scene = Scene::<u8>::new(6);
        scene.text("ab abcdefghijk", Style::new());
        scene.finish();
        assert_eq!(text(&scene), ["ab", "abcdef", "ghijk"]);
        for line in &scene.lines {
            assert!(line.width() <= 6);
        }
    }

    #[test]
    fn an_atom_wider_than_a_line_breaks_after_a_dash_where_it_can() {
        let mut scene = Scene::<u8>::new(38);
        scene.text("303-overview-every-drawn-button-works-on-every-cell", Style::new());
        scene.finish();
        assert_eq!(
            text(&scene),
            ["303-overview-every-drawn-button-works-", "on-every-cell"]
        );
        let mut scene = Scene::<u8>::new(20);
        scene.button("wt:remove 303-overview-every-drawn-button", Tone::Cyan, 1);
        scene.finish();
        assert_eq!(text(&scene), [" wt:remove 303-", "overview-every-", "drawn-button "]);
        assert_eq!(scene.regions.len(), 3);
    }

    #[test]
    fn a_button_has_a_region_on_every_cell_it_draws() {
        let mut scene = Scene::<u8>::new(16);
        scene.text("hello", Style::new());
        scene.button("looks good", Tone::Cyan, 1);
        scene.button("broken", Tone::Red, 2);
        scene.finish();
        assert_eq!(text(&scene), ["hello", " looks good ", " broken "]);
        let areas: Vec<_> = scene.regions.iter().map(|r| (r.area, r.action().to_owned())).collect();
        assert_eq!(areas, [(Rect::new(0, 1, 12, 1), 1), (Rect::new(0, 2, 8, 1), 2)]);
    }

    #[test]
    fn a_button_wider_than_a_line_is_clickable_on_every_piece() {
        let mut scene = Scene::<u8>::new(5);
        scene.button("abcdefgh", Tone::Cyan, 7);
        scene.finish();
        assert_eq!(text(&scene), [" abcd", "efgh "]);
        assert_eq!(scene.regions.len(), 2);
    }

    #[test]
    fn a_block_spans_its_lines_edge_to_edge() {
        let mut scene = Scene::<u8>::new(30);
        scene.text("before", Style::new());
        scene.block(9, |scene| {
            scene.text("first", Style::new());
            scene.newline();
            scene.text("second", Style::new());
        });
        scene.finish();
        assert_eq!(scene.regions[0].area, Rect::new(0, 1, 30, 2));
    }

    #[test]
    fn parts_wrap_at_their_separator() {
        let mut scene = Scene::<u8>::new(18);
        scene.parts(&["3 open", "1 draft", "2 ready"], Style::new());
        scene.finish();
        assert_eq!(text(&scene), ["3 open · 1 draft ·", "2 ready"]);
    }

    #[test]
    fn a_heading_leaves_out_the_parts_that_dont_fit() {
        let mut scene = Scene::<u8>::new(40);
        heading(
            &mut scene,
            "WORKTREES",
            Some("7 wait for GitLab · 1 will be removed · 41 removed"),
        );
        scene.finish();
        assert_eq!(text(&scene), ["WORKTREES 7 wait for GitLab"]);
        let mut scene = Scene::<u8>::new(80);
        heading(
            &mut scene,
            "WORKTREES",
            Some("7 wait for GitLab · 1 will be removed · 41 removed"),
        );
        scene.finish();
        assert_eq!(
            text(&scene),
            ["WORKTREES 7 wait for GitLab · 1 will be removed · 41 removed"]
        );
        let mut scene = Scene::<u8>::new(16);
        heading(&mut scene, "NEXT", Some("a first part too wide · b"));
        scene.finish();
        assert_eq!(text(&scene), ["NEXT a first", "part too wide"]);
    }

    #[test]
    fn a_no_break_space_glues_words() {
        let mut scene = Scene::<u8>::new(14);
        scene.text("go resolve\u{a0}merge", Style::new());
        scene.finish();
        assert_eq!(text(&scene), ["go", "resolve\u{a0}merge"]);
    }

    #[test]
    fn redraw_in_keeps_the_soonest() {
        let mut scene = Scene::<u8>::new(10);
        scene.redraw_in(Duration::from_secs(60));
        scene.redraw_in(Duration::from_secs(5));
        scene.redraw_in(Duration::from_secs(30));
        assert_eq!(scene.redraw_in, Some(Duration::from_secs(5)));
    }
}
