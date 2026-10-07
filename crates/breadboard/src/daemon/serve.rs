//! The daemon's process: detached from the overview that started it, one per
//! socket (its pid file), a connection a task, gone GRACE after the last one
//! closed or once its socket isn't its own any more.

use super::feed::Feed;
use super::{GRACE, HERDR_FRESH, REFRESH, Request};
use breadboard_core::herdr;
use breadboard_core::lock::PidFile;
use serde_json::Value;
use std::collections::HashMap;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
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
    feeds: Mutex<HashMap<String, Arc<Feed>>>,
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

/// `breadboard serve [--foreground]`: without `--foreground` it detaches first (the overview that starts it waits only for that).
pub fn main(args: &[String]) -> ExitCode {
    if !args.iter().any(|arg| arg == "--foreground") && !detach() {
        return ExitCode::SUCCESS;
    }
    let socket = super::socket().to_path_buf();
    if let Some(dir) = socket.parent() {
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
    }
    // One daemon a socket: a second one started at the same time leaves.
    let Some(_held) = PidFile::hold(&socket.with_extension("pid")) else {
        return ExitCode::SUCCESS;
    };
    let _ = std::fs::remove_file(&socket);
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("serve: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve(&socket)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("serve: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The daemon as a process of its own: forked, in a session of its own, so
/// that neither the overview's end nor its terminal's takes it along. True
/// in the child; the parent returns false and exits at once.
fn detach() -> bool {
    // SAFETY: nothing runs on another thread yet; the child only calls
    // async-signal-safe setsid and chdir before it goes on as a new process.
    unsafe {
        match libc::fork() {
            0 => {
                libc::setsid();
                libc::chdir(c"/".as_ptr());
                true
            }
            _ => false,
        }
    }
}

async fn serve(socket: &Path) -> std::io::Result<()> {
    let listener = UnixListener::bind(socket)?;
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
                state.feeds.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, feed| !feed.is_left());
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
        Ok(Request::Gitlab { main, ask }) => {
            let key = serde_json::to_string(&(&main, &ask)).unwrap_or_default();
            let feed = state
                .feeds
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key)
                .or_insert_with(|| Feed::start(main, ask))
                .clone();
            let result = follow(&feed, &mut lines, &mut write).await;
            feed.leave();
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

/// A follower of a feed: its lines out, `refresh` in, until it closes.
async fn follow(
    feed: &Feed,
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    write: &mut OwnedWriteHalf,
) -> std::io::Result<()> {
    let (mut said, last) = feed.follow();
    if let Some(last) = last {
        write.write_all(last.as_bytes()).await?;
    }
    loop {
        tokio::select! {
            line = said.recv() => match line {
                Ok(line) => write.write_all(line.as_bytes()).await?,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
            },
            asked = lines.next_line() => match asked? {
                Some(line) if line.trim() == REFRESH => feed.refresh(),
                Some(_) => {}
                None => return Ok(()),
            },
        }
    }
}

/// A test's daemon, listening on `socket`.
#[cfg(test)]
pub async fn serve_at(socket: std::path::PathBuf) -> std::io::Result<()> {
    serve(&socket).await
}
