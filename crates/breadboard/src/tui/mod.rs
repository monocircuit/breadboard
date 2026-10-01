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
    /// What a click on a button or a block runs: the app's own.
    type Action: Clone;
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
}

/// What the app is asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Input<A> {
    /// A click on a button or a block, on the frame the user saw.
    Click(A),
    /// `r`.
    Refresh,
    Key(KeyEvent),
}

/// What comes of an `act`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// The app's state changed: build the page again.
    Redraw,
    /// Nothing to show.
    Same,
    Quit,
}

pub fn main(args: &[String]) -> ExitCode {
    if !args.iter().any(|arg| arg == "--demo") {
        eprintln!("overview: not built yet (`breadboard overview --demo` shows the frame)");
        return ExitCode::FAILURE;
    }
    if args.iter().any(|arg| arg == "--once") {
        print!("{}", view::once(&demo::Demo::new(None), columns()));
        return ExitCode::SUCCESS;
    }
    let (sender, models) = mpsc::unbounded_channel();
    let mut app = demo::Demo::new(Some(sender));
    // The view draws on this thread; the readers run on the workers.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("overview: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run::run(&mut app, models));
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
