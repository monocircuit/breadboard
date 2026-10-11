//! The MERGE REQUESTS slice of the orchestration overview on ratatui (#307).

mod app;
mod gitlab;
mod scene;

use app::App;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, MouseButton, MouseEventKind};
use futures_util::StreamExt;
use gitlab::Source;
use scene::Action;
use std::time::Duration;
use tokio::sync::mpsc;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/merge-requests.json");

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let is_fixture = std::env::args().any(|arg| arg == "--fixture");
    let source = match gitlab::token().filter(|_| !is_fixture) {
        Some(token) => Source::Live { client: reqwest::Client::new(), token },
        None => Source::Fixture(FIXTURE.into()),
    };
    let mut terminal = ratatui::init();
    // After init, replacing ratatui's hook: that one restores the terminal on
    // every panic, also on a section's that is caught and drawn in its place,
    // and the terminal leaves raw mode (the mouse reports then show as text).
    // A section's panic stays silent; any other restores and says why.
    std::panic::set_hook(Box::new(|info| {
        if !app::is_in_section() {
            ratatui::restore();
            eprintln!("{info}");
        }
    }));
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, source).await;
    crossterm::execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

async fn run(terminal: &mut ratatui::DefaultTerminal, source: Source) -> std::io::Result<()> {
    let mut app = App::default();
    let (tx, mut rx) = mpsc::channel(4);
    let mut events = EventStream::new();
    let mut refresh = tokio::time::interval(Duration::from_secs(30));
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let read = |app: &mut App| {
        if app.is_reading {
            return;
        }
        app.is_reading = true;
        let (tx, source) = (tx.clone(), source.clone());
        tokio::spawn(async move {
            let _ = tx.send(source.read().await).await;
        });
    };

    loop {
        terminal.draw(|frame| app.draw(frame))?;
        tokio::select! {
            _ = refresh.tick() => read(&mut app),
            // Only the age in the status line changes; ratatui's diff writes just that.
            _ = tick.tick() => {}
            Some((result, took)) = rx.recv() => {
                app.is_reading = false;
                app.read_took = Some(took);
                app.read_at = Some(std::time::Instant::now());
                match result {
                    Ok(mrs) => { app.mrs = Some(mrs); app.error = None; }
                    Err(error) => app.error = Some(error),
                }
            }
            Some(Ok(event)) = events.next() => match event {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => read(&mut app),
                    KeyCode::Char('j') | KeyCode::Down => app.scroll_by(1),
                    KeyCode::Char('k') | KeyCode::Up => app.scroll_by(-1),
                    KeyCode::PageDown => app.scroll_by(app.body.height as i32),
                    KeyCode::PageUp => app.scroll_by(-(app.body.height as i32)),
                    _ => {}
                },
                Event::Mouse(mouse) => {
                    app.mouse = Some((mouse.column, mouse.row));
                    match mouse.kind {
                        MouseEventKind::ScrollDown => app.scroll_by(3),
                        MouseEventKind::ScrollUp => app.scroll_by(-3),
                        MouseEventKind::Down(MouseButton::Left) => {
                            if let Some(Action::Open(url) | Action::Pipeline(url)) = app.click(mouse.column, mouse.row) {
                                let _ = std::process::Command::new("xdg-open").arg(url)
                                    .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests;
