//! GitLab's cadence for a view: the first read full, then every REMOTE_EVERY
//! a fast one; `r` reads again at once and full, so does the read after one
//! that failed, and the one after more MRs got merged. GitLab's budget is
//! the user's, all clients together (#462): while reads find what the one
//! before found, the next waits twice as long, up to QUIET_EVERY (MOVING_EVERY
//! while a pipeline runs); an event reads at once: a new ask of the view, `r`,
//! a ref of origin that moved (a push, a fetch after a merge).
//!
//! One process of the user polls a view of a repo (`Poll`): the daemon, or
//! an overview that can't reach one. Another daemon (another build's) or a
//! second overview on its own takes the poller's answers from a file
//! meanwhile, and polls once the poller is gone. The daemon runs it
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

/// GitLab is read this often (overview.mjs `REMOTE_MS`); `BREADBOARD_REMOTE_EVERY_MS` another (the gauntlet's ticks, #460).
pub const REMOTE_EVERY: Duration = Duration::from_secs(30);

fn remote_every() -> Duration {
    std::env::var("BREADBOARD_REMOTE_EVERY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(REMOTE_EVERY)
}

/// The longest wait between reads while nothing changes.
const QUIET_EVERY: Duration = Duration::from_secs(120);
/// The longest wait while a pipeline of the view runs or waits: its end shows soon.
const MOVING_EVERY: Duration = Duration::from_secs(60);
/// How often origin's refs are looked at between reads (one local git call).
const REFS_EVERY: Duration = Duration::from_secs(5);
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
    view: String,
    mut asks: watch::Receiver<Option<RemoteAsk>>,
    mut refreshes: mpsc::UnboundedReceiver<()>,
    out: mpsc::UnboundedSender<Feed>,
) {
    let mut reader = RemoteReader::new(&main);
    let mut is_full = true;
    let mut has_data = false;
    let mut merged: Option<usize> = None;
    let mut develop: Option<String> = None;
    // What the last read found, its time left out, and how many reads in a row found it.
    let mut found: Option<String> = None;
    let mut same = 0u32;
    let mut poll = Poll::of(&main, &view);
    let mut followed = 0;
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
        // Another process polls this view: its answers, until it is gone.
        if !poll.hold() {
            if let Some(Feed::Remote { result, at, rate }) = poll.shared()
                && at > followed
            {
                followed = at;
                has_data |= result.is_ok();
                if out.send(Feed::Remote { result, at, rate }).is_err() {
                    return;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(REFS_EVERY) => {}
                asked = refreshes.recv() => if asked.is_none() { return },
                changed = asks.changed() => if changed.is_err() { return },
            }
            continue;
        }
        reader.gitlab = reader.gitlab.for_task(task_of(&ask));
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
                let now = fingerprint(&remote);
                same = if found.as_ref() == Some(&now) { same + 1 } else { 0 };
                found = Some(now);
                let count = merged_in(&remote);
                is_full = merged.is_some_and(|was| count > was);
                merged = Some(count);
                // develop moved on GitLab: the main checkout follows now, not at its next look (#348).
                let now = develop_in(&remote);
                if now.is_some() && develop.is_some() && now != develop {
                    breadboard_core::main_update::wake(&main);
                }
                develop = now.or(develop);
                Ok(Box::new(remote))
            }
            Err(error) => {
                is_full = was_full;
                same = 0;
                Err(error.to_string())
            }
        };
        let message = Feed::Remote {
            result,
            at: now_ms(),
            rate: gitlab.rate(),
        };
        poll.share(&message);
        if out.send(message).is_err() {
            return;
        }
        // GitLab paused after a 429 (#462): the next read once the pause ended, not one before.
        let wait = match breadboard_core::gitlab::budget::paused() {
            Some(pause) => Duration::from_millis(u64::try_from(pause.until - now_ms()).unwrap_or(0)).max(remote_every()),
            None => wait_after(same, found.as_deref().is_some_and(is_moving)),
        };
        let refs = origin_refs(&main).await;
        let mut look = tokio::time::interval_at(tokio::time::Instant::now() + REFS_EVERY, REFS_EVERY);
        let sleep = tokio::time::sleep(wait);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                asked = refreshes.recv() => match asked {
                    Some(()) => {
                        is_full = true;
                        break;
                    }
                    None => return,
                },
                changed = asks.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    break;
                }
                _ = look.tick() => {
                    if origin_refs(&main).await != refs {
                        break;
                    }
                }
            }
        }
        // Asks that came in meanwhile count as one.
        while refreshes.try_recv().is_ok() {}
    }
}

/// The right to poll a view of a repo, one process of the user at a time
/// (#462): an flock of a file in the runtime dir, held while the process
/// lives; the answers of the holder in a file next to it.
struct Poll {
    lock: PathBuf,
    answer: PathBuf,
    held: Option<std::fs::File>,
}

impl Poll {
    fn of(main: &std::path::Path, view: &str) -> Poll {
        Poll::in_dir(&breadboard_core::gitlab::budget::dir(), main, view)
    }

    fn in_dir(dir: &std::path::Path, main: &std::path::Path, view: &str) -> Poll {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        (main, view).hash(&mut hasher);
        let name = format!("gitlab-poll-{:016x}", hasher.finish());
        Poll {
            lock: dir.join(format!("{name}.lock")),
            answer: dir.join(format!("{name}.json")),
            held: None,
        }
    }

    /// Whether this process polls: it held the lock, or takes it now. Where
    /// there is no runtime dir of the user's own, every process polls.
    fn hold(&mut self) -> bool {
        use std::os::fd::AsRawFd;
        if self.held.is_some() {
            return true;
        }
        if !self.lock.parent().is_some_and(breadboard_core::local::own_dir) {
            return true;
        }
        let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.lock)
        else {
            return true;
        };
        // SAFETY: flock on a descriptor this File owns; it is released when the File is dropped.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return false;
        }
        self.held = Some(file);
        true
    }

    /// An answer of the poller, for the processes that follow it.
    fn share(&self, message: &Feed) {
        if self.held.is_some()
            && let Ok(line) = serde_json::to_string(message)
        {
            let _ = breadboard_core::json::write_whole(&self.answer, &line);
        }
    }

    /// The poller's last answer.
    fn shared(&self) -> Option<Feed> {
        serde_json::from_str(&std::fs::read_to_string(&self.answer).ok()?).ok()
    }
}

/// How long to wait after `same` reads in a row found what the one before did.
fn wait_after(same: u32, is_moving: bool) -> Duration {
    let most = if is_moving { MOVING_EVERY } else { QUIET_EVERY };
    (remote_every() * 2u32.saturating_pow(same.min(8))).min(most)
}

/// What a read found as JSON, its time left out: two reads alike say the same.
fn fingerprint(remote: &RemoteOf) -> String {
    let mut value = serde_json::to_value(remote).unwrap_or_default();
    if let Some(view) = value.as_object_mut().and_then(|view| view.values_mut().next())
        && let Some(view) = view.as_object_mut()
    {
        view.remove("at");
    }
    sorted(value).to_string()
}

/// `value` with every object's keys sorted: a HashMap's order is another on each read.
fn sorted(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut fields: Vec<_> = object.into_iter().collect();
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(fields.into_iter().map(|(key, value)| (key, sorted(value))).collect())
        }
        serde_json::Value::Array(list) => serde_json::Value::Array(list.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// Whether a read found a pipeline that runs or waits to.
fn is_moving(found: &str) -> bool {
    ["running", "pending", "created", "preparing", "waiting_for_resource"]
        .iter()
        .any(|status| {
            found.contains(&format!("\"status\":\"{status}\""))
                || found.contains(&format!("\"status\":\"{}\"", status.to_uppercase()))
        })
}

/// origin's refs in the main checkout, hashed: a push or a fetch changes them.
async fn origin_refs(main: &std::path::Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let main = main.to_path_buf();
    let refs = tokio::task::spawn_blocking(move || {
        breadboard_core::git::git(
            &main,
            &[
                "for-each-ref",
                "--format=%(objectname) %(refname)",
                "refs/remotes/origin",
            ],
        )
        .out()
        .to_string()
    })
    .await
    .unwrap_or_default();
    let mut hasher = std::hash::DefaultHasher::new();
    refs.hash(&mut hasher);
    hasher.finish()
}

/// Who asks in the request log (#462): the feed of which kind of view.
fn task_of(ask: &RemoteAsk) -> &'static str {
    match ask {
        RemoteAsk::Orchestration(_) => "feed orchestration",
        RemoteAsk::Worktree { .. } => "feed worktree",
        RemoteAsk::Everything => "feed everything",
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

/// What says develop moved: its head on origin (the view of everything), or
/// its newest pipeline (an orchestration's). None: the read has neither.
fn develop_in(remote: &RemoteOf) -> Option<String> {
    match remote {
        RemoteOf::Everything(remote) => remote.head.clone(),
        RemoteOf::Orchestration(remote) => remote.develop.as_ref().map(|pipeline| pipeline.id.to_string()),
        RemoteOf::Worktree(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::gitlab::client::Rate;
    use breadboard_core::gitlab::read::{EverythingRemote, ListedMr};
    use breadboard_core::gitlab::{BranchRemote, Mr, Remote};
    use std::path::Path;

    #[test]
    fn one_process_polls_a_view_the_others_take_its_answers() {
        let dir = std::env::temp_dir().join(format!("breadboard-poll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut perms = std::fs::metadata(&dir).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o700);
        std::fs::set_permissions(&dir, perms).unwrap();
        let main = Path::new("/repo");
        let mut poller = Poll::in_dir(&dir, main, "orchestration b");
        let mut other = Poll::in_dir(&dir, main, "orchestration b");
        let mut elsewhere = Poll::in_dir(&dir, main, "everything");
        assert!(poller.hold());
        assert!(!other.hold(), "a second poller of the same view waits");
        assert!(elsewhere.hold(), "another view polls by itself");
        let said = Feed::Remote {
            result: Err("GitLab didn't answer".into()),
            at: 42,
            rate: None,
        };
        other.share(&said);
        assert!(poller.shared().is_none(), "only the poller shares");
        poller.share(&said);
        assert!(matches!(other.shared(), Some(Feed::Remote { at: 42, .. })));
        drop(poller);
        // A test forking meanwhile holds the descriptor until its exec: a moment later it is free.
        let took = (0..100).any(|_| {
            other.hold() || {
                std::thread::sleep(std::time::Duration::from_millis(10));
                false
            }
        });
        assert!(took, "the poller gone, the next takes over");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn waits_longer_while_nothing_changes() {
        let secs = |same, is_moving| wait_after(same, is_moving).as_secs();
        assert_eq!([0, 1, 2, 3, 40].map(|same| secs(same, false)), [30, 60, 120, 120, 120]);
        assert_eq!([0, 1, 2].map(|same| secs(same, true)), [30, 60, 60]);
        assert!(is_moving(r#"{"pipeline":{"id":1,"status":"running"}}"#));
        assert!(!is_moving(r#"{"pipeline":{"id":1,"status":"success"}}"#));
    }

    #[test]
    fn two_reads_alike_but_their_time_say_the_same() {
        let read = |at| {
            RemoteOf::Orchestration(Remote {
                at,
                ..Remote::default()
            })
        };
        assert_eq!(fingerprint(&read(1)), fingerprint(&read(2)));
        // Many branches: two maps alike hold them in another order.
        let many = |at| {
            let mut remote = Remote {
                at,
                ..Remote::default()
            };
            for n in 0..20 {
                remote.of.insert(format!("feature/{n}"), BranchRemote::default());
            }
            RemoteOf::Orchestration(remote)
        };
        assert_eq!(fingerprint(&many(1)), fingerprint(&many(2)));
        let mut other = Remote::default();
        other.awaited.insert(3, "merged".into());
        assert_ne!(fingerprint(&read(1)), fingerprint(&RemoteOf::Orchestration(other)));
    }

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
