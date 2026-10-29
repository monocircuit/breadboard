//! `breadboard overview [--orchestration <name> | --product <name> |
//! --worktree <dir>] [--once] [--herdr]`: the views of the overview on the
//! frame of `tui`; a product's (#405) holds each of its orchestrations' as a
//! section (`product`).
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
//!   `actions` (ACTIONS, UP NEXT), `work` (the worktrees and their MRs in
//!   one, from the pieces of `worktrees` and `merge_requests`), `agents`,
//!   `footer` (end orchestration, the hints);
//! - one worktree: `worktree_view`; everything: `everything`.
//!
//! A click's `Act` goes to `act`, the one place the overview does anything;
//! what it does by itself after a read too (`actor/`; the repo's changelog:
//! `overall`).

mod act;
mod actions;
pub(crate) mod actor;
mod agents;
mod boxes;
mod epic;
mod everything;
mod footer;
mod header;
mod look;
mod merge_requests;
mod next;
mod overall;
mod product;
mod settings;
mod work;
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
    /// `--product <name>` (#405): every orchestration of the product, each a section.
    pub product: Option<String>,
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
        product: value("--product").flatten(),
    }
}

pub fn main(args: &[String]) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut args = args_of(args, &cwd);
    args.is_once |= !std::io::stdout().is_terminal();
    // `here` decides the → of the view of everything: where pnpm was started.
    let here = std::env::var_os("INIT_CWD").map(PathBuf::from).unwrap_or(cwd.clone());
    let Some(reader) = LocalReader::new(&here, args.view.clone()) else {
        eprintln!("overview: {} is in no git repo", here.display());
        return ExitCode::FAILURE;
    };
    let main = reader.main.clone();
    let _ = std::env::set_current_dir(&main);
    look::init(&main);
    if let Some(name) = &args.product {
        return product::main(name, &args, &main, &cwd);
    }
    let mut app = Overview::new(&args, &main);
    if args.is_once {
        let code = once(&mut app, reader);
        if code == ExitCode::SUCCESS {
            print!("{}", view::once(&app, super::columns()));
        }
        return code;
    }
    // The folds of the overview this one replaced (#348): it went on in a new build.
    if let Some(folds) = std::env::var(RESUME)
        .ok()
        .and_then(|folds| serde_json::from_str(&folds).ok())
    {
        // The boxes come from their file, as the new build read them.
        app.folds = Folds {
            boxes: std::mem::take(&mut app.folds.boxes),
            ..folds
        };
    }
    let binary = crate::binary::Started::now();
    let runtime = match super::runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("overview: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (sender, models) = mpsc::unbounded_channel();
    daemon::client::share_reads();
    start_reading(&mut app, reader, runtime.handle(), &sender);
    let replaced = std::sync::Arc::new(binary);
    runtime.spawn(watch_binary(replaced.clone(), sender));
    let code = super::serve(runtime, &mut app, models);
    if app.is_replaced {
        // Its arguments as they were given: from where it was started.
        let _ = std::env::set_current_dir(&cwd);
        let folds = serde_json::to_string(&app.folds).unwrap_or_default();
        let error = replaced.exec(std::env::args_os().skip(1), &[(RESUME, folds)]);
        eprintln!("overview: the new build didn't start: {error}");
    }
    code
}

/// Where a headless overview's GitLab comes from: the daemon's feed of its
/// view, in its process (`asks`: what its local read asks for, `refreshes`:
/// read again now, `out`: what was read).
pub type GitlabFeed = Box<
    dyn FnOnce(
            watch::Receiver<Option<RemoteAsk>>,
            mpsc::UnboundedReceiver<()>,
            mpsc::UnboundedSender<Feed>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send,
>;

/// The overview of the orchestration `name` of `main` with no terminal
/// (#498): the daemon's, which acts for it as the overview in a pane did
/// (auto, merges, answers, `actor/`): same reads, same model, same actor,
/// only nothing is drawn. It writes what its actor keeps for the overviews
/// that draw it (`publish_acting`). It runs on the calling thread until
/// `stop` is set and nothing it does would be cut off; `busy` says whether
/// something is (a merge, the end of the orchestration). Its acting file goes with it.
pub fn headless(
    main: &Path,
    name: &str,
    gitlab: GitlabFeed,
    runtime: &tokio::runtime::Handle,
    stop: &std::sync::atomic::AtomicBool,
    busy: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;
    let view = View::Orchestration(name.to_string());
    let Some(reader) = LocalReader::new(main, view.clone()) else {
        return;
    };
    let args = Args {
        view: view.clone(),
        is_once: false,
        is_herdr: false,
        product: None,
    };
    let mut app = Overview::new(&args, main);
    app.actor.is_headless = true;
    let (sender, mut models) = mpsc::unbounded_channel();
    let (asks, asked) = watch::channel(None);
    let (refresh, refreshes) = mpsc::unbounded_channel();
    app.refresh = Some(refresh);
    let (wake, woken) = std::sync::mpsc::sync_channel(1);
    app.wake_local = Some(wake);
    act::start(&mut app, sender.clone());
    read_locally(reader, sender.clone(), asks, woken, false);
    let (out, mut said) = mpsc::unbounded_channel();
    runtime.spawn(gitlab(asked, refreshes, out));
    runtime.spawn(async move {
        while let Some(feed) = said.recv().await {
            if sender.send(Msg::Gitlab(feed)).is_err() {
                return;
            }
        }
    });
    while let Some(message) = models.blocking_recv() {
        app.update(message);
        let is_busy = app.actor.is_busy();
        busy.store(is_busy, Ordering::SeqCst);
        if stop.load(Ordering::SeqCst) && !is_busy {
            break;
        }
    }
    if app.actor.acting_written.is_some() {
        let _ = std::fs::remove_file(breadboard_core::model::acting::acting_file(main, name));
    }
    busy.store(false, Ordering::SeqCst);
}

/// The readers of a live view started, their models to `sender`: the local
/// one on its thread, GitLab's on `runtime`, the actor's jobs.
fn start_reading(
    app: &mut Overview,
    reader: LocalReader,
    runtime: &tokio::runtime::Handle,
    sender: &mpsc::UnboundedSender<Msg>,
) {
    let (asks, asked) = watch::channel(None);
    let (refresh, refreshes) = mpsc::unbounded_channel();
    app.refresh = Some(refresh);
    let (wake, woken) = std::sync::mpsc::sync_channel(1);
    app.wake_local = Some(wake);
    act::start(app, sender.clone());
    let (main, key) = (reader.main.clone(), view_key(&app.view));
    read_locally(reader, sender.clone(), asks, woken, app.actor.is_sync);
    runtime.spawn(read_gitlab(main, key, asked, refreshes, sender.clone()));
}

/// The folds an overview hands the build it goes on in.
const RESUME: &str = "BREADBOARD_RESUME_FOLDS";

/// Says `Replaced` once a build was installed over this binary, and again
/// every look while the overview waits for its actor to finish.
async fn watch_binary(binary: std::sync::Arc<crate::binary::Started>, sender: mpsc::UnboundedSender<Msg>) {
    let mut look = tokio::time::interval(Duration::from_millis(LOCAL_EVERY_MS as u64));
    loop {
        look.tick().await;
        if binary.is_replaced() && sender.send(Msg::Replaced).is_err() {
            return;
        }
    }
}

/// `--once`: the local read, then GitLab's, into `app`, to be printed (a print never acts).
fn once(app: &mut Overview, mut reader: LocalReader) -> ExitCode {
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
    ExitCode::SUCCESS
}

/// `view` of the repo `main` drawn once for the web UI (#435): its local
/// read, then what the daemon's GitLab feed of the view said, as `--once`
/// draws it (a print never acts).
pub fn drawn(main: &Path, view: View, local: Local, gitlab: Vec<Feed>, width: u16) -> ratatui::buffer::Buffer {
    look::init(main);
    let args = Args {
        view,
        is_once: true,
        is_herdr: false,
        product: None,
    };
    let mut app = Overview::new(&args, main);
    app.update(Msg::Local(Box::new(local)));
    for said in gitlab {
        app.update(Msg::Gitlab(said));
    }
    view::buffer(&app, width)
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
        daemon::client::Followed::Lost => daemon::read_gitlab(main, view, asks, refreshes, out).await,
        daemon::client::Followed::Silent => {
            let _ = out.send(Feed::Remote {
                result: Err(daemon::SILENT_TEXT.into()),
                at: now_ms(),
                rate: None,
            });
            daemon::read_gitlab(main, view, asks, refreshes, out).await;
        }
    }
}

/// Which view the daemon's GitLab feed is of: every overview of it shares one.
pub fn view_key(view: &View) -> String {
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
    /// A build was installed over this binary: the overview goes on in it (#348).
    Replaced,
    /// A product view's clock: time to look at its records (#447).
    Look,
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

/// The user's folds, kept over redraws and into a new build; None: the model's default.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Folds {
    /// WORK's done ones (#347: the merged MRs and the removed worktrees), shown until folded.
    pub is_done_folded: bool,
    pub is_queued_folded: bool,
    /// The view of everything: every open MR, every issue, past what fits the pane.
    pub is_every_mr_shown: bool,
    pub is_every_issue_shown: bool,
    /// DONE: every entry's command under it, not only a failed one's.
    pub is_every_command_shown: bool,
    /// The settings panel is open (#415): `,` or `⚙ settings`.
    pub is_settings_open: bool,
    /// BATCH's task MRs (#414), shown until folded (#491).
    #[serde(default)]
    pub is_batch_folded: bool,
    /// The boxes opened under an MR's row and a worktree's node (#346).
    /// Not handed to a new build: it reads them from their file (`folds_file`).
    #[serde(skip)]
    pub boxes: BoxFolds,
}

/// The MRs (by iid) and the worktrees (by issue) whose boxes are open
/// (#346), and the MRs folded by a click (#354): an MR with open boxes shows
/// those until it is folded, every other list is folded until it is opened.
/// Kept per viewer in a file next to the orchestration's record, so they
/// stay over a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BoxFolds {
    pub mrs: std::collections::BTreeSet<i64>,
    pub worktrees: std::collections::BTreeSet<i64>,
    pub folded_mrs: std::collections::BTreeSet<i64>,
    /// The worktrees (by issue) whose collisions are listed (#390): folded until a click.
    pub collides: std::collections::BTreeSet<i64>,
    /// The worktrees (by issue) whose whole problem shows (#390): two lines until a click.
    pub problems: std::collections::BTreeSet<i64>,
}

impl BoxFolds {
    /// What an MR's fold shows: every box, its open ones (#354: before any
    /// click, when it has some), or nothing.
    pub fn mr_list(&self, iid: i64, has_open: bool) -> MrList {
        if self.mrs.contains(&iid) {
            MrList::All
        } else if has_open && !self.folded_mrs.contains(&iid) {
            MrList::Open
        } else {
            MrList::Folded
        }
    }

    /// A click on an MR's fold: a list shown (its open boxes, or all) folds,
    /// a folded one opens every box.
    pub fn fold_mr(&mut self, iid: i64, has_open: bool) {
        if self.mr_list(iid, has_open) != MrList::Folded {
            self.mrs.remove(&iid);
            self.folded_mrs.insert(iid);
        } else {
            self.folded_mrs.remove(&iid);
            self.mrs.insert(iid);
        }
    }

    /// Where an orchestration's are kept: `.claude/orchestrations/<name>.folds`,
    /// not `.json`, or it would read as a record.
    pub fn file(main: &Path, name: &str) -> PathBuf {
        breadboard_core::record::records_dir(main).join(format!("{name}.folds"))
    }

    /// The folds kept in `file`; none open without one or with a broken one.
    pub fn read(file: &Path) -> Self {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Kept for the next start; a write that fails costs only that.
    pub fn write(&self, file: &Path) {
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(file, text + "\n");
        }
    }
}

/// What an MR's fold shows (#354).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrList {
    All,
    Open,
    Folded,
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
    /// How the main checkout followed develop (#348): the band's, `true` a warning.
    pub develop: Option<&'a (String, bool)>,
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
    /// Where the boxes' folds are kept: an orchestration's view, not `--once`.
    folds_file: Option<PathBuf>,
    refresh: Option<mpsc::UnboundedSender<()>>,
    /// Wakes the local reader: after something the actor did, its result shows at once.
    wake_local: Option<std::sync::mpsc::SyncSender<()>>,
    /// The clone's shared folder (`main_update::store_of`): where the band reads how develop was updated.
    store: Option<PathBuf>,
    /// What the band says of the main checkout's update (#348), read with every local read; `true`: a warning.
    develop: Option<(String, bool)>,
    /// What the view of a worktree started: delete armed, stack, migrations, nvim.
    worktree: worktree_view::State,
    actor: actor::Actor,
    words: actor::view::Words,
    /// A build was installed over its binary: it ends once its actor is idle and goes on in it.
    is_replaced: bool,
}

impl Overview {
    pub fn new(args: &Args, main: &Path) -> Self {
        let folds_file = match &args.view {
            View::Orchestration(name) if !args.is_once => Some(BoxFolds::file(main, name)),
            _ => None,
        };
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
            folds: Folds {
                boxes: folds_file.as_deref().map(BoxFolds::read).unwrap_or_default(),
                ..Folds::default()
            },
            folds_file,
            refresh: None,
            wake_local: None,
            store: breadboard_core::main_update::store_of(main),
            develop: None,
            worktree: worktree_view::State::default(),
            actor: actor::Actor::new(&args.view, main, args.is_once, args.is_herdr),
            words: actor::view::Words::new(),
            is_replaced: false,
        }
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            is_once: self.is_once,
            folds: self.folds.clone(),
            gitlab: &self.gitlab,
            now: now_ms(),
            main_name: &self.main_name,
            actor: &self.actor,
            words: &self.words,
            develop: self.develop.as_ref(),
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

    /// The orchestration this live overview shows, None in a print or another view.
    fn acting_name(&self) -> Option<String> {
        match &self.view {
            View::Orchestration(name) if !self.is_once => Some(name.clone()),
            _ => None,
        }
    }

    /// What the actor keeps, written for the overviews that don't act (#498): only the one that acts.
    fn publish_acting(&mut self) {
        use breadboard_core::model::acting::{self, Acting, Merging};
        let Some(name) = self.acting_name().filter(|_| self.state.is_deliverer) else {
            return;
        };
        let acting = Acting {
            pid: i64::from(std::process::id()),
            did: self.actor.did.iter().cloned().collect(),
            removals: self.state.removals.clone().into_iter().collect(),
            starts: self.state.runs.clone().into_iter().collect(),
            pipeline_tries: self.state.pipeline_tries.clone().into_iter().collect(),
            undelivered_waits: self.state.undelivered_waits.iter().cloned().collect(),
            merges: self
                .actor
                .merges
                .of
                .iter()
                .map(|(iid, merge)| {
                    (
                        *iid,
                        Merging {
                            phase: merge.phase,
                            error: merge.error.clone(),
                        },
                    )
                })
                .collect(),
        };
        acting::write(&self.actor.main, &name, &acting, &mut self.actor.acting_written);
    }

    /// What the actor of the orchestration (the daemon's, #498) keeps, drawn
    /// here: what came from its file before goes, what it says now comes,
    /// this overview's own clicks stay and win.
    fn adopt_acting(&mut self) {
        use breadboard_core::model::acting::{self, merged_log};
        let Some(name) = self.acting_name().filter(|_| !self.state.is_deliverer) else {
            return;
        };
        let now = acting::read(&self.actor.main, &name).unwrap_or_default();
        if now == self.actor.adopted {
            return;
        }
        let was = std::mem::replace(&mut self.actor.adopted, now);
        let now = &self.actor.adopted;
        let own: Vec<actor::Did> = self
            .actor
            .did
            .iter()
            .filter(|did| !was.did.contains(did))
            .cloned()
            .collect();
        self.actor.did = merged_log(&own, &now.did, actor::DID_KEPT).into();
        adopt(&mut self.state.removals, &was.removals, &now.removals);
        adopt(&mut self.state.runs, &was.starts, &now.starts);
        adopt(&mut self.state.pipeline_tries, &was.pipeline_tries, &now.pipeline_tries);
        for dir in &was.undelivered_waits {
            self.state.undelivered_waits.remove(dir);
        }
        self.state
            .undelivered_waits
            .extend(now.undelivered_waits.iter().cloned());
        let merges = &mut self.actor.merges.of;
        for (iid, merging) in &was.merges {
            if merges
                .get(iid)
                .is_some_and(|merge| merge.phase == merging.phase && merge.error == merging.error)
            {
                merges.remove(iid);
            }
        }
        for (iid, merging) in &now.merges {
            merges.entry(*iid).or_insert_with(|| actor::merge::Merge {
                phase: merging.phase,
                error: merging.error.clone(),
                seq: 0,
            });
        }
    }
}

/// `now` into `map` in place of `was`: an entry of `was` this overview
/// didn't change goes, one of `now` comes where this overview has none.
fn adopt<K: Ord + Clone + std::hash::Hash + Eq, V: Clone + PartialEq>(
    map: &mut std::collections::HashMap<K, V>,
    was: &std::collections::BTreeMap<K, V>,
    now: &std::collections::BTreeMap<K, V>,
) {
    for (key, value) in was {
        if map.get(key) == Some(value) {
            map.remove(key);
        }
    }
    for (key, value) in now {
        map.entry(key.clone()).or_insert_with(|| value.clone());
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
                settings::build(page, model, &ctx);
                next::build(page, model, &ctx);
                epic::build(page, model, &ctx);
                actions::build(page, model, &ctx);
                work::build(page, model, &ctx);
                agents::build(page, model, &ctx);
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
            Msg::Replaced => {
                self.is_replaced = true;
                return;
            }
            Msg::Look => return,
            Msg::Acted(effect) => {
                effect(self);
                self.publish_acting();
                // A full channel: a look is already due.
                if let Some(wake) = &self.wake_local {
                    let _ = wake.try_send(());
                }
                return;
            }
            Msg::Local(local) => {
                self.local = Some(*local);
                // Nothing to say with `[main_checkout] follow = false`: it doesn't follow.
                self.develop = self
                    .store
                    .as_deref()
                    .filter(|_| breadboard_core::main_update::Settings::of(&self.actor.main).is_on)
                    .and_then(|store| breadboard_core::main_update::band(&breadboard_core::main_update::read(store)));
            }
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
        self.adopt_acting();
        self.rebuild();
        act::after_read(self, is_local, is_remote);
        // Answers that didn't get in, again once their agent's pane changed: only the deliverer.
        if let Some(Model::Orchestration(model)) = &self.model
            && self.state.is_deliverer
            && !self.is_once
        {
            actions::retry_answers(&self.actor, model);
        }
        self.publish_acting();
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

    fn picked(&mut self, picked: Option<&Act>) -> Flow {
        worktree_view::picked(self, picked)
    }

    fn is_over(&self) -> bool {
        // Not while a merge runs on its thread, nor while a prompt is typed into: both would be lost.
        self.is_replaced && !self.actor.is_busy() && !self.is_typing()
    }

    fn is_typing(&self) -> bool {
        self.actor.prompt.is_some()
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
        assert_eq!(args(&["--product", "vita"]).product.as_deref(), Some("vita"));
        assert_eq!(args(&["--once"]).product, None);
        let once = args(&["--once", "--herdr"]);
        assert_eq!(once.view, View::Everything);
        assert!(once.is_once && once.is_herdr);
    }

    #[test]
    fn a_replaced_binary_ends_the_view_once_its_actor_is_idle() {
        let main = Path::new("/nowhere");
        let mut app = Overview::new(&args(&["--once"]), main);
        assert!(!app.is_over());
        app.update(Msg::Replaced);
        assert!(app.is_over());
        // A merge on its thread: not cut off, the next look ends it.
        app.actor.merges.of.insert(
            7,
            actor::merge::Merge {
                phase: actor::merge::Phase::Running,
                error: None,
                seq: 0,
            },
        );
        assert!(!app.is_over());
        app.actor.merges.of.clear();
        assert!(app.is_over());
        // Its folds go along as text and come back.
        let mut folds = Folds {
            is_done_folded: true,
            is_every_issue_shown: true,
            ..Folds::default()
        };
        folds.boxes.mrs.insert(311);
        let back: Folds = serde_json::from_str(&serde_json::to_string(&folds).unwrap()).unwrap();
        assert!(back.is_done_folded && back.is_every_issue_shown && !back.is_queued_folded);
        // The boxes stay out: the new build reads them from their file.
        assert!(back.boxes.mrs.is_empty());
    }
}
