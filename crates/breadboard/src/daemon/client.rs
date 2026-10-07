//! An overview as the daemon's client: herdr's reads through it from any
//! thread (`share_herdr`), its view's GitLab feed followed (`follow`), the
//! daemon started when none answers. Whatever fails, the overview reads by
//! itself: a herdr read goes to herdr, a lost feed to `read_gitlab`.

use super::gitlab::Feed;
use super::{REFRESH, Request};
use breadboard_core::herdr;
use breadboard_core::reader::RemoteAsk;
use serde_json::Value;
use std::io::{BufRead, BufReader as StdBufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};

/// A herdr read through the daemon may take this long (it waits for another's read at most): else herdr is asked here.
const HERDR_WAIT: Duration = Duration::from_secs(3);
/// Daemons lost in a row, no answer between them, before the overview reads by itself.
const MAX_LOSSES: usize = 3;
/// How long a started daemon gets to answer.
const START_WAIT: Duration = Duration::from_millis(1500);

/// From now on this process's herdr reads go through the daemon while it answers.
pub fn share_herdr() {
    herdr::share_reads(herdr_read);
}

/// herdr's answer to `args` from the daemon; None when it doesn't answer (herdr is asked here then).
fn herdr_read(args: &[&str]) -> Option<Option<Value>> {
    herdr_read_at(super::socket(), args)
}

fn herdr_read_at(socket: &Path, args: &[&str]) -> Option<Option<Value>> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(HERDR_WAIT)).ok()?;
    stream.set_write_timeout(Some(HERDR_WAIT)).ok()?;
    let request = Request::Herdr {
        args: args.iter().map(|arg| arg.to_string()).collect(),
    };
    let mut line = serde_json::to_string(&request).ok()?;
    line.push('\n');
    stream.write_all(line.as_bytes()).ok()?;
    let mut answer = String::new();
    StdBufReader::new(stream).read_line(&mut answer).ok()?;
    serde_json::from_str(&answer).ok()
}

/// A connection to the daemon, which is started when none answers; None when it can't be.
pub async fn connect() -> Option<UnixStream> {
    connect_at(super::socket(), || {
        let exe = std::env::current_exe().ok()?;
        // It detaches at once: what is waited for is only its first half.
        Command::new(exe)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
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
}

/// The GitLab feed of the ask `asks` holds, from the daemon, into `out`; `r`
/// (`refreshes`) goes to it; a new ask follows another feed. A daemon that
/// went away is started again once; one that can't be is `Lost`.
pub async fn follow(
    main: &Path,
    asks: &mut watch::Receiver<Option<RemoteAsk>>,
    refreshes: &mut mpsc::UnboundedReceiver<()>,
    out: &mpsc::UnboundedSender<Feed>,
) -> Followed {
    let mut stream = connect().await;
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
        let Some(open) = stream.take() else {
            return Followed::Lost;
        };
        let (read, mut write) = open.into_split();
        let request = Request::Gitlab {
            main: main.to_path_buf(),
            ask: ask.clone(),
        };
        let Ok(mut line) = serde_json::to_string(&request) else {
            return Followed::Lost;
        };
        line.push('\n');
        if write.write_all(line.as_bytes()).await.is_err() {
            losses += 1;
            if losses > MAX_LOSSES {
                return Followed::Lost;
            }
            stream = connect().await;
            continue;
        }
        let mut lines = BufReader::new(read).lines();
        let is_lost = loop {
            tokio::select! {
                said = lines.next_line() => match said {
                    Ok(Some(said)) => {
                        let Ok(feed) = serde_json::from_str::<Feed>(&said) else { continue };
                        losses = 0;
                        if out.send(feed).is_err() {
                            return Followed::Done;
                        }
                    }
                    _ => break true,
                },
                changed = asks.changed() => {
                    if changed.is_err() {
                        return Followed::Done;
                    }
                    if asks.borrow().as_ref() != Some(&ask) {
                        break false;
                    }
                },
                asked = refreshes.recv() => {
                    if asked.is_none() {
                        return Followed::Done;
                    }
                    if write.write_all(format!("{REFRESH}\n").as_bytes()).await.is_err() {
                        break true;
                    }
                },
            }
        };
        if is_lost {
            // The daemon went (killed, its binary rebuilt): one again, but not one that keeps going.
            losses += 1;
            if losses > MAX_LOSSES {
                return Followed::Lost;
            }
        }
        stream = connect().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // No daemon: the caller asks herdr itself.
        assert_eq!(herdr_read_at(&socket, &["pane", "list"]), None);
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
        assert_eq!(herdr_read_at(&socket, &["tab", "close", "t1"]), Some(None));
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
