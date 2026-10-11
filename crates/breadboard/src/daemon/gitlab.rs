//! GitLab's cadence for a view: the first read full, then every REMOTE_EVERY
//! a fast one; `r` reads again at once and full, so does the read after one
//! that failed, and the one after more MRs got merged. The daemon runs it
//! once per view's ask for all its overviews (`feed`); an overview that can't
//! reach the daemon runs it itself.

use breadboard_core::gitlab::client::Rate;
use breadboard_core::gitlab::waiting_text;
use breadboard_core::reader::{RemoteAsk, RemoteOf, RemoteReader};
use breadboard_core::time::now_ms;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// GitLab is read this often (overview.mjs `REMOTE_MS`).
pub const REMOTE_EVERY: Duration = Duration::from_secs(30);
/// While GitLab hasn't answered yet, its progress shows at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// What a view's GitLab reads say, one line each over the daemon's socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Feed {
    /// A read under way, before the first answer: `GitLab: 4/7 · pipelines`.
    Progress(String),
    /// A read ended: what it found or why it failed, when, GitLab's rate.
    Remote {
        result: Result<Box<RemoteOf>, String>,
        at: i64,
        rate: Option<Rate>,
    },
}

/// The reads of the ask `asks` holds, one at a time, each sent to `out`; it
/// ends when `out` or `refreshes` closes.
pub async fn read_gitlab(
    main: PathBuf,
    mut asks: watch::Receiver<Option<RemoteAsk>>,
    mut refreshes: mpsc::UnboundedReceiver<()>,
    out: mpsc::UnboundedSender<Feed>,
) {
    let mut reader = RemoteReader::new(&main);
    let mut is_full = true;
    let mut has_data = false;
    let mut merged: Option<usize> = None;
    loop {
        let ask = loop {
            if let Some(ask) = asks.borrow_and_update().clone() {
                break ask;
            }
            if asks.changed().await.is_err() {
                return;
            }
        };
        // Its repo gone (a gauntlet world removed): nothing to read, no cache to write.
        if !main.is_dir() {
            return;
        }
        let gitlab = reader.gitlab.clone();
        let result = {
            let read = reader.read(&ask, is_full);
            tokio::pin!(read);
            let mut tick = tokio::time::interval(PROGRESS_EVERY);
            loop {
                tokio::select! {
                    result = &mut read => break result,
                    _ = tick.tick(), if !has_data => {
                        let _ = out.send(Feed::Progress(waiting_text(&gitlab.progress())));
                    }
                }
            }
        };
        let was_full = std::mem::take(&mut is_full);
        let result = match result {
            Ok(remote) => {
                has_data = true;
                let count = merged_in(&remote);
                is_full = merged.is_some_and(|was| count > was);
                merged = Some(count);
                Ok(Box::new(remote))
            }
            Err(error) => {
                is_full = was_full;
                Err(error.to_string())
            }
        };
        let message = Feed::Remote {
            result,
            at: now_ms(),
            rate: gitlab.rate(),
        };
        if out.send(message).is_err() {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(REMOTE_EVERY) => {}
            asked = refreshes.recv() => match asked {
                Some(()) => is_full = true,
                None => return,
            },
        }
        // Asks that came in meanwhile count as one.
        while refreshes.try_recv().is_ok() {}
    }
}

/// How many of an orchestration's MRs are merged: more, and the next read is full.
fn merged_in(remote: &RemoteOf) -> usize {
    match remote {
        RemoteOf::Orchestration(remote) => remote
            .of
            .values()
            .filter(|part| part.mr.as_ref().is_some_and(|mr| mr.is_merged()))
            .count(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::gitlab::client::Rate;
    use breadboard_core::gitlab::read::{EverythingRemote, ListedMr};
    use breadboard_core::gitlab::{BranchRemote, Mr, Remote};

    #[test]
    fn sends_what_gitlab_said_whole() {
        let mut remote = Remote {
            at: 7,
            ..Remote::default()
        };
        remote.of.insert(
            "feature/1-one".into(),
            BranchRemote {
                mr: Some(Mr {
                    iid: 3,
                    title: "feat: one".into(),
                    ..Mr::default()
                }),
                ..BranchRemote::default()
            },
        );
        remote.awaited.insert(12, "merged".into());
        let sent = Feed::Remote {
            result: Ok(Box::new(RemoteOf::Orchestration(remote.clone()))),
            at: 9,
            rate: Some(Rate {
                limit: 2000,
                observed: 3,
                remaining: 1997,
            }),
        };
        let line = serde_json::to_string(&sent).unwrap();
        let Feed::Remote {
            result: Ok(back),
            at: 9,
            rate: Some(rate),
        } = serde_json::from_str(&line).unwrap()
        else {
            panic!("{line}")
        };
        let RemoteOf::Orchestration(back) = *back else { panic!() };
        assert_eq!(back, remote);
        assert_eq!(rate.remaining, 1997);
        let everything = Feed::Remote {
            result: Ok(Box::new(RemoteOf::Everything(EverythingRemote {
                mrs: vec![ListedMr {
                    iid: 4,
                    ..ListedMr::default()
                }],
                ..EverythingRemote::default()
            }))),
            at: 1,
            rate: None,
        };
        let line = serde_json::to_string(&everything).unwrap();
        assert!(matches!(
            serde_json::from_str(&line).unwrap(),
            Feed::Remote { result: Ok(_), .. }
        ));
        let failed = serde_json::to_string(&Feed::Remote {
            result: Err("GitLab didn't answer".into()),
            at: 1,
            rate: None,
        })
        .unwrap();
        assert!(matches!(
            serde_json::from_str(&failed).unwrap(),
            Feed::Remote { result: Err(why), .. } if why == "GitLab didn't answer"
        ));
    }
}
