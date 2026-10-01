//! Input: what a key or a mouse report means to the frame (overview.mjs,
//! lib/mouse.mjs), and the mouse reports a read cut after their escape put
//! back together (#303).

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

/// Lines per notch of the wheel.
pub const WHEEL: i32 = 3;

/// What the frame does with an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Quit,
    Refresh,
    /// By lines.
    Scroll(i32),
    /// By pages.
    Page(i32),
    /// By half pages: Ctrl-D, Ctrl-U.
    Half(i32),
    Top,
    Bottom,
    /// The pointer moved there; None: it left the pane (the focus went).
    Hover(Option<(u16, u16)>),
    Click(u16, u16),
    /// The left button let go: a press goes before it, else the log says so (#303).
    Release(u16, u16),
    Resize,
    /// A key the frame has no use for: the app's.
    Key(KeyEvent),
}

pub fn command_of(event: &Event) -> Option<Command> {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => Some(key_command(key)),
        Event::Mouse(mouse) => mouse_command(mouse),
        Event::FocusLost => Some(Command::Hover(None)),
        Event::Resize(..) => Some(Command::Resize),
        _ => None,
    }
}

fn key_command(key: &KeyEvent) -> Command {
    let is_plain = !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Command::Quit,
        KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => Command::Half(1),
        KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => Command::Half(-1),
        _ if !is_plain => Command::Key(*key),
        KeyCode::Char('q') => Command::Quit,
        KeyCode::Char('r') => Command::Refresh,
        KeyCode::Char('j') | KeyCode::Down => Command::Scroll(1),
        KeyCode::Char('k') | KeyCode::Up => Command::Scroll(-1),
        KeyCode::Char(' ') | KeyCode::PageDown => Command::Page(1),
        KeyCode::PageUp => Command::Page(-1),
        KeyCode::Char('g') | KeyCode::Home => Command::Top,
        KeyCode::Char('G') | KeyCode::End => Command::Bottom,
        _ => Command::Key(*key),
    }
}

fn mouse_command(mouse: &MouseEvent) -> Option<Command> {
    let at = (mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::ScrollDown => Some(Command::Scroll(WHEEL)),
        MouseEventKind::ScrollUp => Some(Command::Scroll(-WHEEL)),
        MouseEventKind::Moved | MouseEventKind::Drag(_) => Some(Command::Hover(Some(at))),
        // The left button alone: Ctrl-click is herdr's, it opens links.
        MouseEventKind::Down(MouseButton::Left) if mouse.modifiers.is_empty() => Some(Command::Click(at.0, at.1)),
        MouseEventKind::Up(MouseButton::Left) if mouse.modifiers.is_empty() => Some(Command::Release(at.0, at.1)),
        _ => None,
    }
}

/// Mouse reports a read cut right after their escape, put back together.
/// crossterm reads a lone `ESC` at the end of a read as the Escape key and
/// the rest of the report as loose characters, so the click was lost (#303).
/// An Escape is held until the next input says what it was; the views have
/// no use for a lone Escape (as in the Node overview).
#[derive(Default)]
pub struct Rejoin {
    held: Vec<Event>,
    text: String,
}

impl Rejoin {
    /// The events `event` completes: none while a report may still be coming.
    pub fn feed(&mut self, event: Event) -> Vec<Event> {
        if self.held.is_empty() {
            if is_escape(&event) {
                self.held.push(event);
                return vec![];
            }
            return vec![event];
        }
        if let Event::Key(KeyEvent {
            code: KeyCode::Char(ch),
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            self.text.push(ch);
            self.held.push(event);
            match report_of(&self.text) {
                Report::Done(mouse) => {
                    self.held.clear();
                    self.text.clear();
                    return vec![Event::Mouse(mouse)];
                }
                Report::More => return vec![],
                Report::Not => {}
            }
        } else {
            self.held.push(event);
        }
        // Not a report: everything as it came.
        self.text.clear();
        std::mem::take(&mut self.held)
    }
}

fn is_escape(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(KeyEvent {
            code: KeyCode::Esc,
            kind: KeyEventKind::Press,
            ..
        })
    )
}

enum Report {
    Done(MouseEvent),
    More,
    Not,
}

/// `[<b;x;yM` after the escape: an SGR mouse report, whole or begun.
fn report_of(text: &str) -> Report {
    let Some(rest) = text.strip_prefix("[<") else {
        return if "[<".starts_with(text) {
            Report::More
        } else {
            Report::Not
        };
    };
    let (body, end) = match rest.strip_suffix('M').or_else(|| rest.strip_suffix('m')) {
        Some(body) => (body, rest.chars().last()),
        None => (rest, None),
    };
    if body.len() > 24 || !body.chars().all(|ch| ch.is_ascii_digit() || ch == ';') {
        return Report::Not;
    }
    let numbers: Vec<&str> = body.split(';').collect();
    if numbers.len() > 3 {
        return Report::Not;
    }
    let Some(end) = end else { return Report::More };
    let [b, x, y] = numbers[..] else { return Report::Not };
    match (b.parse(), x.parse::<u16>(), y.parse::<u16>()) {
        (Ok(b), Ok(x), Ok(y)) if x > 0 && y > 0 => Report::Done(mouse_of(b, x - 1, y - 1, end == 'M')),
        _ => Report::Not,
    }
}

/// An SGR report's button code as crossterm reads it: the modifier bits,
/// the wheel, motion, the button.
fn mouse_of(code: u16, column: u16, row: u16, is_press: bool) -> MouseEvent {
    let mut modifiers = KeyModifiers::NONE;
    if code & 4 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if code & 8 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if code & 16 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    let base = code & !(4 | 8 | 16);
    let button = match base & 3 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        _ => MouseButton::Right,
    };
    let kind = if base & 64 != 0 {
        match base & 3 {
            0 => MouseEventKind::ScrollUp,
            1 => MouseEventKind::ScrollDown,
            2 => MouseEventKind::ScrollLeft,
            _ => MouseEventKind::ScrollRight,
        }
    } else if base & 32 != 0 {
        if base & 3 == 3 {
            MouseEventKind::Moved
        } else {
            MouseEventKind::Drag(button)
        }
    } else if is_press && base & 3 != 3 {
        MouseEventKind::Down(button)
    } else {
        MouseEventKind::Up(button)
    };
    MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(text: &str) -> Vec<Event> {
        text.chars().map(|ch| key(KeyCode::Char(ch))).collect()
    }

    /// The release cut after its escape: `ESC` | `[<0;12;15m`, as crossterm hands it over.
    #[test]
    fn a_report_cut_after_its_escape_comes_back_whole() {
        let mut rejoin = Rejoin::default();
        let mut out = rejoin.feed(key(KeyCode::Esc));
        for event in typed("[<0;12;15M") {
            out.extend(rejoin.feed(event));
        }
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 11,
            row: 14,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(out, [Event::Mouse(click)]);
        assert_eq!(command_of(&out[0]), Some(Command::Click(11, 14)));
    }

    #[test]
    fn a_move_and_the_wheel_come_back_too() {
        for (text, kind) in [
            ("[<35;3;4M", MouseEventKind::Moved),
            ("[<65;3;4M", MouseEventKind::ScrollDown),
            ("[<0;3;4m", MouseEventKind::Up(MouseButton::Left)),
        ] {
            let mut rejoin = Rejoin::default();
            let mut out = rejoin.feed(key(KeyCode::Esc));
            for event in typed(text) {
                out.extend(rejoin.feed(event));
            }
            assert_eq!(out.len(), 1, "{text}");
            let Event::Mouse(mouse) = &out[0] else {
                panic!("{out:?}")
            };
            assert_eq!((mouse.kind, mouse.column, mouse.row), (kind, 2, 3), "{text}");
        }
    }

    #[test]
    fn escape_then_a_key_is_both_keys() {
        let mut rejoin = Rejoin::default();
        assert!(rejoin.feed(key(KeyCode::Esc)).is_empty());
        assert_eq!(
            rejoin.feed(key(KeyCode::Char('q'))),
            [key(KeyCode::Esc), key(KeyCode::Char('q'))]
        );
        // And after that, keys pass straight through.
        assert_eq!(rejoin.feed(key(KeyCode::Char('j'))), [key(KeyCode::Char('j'))]);
    }

    #[test]
    fn keys() {
        let command = |code| command_of(&key(code));
        assert_eq!(command(KeyCode::Char('q')), Some(Command::Quit));
        assert_eq!(command(KeyCode::Char('G')), Some(Command::Bottom));
        assert_eq!(command(KeyCode::Char(' ')), Some(Command::Page(1)));
        assert_eq!(command(KeyCode::PageUp), Some(Command::Page(-1)));
        let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(command_of(&ctrl_c), Some(Command::Quit));
        let ctrl_r = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(matches!(command_of(&ctrl_r), Some(Command::Key(_))));
        let ctrl = |ch| command_of(&Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)));
        assert_eq!(ctrl('d'), Some(Command::Half(1)));
        assert_eq!(ctrl('u'), Some(Command::Half(-1)));
        assert_eq!(command(KeyCode::Down), Some(Command::Scroll(1)));
        assert_eq!(command(KeyCode::Home), Some(Command::Top));
        assert!(matches!(command(KeyCode::Char('u')), Some(Command::Key(_))));
    }

    #[test]
    fn a_ctrl_click_is_not_a_click() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: KeyModifiers::CONTROL,
        };
        assert_eq!(command_of(&Event::Mouse(mouse)), None);
    }
}
