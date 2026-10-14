//! An overview as the daemon's client: herdr's reads and its worktrees'
//! `git status` through it from any thread (`share_reads`), its view's GitLab feed followed (`follow`), the
//! daemon started when none answers. Whatever fails, the overview reads by
//! itself: a herdr read goes to herdr, a lost feed to `read_gitlab`. A daemon
//! that hangs (stopped, wedged) counts as lost too: a herdr read it doesn't
//! answer in HERDR_WAIT skips it for HERDR_SKIP, a feed silent for SILENT
//! (the daemon says it is there every ALIVE_EVERY) is given up at once.

use super::gitlab::Feed;
use super::{ALIVE_EVERY, Request, Said};
use breadboard_core::herdr;
use breadboard_core::reader::RemoteAsk;
use breadboard_core::time::now_ms;
use serde_json::Value;
use std::io::{BufRead, BufReader as StdBufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};

/// A herdr read through the daemon may take this long (it waits for another's read at most): else herdr is asked here.
const HERDR_WAIT: Duration = Duration::from_secs(3);
/// A worktree's `git status` through the daemon may take this long (git in a big worktree, another's read waited for).
const STATUS_WAIT: Duration = Duration::from_secs(10);
/// After a read the daemon didn't answer, herdr and git are asked here this long before the daemon is tried again.
const HERDR_SKIP: Duration = Duration::from_secs(30);
/// A feed that said nothing this long (not even that it is there): its daemon hangs.
const SILENT: Duration = Duration::from_secs(ALIVE_EVERY.as_secs() * 3);
/// Daemons lost in a row, no answer between them, before the overview reads by itself.
const MAX_LOSSES: usize = 3;
/// How long a daemon that said it listens gets to answer.
const START_WAIT: Duration = Duration::from_millis(500);

/// From now on this process's herdr reads and its worktrees' `git status` go through the daemon while it answers.
pub fn share_reads() {
    herdr::share_reads(herdr_read);
    breadboard_core::reader::share_status(status_read);
}

/// Until when (ms) the reads skip the daemon: it left one unanswered.
static SKIPPED_UNTIL: AtomicI64 = AtomicI64::new(0);

/// herdr's answer to `args` from the daemon; None when it doesn't answer (herdr is asked here then).
fn herdr_read(args: &[&str]) -> Option<Option<Value>> {
    if !super::is_socket_ours() {
        return None;
    }
    let request = Request::Herdr {
        args: args.iter().map(|arg| arg.to_string()).collect(),
    };
    ask_at(super::socket(), &request, HERDR_WAIT, &SKIPPED_UNTIL)
}

/// The `git status` of the worktree `dir` under `stamp` from the daemon, with
/// when it was read; None when it doesn't answer (git is asked here then).
fn status_read(dir: &Path, stamp: u64) -> Option<(i64, String)> {
    if !super::is_socket_ours() {
        return None;
    }
    let request = Request::Status {
        dir: dir.to_path_buf(),
        stamp,
    };
    ask_at::<Option<(i64, String)>>(super::socket(), &request, STATUS_WAIT, &SKIPPED_UNTIL).flatten()
}

/// One question to the daemon at `socket`, its one-line answer, waiting
/// `wait` at most; one it leaves unanswered makes the reads skip it for
/// HERDR_SKIP (`skipped_until`). None: no daemon, or it didn't answer.
fn ask_at<T: serde::de::DeserializeOwned>(
    socket: &Path,
    request: &Request,
    wait: Duration,
    skipped_until: &AtomicI64,
) -> Option<T> {
    if now_ms() < skipped_until.load(Ordering::Relaxed) {
        return None;
    }
    let mut stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
    let mut line = serde_json::to_string(request).ok()?;
    line.push('\n');
    let answer = (|| {
        stream.set_read_timeout(Some(wait)).ok()?;
        stream.set_write_timeout(Some(wait)).ok()?;
        stream.write_all(line.as_bytes()).ok()?;
        let mut answer = String::new();
        StdBufReader::new(&stream).read_line(&mut answer).ok()?;
        Some(answer)
    })();
    // Connected but no answer: a daemon that hangs, not one that is gone.
    let Some(answer) = answer.filter(|answer| answer.ends_with('\n')) else {
        let skip = i64::try_from(HERDR_SKIP.as_millis()).unwrap_or(i64::MAX);
        skipped_until.store(now_ms().saturating_add(skip), Ordering::Relaxed);
        return None;
    };
    serde_json::from_str(&answer).ok()
}

/// `herdr_read` of the daemon at `socket` (tests).
#[cfg(test)]
fn herdr_read_at(socket: &Path, args: &[&str], wait: Duration, skipped_until: &AtomicI64) -> Option<Option<Value>> {
    let request = Request::Herdr {
        args: args.iter().map(|arg| arg.to_string()).collect(),
    };
    ask_at(socket, &request, wait, skipped_until)
}

/// The daemon told that lines were queued for the headless agents of
/// `main` (#358), started when none answers; false when it can't be (the
/// lines wait in the queue for the next daemon, an overview starts one).
pub fn wake_runs(main: &Path) -> bool {
    if !super::is_socket_ours() {
        return false;
    }
    let request = Request::Runs {
        main: main.to_path_buf(),
    };
    let ask = || ask_at::<bool>(super::socket(), &request, HERDR_WAIT, &AtomicI64::new(0)).unwrap_or(false);
    if ask() {
        return true;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let is_started = serve_command(&exe).status().is_ok_and(|status| status.success());
    is_started && ask()
}

/// `<exe> serve`, no pane's (#386): the daemon outlives the pane it was
/// started from, and every `claude -p` it runs would report its session as
/// that pane's agent to herdr, which resumes it there after a restart.
fn serve_command(exe: &Path) -> Command {
    let mut command = Command::new(exe);
    command
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in breadboard_core::herdr::PANE_ENV {
        command.env_remove(name);
    }
    command
}

/// A connection to the daemon, which is started when none answers; None when it can't be.
pub async fn connect() -> Option<UnixStream> {
    if !super::is_socket_ours() {
        return None;
    }
    connect_at(super::socket(), || {
        let exe = std::env::current_exe().ok()?;
        // It detaches: what is waited for is its first half, until the daemon listens or can't.
        serve_command(&exe)
            .status()
            .ok()
            .filter(|status| status.success())
            .map(|_| ())
    })
    .await
}

async fn connect_at(socket: &Path, start: impl FnOnce() -> Option<()> + Send + 'static) -> Option<UnixStream> {
    if let Ok(stream) = UnixStream::connect(socket).await {
        return Some(stream);
    }
    tokio::task::spawn_blocking(start).await.ok()??;
    let deadline = tokio::time::Instant::now() + START_WAIT;
    while tokio::time::Instant::now() < deadline {
        if let Ok(stream) = UnixStream::connect(socket).await {
            return Some(stream);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    None
}

/// How following the daemon ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// The overview is gone (its channels closed).
    Done,
    /// The daemon can't be reached: the overview reads GitLab itself now.
    Lost,
    /// The daemon is there but says nothing (stopped, wedged): the overview
    /// reads GitLab itself now and its error line says why.
    Silent,
}

/// The GitLab feed of the view `view` from the daemon, into `out`: the ask
/// `asks` holds goes to it whenever it changes (its next read takes it, as
/// here), `r` (`refreshes`) too. A daemon that went away is started again,
/// a few times in a row at most; then it is `Lost`. One that is there but
/// silent for SILENT is `Silent` at once: it holds its socket and pid file,
/// so no other one can be started in its place.
pub async fn follow(
    main: &Path,
    view: &str,
    asks: &mut watch::Receiver<Option<RemoteAsk>>,
    refreshes: &mut mpsc::UnboundedReceiver<()>,
    out: &mpsc::UnboundedSender<Feed>,
) -> Followed {
    let followed = follow_at(main, view, asks, refreshes, out, SILENT, connect).await;
    if followed == Followed::Silent {
        // Given up for GitLab, given up for herdr: no read waits for it again.
        SKIPPED_UNTIL.store(i64::MAX, Ordering::Relaxed);
    }
    followed
}

/// `follow` with its silence and its way to the daemon given (tests).
async fn follow_at<C: Future<Output = Option<UnixStream>>>(
    main: &Path,
    view: &str,
    asks: &mut watch::Receiver<Option<RemoteAsk>>,
    refreshes: &mut mpsc::UnboundedReceiver<()>,
    out: &mpsc::UnboundedSender<Feed>,
    silent: Duration,
    connect: impl Fn() -> C,
) -> Followed {
    let mut losses = 0;
    loop {
        let ask = loop {
            if let Some(ask) = asks.borrow_and_update().clone() {
                break ask;
            }
            if asks.changed().await.is_err() {
                return Followed::Done;
            }
        };
        if losses > MAX_LOSSES {
            return Followed::Lost;
        }
        losses += 1;
        let Some(stream) = connect().await else {
            return Followed::Lost;
        };
        let (read, mut write) = stream.into_split();
        let request = Request::Gitlab {
            main: main.to_path_buf(),
            view: view.to_string(),
            ask,
        };
        if !send(&mut write, &request).await {
            continue;
        }
        let mut lines = BufReader::new(read).lines();
        let silence = tokio::time::sleep(silent);
        tokio::pin!(silence);
        loop {
            tokio::select! {
                said = lines.next_line() => {
                    // The daemon went (killed, its binary rebuilt): another one.
                    let Ok(Some(said)) = said else { break };
                    silence.as_mut().reset(tokio::time::Instant::now() + silent);
                    // An empty line: it is there (ALIVE_EVERY).
                    if said.is_empty() {
                        continue;
                    }
                    // A line it can't read: a daemon of another protocol, never taken for no news.
                    let Ok(feed) = serde_json::from_str::<Feed>(&said) else { return Followed::Lost };
                    losses = 0;
                    if out.send(feed).is_err() {
                        return Followed::Done;
                    }
                },
                () = &mut silence => return Followed::Silent,
                changed = asks.changed() => {
                    if changed.is_err() {
                        return Followed::Done;
                    }
                    let ask = asks.borrow_and_update().clone();
                    if let Some(ask) = ask && !send(&mut write, &Said::Ask(ask)).await {
                        break;
                    }
                },
                asked = refreshes.recv() => {
                    if asked.is_none() {
                        return Followed::Done;
                    }
                    if !send(&mut write, &Said::Refresh).await {
                        break;
                    }
                },
            }
        }
    }
}

/// One line to the daemon; false when it didn't go.
async fn send(write: &mut tokio::net::unix::OwnedWriteHalf, what: &impl serde::Serialize) -> bool {
    let Ok(mut line) = serde_json::to_string(what) else {
        return false;
    };
    line.push('\n');
    write.write_all(line.as_bytes()).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_is_no_panes() {
        let command = serve_command(Path::new("/x/breadboard"));
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(removed, ["HERDR_PANE_ID", "HERDR_TAB_ID"]);
    }
    use std::path::PathBuf;

    /// A folder of its own per test, gone after it (short: a socket's path is).
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = PathBuf::from(format!("/tmp/bb-daemon-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn join(&self, part: &str) -> PathBuf {
            self.0.join(part)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_herdr_through_the_daemon_and_by_itself_without_one() {
        let scratch = Scratch::new("herdr");
        let socket = scratch.join("serve.sock");
        let skipped = AtomicI64::new(0);
        // No daemon: the caller asks herdr itself, at once next time too (nothing to skip).
        assert_eq!(herdr_read_at(&socket, &["pane", "list"], HERDR_WAIT, &skipped), None);
        assert_eq!(skipped.load(Ordering::Relaxed), 0);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.spawn(super::super::serve::serve_at(socket.clone()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !socket.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        // A write is never made by the daemon, whatever is asked.
        assert_eq!(
            herdr_read_at(&socket, &["tab", "close", "t1"], HERDR_WAIT, &skipped),
            Some(None)
        );
        assert_eq!(skipped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn reads_a_worktrees_status_once_for_every_overview_under_the_same_stamp() {
        let scratch = Scratch::new("status");
        let socket = scratch.join("serve.sock");
        let repo = scratch.join("repo");
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        std::fs::create_dir_all(&repo).unwrap();
        git(&["init", "--quiet"]);
        std::fs::write(repo.join("a.md"), "a\n").unwrap();
        let skipped = AtomicI64::new(0);
        let status = |dir: &Path, stamp: u64| -> Option<(i64, String)> {
            let request = Request::Status {
                dir: dir.to_path_buf(),
                stamp,
            };
            ask_at::<Option<(i64, String)>>(&socket, &request, STATUS_WAIT, &skipped).flatten()
        };
        // No daemon: git is asked here.
        assert_eq!(status(&repo, 1), None);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.spawn(super::super::serve::serve_at(socket.clone()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !socket.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let (at, first) = status(&repo, 1).unwrap();
        assert_eq!(first, "?? a.md\n");
        assert!(now_ms() - at < 5_000);
        // Another overview under the same stamp: the same read, not a new one (b.md isn't in it).
        std::fs::write(repo.join("b.md"), "b\n").unwrap();
        assert_eq!(status(&repo, 1), Some((at, first)));
        // The stamp moved: read again.
        let (_, second) = status(&repo, 2).unwrap();
        assert_eq!(second, "?? a.md\n?? b.md\n");
        // No worktree there, or a relative path: git is asked here.
        assert_eq!(status(&scratch.join("gone"), 1), None);
        assert_eq!(status(Path::new("repo"), 1), None);
        assert_eq!(skipped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn skips_a_daemon_that_hangs_after_one_wait() {
        let scratch = Scratch::new("hang");
        let socket = scratch.join("serve.sock");
        // Bound, never accepted: connects go through, nothing answers (a stopped daemon).
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let skipped = AtomicI64::new(0);
        let wait = Duration::from_millis(200);
        let started = std::time::Instant::now();
        assert_eq!(herdr_read_at(&socket, &["pane", "list"], wait, &skipped), None);
        assert!(started.elapsed() >= wait);
        assert!(skipped.load(Ordering::Relaxed) > now_ms());
        // The next reads go to herdr at once, not after another wait.
        let started = std::time::Instant::now();
        for _ in 0..5 {
            assert_eq!(herdr_read_at(&socket, &["pane", "list"], wait, &skipped), None);
        }
        assert!(started.elapsed() < wait, "{:?}", started.elapsed());
    }

    /// `follow_at` on a daemon at `socket` that answers each connection with `answer`, silent after `silent`.
    fn followed(
        name: &str,
        silent: Duration,
        answer: fn(tokio::net::UnixStream) -> tokio::task::JoinHandle<()>,
    ) -> Option<Followed> {
        let scratch = Scratch::new(name);
        let socket = scratch.join("serve.sock");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            tokio::spawn(async move {
                let mut held = Vec::new();
                while let Ok((stream, _)) = listener.accept().await {
                    held.push(answer(stream));
                }
            });
            let (_asks, mut asked) = watch::channel(Some(RemoteAsk::Everything));
            let (_refresh, mut refreshes) = mpsc::unbounded_channel();
            let (out, _said) = mpsc::unbounded_channel();
            let connect = || async { UnixStream::connect(&socket).await.ok() };
            let follow = follow_at(
                Path::new("/repo"),
                "everything",
                &mut asked,
                &mut refreshes,
                &out,
                silent,
                connect,
            );
            tokio::time::timeout(silent * 4, follow).await.ok()
        })
    }

    #[test]
    fn gives_up_a_feed_whose_daemon_says_nothing() {
        let silent = Duration::from_millis(200);
        // It takes the request and never answers: given up after `silent`, not reconnected.
        let started = std::time::Instant::now();
        let quiet = |stream: tokio::net::UnixStream| {
            tokio::spawn(async move {
                let _held = stream;
                std::future::pending::<()>().await;
            })
        };
        assert_eq!(followed("silent", silent, quiet), Some(Followed::Silent));
        assert!(started.elapsed() < silent * 2, "{:?}", started.elapsed());
        // It says it is there, nothing else (GitLab slow): followed on.
        let alive = |mut stream: tokio::net::UnixStream| {
            tokio::spawn(async move {
                loop {
                    if stream.write_all(b"\n").await.is_err() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
        };
        assert_eq!(followed("alive", silent, alive), None);
        // It says what this overview can't read (another build's feed): read here, not waited on.
        let other = |mut stream: tokio::net::UnixStream| {
            tokio::spawn(async move {
                let _ = stream.write_all(b"{\"Remote\":{\"other\":1}}\n").await;
                std::future::pending::<()>().await;
            })
        };
        assert_eq!(followed("other", silent, other), Some(Followed::Lost));
    }

    #[test]
    fn starts_no_daemon_where_none_can_be_started() {
        let scratch = Scratch::new("start");
        let socket = scratch.join("serve.sock");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(runtime.block_on(connect_at(&socket, || None)).is_none());
    }
}
