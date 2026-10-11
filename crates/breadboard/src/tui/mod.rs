//! `breadboard overview`: the live view, a frame its sections plug into.
//!
//! An app says what the band shows (`header`) and adds its sections to the
//! page (`build`), each a closure that writes atoms into a `Scene`: words,
//! buttons, blocks. The frame draws them, scrolls, lights what the pointer
//! is on and hands the app what a click hit (`act`). Data comes in as models
//! on a channel (`update`); after an update or an act the page is built
//! again, nothing else builds it.

mod band;
mod clicks;
mod demo;
mod input;
pub(crate) mod links;
mod overview;
mod page;
mod run;
mod scene;
mod term;
mod theme;
mod view;

pub use band::{Header, clock};
pub use crossterm::event::KeyEvent;
pub use page::{Page, Section};
pub use scene::Scene;
pub use theme::Tone;

use std::process::ExitCode;
use std::time::Duration;
use tokio::sync::mpsc;

/// A view the frame runs.
pub trait App {
    /// What a click on a button or a block runs: the app's own. Equal
    /// actions are the same block, so the scroll stays on it over a rebuild.
    type Action: Clone + PartialEq;
    /// What the app's readers send while it runs.
    type Model;

    /// The title band: the view's name, the state of its data.
    fn header(&self) -> Header;
    /// The sections, in order: `page.section("NAME", |scene| …)`.
    fn build(&self, page: &mut Page<Self::Action>);
    /// A model came in; the page is built again after it.
    fn update(&mut self, model: Self::Model);
    /// A click, `r`, or a key the frame has no use for.
    fn act(&mut self, input: Input<Self::Action>) -> Flow;
    /// Tab moved the pick to `picked` (lazygit's j/k): an app whose panel
    /// follows the selection (the worktree view's hunks) changes with it and
    /// says `Show`; the page is built again with the pick kept on screen.
    fn picked(&mut self, _picked: Option<&Self::Action>) -> Flow {
        Flow::Same
    }
    /// A one-line prompt is open: every key is the app's, `q` and `j` too,
    /// as typed text (Ctrl-C still quits).
    fn is_typing(&self) -> bool {
        false
    }
}

/// What the app is asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Input<A> {
    /// A click on a button or a block, on the frame the user saw.
    Click(A),
    /// `r`.
    Refresh,
    /// A key, and the block picked with Tab if one is: keys act on the pick, as lazygit's on its selected row.
    Key(KeyEvent, Option<A>),
}

/// What comes of an `act`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// The app's state changed: build the page again.
    Redraw,
    /// As `Redraw`, and the act opened something under the block it ran on
    /// (a file's hunks): scrolled into view, a click's as Enter's, its block
    /// no further than the top. A `Redraw` click keeps its row under the pointer.
    Show,
    /// Nothing to show.
    Same,
    Quit,
}

pub fn main(args: &[String]) -> ExitCode {
    if !args.iter().any(|arg| arg == "--demo") {
        return overview::main(args);
    }
    if args.iter().any(|arg| arg == "--once") {
        print!("{}", view::once(&demo::Demo::new(None), columns()));
        return ExitCode::SUCCESS;
    }
    let (sender, models) = mpsc::unbounded_channel();
    let mut app = demo::Demo::new(Some(sender));
    match runtime() {
        Ok(runtime) => serve(runtime, &mut app, models),
        Err(error) => {
            eprintln!("overview: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The runtime of a live view: it draws on this thread, the readers run on the workers.
fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
}

/// Runs `app` live on `runtime` until it ends.
fn serve<P: App>(runtime: tokio::runtime::Runtime, app: &mut P, models: mpsc::UnboundedReceiver<P::Model>) -> ExitCode {
    let result = runtime.block_on(run::run(app, models));
    // The readers may sit in a call: the view doesn't wait for them.
    runtime.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("overview: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The width of a printed view: the terminal's, `$COLUMNS`, else 120.
fn columns() -> u16 {
    crossterm::terminal::size()
        .map(|(columns, _)| columns)
        .ok()
        .filter(|columns| *columns > 0)
        .or_else(|| std::env::var("COLUMNS").ok()?.parse().ok())
        .unwrap_or(120)
}
