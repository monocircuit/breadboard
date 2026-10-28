//! The daemon's process: detached from the overview that started it, one per
//! socket (its pid file), a connection a task, gone GRACE after the last one
//! closed or once its socket isn't its own any more. A build installed over
//! its binary (#348) makes it go on in that one: socket and pid file given
//! up, the new binary run in its place.

use super::feed::Feed;
use super::{ALIVE_EVERY, GRACE, HERDR_FRESH, Request, Said};
use breadboard_core::lock::PidFile;
use breadboard_core::reader::{STALE_MS, STATUS};
use breadboard_core::{git, herdr};
use serde_json::Value;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// How often it looks whether it is still wanted.
const LOOK_EVERY: Duration = Duration::from_secs(5);

/// A herdr read as last made, shared by every overview while HERDR_FRESH.
type Answer = Arc<tokio::sync::Mutex<Option<(Instant, Option<Value>)>>>;

/// A worktree's `git status` as last read: the stamp of its files it was
/// asked under, and when (ms) and what git said (None: it failed).
type Status = Arc<tokio::sync::Mutex<Option<(u64, i64, Option<String>)>>>;

#[derive(Default)]
struct State {
    connections: AtomicUsize,
    idle_since: Mutex<Option<Instant>>,
    herdr: Mutex<HashMap<Vec<String>, Answer>>,
    /// The last `git status` by worktree.
    status: Mutex<HashMap<std::path::PathBuf, Status>>,
    /// The feeds followed now, by view.
    feeds: Mutex<HashMap<String, Arc<Feed>>>,
    /// The repos whose CI queue runs (`ci`), one each.
    queues: Mutex<std::collections::HashSet<std::path::PathBuf>>,
    /// The run engines of the repos (`runs`), each busy while lines are queued or runs in flight.
    engines: Mutex<HashMap<std::path::PathBuf, Arc<std::sync::atomic::AtomicBool>>>,
    /// The last answer of a feed that ended, by view, for GRACE.
    kept: Mutex<HashMap<String, (Instant, Arc<str>)>>,
    /// Whether it keeps the main checkouts on develop (`keep_up`; not a test's daemon).
    is_keeping_up: bool,
    /// The main checkouts kept on develop: each one an overview asked GitLab about.
    kept_up: Mutex<std::collections::HashSet<std::path::PathBuf>>,
}

/// How the serving ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// Not wanted any more: no connection for GRACE, or its socket another's.
    Done,
    /// A build was installed over its binary: that one goes on.
    Replaced,
}

/// One open connection, counted while it lives.
struct Open(Arc<State>);

impl Open {
    fn new(state: &Arc<State>) -> Open {
        state.connections.fetch_add(1, Ordering::SeqCst);
        Open(state.clone())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        if self.0.connections.fetch_sub(1, Ordering::SeqCst) == 1 {
            *self.0.idle_since.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        }
    }
}

/// `breadboard serve [--foreground]`: without `--foreground` it detaches
/// first; the overview that starts it waits only until it listens (exit 0)
/// or can't (1: the overview reads by itself at once).
pub fn main(args: &[String]) -> ExitCode {
    // No pane's (#386): a daemon started from a pane, or one that execs its
    // update (`--foreground`) with the environment it had, would hand the
    // pane to every `claude -p` it runs, and herdr would resume those there.
    for name in breadboard_core::herdr::PANE_ENV {
        // SAFETY: the first thing the process does, before any thread runs.
        unsafe { std::env::remove_var(name) };
    }
    let ready = if args.iter().any(|arg| arg == "--foreground") {
        None
    } else {
        match detach() {
            Detached::Parent(true) => return ExitCode::SUCCESS,
            Detached::Parent(false) => return ExitCode::FAILURE,
            Detached::Child(ready) => Some(ready),
        }
    };
    let socket = super::socket().to_path_buf();
    if !super::is_socket_ours() {
        return ExitCode::FAILURE;
    }
    // One daemon a socket: a second one started at the same time leaves, the first serves.
    let pid_file = socket.with_extension("pid");
    let Some(_held) = PidFile::hold(&pid_file) else {
        if breadboard_core::lock::pid_in(&pid_file) == 0 {
            return ExitCode::FAILURE;
        }
        say_ready(ready);
        return ExitCode::SUCCESS;
    };
    let _ = std::fs::remove_file(&socket);
    let binary = crate::binary::Started::now();
    let served = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .and_then(|runtime| {
            let served = runtime.block_on(async {
                let listener = UnixListener::bind(&socket)?;
                say_ready(ready);
                serve(listener, &socket, true, &binary).await
            });
            // A step after an update may still run (a build): it isn't waited for.
            runtime.shutdown_timeout(Duration::from_millis(100));
            served
        });
    match served {
        Ok(Ended::Done) => ExitCode::SUCCESS,
        Ok(Ended::Replaced) => {
            // Its socket gone already; the pid file too, so the new one can hold it under the same pid.
            drop(_held);
            let error = binary.exec(["serve".into(), "--foreground".into()], &[]);
            eprintln!("serve: the new binary didn't start: {error}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("serve: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Where `detach` left this process.
enum Detached {
    /// The one that was started: whether the daemon listens.
    Parent(bool),
    /// The daemon: it says on this pipe once it listens.
    Child(std::fs::File),
}

/// The daemon as a process of its own: forked, in a session of its own, so
/// that neither the overview's end nor its terminal's takes it along.
fn detach() -> Detached {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    let mut fds = [0; 2];
    // SAFETY: nothing runs on another thread yet; the child only calls
    // async-signal-safe close, setsid and chdir before it goes on as a new
    // process; each end of the pipe is owned by one File in one process.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Detached::Parent(false);
        }
        match libc::fork() {
            -1 => Detached::Parent(false),
            0 => {
                libc::close(fds[0]);
                libc::setsid();
                libc::chdir(c"/".as_ptr());
                Detached::Child(std::fs::File::from_raw_fd(fds[1]))
            }
            _ => {
                libc::close(fds[1]);
                let mut said = [0u8; 1];
                let is_ready = std::fs::File::from_raw_fd(fds[0])
                    .read(&mut said)
                    .is_ok_and(|read| read == 1);
                Detached::Parent(is_ready)
            }
        }
    }
}

/// The one that started the daemon goes on: it listens (or another one does).
fn say_ready(ready: Option<std::fs::File>) {
    use std::io::Write;
    if let Some(mut ready) = ready {
        let _ = ready.write_all(b"1");
    }
}

async fn serve(
    listener: UnixListener,
    socket: &Path,
    is_keeping_up: bool,
    binary: &crate::binary::Started,
) -> std::io::Result<Ended> {
    let own = std::fs::metadata(socket)?.ino();
    let state = Arc::new(State {
        is_keeping_up,
        ..State::default()
    });
    *state.idle_since.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    let mut look = tokio::time::interval(LOOK_EVERY);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    let open = Open::new(&state);
                    tokio::spawn(async move {
                        let (read, write) = stream.into_split();
                        let _ = connection(read, write, &open.0).await;
                        drop(open);
                    });
                }
            }
            _ = look.tick() => {
                // Its socket gone or another's (a world removed, a new daemon): it isn't asked any more.
                if std::fs::metadata(socket).map(|meta| meta.ino()).ok() != Some(own) {
                    return Ok(Ended::Done);
                }
                state.kept.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, (at, _)| at.elapsed() < GRACE);
                // A worktree no overview asked about for a while (removed, its views closed): forgotten.
                let old = breadboard_core::time::now_ms() - STALE_MS - GRACE.as_millis() as i64;
                state.status.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, status| match status.try_lock() {
                    // Read now: kept.
                    Err(_) => true,
                    Ok(held) => held.as_ref().is_none_or(|(_, at, _)| *at > old),
                });
                let idle = *state.idle_since.lock().unwrap_or_else(|e| e.into_inner());
                // A run queued or in flight keeps it: nothing else would start or watch it.
                let is_running = state
                    .engines
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .values()
                    .any(|busy| busy.load(Ordering::SeqCst));
                if is_running {
                    *state.idle_since.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                }
                // A build installed over its binary: the overviews connect to the new one. Not
                // while a run is queued or in flight: the new one wouldn't know its claude.
                if !is_running && binary.is_replaced() {
                    let _ = std::fs::remove_file(socket);
                    return Ok(Ended::Replaced);
                }
                if !is_running && state.connections.load(Ordering::SeqCst) == 0 && idle.is_some_and(|at| at.elapsed() >= GRACE) {
                    let _ = std::fs::remove_file(socket);
                    return Ok(Ended::Done);
                }
            }
        }
    }
}

async fn connection(read: OwnedReadHalf, mut write: OwnedWriteHalf, state: &Arc<State>) -> std::io::Result<()> {
    let mut lines = BufReader::new(read).lines();
    let Some(first) = lines.next_line().await? else {
        return Ok(());
    };
    match serde_json::from_str::<Request>(&first) {
        Ok(Request::Herdr { args }) => {
            let answer = herdr_read(state, args).await;
            let mut line = serde_json::to_string(&answer).unwrap_or_else(|_| "null".into());
            line.push('\n');
            write.write_all(line.as_bytes()).await
        }
        Ok(Request::Status { dir, stamp }) => {
            let answer = status_read(state, dir, stamp).await;
            let mut line = serde_json::to_string(&answer).unwrap_or_else(|_| "null".into());
            line.push('\n');
            write.write_all(line.as_bytes()).await
        }
        Ok(Request::Runs { main }) => {
            start_engine(state, &main);
            write.write_all(b"true\n").await
        }
        Ok(Request::Gitlab { main, view, ask }) => {
            // The first overview of a repo starts its CI queue, its marks, its pushes and its notifications, for as long as the daemon runs.
            if main.is_absolute()
                && state
                    .queues
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(main.clone())
            {
                tokio::spawn(super::ci::schedule(main.clone()));
                tokio::spawn(super::ready::mark(main.clone()));
                tokio::spawn(super::notify::notify(main.clone()));
                tokio::spawn(super::watch::watch(main.clone()));
                tokio::spawn(super::push::push(main.clone()));
            }
            // An overview of a repo starts its run engine: lines left queued by a daemon before run.
            start_engine(state, &main);
            if state.is_keeping_up
                && state
                    .kept_up
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(main.clone())
            {
                tokio::spawn(super::keep_up::keep_up(main.clone()));
            }
            let key = serde_json::to_string(&(&main, &view)).unwrap_or_default();
            let (feed, said, last) = {
                let mut feeds = state.feeds.lock().unwrap_or_else(|e| e.into_inner());
                let feed = feeds
                    .entry(key.clone())
                    .or_insert_with(|| {
                        let kept = state.kept.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
                        Feed::start(main, view.clone(), ask.clone(), kept.map(|(_, last)| last))
                    })
                    .clone();
                let (said, last) = feed.follow();
                (feed, said, last)
            };
            feed.ask(ask);
            let result = follow(&feed, said, last, &mut lines, &mut write).await;
            let mut feeds = state.feeds.lock().unwrap_or_else(|e| e.into_inner());
            if feed.leave() {
                feeds.remove(&key);
                if let Some(last) = feed.last() {
                    state
                        .kept
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(key, (Instant::now(), last));
                }
            }
            result
        }
        Err(_) => Ok(()),
    }
}

/// The run engine of `main` (`runs`), once per daemon.
fn start_engine(state: &State, main: &Path) {
    if !main.is_absolute() {
        return;
    }
    // It stands in `/`: what reads the repo's config without a path reads this one's (#380).
    breadboard_core::config::act_on(main);
    let mut engines = state.engines.lock().unwrap_or_else(|e| e.into_inner());
    if engines.contains_key(main) {
        return;
    }
    let busy = Arc::new(std::sync::atomic::AtomicBool::new(true));
    engines.insert(main.to_path_buf(), busy.clone());
    tokio::spawn(super::runs::schedule(main.to_path_buf(), busy));
    // herdr's stale records of the repo's spaces go at every start (#425), the
    // status's tick after a restore of herdr: `herdr heal`, on its own.
    if herdr::is_in_herdr() {
        let _ = crate::herdr::plugin::start_heal(main);
    }
}

/// herdr's answer to `args`, read once for every overview that asks within HERDR_FRESH; only reads are ever made.
async fn herdr_read(state: &State, args: Vec<String>) -> Option<Value> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    if !herdr::is_read(&words) {
        return None;
    }
    let answer = state
        .herdr
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(args.clone())
        .or_default()
        .clone();
    // One read at a time per question: the others wait for it and take its answer.
    let mut answer = answer.lock().await;
    if let Some((at, value)) = answer.as_ref()
        && at.elapsed() < HERDR_FRESH
    {
        return value.clone();
    }
    let value = tokio::task::spawn_blocking(move || {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        herdr::herdr_here(&words)
    })
    .await
    .ok()
    .flatten();
    *answer = Some((Instant::now(), value.clone()));
    value
}

/// `git status` of the worktree `dir` under the stamp of its files: one
/// call for every overview that asks under the same stamp within STALE_MS
/// (the others wait for it); a stamp that moved, or an answer older, reads
/// again. None (the overview asks git itself): no worktree there, or git failed.
async fn status_read(state: &State, dir: std::path::PathBuf, stamp: u64) -> Option<(i64, String)> {
    if !dir.is_absolute() || !dir.is_dir() {
        return None;
    }
    let status = state
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(dir.clone())
        .or_default()
        .clone();
    let mut status = status.lock().await;
    let now = breadboard_core::time::now_ms();
    if let Some((held, at, out)) = status.as_ref()
        && *held == stamp
        && now - at < STALE_MS
    {
        return out.clone().map(|out| (*at, out));
    }
    let out = tokio::task::spawn_blocking(move || git::git(&dir, &STATUS))
        .await
        .ok()
        .filter(|out| out.is_ok)
        .map(|out| out.stdout);
    *status = Some((stamp, now, out.clone()));
    out.map(|out| (now, out))
}

/// A follower of a feed: its lines out, what it says in, until it closes;
/// an empty line every ALIVE_EVERY says the daemon is there (`client::SILENT`).
async fn follow(
    feed: &Feed,
    mut said: tokio::sync::broadcast::Receiver<Arc<str>>,
    last: Option<Arc<str>>,
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    write: &mut OwnedWriteHalf,
) -> std::io::Result<()> {
    if let Some(last) = last {
        write.write_all(last.as_bytes()).await?;
    }
    let mut alive = tokio::time::interval_at(tokio::time::Instant::now() + ALIVE_EVERY, ALIVE_EVERY);
    alive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = alive.tick() => write.write_all(b"\n").await?,
            line = said.recv() => match line {
                Ok(line) => write.write_all(line.as_bytes()).await?,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
            },
            said = lines.next_line() => match said?.map(|line| serde_json::from_str::<Said>(&line)) {
                Some(Ok(Said::Ask(ask))) => feed.ask(ask),
                Some(Ok(Said::Refresh)) => feed.refresh(),
                Some(Err(_)) => {}
                None => return Ok(()),
            },
        }
    }
}

/// A test's daemon, listening on `socket`.
#[cfg(test)]
pub async fn serve_at(socket: std::path::PathBuf) -> std::io::Result<()> {
    let binary = crate::binary::Started::at(socket.with_extension("binary"));
    serve(UnixListener::bind(&socket)?, &socket, false, &binary)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goes_on_in_a_build_installed_over_its_binary() {
        let dir = std::path::PathBuf::from(format!("/tmp/bb-serve-replaced-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (socket, file) = (dir.join("serve.sock"), dir.join("breadboard"));
        std::fs::write(&file, "old").unwrap();
        let binary = crate::binary::Started::at(file.clone());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ended = runtime.block_on(async {
            let listener = UnixListener::bind(&socket).unwrap();
            let serving = tokio::spawn({
                let socket = socket.clone();
                async move { serve(listener, &socket, false, &binary).await.unwrap() }
            });
            // The launcher's install: copied beside it, renamed over it.
            let new = dir.join("breadboard.new");
            std::fs::write(&new, "new").unwrap();
            std::fs::set_permissions(&new, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
            std::fs::rename(&new, &file).unwrap();
            tokio::time::timeout(LOOK_EVERY * 3, serving).await.unwrap().unwrap()
        });
        assert_eq!(ended, Ended::Replaced);
        // Its socket given up: the overviews connect to the new one.
        assert!(!socket.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
