//! `breadboard overview [--orchestration <name> | --worktree <dir>] [--once]
//! [--herdr]`: the three views of the overview on the frame of `tui`.
//!
//! The readers run beside the view: the local one on a thread of its own
//! (git, herdr and the disk block), every 2 s but only when a file it reads
//! or herdr's state of an agent changed, or 15 s went by; GitLab every 30 s
//! and on `r`, read by the daemon (`breadboard serve`) for every overview of
//! the same view, or by the overview itself where it can't be reached. herdr's
//! reads go through the daemon too; the looks of all overviews fall on the
//! same even seconds, so one herdr call answers them all.
//! Each sends what it read; the app builds the view's model from both
//! (core's `model_of`) and the page from the model, one file per section:
//!
//! - orchestration: `header` (the chat row, GitLab's error), `next`,
//!   `actions` (ACTIONS, UP NEXT), `worktrees`, `agents`, `merge_requests`,
//!   `footer` (end orchestration, the hints);
//! - one worktree: `worktree_view`; everything: `everything`.
//!
//! A click's `Act` goes to `act`, the one place the overview does anything;
//! what it does by itself after a read too (`actor/`; the repo's changelog:
//! `overall`).

mod act;
mod actions;
mod actor;
mod agents;
mod everything;
mod footer;
mod header;
mod look;
mod merge_requests;
mod next;
mod overall;
mod worktree_view;
mod worktrees;

pub use act::{Act, Fold};

use super::page::guarded;
use super::{App, Flow, Header, Input, Page, view};
use crate::daemon::{self, Feed};
use breadboard_core::gitlab;
use breadboard_core::gitlab::client::Rate;
use breadboard_core::model::orchestration::ViewState;
use breadboard_core::reader::{Local, LocalReader, Model, RemoteAsk, RemoteOf, RemoteReader, View, model_of};
use breadboard_core::time::now_ms;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// How often the local read looks whether a file it reads changed (args.mjs `LOCAL_MS`), on the clock's even seconds.
const LOCAL_EVERY_MS: i64 = 2000;
/// A local read at the latest after this: herdr's states and edits in a working tree touch no file.
const LOCAL_AT_LEAST: Duration = Duration::from_millis(breadboard_core::reader::STALE_MS as u64);

/// What the command line asks for (overview/args.mjs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub view: View,
    /// One frame printed, then exit: `--once`, or stdout isn't a terminal.
    pub is_once: bool,
    /// The main space's overview keeps herdr in step (`--herdr`): the actor's piece.
    pub is_herdr: bool,
}

/// `--orchestration <name>` wins over `--worktree <dir>`; a flag without its
/// value means the view of everything; the dir is taken from `cwd`.
pub fn args_of(args: &[String], cwd: &Path) -> Args {
    let value = |flag: &str| {
        let at = args.iter().position(|arg| arg == flag)?;
        Some(args.get(at + 1).filter(|value| !value.starts_with("--")).cloned())
    };
    let view = match (value("--orchestration"), value("--worktree")) {
        (Some(Some(name)), _) => View::Orchestration(name),
        (Some(None), _) => View::Everything,
        (None, Some(dir)) => {
            let dir = dir.map_or_else(|| cwd.to_path_buf(), |dir| cwd.join(dir));
            View::Worktree(dir.canonicalize().unwrap_or(dir))
        }
        (None, None) => View::Everything,
    };
    Args {
        view,
        is_once: args.iter().any(|arg| arg == "--once"),
        is_herdr: args.iter().any(|arg| arg == "--herdr"),
    }
}

pub fn main(args: &[String]) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut args = args_of(args, &cwd);
    args.is_once |= !std::io::stdout().is_terminal();
    // `here` decides the → of the view of everything: where pnpm was started.
    let here = std::env::var_os("INIT_CWD").map(PathBuf::from).unwrap_or(cwd);
    let Some(reader) = LocalReader::new(&here, args.view.clone()) else {
        eprintln!("overview: {} is in no git repo", here.display());
        return ExitCode::FAILURE;
    };
    let main = reader.main.clone();
    let _ = std::env::set_current_dir(&main);
    look::init(&main);
    let mut app = Overview::new(&args, &main);
    if args.is_once {
        return once(app, reader);
    }
    let runtime = match super::runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("overview: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (sender, models) = mpsc::unbounded_channel();
    let (asks, asked) = watch::channel(None);
    let (refresh, refreshes) = mpsc::unbounded_channel();
    app.refresh = Some(refresh);
    let (wake, woken) = std::sync::mpsc::sync_channel(1);
    app.wake_local = Some(wake);
    act::start(&mut app, sender.clone());
    daemon::client::share_herdr();
    read_locally(reader, sender.clone(), asks, woken, app.actor.is_sync);
    runtime.spawn(read_gitlab(main, view_key(&args.view), asked, refreshes, sender));
    super::serve(runtime, &mut app, models)
}

/// `--once`: the local read, then GitLab's, the frame printed (a print never acts).
fn once(mut app: Overview, mut reader: LocalReader) -> ExitCode {
    // One frame: with docker's answer in it.
    reader.should_wait_for_docker = true;
    let local = reader.read();
    let ask = reader.ask(&local);
    app.update(Msg::Local(Box::new(local)));
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("overview: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut remote = RemoteReader::new(&reader.main);
    let result = runtime.block_on(remote.read(&ask, true));
    app.update(Msg::Gitlab(Feed::Remote {
        result: result.map(Box::new).map_err(|error| error.to_string()),
        at: now_ms(),
        rate: remote.gitlab.rate(),
    }));
    print!("{}", view::once(&app, super::columns()));
    ExitCode::SUCCESS
}

/// The local read on a thread of its own: again when a file it reads or
/// herdr's state of a pane changed, at the latest after LOCAL_AT_LEAST; what GitLab is asked for goes to `asks`.
/// `woken` looks at once: what the overview did by itself (a `wt:remove` ended) shows without waiting for the poll.
fn read_locally(
    mut reader: LocalReader,
    sender: mpsc::UnboundedSender<Msg>,
    asks: watch::Sender<Option<RemoteAsk>>,
    woken: std::sync::mpsc::Receiver<()>,
    is_sync: bool,
) {
    std::thread::spawn(move || {
        let mut stamp = None;
        let mut states = None;
        let mut read_at = Instant::now();
        loop {
            // herdr's states hold no file: asked on every look (one `pane list`), as Node's every local read.
            let now_states = actor::herdr::states_stamp(&reader, is_sync);
            if stamp != Some(reader.stamp()) || now_states != states || read_at.elapsed() >= LOCAL_AT_LEAST {
                states = now_states;
                let local = reader.read();
                let ask = reader.ask(&local);
                asks.send_if_modified(|was| {
                    let is_new = was.as_ref() != Some(&ask);
                    if is_new {
                        *was = Some(ask);
                    }
                    is_new
                });
                stamp = Some(reader.stamp());
                read_at = Instant::now();
                if sender.send(Msg::Local(Box::new(local))).is_err() {
                    return;
                }
            }
            // The next even second: every overview looks then, the daemon asks herdr once for all.
            let wait = LOCAL_EVERY_MS - now_ms().rem_euclid(LOCAL_EVERY_MS);
            let wait = Duration::from_millis(u64::try_from(wait).unwrap_or(0));
            if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) = woken.recv_timeout(wait) {
                return;
            }
        }
    });
}

/// GitLab's reads of the view: the daemon's feed of it, else (no daemon to
/// be had, or it got lost) read here on the same cadence (`daemon::read_gitlab`).
async fn read_gitlab(
    main: PathBuf,
    view: String,
    mut asks: watch::Receiver<Option<RemoteAsk>>,
    mut refreshes: mpsc::UnboundedReceiver<()>,
    sender: mpsc::UnboundedSender<Msg>,
) {
    let (out, mut said) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(feed) = said.recv().await {
            if sender.send(Msg::Gitlab(feed)).is_err() {
                return;
            }
        }
    });
    match daemon::client::follow(&main, &view, &mut asks, &mut refreshes, &out).await {
        daemon::client::Followed::Done => {}
        daemon::client::Followed::Lost => daemon::read_gitlab(main, asks, refreshes, out).await,
        daemon::client::Followed::Silent => {
            let _ = out.send(Feed::Remote {
                result: Err(daemon::SILENT_TEXT.into()),
                at: now_ms(),
                rate: None,
            });
            daemon::read_gitlab(main, asks, refreshes, out).await;
        }
    }
}

/// Which view the daemon's GitLab feed is of: every overview of it shares one.
fn view_key(view: &View) -> String {
    match view {
        View::Orchestration(name) => format!("orchestration {name}"),
        View::Worktree(dir) => format!("worktree {}", dir.display()),
        View::Everything => "everything".into(),
    }
}

/// What the readers send.
pub enum Msg {
    Local(Box<Local>),
    /// A GitLab read ended, or one under way before the first answer (`GitLab: 4/7 · pipelines`).
    Gitlab(Feed),
    /// What the actor did, back on the view (act.rs).
    Acted(actor::Effect),
}

/// Where GitLab stands for the view.
#[derive(Debug, Clone, Default)]
pub struct GitlabState {
    /// The red line of the last read that failed: `GitLab didn't answer 14:57 (…), data from 14:50, tried again`.
    pub error: Option<String>,
    /// The progress of the first read.
    pub progress: String,
    /// When the data shown was read.
    pub at: Option<i64>,
    pub rate: Option<Rate>,
}

impl GitlabState {
    /// `GitLab 14:57 · 122/2000 a minute`, or the progress while nothing came yet.
    pub fn fresh(&self) -> String {
        match self.at {
            Some(at) => gitlab::fresh_text(at, self.rate),
            None => self.progress.clone(),
        }
    }
}

/// The user's folds, kept over redraws; None: the model's default.
#[derive(Debug, Clone, Copy, Default)]
pub struct Folds {
    pub removed: Option<bool>,
    pub is_queued_folded: bool,
    pub is_merged_folded: bool,
    /// The view of everything: every open MR, every issue, past what fits the pane.
    pub is_every_mr_shown: bool,
    pub is_every_issue_shown: bool,
}

/// What every section knows besides its model.
pub struct Ctx<'a> {
    pub is_once: bool,
    pub folds: Folds,
    pub gitlab: &'a GitlabState,
    pub now: i64,
    /// The main checkout's folder name: `platform`.
    pub main_name: &'a str,
    /// What the overview does, and its words for the rows while it does it.
    pub actor: &'a actor::Actor,
    pub words: &'a actor::view::Words,
}

/// The overview: what was read, the model of its view, the user's folds.
pub struct Overview {
    view: View,
    is_once: bool,
    main_name: String,
    local: Option<Local>,
    remote: Option<RemoteOf>,
    gitlab: GitlabState,
    state: ViewState,
    model: Option<Model>,
    /// Why the model couldn't be built: the last good one stays.
    broken: Option<String>,
    folds: Folds,
    refresh: Option<mpsc::UnboundedSender<()>>,
    /// Wakes the local reader: after something the actor did, its result shows at once.
    wake_local: Option<std::sync::mpsc::SyncSender<()>>,
    /// What the view of a worktree started: delete armed, stack, migrations, nvim.
    worktree: worktree_view::State,
    actor: actor::Actor,
    words: actor::view::Words,
}

impl Overview {
    pub fn new(args: &Args, main: &Path) -> Self {
        Self {
            view: args.view.clone(),
            is_once: args.is_once,
            main_name: main
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            local: None,
            remote: None,
            gitlab: GitlabState {
                progress: "GitLab …".into(),
                ..GitlabState::default()
            },
            state: ViewState::default(),
            model: None,
            broken: None,
            folds: Folds::default(),
            refresh: None,
            wake_local: None,
            worktree: worktree_view::State::default(),
            actor: actor::Actor::new(&args.view, main, args.is_once, args.is_herdr),
            words: actor::view::Words::new(),
        }
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            is_once: self.is_once,
            folds: self.folds,
            gitlab: &self.gitlab,
            now: now_ms(),
            main_name: &self.main_name,
            actor: &self.actor,
            words: &self.words,
        }
    }

    /// Whether "done and removed" is folded by the model's default (past FOLD_OVER).
    fn is_removed_folded(&self) -> bool {
        match &self.model {
            Some(Model::Orchestration(model)) => model
                .worktrees
                .as_ref()
                .is_ok_and(|worktrees| worktrees.is_removed_folded),
            _ => false,
        }
    }

    /// The model again from what was read; a panic keeps the last one and says why.
    fn rebuild(&mut self) {
        let Some(local) = &self.local else { return };
        let (remote, state) = (self.remote.as_ref(), &mut self.state);
        match guarded(|| model_of(local, remote, state, now_ms())) {
            Ok(model) => {
                self.words = match (&model, &self.local) {
                    (Model::Orchestration(model), Some(Local::Orchestration(local))) => {
                        actor::view::overlay(&self.actor, model, &local.trees, &self.state.removals)
                    }
                    _ => actor::view::Words::new(),
                };
                self.model = Some(model);
                self.broken = None;
            }
            Err(why) => self.broken = Some(why),
        }
    }
}

impl App for Overview {
    type Action = Act;
    type Model = Msg;

    fn header(&self) -> Header {
        header::header(&self.view, self.model.as_ref(), self.remote.as_ref(), &self.ctx())
    }

    fn build(&self, page: &mut Page<Act>) {
        let ctx = self.ctx();
        if let Some(why) = &self.broken {
            page.failed("VIEW", why);
        }
        match &self.model {
            None => page.section(super::Section::new("READING").plain(), |scene| {
                scene.text("reading the worktrees …", super::theme::dim());
            }),
            Some(Model::Orchestration(model)) => {
                header::build(page, model, &ctx);
                if model.is_empty {
                    next::empty(page, model);
                    return;
                }
                next::build(page, model, &ctx);
                actions::build(page, model, &ctx);
                worktrees::build(page, model, &ctx);
                agents::build(page, model, &ctx);
                merge_requests::build(page, model, &ctx);
                footer::build(page, model, &ctx);
                actor::view::done(page, &ctx);
            }
            Some(Model::Worktree(model)) => {
                worktree_view::build(page, model, &ctx, &self.worktree);
                actor::view::done(page, &ctx);
            }
            Some(Model::Everything(model)) => {
                everything::build(page, model, &ctx);
                actor::view::done(page, &ctx);
            }
        }
    }

    fn update(&mut self, message: Msg) {
        let (is_local, is_remote) = (
            matches!(message, Msg::Local(_)),
            matches!(message, Msg::Gitlab(Feed::Remote { .. })),
        );
        match message {
            Msg::Acted(effect) => {
                effect(self);
                // A full channel: a look is already due.
                if let Some(wake) = &self.wake_local {
                    let _ = wake.try_send(());
                }
                return;
            }
            Msg::Local(local) => self.local = Some(*local),
            Msg::Gitlab(Feed::Remote { result, at, rate }) => {
                self.gitlab.rate = rate;
                match result {
                    Ok(remote) => {
                        self.gitlab.at = Some(remote.at());
                        self.gitlab.error = None;
                        self.remote = Some(*remote);
                    }
                    Err(reason) => {
                        self.gitlab.error = Some(gitlab::error_text(&reason, Some(at), self.gitlab.at, !self.is_once));
                    }
                }
            }
            Msg::Gitlab(Feed::Progress(progress)) => {
                self.gitlab.progress = progress;
                return;
            }
        }
        act::gate(self);
        self.rebuild();
        act::after_read(self, is_local, is_remote);
        // Answers that didn't get in, again once their agent's pane changed: only the deliverer.
        if let Some(Model::Orchestration(model)) = &self.model
            && self.state.is_deliverer
            && !self.is_once
        {
            actions::retry_answers(&self.actor, model);
        }
    }

    fn act(&mut self, input: Input<Act>) -> Flow {
        match input {
            Input::Refresh => {
                if let Some(refresh) = &self.refresh {
                    let _ = refresh.send(());
                }
                Flow::Same
            }
            Input::Click(action) => act::act(self, action),
            Input::Key(key, picked) => act::key(self, key, picked.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        let list: Vec<String> = list.iter().map(|arg| arg.to_string()).collect();
        args_of(&list, Path::new("/nowhere"))
    }

    #[test]
    fn reads_the_view_from_the_command_line() {
        assert_eq!(args(&["--orchestration", "b"]).view, View::Orchestration("b".into()));
        assert_eq!(
            args(&["--worktree", "x", "--orchestration", "b"]).view,
            View::Orchestration("b".into())
        );
        assert_eq!(args(&["--worktree", "x"]).view, View::Worktree("/nowhere/x".into()));
        assert_eq!(args(&["--worktree"]).view, View::Worktree("/nowhere".into()));
        assert_eq!(args(&["--orchestration", "--once"]).view, View::Everything);
        let once = args(&["--once", "--herdr"]);
        assert_eq!(once.view, View::Everything);
        assert!(once.is_once && once.is_herdr);
    }
}
