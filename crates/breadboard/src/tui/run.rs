//! The loop of a live view: input, the app's models from a channel, a
//! clock only when the page shows time. Nothing is drawn without a change;
//! a burst of input (the wheel at 40 reports a second) draws at most once a
//! frame, and the frame after the last report shows where it ended.

use super::clicks;
use super::input::{Command, Rejoin, command_of};
use super::links::Links;
use super::term::{self, Tty};
use super::view::{Held, View};
use super::{App, Flow, Input};
use crossterm::event::{Event, EventStream, KeyCode};
use futures_util::StreamExt;
use std::io;
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};

/// At most one draw per this, about a frame of the screen.
pub const FRAME: Duration = Duration::from_millis(16);
/// How often the pane's size is asked for: a pane resized while its tab
/// was hidden may never get SIGWINCH (#253).
const SIZE_EVERY: Duration = Duration::from_millis(250);

/// Runs `app` until it or the user quits, or a signal ends it; the terminal
/// is restored on every way out.
pub async fn run<P: App>(app: &mut P, models: mpsc::UnboundedReceiver<P::Model>) -> io::Result<()> {
    let (mut tty, _guard) = term::enter()?;
    let result = drive(&mut tty, app, models).await;
    drop(tty);
    result
}

async fn drive<P: App>(tty: &mut Tty, app: &mut P, mut models: mpsc::UnboundedReceiver<P::Model>) -> io::Result<()> {
    let mut view = View::<P::Action>::default();
    let mut links = Links::default();
    let mut events = EventStream::new();
    let mut rejoin = Rejoin::default();
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut size_check = tokio::time::interval(SIZE_EVERY);
    size_check.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut size = crossterm::terminal::size()?;
    let mut is_models_open = true;
    let mut is_dirty = true;
    let mut is_pressed = false;
    let mut last_draw = Instant::now() - FRAME;

    loop {
        if is_dirty && last_draw.elapsed() >= FRAME {
            tty.draw(|frame| {
                view.draw(frame, app);
                links.take(frame.buffer_mut());
            })?;
            links.write(tty.backend_mut())?;
            last_draw = Instant::now();
            is_dirty = false;
        }
        let draw_at = is_dirty.then(|| last_draw + FRAME);
        let tick_at = view.redraw_at().map(Instant::from_std);
        let escape_at = rejoin.escape_at().map(Instant::from_std);
        let wake = [draw_at, tick_at, escape_at].into_iter().flatten().min();

        tokio::select! {
            biased;
            _ = terminate.recv() => return Ok(()),
            _ = hangup.recv() => return Ok(()),
            _ = interrupt.recv() => return Ok(()),
            event = events.next() => {
                // The input is gone (the pane closed): so is the view.
                let Some(event) = event else { return Ok(()) };
                for event in rejoin.feed(event?) {
                    let Some(command) = command_of(&event) else { continue };
                    // The block a click or Enter runs: it keeps its place over the rebuild.
                    let mut held = None;
                    let input = match command {
                        Command::Quit => return Ok(()),
                        Command::Scroll(by) => {
                            is_dirty |= view.scroll_by(by);
                            continue;
                        }
                        Command::Page(by) => {
                            is_dirty |= view.page_by(by);
                            continue;
                        }
                        Command::Half(by) => {
                            is_dirty |= view.half_by(by);
                            continue;
                        }
                        Command::Top => {
                            is_dirty |= view.scroll_top();
                            continue;
                        }
                        Command::Bottom => {
                            is_dirty |= view.scroll_bottom();
                            continue;
                        }
                        Command::Select(by) => {
                            let was = view.selected();
                            is_dirty |= view.select_by(by);
                            let picked = view.selected();
                            // An app that follows the pick changes with it: the pick stays on screen.
                            if picked != was && app.picked(picked.as_ref()) != Flow::Same {
                                view.acted(view.hold_pick());
                                is_dirty = true;
                            }
                            continue;
                        }
                        Command::Hover(at) => {
                            is_dirty |= view.hover(at);
                            continue;
                        }
                        Command::Resize => {
                            size = crossterm::terminal::size()?;
                            is_dirty = true;
                            continue;
                        }
                        Command::Refresh => Input::Refresh,
                        // Esc drops the pick, as lazygit's Esc steps back, and is the app's too.
                        Command::Key(key) if key.code == KeyCode::Esc => {
                            is_dirty |= view.unpick();
                            Input::Key(key, None)
                        }
                        // Enter runs the block picked with Tab, as its click; else it is the app's.
                        Command::Key(key) => match view.selected() {
                            Some(action) if key.code == KeyCode::Enter && key.modifiers.is_empty() => {
                                if view.show_selected() {
                                    is_dirty = true;
                                    continue;
                                }
                                held = view.hold_selected();
                                Input::Click(action)
                            }
                            picked => Input::Key(key, picked),
                        },
                        Command::Click(x, y) => {
                            is_pressed = true;
                            let mut entry = view.entry(x, y);
                            // A button whose block's head is off screen: the block is shown, nothing runs.
                            let is_shown = view.show_clicked(x, y);
                            if is_shown {
                                entry.ran = format!("{}, not run: its block shown first", entry.ran);
                            }
                            clicks::log(&entry);
                            // The pointer is where it clicked: a terminal may report no move before it.
                            is_dirty |= view.hover(Some((x, y)));
                            if is_shown {
                                is_dirty = true;
                                continue;
                            }
                            match view.click(x, y) {
                                Some(action) => {
                                    held = view.hold(x, y);
                                    Input::Click(action)
                                }
                                None => continue,
                            }
                        }
                        Command::Release(x, y) => {
                            // A release whose press was lost: the press was read as text (#303).
                            if !std::mem::take(&mut is_pressed) {
                                let ran = "release without its press".into();
                                clicks::log(&clicks::Entry { ran, ..view.entry(x, y) });
                            }
                            continue;
                        }
                    };
                    match app.act(input) {
                        Flow::Quit => return Ok(()),
                        Flow::Redraw => {
                            view.acted(held);
                            is_dirty = true;
                        }
                        Flow::Show => {
                            view.acted(held.map(Held::showing));
                            is_dirty = true;
                        }
                        Flow::Same => {}
                    }
                }
            }
            model = models.recv(), if is_models_open => match model {
                Some(model) => {
                    app.update(model);
                    view.stale();
                    is_dirty = true;
                }
                None => is_models_open = false,
            },
            _ = size_check.tick() => {
                let now = crossterm::terminal::size()?;
                if now != size {
                    size = now;
                    is_dirty = true;
                }
            }
            _ = sleep_until(wake), if wake.is_some() => {
                if tick_at.is_some_and(|at| at <= Instant::now()) {
                    view.stale();
                    is_dirty = true;
                }
                // A lone Escape nothing followed: the Escape key. It drops
                // the pick, and is the app's too.
                if let Some(Event::Key(key)) = rejoin.take_escape() {
                    is_dirty |= view.unpick();
                    match app.act(Input::Key(key, None)) {
                        Flow::Quit => return Ok(()),
                        Flow::Redraw | Flow::Show => {
                            view.acted(None);
                            is_dirty = true;
                        }
                        Flow::Same => {}
                    }
                }
            }
        }
    }
}

async fn sleep_until(at: Option<Instant>) {
    if let Some(at) = at {
        tokio::time::sleep_until(at).await;
    }
}
