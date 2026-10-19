//! The run engine of a repo's headless task agents (#358, core's `runs`):
//! every EVERY it takes the
//! queued lines of the sessions that may run now and runs `claude -p` with
//! them, one run per session at a time, in the session's worktree. Lines
//! queued while a run is in flight wait for the next. A session open in an
//! interactive claude (`~/.claude/sessions`, a live pid that isn't a run's)
//! holds its queue. Runs start while memory allows (`runs::room_for`), in
//! merge order, then by age. One engine per repo across daemons of other
//! builds: `.claude/runs/engine.pid`. A run that stopped at the Claude usage
//! limit holds every run until it resets; then the runs it stopped go on
//! (#393, core's `Usage`). A run that ends on a background command or
//! saying it waits leaves a wake (#417, core's `wake`): its line is queued
//! here once it is due. While the user paused them (#397, `breadboard runs
//! pause`) none starts; a run a pause ended gives its lines back.

use breadboard_core::conversations::{interactive_pids_of, interactive_resumes, running_claudes, transcript_of};
use breadboard_core::lock::PidFile;
use breadboard_core::permissions;
use breadboard_core::runs::{
    self, Event, Queue, Run, STARTING_MS, State, Table, Usage, Waiting, allowed_of, args_of, change_queue,
    change_usage, prompt_of, read_or_default, room_for, status_file, write_whole,
};
use breadboard_core::stage;
use breadboard_core::time::now_ms;
use breadboard_core::wake;
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
    let mut is_wake_pending = false;
    loop {
        if !main.is_dir() {
            busy.store(false, Ordering::SeqCst);
            return;
        }
        if held.is_none() {
            held = PidFile::hold(&runs::dir_of(&main).join("engine.pid"));
        }
        if held.is_some() {
            let (starts, is_waking) = {
                let (main, in_flight, table) = (main.clone(), in_flight.clone(), table.clone());
                tokio::task::spawn_blocking(move || {
                    let is_waking = wake_due(&main, &in_flight, now_ms());
                    (take_starts(&main, &in_flight, &table), is_waking)
                })
                .await
                .unwrap_or_default()
            };
            is_wake_pending = is_waking;
            for start in starts {
                tokio::spawn(run(main.clone(), start, in_flight.clone(), table.clone()));
            }
        }
        // Notices alone start nothing (#445): no reason to stay.
        let is_queued = read_or_default::<Queue>(&runs::queue_file(&main))
            .events
            .iter()
            .any(|event| !event.is_notice);
        // Runs the usage limit stopped wait for its reset here (#393).
        let is_stopped = !read_or_default::<Usage>(&runs::usage_file(&main)).sessions.is_empty();
        busy.store(
            is_queued || is_stopped || is_wake_pending || !locked(&in_flight).is_empty(),
            Ordering::SeqCst,
        );
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
/// session whose worktree is gone loses its lines. Nothing while the usage
/// limit holds; once it reset, the sessions it stopped first get their line.
fn take_starts(main: &Path, in_flight: &InFlight, table: &Mutex<()>) -> Vec<Start> {
    if !release_usage(main, table, now_ms()) {
        return Vec::new();
    }
    let queue: Queue = read_or_default(&runs::queue_file(main));
    if queue.events.is_empty() {
        return Vec::new();
    }
    // Paused (#397), or past the daily limit (#372): the lines wait; the overview says why.
    if runs::paused_of(main).is_some() || runs::held_now(main, now_ms()).is_some() {
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

/// The wakes of the sessions without a run (#417): a due one's line queued,
/// one overtaken dropped (a line queued for its session anyway, a `wt:wait`
/// recorded since it said it waits). Whether any is left to wait for.
fn wake_due(main: &Path, in_flight: &InFlight, now: i64) -> bool {
    let table: Table = read_or_default(&status_file(main));
    let queue: Queue = read_or_default(&runs::queue_file(main));
    let flying = locked(in_flight).clone();
    let idle = table
        .runs
        .iter()
        .filter(|(session, run)| run.state != State::Running && !flying.contains_key(*session));
    let mut is_left = false;
    for (dir, found) in wake::wakes_in(idle.map(|(session, run)| (session.as_str(), run.dir.as_path()))) {
        let is_overtaken = queue
            .events
            .iter()
            .any(|event| event.session == found.session && !event.is_notice)
            || (found.kind == wake::Kind::Itself && breadboard_core::wait::wait_of(&dir).is_some());
        if is_overtaken {
            wake::clear_wake(&dir);
            continue;
        }
        let Some(line) = wake::due_line(&found, now, breadboard_core::lock::is_alive) else {
            is_left = true;
            continue;
        };
        let name = table
            .runs
            .get(&found.session)
            .map(|run| run.name.clone())
            .unwrap_or_default();
        let queued = runs::enqueue(
            main,
            Event {
                session: found.session.clone(),
                dir: dir.clone(),
                name,
                line,
                at: now,
                allowed: Vec::new(),
                is_notice: false,
            },
        );
        match queued {
            Ok(()) => wake::clear_wake(&dir),
            Err(_) => is_left = true,
        }
    }
    is_left
}

/// Whether runs may start at `now` as far as the Claude usage limit goes
/// (#393): false while it holds; once it reset, each session it stopped
/// gets `runs::RESET_LINE` ahead of the lines queued for it meanwhile.
fn release_usage(main: &Path, table: &Mutex<()>, now: i64) -> bool {
    let usage: Usage = read_or_default(&runs::usage_file(main));
    if usage.holds(now) {
        return false;
    }
    if !usage.sessions.is_empty() {
        let _ = change_usage(main, |usage| {
            change_queue(main, |queue| {
                change_table(main, table, |read| {
                    runs::release(usage, read, queue, now);
                });
            })
        });
    }
    true
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

/// The command of a run: `claude <args>` in `dir`, no pane's (#386), so
/// herdr never takes the headless session for a pane's agent and resumes it
/// in a tab after a restart.
fn run_command(dir: &Path, args: Vec<String>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(CLAUDE);
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    for name in breadboard_core::herdr::PANE_ENV {
        command.env_remove(name);
    }
    command
}

/// One run of `start`: `claude -p` in its worktree, its stream into its
/// log, its row of the table `running` until it ends, then `idle`, `asks`
/// when a tool was denied, `limit` when it stopped at the usage limit.
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
    // A wake of a run before is overtaken by this one (#417).
    wake::clear_wake(&dir);
    let mut args = args_of(
        &session,
        &name,
        &prompt,
        is_known,
        &allowed_of(&events),
        &runs::lean_flags_of(&main),
    );
    // The model of its kind of work (#373): a run of failed pipelines only is a CI fix.
    let stage = stage::of_run(&dir, events.iter().map(|event| event.line.as_str()));
    args.extend(stage::model_args(&breadboard_core::config::of(&main).config, stage));
    let mut command = run_command(&dir, args);
    // What it leaves running is found by this (`wake::leftover_of`).
    command.env(wake::RUN_ENV, &session);
    let spawned = command.spawn();
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
    // Ended by `pause --now` (#397): its lines come again after the resume, nothing lost.
    if code != Some(0) && runs::paused_of(&main).is_some() {
        let mut again = events.clone();
        if let Some(first) = events.first() {
            again.insert(
                0,
                Event {
                    line: runs::ENDED_BY_PAUSE.into(),
                    allowed: Vec::new(),
                    is_notice: false,
                    ..first.clone()
                },
            );
        }
        let _ = change_queue(&main, |queue| {
            again.append(&mut queue.events);
            queue.events = again;
        });
    }
    let now = now_ms();
    let hit = runs::hit_of(&lines, now);
    let denials = asked_of(&dir, &name, &lines);
    let state = match hit {
        Some(_) => State::Limit,
        None if denials.is_empty() => State::Idle,
        None => State::Asks,
    };
    // A run the usage limit stopped goes on at its reset (#393): nothing to wake.
    if hit.is_none() {
        let left = {
            let session = session.clone();
            tokio::task::spawn_blocking(move || wake::leftover_of(&session))
                .await
                .unwrap_or_default()
        };
        if let Some(found) = wake::wake_of(&lines, &prompt, &session, &dir, now, &left) {
            let _ = wake::write_wake(&dir, &found);
        }
    }
    change_table(&main, &table, |read| {
        read.runs.insert(session.clone(), row(state, None, code, denials));
    });
    // Held until the reset, its session in line for a run then (#393); a run that went through ends a backoff.
    let _ = change_usage(&main, |usage| match hit {
        Some(hit) => runs::on_hit(usage, &session, hit, now),
        None => usage.tries = 0,
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

/// `breadboard runs [tell [--notice] <dir> <line…> | continue | pause [--now] [--ci] |
/// resume]`: the status table of the repo's headless agents with what they
/// spent today, a line queued for the agent of the worktree `dir`, the daily
/// limit lifted until midnight, or every run paused (#397: the runs in
/// flight let finish, or ended with `--now`; the CI queue off too with
/// `--ci`) and resumed.
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
            println!(
                "today {}{} · {}",
                today.total.text(),
                breadboard_core::spend::wakes_text(&today),
                breadboard_core::spend::BASIS
            );
            match runs::paused_now(&main) {
                Some(text) => println!("{text} (breadboard runs resume)"),
                None => println!("agents run (breadboard runs pause stops them)"),
            }
            if let Some(why) = runs::held_now(&main, now) {
                println!("{why}: no run starts (breadboard runs continue lifts it until midnight)");
            }
            if let Some(why) = runs::usage_now(&main, now) {
                println!("{why}: no run starts, the stopped runs go on after it");
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
                if let Some(line) = held_line(&queue, &session, &run.dir, now, interactive_pids_of(&session)) {
                    println!("  {line}");
                }
                if let Some(line) = snapshot_of(&main, &run.dir, now) {
                    println!("  {line}");
                }
            }
            ExitCode::SUCCESS
        }
        Some("release") if args.len() == 2 => {
            let dir = PathBuf::from(&args[1]);
            let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
            match crate::tui::overview::actor::end_held::end(&main, &dir) {
                Ok(did) => {
                    println!("{did}");
                    ExitCode::SUCCESS
                }
                Err(why) => {
                    eprintln!("breadboard runs: {why}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("tell") if args.len() >= 3 && !(args[1] == "--notice" && args.len() < 4) => {
            // `--notice`: it starts no run, it rides along with the next (#445).
            let is_notice = args[1] == "--notice";
            let rest = &args[usize::from(is_notice) + 1..];
            let dir = PathBuf::from(&rest[0]);
            let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
            let line = rest[1..].join(" ");
            let herdr = |line: &str| {
                if is_notice {
                    crate::tui::overview::actor::herdr::enqueue_notice(&main, &dir, line)
                } else {
                    crate::tui::overview::actor::herdr::enqueue(&main, &dir, line)
                }
            };
            if herdr(&line) {
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
        Some("pause") if args[1..].iter().all(|arg| arg == "--now" || arg == "--ci") => {
            let has = |flag: &str| args[1..].iter().any(|arg| arg == flag);
            let paused = match runs::pause(&main, now_ms(), has("--ci")) {
                Ok(paused) => paused,
                Err(error) => {
                    eprintln!("breadboard runs: {error}");
                    return ExitCode::FAILURE;
                }
            };
            if has("--now") {
                let ended = runs::end_running(&main);
                println!("{ended} runs ended: their lines wait for the resume");
            }
            if paused.ci {
                println!("the CI queue is off (git config breadboard.ci push) until the resume");
            }
            println!(
                "{}: no run starts until breadboard runs resume",
                runs::paused_now(&main).unwrap_or_default()
            );
            ExitCode::SUCCESS
        }
        Some("resume") if args.len() == 1 => match runs::resume(&main) {
            Ok(None) => {
                println!("the agents weren't paused");
                ExitCode::SUCCESS
            }
            Ok(Some(paused)) => {
                crate::daemon::client::wake_runs(&main);
                if paused.ci {
                    println!("the CI queue is on again");
                }
                println!("the agents run again: the lines that waited start their runs");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("breadboard runs: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!(
                "usage: breadboard runs [tell [--notice] <dir> <line…> | release <dir> | continue | pause [--now] [--ci] | resume]"
            );
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

/// What `breadboard runs` says under an agent whose lines an open claude
/// (`pids`) holds past ten minutes (#386), None for any other.
fn held_line(queue: &Queue, session: &str, dir: &Path, now: i64, pids: Vec<i64>) -> Option<String> {
    let oldest = queue
        .events
        .iter()
        .filter(|event| event.session == session)
        .map(|event| event.at)
        .min()?;
    if pids.is_empty() || now - oldest < runs::HELD_MS {
        return None;
    }
    let pids: Vec<String> = pids.iter().map(i64::to_string).collect();
    Some(format!(
        "held by an open claude (pid {}) for {} min: breadboard runs release {} ends it once it is idle",
        pids.join(", "),
        (now - oldest) / 60_000,
        dir.display()
    ))
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
    fn a_queue_held_by_an_open_claude_says_so_after_ten_minutes() {
        let event = |at| Event {
            session: "s".into(),
            dir: PathBuf::from("/w/1-x"),
            name: "1-x".into(),
            line: "go".into(),
            at,
            allowed: Vec::new(),
            is_notice: false,
        };
        let queue = Queue {
            events: vec![event(0), event(5 * 60_000)],
        };
        let at = |now| held_line(&queue, "s", Path::new("/w/1-x"), now, vec![42]);
        assert_eq!(at(runs::HELD_MS - 1), None);
        assert_eq!(
            at(23 * 60_000).as_deref(),
            Some("held by an open claude (pid 42) for 23 min: breadboard runs release /w/1-x ends it once it is idle")
        );
        // No open claude: the run starts as usual.
        assert_eq!(held_line(&queue, "s", Path::new("/w/1-x"), 23 * 60_000, vec![]), None);
        assert_eq!(
            held_line(&queue, "other", Path::new("/w/1-x"), 23 * 60_000, vec![42]),
            None
        );
    }

    #[test]
    fn a_run_is_no_panes_agent() {
        let command = run_command(Path::new("/w/1-x"), vec!["-p".into(), "go".into()]);
        let removed: Vec<_> = command
            .as_std()
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(removed, ["HERDR_PANE_ID", "HERDR_TAB_ID"]);
        assert_eq!(command.as_std().get_current_dir(), Some(Path::new("/w/1-x")));
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
