//! A view's GitLab feed in the daemon: one `read_gitlab` per view, its lines
//! sent to every overview that follows it, the last answer first to one that
//! comes later. It reads while one follows it: the last to leave ends it
//! (`serve` keeps its last answer for GRACE, for an overview opened again).

use super::gitlab::{Feed as Said, read_gitlab};
use breadboard_core::reader::RemoteAsk;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc, watch};

/// Lines a slow follower may fall behind before it skips some (progress, then answers).
const BACKLOG: usize = 64;

pub struct Feed {
    lines: broadcast::Sender<Arc<str>>,
    /// The last answer, sent first to a new follower.
    last: Arc<Mutex<Option<Arc<str>>>>,
    /// Dropped with the feed: its reads end.
    refresh: mpsc::UnboundedSender<()>,
    /// What its next read asks for: the last a follower said.
    asks: watch::Sender<Option<RemoteAsk>>,
    followers: AtomicUsize,
}

impl Feed {
    /// The feed of `ask` in the repo `main`, its first read started; `kept`:
    /// the last answer of the feed of the view before, sent until it reads.
    pub fn start(main: PathBuf, view: String, ask: RemoteAsk, kept: Option<Arc<str>>) -> Arc<Feed> {
        let (out, mut said) = mpsc::unbounded_channel::<Said>();
        let (refresh, refreshes) = mpsc::unbounded_channel();
        let (lines, _) = broadcast::channel(BACKLOG);
        let last = Arc::new(Mutex::new(kept));
        let (asks, asked) = watch::channel(Some(ask));
        tokio::spawn(read_gitlab(main, view, asked, refreshes, out));
        tokio::spawn({
            let (lines, last) = (lines.clone(), last.clone());
            async move {
                while let Some(message) = said.recv().await {
                    let Ok(mut line) = serde_json::to_string(&message) else {
                        continue;
                    };
                    line.push('\n');
                    let line: Arc<str> = line.into();
                    if matches!(message, Said::Remote { .. }) {
                        *last.lock().unwrap_or_else(|e| e.into_inner()) = Some(line.clone());
                    }
                    let _ = lines.send(line);
                }
            }
        });
        Arc::new(Feed {
            lines,
            last,
            refresh,
            asks,
            followers: AtomicUsize::new(0),
        })
    }

    /// A follower: what comes from now on, and the last answer to send first.
    pub fn follow(&self) -> (broadcast::Receiver<Arc<str>>, Option<Arc<str>>) {
        self.followers.fetch_add(1, Ordering::SeqCst);
        let lines = self.lines.subscribe();
        let last = self.last.lock().unwrap_or_else(|e| e.into_inner()).clone();
        (lines, last)
    }

    /// A follower left; true when it was the last (the caller ends the feed).
    pub fn leave(&self) -> bool {
        self.followers.fetch_sub(1, Ordering::SeqCst) == 1
    }

    /// Its last answer.
    pub fn last(&self) -> Option<Arc<str>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A follower's local read asks for something else: the next read takes it.
    pub fn ask(&self, ask: RemoteAsk) {
        self.asks.send_if_modified(|was| {
            let is_new = was.as_ref() != Some(&ask);
            if is_new {
                *was = Some(ask);
            }
            is_new
        });
    }

    /// `r` in one of its overviews: read again now, full, for all.
    pub fn refresh(&self) {
        let _ = self.refresh.send(());
    }
}
