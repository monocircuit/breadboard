//! The run engine of a repo's headless task agents (#358, core's `runs`):
//! every EVERY it takes the
//! queued lines of the sessions that may run now and runs `claude -p` with
//! them, one run per session at a time, in the session's worktree. Lines
//! queued while a run is in flight wait for the next. A session open in an
//! interactive claude (`~/.claude/sessions`, a live pid that isn't a run's)
//! holds its queue. Runs start while memory allows (`runs::room_for`), in
//! merge order, then by age. One engine per repo across daemons of other
//! builds: `.claude/runs/engine.pid`.

use breadboard_core::conversations::{interactive_resumes, running_claudes, transcript_of};
use breadboard_core::lock::PidFile;
use breadboard_core::permissions;
use breadboard_core::runs::{
    self, Event, Queue, Run, STARTING_MS, State, Table, Waiting, allowed_of, args_of, change_queue, prompt_of,
    read_or_default, room_for, status_file, write_whole,
};
use breadboard_core::time::now_ms;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// How often the queue is looked at: a line waits at most this long for its run.
pub const EVERY: Duration = Duration::from_millis(500);
/// The program a run is: `claude` on PATH (the gauntlet's fake there).
const CLAUDE: &str = "claude";

/// The runs this engine started and still waits for: session → (pid, started ms).
type InFlight = Arc<Mutex<HashMap<String, (i64, i64)>>>;

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The engine of `main`, as long as the daemon runs; `busy` says whether
/// anything is queued or in flight (the daemon stays while it is).
pub async fn schedule(main: PathBuf, busy: Arc<AtomicBool>) {
    let in_flight: InFlight = Arc::default();
    let table = Arc::new(Mutex::new(()));
    let mut held: Option<PidFile> = None;
    loop {
        if !main.is_dir() {
            busy.store(false, Ordering::SeqCst);
            return;
        }
        if held.is_none() {
            held = PidFile::hold(&runs::dir_of(&main).join("engine.pid"));
        }
        if held.is_some() {
            let starts = {
                let (main, in_flight) = (main.clone(), in_flight.clone());
                tokio::task::spawn_blocking(move || take_starts(&main, &in_flight))
                    .await
                    .unwrap_or_default()
            };
            for start in starts {
                tokio::spawn(run(main.clone(), start, in_flight.clone(), table.clone()));
            }
        }
        let is_queued = !read_or_default::<Queue>(&runs::queue_file(&main)).events.is_empty();
        busy.store(is_queued || !locked(&in_flight).is_empty(), Ordering::SeqCst);
        tokio::time::sleep(EVERY).await;
    }
}

/// A run to start: its session, where and as whom, its lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub session: String,
    pub dir: PathBuf,
    pub name: String,
    pub events: Vec<Event>,
}

/// The sessions that may run now, their lines taken from the queue: none in
/// flight (here or by an engine before, its claude alive), none open
/// interactively, as many as memory allows, in merge order, then by age. A
/// session whose worktree is gone loses its lines.
fn take_starts(main: &Path, in_flight: &InFlight) -> Vec<Start> {
    let queue: Queue = read_or_default(&runs::queue_file(main));
    if queue.events.is_empty() {
        return Vec::new();
    }
    // Past the daily limit the lines wait (#372); the overview says why.
    if runs::held_now(main, now_ms()).is_some() {
        return Vec::new();
    }
    let table: Table = read_or_default(&status_file(main));
    let flying = locked(in_flight).clone();
    let ours: HashSet<i64> = flying.values().map(|(pid, _)| *pid).collect();
    let mut open: HashSet<String> = running_claudes()
        .into_iter()
        .filter(|claude| !ours.contains(&claude.pid))
        .map(|claude| claude.id)
        .collect();
    // One opened in a tab and still at its folder's trust question has no
    // session file yet: its command line holds the queue too (#361's probe).
    open.extend(interactive_resumes());
    let states = runs::states_of(&table, &queue);
    let now = now_ms();
    let starting = flying.values().filter(|(_, at)| now - at < STARTING_MS).count();
    let waiting: Vec<Waiting> = runs::waiting_in(&queue)
        .into_iter()
        .filter(|(session, ..)| !flying.contains_key(session) && !open.contains(session))
        .filter(|(session, ..)| states.get(session).is_none_or(|run| run.state != State::Running))
        .map(|(session, dir, at)| Waiting {
            place: runs::place_of(main, &dir),
            session,
            at,
        })
        .collect();
    let chosen = pick(waiting, room_for(runs::available(), starting));
    if chosen.is_empty() {
        return Vec::new();
    }
    change_queue(main, |queue| {
        let mut starts = Vec::new();
        for session in &chosen {
            let (events, rest): (Vec<Event>, Vec<Event>) = std::mem::take(&mut queue.events)
                .into_iter()
                .partition(|event| event.session == *session);
            queue.events = rest;
            // The worktree and name it stands in now: the newest line's (a handover moves both).
            let Some(last) = events.last() else { continue };
            if !last.dir.is_dir() {
                continue;
            }
            starts.push(Start {
                session: session.clone(),
                dir: last.dir.clone(),
                name: last.name.clone(),
                events,
            });
        }
        starts
    })
    .unwrap_or_default()
}

/// The sessions to start: the first `room` in merge order, then by age.
pub fn pick(waiting: Vec<Waiting>, room: usize) -> Vec<String> {
    runs::ordered(waiting)
        .into_iter()
        .take(room)
        .map(|waiting| waiting.session)
        .collect()
}

/// `change` of the status table, one at a time in this daemon (`table` held).
fn change_table(main: &Path, table: &Mutex<()>, change: impl FnOnce(&mut Table)) {
    let _held = locked(table);
    let file = status_file(main);
    let mut read: Table = read_or_default(&file);
    change(&mut read);
    let _ = write_whole(&file, &read);
}

/// One run of `start`: `claude -p` in its worktree, its stream into its
/// log, its row of the table `running` until it ends, then `idle`, or
/// `asks` when a tool was denied.
async fn run(main: PathBuf, start: Start, in_flight: InFlight, table: Arc<Mutex<()>>) {
    let Start {
        session,
        dir,
        name,
        events,
    } = start;
    let prompt = prompt_of(&events);
    let is_known = transcript_of(&session).is_some();
    let log = runs::log_file(&main, &session);
    if let Some(folder) = log.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let spawned = tokio::process::Command::new(CLAUDE)
        .args(args_of(&session, &name, &prompt, is_known, &allowed_of(&events)))
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false)
        .spawn();
    let row = |state, pid, code, denials| Run {
        dir: dir.clone(),
        name: name.clone(),
        state,
        since: now_ms(),
        pid,
        lines: events.len(),
        denials,
        code,
    };
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::write(
                &log,
                format!("{{\"type\":\"breadboard\",\"error\":{:?}}}\n", error.to_string()),
            );
            // Its lines go back: the next look tries again (a claude installed meanwhile).
            let _ = change_queue(&main, |queue| {
                let mut events = events.clone();
                events.append(&mut queue.events);
                queue.events = events;
            });
            change_table(&main, &table, |read| {
                read.runs
                    .insert(session.clone(), row(State::Idle, None, None, Vec::new()));
            });
            return;
        }
    };
    let pid = child.id().map(i64::from).unwrap_or_default();
    locked(&in_flight).insert(session.clone(), (pid, now_ms()));
    change_table(&main, &table, |read| {
        read.runs
            .insert(session.clone(), row(State::Running, Some(pid), None, Vec::new()));
    });
    let mut lines = Vec::new();
    if let Ok(mut file) = tokio::fs::File::create(&log).await {
        let mut stdout = child.stdout.take().map(|out| BufReader::new(out).lines());
        let mut stderr = child.stderr.take().map(|err| BufReader::new(err).lines());
        loop {
            let line = tokio::select! {
                line = async { stdout.as_mut()?.next_line().await.ok().flatten() }, if stdout.is_some() => {
                    if line.is_none() { stdout = None; }
                    line
                }
                line = async { stderr.as_mut()?.next_line().await.ok().flatten() }, if stderr.is_some() => {
                    if line.is_none() { stderr = None; }
                    line.map(|line| serde_json::json!({ "type": "stderr", "text": line }).to_string())
                }
                else => break,
            };
            let Some(line) = line else { continue };
            let _ = file.write_all(format!("{line}\n").as_bytes()).await;
            lines.push(line);
        }
    }
    let code = child.wait().await.ok().and_then(|status| status.code());
    let denials = asked_of(&dir, &name, &lines);
    let state = if denials.is_empty() { State::Idle } else { State::Asks };
    change_table(&main, &table, |read| {
        read.runs.insert(session.clone(), row(state, None, code, denials));
    });
    locked(&in_flight).remove(&session);
}

/// What a run that ended was denied that a tab would have asked the user
/// (#360), each posted as a `permission` action in its worktree; a block
/// by the classifier or a deny rule stays the refusal the agent was told.
fn asked_of(dir: &Path, name: &str, lines: &[String]) -> Vec<runs::Denial> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let asked: Vec<_> = permissions::denials_of(lines, &permissions::deny_rules(dir, &home))
        .into_iter()
        .filter(|denial| denial.why == permissions::Why::Asked)
        .collect();
    let _ = permissions::post_denials(dir, name, &asked);
    asked
        .into_iter()
        .map(|denial| runs::Denial {
            tool: denial.tool,
            input: denial.input,
        })
        .collect()
}

/// `breadboard runs [tell <dir> <line…> | continue]`: the status table of
/// the repo's headless agents with what they spent today, a line queued for
/// the agent of the worktree `dir`, or the daily limit lifted until midnight.
pub fn cli(args: &[String]) -> ExitCode {
    let Ok(cwd) = std::env::current_dir() else {
        return ExitCode::FAILURE;
    };
    let Some(main) = breadboard_core::git::main_dir(&cwd) else {
        eprintln!("breadboard runs: not in a checkout of the repo");
        return ExitCode::FAILURE;
    };
    match args.first().map(String::as_str) {
        None => {
            let table: Table = read_or_default(&status_file(&main));
            let queue: Queue = read_or_default(&runs::queue_file(&main));
            let mut states: Vec<(String, Run)> = runs::states_of(&table, &queue).into_iter().collect();
            states.sort_by(|a, b| a.1.name.cmp(&b.1.name));
            let now = now_ms();
            let today = breadboard_core::spend::today_of(&main, now);
            println!("today {}", today.total.text());
            if let Some(why) = runs::held_now(&main, now) {
                println!("{why}: no run starts (breadboard runs continue lifts it until midnight)");
            }
            for (session, run) in states {
                let queued = queue.events.iter().filter(|event| event.session == session).count();
                let spent = today.by_session.get(&session).copied().unwrap_or_default();
                println!(
                    "{:<40} {:<8} {} queued  {:<26} {session}",
                    run.name,
                    run.state.status(),
                    queued,
                    spent.text()
                );
                if let Some(line) = snapshot_of(&main, &run.dir, now) {
                    println!("{:<40} {line}", "");
                }
            }
            ExitCode::SUCCESS
        }
        Some("tell") if args.len() >= 3 => {
            let dir = PathBuf::from(&args[1]);
            let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
            let line = args[2..].join(" ");
            if crate::tui::overview::actor::herdr::enqueue(&main, &dir, &line) {
                ExitCode::SUCCESS
            } else {
                eprintln!("breadboard runs: no agent bound to {}", dir.display());
                ExitCode::FAILURE
            }
        }
        Some("continue") => {
            let day = breadboard_core::time::day(now_ms());
            if let Err(error) = runs::lift_limit(&main, &day) {
                eprintln!("breadboard runs: {error}");
                return ExitCode::FAILURE;
            }
            crate::daemon::client::wake_runs(&main);
            println!("the daily limit is lifted until midnight");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: breadboard runs [tell <dir> <line…> | continue]");
            ExitCode::from(2)
        }
    }
}

/// The line under a run's row naming the newest snapshot of its worktree's
/// uncommitted work (#395), None when it has none.
fn snapshot_of(main: &Path, dir: &Path, now: i64) -> Option<String> {
    if dir == main {
        return None;
    }
    let feature = breadboard_core::git::feature_of(main, dir);
    let newest = breadboard_core::snapshot::newest(main, &feature)?;
    Some(snapshot_line(
        &feature,
        &newest.time,
        &breadboard_core::time::ago(newest.at, now),
    ))
}

/// `snapshot 20261016T142233Z (3 h ago): pnpm wt:snapshot restore 7-x`.
pub fn snapshot_line(feature: &str, time: &str, ago: &str) -> String {
    format!(
        "snapshot {time} ({ago}): {} wt:snapshot restore {feature}",
        breadboard_core::config::run_word()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_newest_snapshot_and_how_to_restore_it() {
        let line = snapshot_line("7-x", "20261016T142233Z", "3 h ago");
        assert!(line.starts_with("snapshot 20261016T142233Z (3 h ago): "));
        assert!(line.ends_with(" wt:snapshot restore 7-x"));
    }

    #[test]
    fn picks_as_many_as_there_is_room_for_in_merge_order() {
        let waiting = |session: &str, place, at| Waiting {
            session: session.into(),
            place,
            at,
        };
        let all = vec![
            waiting("back", Some(4), 1),
            waiting("none", None, 0),
            waiting("front", Some(0), 9),
        ];
        assert_eq!(pick(all.clone(), 0), Vec::<String>::new());
        assert_eq!(pick(all.clone(), 1), ["front"]);
        assert_eq!(pick(all, usize::MAX), ["front", "back", "none"]);
    }
}
