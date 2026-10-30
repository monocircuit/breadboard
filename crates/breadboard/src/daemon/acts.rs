//! The daemon acts for the orchestrations of its repos (#498): what the
//! overview of an orchestration does by itself (the lines it tells, the
//! hand-outs, the merges into a batch, the removals of merged worktrees,
//! the waits and answers it delivers, `tui/overview/actor/`) runs here, one
//! headless overview per orchestration with a record, whether a terminal
//! shows it or not. It holds the orchestration's pid file (`actor::pid_file`),
//! so no overview in a pane acts beside it; one that can't reach the
//! daemon acts by itself, as before. What it did and has under way is
//! written next to the record (core's `model::acting`) for the overviews
//! that draw it.
//!
//! An orchestration acted for keeps the daemon (`serve`): a repo's
//! orchestrations go on with no overview open; one whose record went
//! (ended) stops. A build installed over the binary waits while one of
//! them merges or ends its orchestration.

use super::Feed;
use super::serve::{State, feed_key, feed_of, leave};
use crate::tui::overview::{Doer, GitlabFeed, headless};
use breadboard_core::command::{Asked, Command, Done, PROTOCOL};
use breadboard_core::reader::RemoteAsk;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc, watch};

/// How long a command waits for the actor of an orchestration started just now, and how often it looks.
const STARTING: std::time::Duration = std::time::Duration::from_secs(30);
const STARTING_LOOK: std::time::Duration = std::time::Duration::from_millis(500);

/// One orchestration acted for: told to stop, busy (a merge), ended; the way to its actor once it runs.
#[derive(Default)]
struct Acting {
    stop: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    is_ended: Arc<AtomicBool>,
    doer: Arc<Mutex<Option<Doer>>>,
}

/// The orchestrations acted for, by repo and name.
#[derive(Default)]
pub struct Acts {
    of: Mutex<HashMap<(PathBuf, String), Acting>>,
}

impl Acts {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(PathBuf, String), Acting>> {
        self.of.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether it acts for any orchestration: the daemon stays.
    pub fn is_acting(&self) -> bool {
        self.lock()
            .values()
            .any(|acting| !acting.is_ended.load(Ordering::SeqCst))
    }

    /// Whether one of them does what an exec would cut off.
    pub fn is_busy(&self) -> bool {
        self.lock().values().any(|acting| acting.busy.load(Ordering::SeqCst))
    }

    /// Before the daemon goes on in a new build: each stops, and the pid
    /// files it holds go, so the new one (the same pid) takes them.
    pub fn release(&self) {
        for ((_, name), acting) in self.lock().drain() {
            acting.stop.store(true, Ordering::SeqCst);
            let file = crate::tui::overview::actor::pid_file(&name);
            if breadboard_core::lock::pid_in(&file) == i64::from(std::process::id()) {
                let _ = std::fs::remove_file(file);
            }
        }
    }
}

/// The orchestrations of `main` acted for: one with a record and none yet
/// starts, one whose record went stops. Called on the runtime.
pub fn keep(state: &Arc<State>, main: &Path) {
    if !main.is_absolute() || std::env::var_os("BREADBOARD_NO_ACTS").is_some() {
        return;
    }
    let names = breadboard_core::record::recorded(main);
    let mut of = state.acts.lock();
    of.retain(|(repo, name), acting| {
        let is_kept = repo != main || names.contains(name);
        if !is_kept {
            acting.stop.store(true, Ordering::SeqCst);
        }
        is_kept && !acting.is_ended.load(Ordering::SeqCst)
    });
    for name in names {
        let key = (main.to_path_buf(), name.clone());
        if of.contains_key(&key) {
            continue;
        }
        let acting = Acting::default();
        let (stop, busy, is_ended, doer) = (
            acting.stop.clone(),
            acting.busy.clone(),
            acting.is_ended.clone(),
            acting.doer.clone(),
        );
        let gitlab = feed_in(
            state.clone(),
            main.to_path_buf(),
            crate::tui::overview::view_key(&breadboard_core::reader::View::Orchestration(name.clone())),
        );
        let (runtime, main) = (tokio::runtime::Handle::current(), main.to_path_buf());
        let spawned = std::thread::Builder::new().name(format!("acts {name}")).spawn(move || {
            // A panic costs this orchestration's actor, not the daemon: the next look starts it again.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                headless(&main, &name, gitlab, &runtime, &stop, &busy, |ready| {
                    *doer.lock().unwrap_or_else(|e| e.into_inner()) = Some(ready);
                });
            }));
            busy.store(false, Ordering::SeqCst);
            is_ended.store(true, Ordering::SeqCst);
        });
        if spawned.is_ok() {
            of.insert(key, acting);
        }
    }
}

/// A command of the table (#498): asked once by its id (asked again, the
/// first result), done by the actor of its orchestration or here.
pub async fn command(state: &Arc<State>, asked: Asked) -> Done {
    let Asked {
        id,
        protocol,
        main,
        command,
    } = asked;
    if protocol != PROTOCOL {
        return Done::of(
            &id,
            Err(format!("protocol {protocol} asked, this node speaks {PROTOCOL}")),
        );
    }
    if !main.is_absolute() {
        return Done::of(&id, Err(format!("{} is no repo", main.display())));
    }
    if let Some(done) = breadboard_core::command::kept(&main, &id) {
        return done;
    }
    if !state.asked.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone()) {
        return Done::of(&id, Err(format!("command {id} is under way")));
    }
    let result = match command.orchestration().map(String::from) {
        Some(name) => by_actor(state, &main, &name, command).await,
        None => {
            let (main, state) = (main.clone(), state.clone());
            tokio::task::spawn_blocking(move || by_node(&state, &main, &command))
                .await
                .unwrap_or_else(|error| Err(error.to_string()))
        }
    };
    let done = Done::of(&id, result);
    breadboard_core::command::keep(&main, &done);
    state.asked.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    done
}

/// A command of an orchestration's actor: to its headless overview.
async fn by_actor(state: &Arc<State>, main: &Path, name: &str, command: Command) -> Result<String, String> {
    if !breadboard_core::record::recorded(main)
        .iter()
        .any(|known| known == name)
    {
        return Err(format!("no orchestration {name} in {}", main.display()));
    }
    keep(state, main);
    // An actor started just now acts once it read: a while for that, then it is refused.
    let until = tokio::time::Instant::now() + STARTING;
    loop {
        let doer = state
            .acts
            .lock()
            .get(&(main.to_path_buf(), name.to_string()))
            .and_then(|acting| acting.doer.lock().unwrap_or_else(|e| e.into_inner()).clone());
        let result = match doer.and_then(|doer| doer.act(command.clone())) {
            Some(result) => result
                .await
                .unwrap_or_else(|_| Err(format!("the actor of {name} ended"))),
            None => Err(crate::tui::overview::NOT_YET.into()),
        };
        let is_not_yet = result.as_ref().is_err_and(|why| why == crate::tui::overview::NOT_YET);
        if !is_not_yet || tokio::time::Instant::now() >= until {
            return result.map_err(|why| {
                if is_not_yet {
                    format!("the actor of {name} {why}")
                } else {
                    why
                }
            });
        }
        tokio::time::sleep(STARTING_LOOK).await;
    }
}

/// A command of the node's alone: the runs of the repo, a worktree's stack.
fn by_node(state: &Arc<State>, main: &Path, command: &Command) -> Result<String, String> {
    use breadboard_core::runs;
    match command {
        Command::PauseRuns { now } => {
            runs::pause(main, breadboard_core::time::now_ms(), false).map_err(|error| error.to_string())?;
            let ended = if *now { runs::end_running(main) } else { 0 };
            Ok(match ended {
                0 => "paused: no run starts until the resume".into(),
                ended => format!("paused: {ended} runs ended, their lines wait for the resume"),
            })
        }
        Command::ResumeRuns => match runs::resume(main).map_err(|error| error.to_string())? {
            None => Ok("the agents weren't paused".into()),
            Some(_) => {
                super::serve::start_engine(state, main);
                Ok("the agents run again".into())
            }
        },
        Command::Tell { dir, line } => {
            if crate::tui::overview::actor::herdr::enqueue(main, dir, line) {
                Ok(format!(
                    "queued for the agent of {}",
                    breadboard_core::git::feature_of(main, dir)
                ))
            } else {
                Err(format!("no agent bound to {}", dir.display()))
            }
        }
        Command::StopAgent { dir } => {
            let session = breadboard_core::conversations::bound_session(dir)
                .ok_or_else(|| format!("no agent bound to {}", dir.display()))?;
            let ended = runs::end_running_of(main, Some(&session));
            let held = crate::tui::overview::actor::end_held::end(main, dir).ok();
            let who = breadboard_core::git::feature_of(main, dir);
            Ok(match (ended, held) {
                (0, None) => format!("the agent of {who} had no run in flight"),
                (0, Some(held)) => held,
                (_, None) => format!("the run of {who}'s agent ended"),
                (_, Some(held)) => format!("the run of {who}'s agent ended; {held}"),
            })
        }
        Command::StopStack { dir } => stop_stack(main, dir),
        _ => Err("an orchestration's actor does it".into()),
    }
}

/// The stack of the worktree `dir` stopped, its dev server and tools with it
/// (`stack:down` in a child): never the main checkout's, which the
/// worktrees without one of their own share.
fn stop_stack(main: &Path, dir: &Path) -> Result<String, String> {
    let who = breadboard_core::git::feature_of(main, dir);
    if dir == main || breadboard_core::stack::offset(dir) == 0 {
        return Err(format!("{who} has no stack of its own: the main checkout's is shared"));
    }
    let base = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let output = std::process::Command::new(crate::cli::lifecycle::proc::exe())
        .args(["stack:down", &base])
        .current_dir(main)
        .output()
        .map_err(|error| format!("stack:down didn't start: {error}"))?;
    if output.status.success() {
        Ok(format!("the stack of {who} stopped"))
    } else {
        let said = String::from_utf8_lossy(&output.stderr);
        let last = said
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("it failed");
        Err(format!("the stack of {who} didn't stop: {}", last.trim()))
    }
}

/// The GitLab of a headless overview: the daemon's feed of `view`, followed
/// in its process as an overview's connection follows it (`serve::follow`).
fn feed_in(state: Arc<State>, main: PathBuf, view: String) -> GitlabFeed {
    Box::new(move |mut asks, mut refreshes, out| {
        Box::pin(async move {
            let ask = loop {
                if let Some(ask) = asks.borrow_and_update().clone() {
                    break ask;
                }
                if asks.changed().await.is_err() {
                    return;
                }
            };
            let key = feed_key(&main, &view);
            let (feed, said, last) = feed_of(&state, &key, main, view, ask);
            follow_in(&feed, said, last, &mut asks, &mut refreshes, &out).await;
            leave(&state, key, &feed);
        })
    })
}

async fn follow_in(
    feed: &super::feed::Feed,
    mut said: broadcast::Receiver<Arc<str>>,
    last: Option<Arc<str>>,
    asks: &mut watch::Receiver<Option<RemoteAsk>>,
    refreshes: &mut mpsc::UnboundedReceiver<()>,
    out: &mpsc::UnboundedSender<Feed>,
) {
    if let Some(last) = last
        && !send(&last, out)
    {
        return;
    }
    loop {
        tokio::select! {
            line = said.recv() => match line {
                Ok(line) => if !send(&line, out) { return },
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            changed = asks.changed() => {
                if changed.is_err() {
                    return;
                }
                let ask = asks.borrow_and_update().clone();
                if let Some(ask) = ask {
                    feed.ask(ask);
                }
            },
            asked = refreshes.recv() => {
                if asked.is_none() {
                    return;
                }
                feed.refresh();
            },
        }
    }
}

/// A feed's lines (one, or the last with data and a failure after it) to
/// the overview; false once it is gone.
fn send(lines: &str, out: &mpsc::UnboundedSender<Feed>) -> bool {
    lines
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str::<Feed>(line).ok())
        .all(|feed| out.send(feed).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_each_line_of_a_kept_answer() {
        let (out, mut said) = mpsc::unbounded_channel();
        let progress = serde_json::to_string(&Feed::Progress("GitLab 1/2".into())).unwrap();
        assert!(send(&format!("{progress}\n\n{progress}\n"), &out));
        assert!(matches!(said.try_recv(), Ok(Feed::Progress(_))));
        assert!(matches!(said.try_recv(), Ok(Feed::Progress(_))));
        assert!(said.try_recv().is_err());
        drop(said);
        assert!(!send(&progress, &out));
    }
}
