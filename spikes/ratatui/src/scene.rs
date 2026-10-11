//! A scene: the lines of the scrollable view and, from the same code that
//! places their text, the regions a mouse can hit (#303, #296). Text is laid
//! out in atoms that wrap as a whole (#304): a word, a job, a button.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// What a click runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Open(String),
    Pipeline(String),
    ToggleJobs(String),
    Select(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A whole block: hover lights all of its lines; a click selects it.
    Block(Action),
    /// A button inside a block: its own stronger highlight, its own action.
    Button { label: String, action: Action },
}

/// A target and where it is drawn, in scene coordinates (y counts lines).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub area: Rect,
    pub target: Target,
}

pub struct Scene {
    pub width: u16,
    pub lines: Vec<Line<'static>>,
    pub regions: Vec<Region>,
    current: Vec<Span<'static>>,
    x: u16,
    indent: u16,
}

impl Scene {
    pub fn new(width: u16) -> Self {
        Self { width, lines: vec![], regions: vec![], current: vec![], x: 0, indent: 0 }
    }

    /// The line the next atom lands on.
    pub fn y(&self) -> u16 {
        self.lines.len() as u16
    }

    /// Where wrapped lines start from now on.
    pub fn indent(&mut self, indent: u16) {
        self.indent = indent;
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

    fn wrap(&mut self) {
        self.newline();
        self.pad_to(self.indent);
    }

    /// Spaces up to column `x` on the current line.
    pub fn pad_to(&mut self, x: u16) {
        if x > self.x {
            self.current.push(Span::raw(" ".repeat((x - self.x) as usize)));
            self.x = x;
        }
    }

    /// One atom: on this line after a gap if it fits, else on the next.
    /// Returns where it was drawn.
    pub fn atom(&mut self, spans: Vec<Span<'static>>, gap: u16) -> Rect {
        let width: u16 = spans.iter().map(|span| span.width() as u16).sum();
        let at_start = self.x <= self.indent;
        if !at_start && self.x + gap + width > self.width {
            self.wrap();
        } else if !at_start {
            self.pad_to(self.x + gap);
        } else {
            self.pad_to(self.indent);
        }
        let area = Rect::new(self.x, self.y(), width, 1);
        self.current.extend(spans);
        self.x += width;
        area
    }

    /// Words that wrap as words; a word wider than a line is split.
    pub fn words(&mut self, text: &str, style: Style) {
        let room = self.width.saturating_sub(self.indent).max(1) as usize;
        for word in text.split_whitespace() {
            let chars: Vec<char> = word.chars().collect();
            for piece in chars.chunks(room) {
                let piece: String = piece.iter().collect();
                self.atom(vec![Span::styled(piece, style)], 1);
            }
        }
    }

    pub fn button(&mut self, label: &str, action: Action, style: Style) -> Rect {
        let text = format!("[{label}]");
        let area = self.atom(vec![Span::styled(text, style)], 1);
        self.regions.push(Region {
            area,
            target: Target::Button { label: label.into(), action },
        });
        area
    }

    /// Marks the lines from `from` to the current one as a block, edge to edge.
    pub fn block(&mut self, from: u16, action: Action) {
        let to = self.y() + u16::from(self.x > 0);
        self.regions.push(Region {
            area: Rect::new(0, from, self.width, to.saturating_sub(from)),
            target: Target::Block(action),
        });
    }

    /// Appends another scene below this one.
    pub fn append(&mut self, other: Scene) {
        self.end_line();
        let offset = self.y();
        self.lines.extend(other.lines);
        self.regions.extend(other.regions.into_iter().map(|mut region| {
            region.area.y += offset;
            region
        }));
    }

    pub fn finish(mut self) -> Self {
        self.end_line();
        self
    }

    /// What a mouse at scene position (x, y) hits: a button before its block.
    pub fn hit(&self, x: u16, y: u16) -> Option<&Region> {
        let inside = |region: &&Region| {
            let a = region.area;
            x >= a.x && x < a.x + a.width && y >= a.y && y < a.y + a.height
        };
        let buttons = self.regions.iter().filter(|r| matches!(r.target, Target::Button { .. }));
        let blocks = self.regions.iter().filter(|r| matches!(r.target, Target::Block(_)));
        buttons.clone().find(inside).or_else(|| blocks.clone().find(inside))
    }
}
