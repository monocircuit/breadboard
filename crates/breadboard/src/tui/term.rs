//! The terminal of a live view: the alternate screen, raw mode, the mouse
//! (SGR reports, every move for hover, the focus for the pointer leaving),
//! and the way back however the view ends: quit, a signal, a panic.

use super::page;
use crossterm::event::{DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture};
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::cell::Cell;
use std::io::{self, BufWriter, Stdout};

/// One write per frame: ratatui flushes after each draw.
pub type Tty = Terminal<CrosstermBackend<BufWriter<Stdout>>>;

thread_local! {
    /// The thread that draws: only its panic is the view's end.
    static IS_UI: Cell<bool> = const { Cell::new(false) };
}

/// Restores the terminal when dropped: every way out of the loop passes it.
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

/// Takes the terminal over. crossterm's mouse capture asks for SGR reports
/// (?1006) and any-motion tracking (?1003). Autowrap off (?7l), as Node: a
/// glyph the terminal draws wider than we measure never wraps or scrolls.
pub fn enter() -> io::Result<(Tty, Guard)> {
    IS_UI.with(|flag| flag.set(true));
    install_hook();
    enable_raw_mode()?;
    let guard = Guard;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        DisableLineWrap,
        EnableMouseCapture,
        EnableFocusChange,
        cursor::Hide
    )?;
    let terminal = Terminal::new(CrosstermBackend::new(BufWriter::with_capacity(1 << 16, io::stdout())))?;
    Ok((terminal, guard))
}

/// The terminal as it was. Twice does no harm.
pub fn restore() {
    let _ = execute!(
        io::stdout(),
        DisableFocusChange,
        DisableMouseCapture,
        EnableLineWrap,
        LeaveAlternateScreen,
        cursor::Show
    );
    let _ = disable_raw_mode();
}

/// A section's panic is caught and drawn in its place (#306): the terminal
/// stays as it is, raw, or the mouse reports would show as text. Any other
/// panic of the drawing thread ends the view: the terminal back first, then
/// why. A panic of a worker (a read) is caught by tokio; written to the
/// screen it would land on the view, so it stays silent.
fn install_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if page::is_in_section() {
            if let Some(at) = info.location() {
                page::note_panic_at(at.to_string());
            }
            return;
        }
        if IS_UI.with(Cell::get) {
            restore();
            default(info);
        }
    }));
}
