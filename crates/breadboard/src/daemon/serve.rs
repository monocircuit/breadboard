//! The daemon's process: detached from the overview that started it, one per
//! socket (its pid file), a connection a task, gone GRACE after the last one
//! closed or once its socket isn't its own any more.

use super::feed::Feed;
use super::{ALIVE_EVERY, GRACE, HERDR_FRESH, Request, Said};
use breadboard_core::herdr;
use breadboard_core::lock::PidFile;
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

#[derive(Default)]
struct State {
    connections: AtomicUsize,
    idle_since: Mutex<Option<Instant>>,
    herdr: Mutex<HashMap<Vec<String>, Answer>>,
    /// The feeds followed now, by view.
    feeds: Mutex<HashMap<String, Arc<Feed>>>,
    /// The last answer of a feed that ended, by view, for GRACE.
    kept: Mutex<HashMap<String, (Instant, Arc<str>)>>,
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
    let served = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .and_then(|runtime| {
            runtime.block_on(async {
                let listener = UnixListener::bind(&socket)?;
                say_ready(ready);
                serve(listener, &socket).await
            })
        });
    match served {
        Ok(()) => ExitCode::SUCCESS,
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

async fn serve(listener: UnixListener, socket: &Path) -> std::io::Result<()> {
    let own = std::fs::metadata(socket)?.ino();
    let state = Arc::new(State::default());
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
                    return Ok(());
                }
                state.kept.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, (at, _)| at.elapsed() < GRACE);
                let idle = *state.idle_since.lock().unwrap_or_else(|e| e.into_inner());
                if state.connections.load(Ordering::SeqCst) == 0 && idle.is_some_and(|at| at.elapsed() >= GRACE) {
                    let _ = std::fs::remove_file(socket);
                    return Ok(());
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
        Ok(Request::Gitlab { main, view, ask }) => {
            let key = serde_json::to_string(&(&main, &view)).unwrap_or_default();
            let (feed, said, last) = {
                let mut feeds = state.feeds.lock().unwrap_or_else(|e| e.into_inner());
                let feed = feeds
                    .entry(key.clone())
                    .or_insert_with(|| {
                        let kept = state.kept.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
                        Feed::start(main, ask.clone(), kept.map(|(_, last)| last))
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
    serve(UnixListener::bind(&socket)?, &socket).await
}
