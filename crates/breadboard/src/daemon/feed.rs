//! A view's GitLab feed in the daemon: one `read_gitlab` per ask, its lines
//! sent to every overview that follows it, the last answer kept for one that
//! comes later. A feed nobody followed for GRACE goes.

use super::gitlab::{Feed as Said, read_gitlab};
use breadboard_core::reader::RemoteAsk;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{broadcast, mpsc, watch};

/// Lines a slow follower may fall behind before it skips some (progress, then answers).
const BACKLOG: usize = 64;

pub struct Feed {
    lines: broadcast::Sender<Arc<str>>,
    /// The last answer, sent first to a new follower.
    last: Arc<Mutex<Option<Arc<str>>>>,
    /// Dropped with the feed: its reads end.
    refresh: mpsc::UnboundedSender<()>,
    followers: AtomicUsize,
    left_at: Mutex<Instant>,
}

impl Feed {
    /// The feed of `ask` in the repo `main`, its first read started.
    pub fn start(main: PathBuf, ask: RemoteAsk) -> Arc<Feed> {
        let (out, mut said) = mpsc::unbounded_channel::<Said>();
        let (refresh, refreshes) = mpsc::unbounded_channel();
        let (lines, _) = broadcast::channel(BACKLOG);
        let last = Arc::new(Mutex::new(None));
        let (asks, asked) = watch::channel(Some(ask));
        tokio::spawn(async move {
            read_gitlab(main, asked, refreshes, out).await;
            drop(asks);
        });
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
            followers: AtomicUsize::new(0),
            left_at: Mutex::new(Instant::now()),
        })
    }

    /// A follower: what comes from now on, and the last answer to send first.
    pub fn follow(&self) -> (broadcast::Receiver<Arc<str>>, Option<Arc<str>>) {
        self.followers.fetch_add(1, Ordering::SeqCst);
        let lines = self.lines.subscribe();
        let last = self.last.lock().unwrap_or_else(|e| e.into_inner()).clone();
        (lines, last)
    }

    pub fn leave(&self) {
        if self.followers.fetch_sub(1, Ordering::SeqCst) == 1 {
            *self.left_at.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        }
    }

    /// `r` in one of its overviews: read again now, full, for all.
    pub fn refresh(&self) {
        let _ = self.refresh.send(());
    }

    /// Nobody followed it for GRACE.
    pub fn is_left(&self) -> bool {
        self.followers.load(Ordering::SeqCst) == 0
            && self.left_at.lock().unwrap_or_else(|e| e.into_inner()).elapsed() >= super::GRACE
    }
}
