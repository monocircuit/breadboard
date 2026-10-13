//! Headless task agents (#358): a task agent is a Claude Code session id, not
//! a process. A line for it is queued (`enqueue`), and `breadboard serve`
//! runs `claude -p --resume <id>` in its worktree only when something waits
//! in its queue; nothing runs while it waits. Behind a switch, `git config
//! breadboard.agents headless` (`is_headless`); interactive tabs stay the
//! default until #361.
//!
//! The files of `<main>/.claude/runs/`:
//! - `queue.json` (`Queue`): the lines waiting for a run, appended by every
//!   process that talks to an agent, taken by the daemon, under its lock;
//! - `status.json` (`Table`): what each session's run does, by the daemon;
//! - `log/<session>.jsonl`: the stream of its last run;
//! - `limit.json` (`Lifted`): the day a `continue anyway` lifted the daily
//!   limit for (#372).

use crate::herdr::Agent;
use crate::lock::{is_alive, with_dir_lock};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `git config breadboard.agents headless`: task agents run headless.
pub const MODE_KEY: &str = "breadboard.agents";
/// What a run needs, roughly (a claude is ~200 MB, its tools more): what
/// the memory model counts for an agent, now only while it runs.
pub const RUN: u64 = crate::model::memory::AGENT;
/// What stays free besides the runs, like `gitlab-runner-autoscale`.
pub const RESERVE: u64 = 6 << 30;
/// How long a run counts as starting: its memory isn't in `MemAvailable` yet.
pub const STARTING_MS: i64 = 15_000;
/// A lock of the queue older than this belongs to a process that died.
const STALE: Duration = Duration::from_secs(10);

/// Whether `main` runs its task agents headless.
pub fn is_headless(main: &Path) -> bool {
    crate::git::git(main, &["config", "--get", MODE_KEY]).out() == "headless"
}

pub fn dir_of(main: &Path) -> PathBuf {
    main.join(".claude").join("runs")
}

pub fn queue_file(main: &Path) -> PathBuf {
    dir_of(main).join("queue.json")
}

pub fn status_file(main: &Path) -> PathBuf {
    dir_of(main).join("status.json")
}

pub fn limit_file(main: &Path) -> PathBuf {
    dir_of(main).join("limit.json")
}

/// The daily limit lifted (#372): for which day (`2026-10-13`), None: not lifted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Lifted {
    pub day: Option<String>,
}

/// The daily limit lifted for `day`, until midnight: `continue anyway`.
pub fn lift_limit(main: &Path, day: &str) -> std::io::Result<()> {
    write_whole(&limit_file(main), &Lifted { day: Some(day.into()) })
}

/// Why no run starts (#372): the daily limit `limit` (`[agents]
/// daily_limit`) reached by `today`, not lifted for its day.
/// `daily limit reached: $41 of $40`
pub fn held_by(limit: Option<crate::spend::Limit>, today: &crate::spend::Today, lifted: &Lifted) -> Option<String> {
    let limit = limit?;
    (limit.is_reached(&today.total) && lifted.day.as_deref() != Some(today.day.as_str()))
        .then(|| format!("daily limit reached: {}", limit.against(&today.total)))
}

/// `held_by` of the repo of `main` now: its config, its agents' spend today, its `limit.json`.
pub fn held_now(main: &Path, now: i64) -> Option<String> {
    let limit = crate::config::of_main(main).config.agents.daily_limit?;
    let today = crate::spend::today_of(main, now);
    held_by(Some(limit), &today, &read_or_default(&limit_file(main)))
}

pub fn log_file(main: &Path, session: &str) -> PathBuf {
    dir_of(main).join("log").join(format!("{session}.jsonl"))
}

/// A line for an agent: its session, the worktree it stands in now and its
/// name there (a handover moves both), when it was queued (ms).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub session: String,
    pub dir: PathBuf,
    pub name: String,
    pub line: String,
    pub at: i64,
    /// Rules allowed for the one run that takes this line (`--allowedTools`,
    /// #360: the user's `allow once`), none for later runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
}

/// The lines waiting for a run, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Queue {
    pub events: Vec<Event>,
}

/// What a session's run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// A run in flight.
    Running,
    /// No run, nothing queued.
    Idle,
    /// Its last run was denied a tool: it waits for the user (#360: a `permission` action).
    Asks,
    /// Lines wait, no run yet (no memory, an interactive claude holds it).
    Queued,
}

impl State {
    /// The word herdr would say of the agent, for `agent_state_of` and its callers.
    pub fn status(self) -> &'static str {
        match self {
            State::Running => "working",
            State::Idle => "idle",
            State::Asks => "blocked",
            State::Queued => "queued",
        }
    }
}

/// A tool a run was denied (`permission_denials` of the stream's result).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Denial {
    pub tool: String,
    #[serde(default)]
    pub input: Value,
}

/// A session's row of the status table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub dir: PathBuf,
    pub name: String,
    pub state: State,
    /// Since when (ms).
    pub since: i64,
    /// The claude of the run in flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i64>,
    /// How many lines it was given.
    #[serde(default)]
    pub lines: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denials: Vec<Denial>,
    /// How its last run ended: its exit code, None while it runs or when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
}

/// The status table, by session.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Table {
    pub runs: BTreeMap<String, Run>,
}

pub fn read_or_default<T: Default + serde::de::DeserializeOwned>(file: &Path) -> T {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// `value` into `file` whole: written beside it, then renamed, so a reader never sees half.
pub fn write_whole<T: Serialize>(file: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    let temporary = file.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, format!("{text}\n"))?;
    std::fs::rename(&temporary, file)
}

/// `change` of the queue under its lock: the overviews, the CLI and the daemon change it from processes of their own.
pub fn change_queue<T>(main: &Path, change: impl FnOnce(&mut Queue) -> T) -> std::io::Result<T> {
    let file = queue_file(main);
    with_dir_lock(&file, STALE, || {
        let mut queue: Queue = read_or_default(&file);
        let before = queue.clone();
        let result = change(&mut queue);
        if queue != before {
            write_whole(&file, &queue)?;
        }
        Ok(result)
    })?
}

/// A line queued for the agent `session` in `dir`.
pub fn enqueue(main: &Path, event: Event) -> std::io::Result<()> {
    change_queue(main, |queue| queue.events.push(event))
}

/// A line queued for the agent of the worktree `dir`: its session the one
/// bound there (`.claude/herdr-session`), its name the worktree's feature.
/// NotFound when no session is bound. The daemon is woken by the caller
/// (`daemon::client::wake_runs`).
pub fn enqueue_line(main: &Path, dir: &Path, line: &str) -> std::io::Result<()> {
    enqueue_allowing(main, dir, line, Vec::new())
}

/// `enqueue_line` with tool rules allowed for the run that takes it (#360).
pub fn enqueue_allowing(main: &Path, dir: &Path, line: &str, allowed: Vec<String>) -> std::io::Result<()> {
    let session = crate::conversations::bound_session(dir)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no session bound to the worktree"))?;
    enqueue(
        main,
        Event {
            session,
            dir: dir.to_path_buf(),
            name: crate::git::feature_of(main, dir),
            line: line.to_string(),
            at: crate::time::now_ms(),
            allowed,
        },
    )
}

/// The lines of one run, in the order they came; a line that came twice in
/// a row (a retry of the same tell) once.
pub fn prompt_of(events: &[Event]) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for event in events {
        if lines.last() != Some(&event.line.as_str()) {
            lines.push(&event.line);
        }
    }
    lines.join("\n\n")
}

/// How many runs may start now: `MemAvailable` (`available`) less the
/// reserve and what the runs started within STARTING_MS will still take, a
/// RUN each. Unknown memory (not Linux) holds no run back.
pub fn room_for(available: Option<u64>, starting: usize) -> usize {
    let Some(available) = available else {
        return usize::MAX;
    };
    let left = available.saturating_sub(RESERVE + RUN * starting as u64);
    (left / RUN) as usize
}

/// The memory free for runs: `MemAvailable` of `/proc/meminfo`, or of the
/// file `BREADBOARD_MEMINFO` names (a test's low memory).
pub fn available() -> Option<u64> {
    match std::env::var_os("BREADBOARD_MEMINFO") {
        Some(file) => crate::model::memory::available_in(&std::fs::read_to_string(file).ok()?),
        None => crate::model::memory::free_memory(),
    }
}

/// A worktree's place in its orchestration's merge order (`wt:plan`, the
/// plan's `merge`): its issue's index there, None when it has none.
pub fn place_of(main: &Path, dir: &Path) -> Option<usize> {
    let origin = crate::record::origin_of(dir)?;
    let plan = crate::record::record_of(main, &origin.name)?.plan?;
    let branch = crate::git::git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .out()
        .to_string();
    let number = crate::gitlab::number_of(&branch)?;
    plan.merge.iter().position(|planned| *planned == number)
}

/// A session waiting for a run: when its oldest line came, its place in the merge order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub session: String,
    pub place: Option<usize>,
    pub at: i64,
}

/// Which waiting sessions start first: by merge order (a session without a
/// place after those with one), then by age.
pub fn ordered(mut waiting: Vec<Waiting>) -> Vec<Waiting> {
    waiting.sort_by_key(|waiting| (waiting.place.is_none(), waiting.place, waiting.at));
    waiting
}

/// The sessions with lines waiting, each once, with its oldest line's time.
pub fn waiting_in(queue: &Queue) -> Vec<(String, PathBuf, i64)> {
    let mut found: Vec<(String, PathBuf, i64)> = Vec::new();
    for event in &queue.events {
        if !found.iter().any(|(session, ..)| *session == event.session) {
            found.push((event.session.clone(), event.dir.clone(), event.at));
        }
    }
    found
}

/// The rules the lines of one run allow, each once, in the order they came.
pub fn allowed_of(events: &[Event]) -> Vec<String> {
    let mut allowed: Vec<String> = Vec::new();
    for rule in events.iter().flat_map(|event| &event.allowed) {
        if !allowed.contains(rule) {
            allowed.push(rule.clone());
        }
    }
    allowed
}

/// The command line of a run of `session` (claude's arguments): `--resume`
/// a conversation that has a transcript, else `--session-id` starts it. The
/// prompt comes right after `-p`: the tool lists take every word up to the
/// next flag, a prompt after them would be one more tool.
pub fn args_of(session: &str, name: &str, prompt: &str, is_known: bool, allowed: &[String]) -> Vec<String> {
    // A prompt starting with `-` would read as a flag.
    let prompt = if prompt.starts_with('-') {
        format!(" {prompt}")
    } else {
        prompt.to_string()
    };
    let mut args: Vec<String> = vec!["-p".into(), prompt];
    args.extend([
        if is_known { "--resume" } else { "--session-id" }.into(),
        session.into(),
    ]);
    args.extend(["--name".into(), name.into()]);
    // Auto mode as in a tab, what would ask denied, the user's allows of this run (#360).
    args.extend(crate::permissions::run_flags(allowed));
    // `-p` with stream-json needs `--verbose`.
    args.extend(["--output-format".into(), "stream-json".into(), "--verbose".into()]);
    args
}

/// What each session's run does now: the table, a run whose claude is gone
/// idle, a session with lines waiting and no run `Queued`.
pub fn states_of(table: &Table, queue: &Queue) -> HashMap<String, Run> {
    let mut states: HashMap<String, Run> = table
        .runs
        .iter()
        .map(|(session, run)| {
            let mut run = run.clone();
            if run.state == State::Running && !run.pid.is_some_and(is_alive) {
                run.state = State::Idle;
                run.pid = None;
            }
            (session.clone(), run)
        })
        .collect();
    for event in &queue.events {
        let run = states.entry(event.session.clone()).or_insert_with(|| Run {
            dir: event.dir.clone(),
            name: event.name.clone(),
            state: State::Queued,
            since: event.at,
            pid: None,
            lines: 0,
            denials: Vec::new(),
            code: None,
        });
        if run.state == State::Idle {
            run.state = State::Queued;
            run.since = event.at;
        }
    }
    states
}

/// The headless agents as herdr's agents, by session (`reader`'s `agents`):
/// every known session counts as running an agent, its status the table's word.
pub fn agents_of(main: &Path) -> HashMap<String, Agent> {
    let table: Table = read_or_default(&status_file(main));
    let queue: Queue = read_or_default(&queue_file(main));
    states_of(&table, &queue)
        .into_iter()
        .map(|(session, run)| {
            (
                session,
                Agent {
                    pane: String::new(),
                    tab: String::new(),
                    status: Some(run.state.status().into()),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(session: &str, line: &str, at: i64) -> Event {
        Event {
            session: session.into(),
            dir: PathBuf::from("/r/p-1"),
            name: "1-x".into(),
            line: line.into(),
            at,
            allowed: Vec::new(),
        }
    }

    #[test]
    fn holds_the_runs_past_the_daily_limit_unless_lifted_for_the_day() {
        use crate::spend::{Limit, Spend, Today};
        let today = Today {
            day: "2026-10-13".into(),
            total: Spend {
                tokens: 1,
                dollars: 41.0,
            },
            ..Today::default()
        };
        let none = Lifted::default();
        assert_eq!(held_by(None, &today, &none), None);
        assert_eq!(held_by(Some(Limit::Dollars(50.0)), &today, &none), None);
        assert_eq!(
            held_by(Some(Limit::Dollars(40.0)), &today, &none).as_deref(),
            Some("daily limit reached: $41 of $40")
        );
        let lifted = |day: &str| Lifted { day: Some(day.into()) };
        assert_eq!(held_by(Some(Limit::Dollars(40.0)), &today, &lifted("2026-10-13")), None);
        assert!(held_by(Some(Limit::Dollars(40.0)), &today, &lifted("2026-10-12")).is_some());
        let scratch = crate::lock::scratch::Scratch::new();
        lift_limit(scratch.path(), "2026-10-13").unwrap();
        let read: Lifted = read_or_default(&limit_file(scratch.path()));
        assert_eq!(read, lifted("2026-10-13"));
    }

    #[test]
    fn batches_the_lines_of_a_run() {
        let events = [event("a", "one", 1), event("a", "two", 2), event("a", "two", 3)];
        assert_eq!(prompt_of(&events), "one\n\ntwo");
    }

    #[test]
    fn allows_the_rules_of_its_lines_for_one_run() {
        let mut first = event("a", "one", 1);
        first.allowed = vec!["Bash(x)".into()];
        let mut second = event("a", "two", 2);
        second.allowed = vec!["Bash(x)".into(), "WebFetch".into()];
        assert_eq!(
            allowed_of(&[first, second, event("a", "three", 3)]),
            ["Bash(x)", "WebFetch"]
        );
    }

    #[test]
    fn leaves_room_for_the_reserve_and_the_runs_starting() {
        assert_eq!(room_for(None, 3), usize::MAX);
        assert_eq!(room_for(Some(RESERVE), 0), 0);
        assert_eq!(room_for(Some(RESERVE + 3 * RUN), 0), 3);
        assert_eq!(room_for(Some(RESERVE + 3 * RUN), 2), 1);
        assert_eq!(room_for(Some(1 << 30), 0), 0);
    }

    #[test]
    fn starts_in_merge_order_then_by_age() {
        let waiting = |session: &str, place, at| Waiting {
            session: session.into(),
            place,
            at,
        };
        let order = ordered(vec![
            waiting("late", None, 1),
            waiting("second", Some(1), 5),
            waiting("first", Some(0), 9),
            waiting("later", None, 2),
        ]);
        let sessions: Vec<&str> = order.iter().map(|waiting| waiting.session.as_str()).collect();
        assert_eq!(sessions, ["first", "second", "late", "later"]);
    }

    #[test]
    fn resumes_a_known_conversation_and_starts_a_new_one() {
        let args = args_of("abc", "1-x", "go", true, &[]);
        assert_eq!(&args[..4], ["-p", "go", "--resume", "abc"]);
        assert!(args.windows(2).any(|pair| pair == ["--permission-mode", "auto"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--disallowedTools", "AskUserQuestion"])
        );
        assert!(!args.contains(&"--allowedTools".to_string()));
        assert_eq!(&args_of("abc", "1-x", "go", false, &[])[2..4], ["--session-id", "abc"]);
        assert_eq!(args_of("abc", "1-x", "-x", true, &[])[1], " -x");
        let rules = ["Bash(git push:*)".to_string(), "WebFetch".to_string()];
        let allowing = args_of("abc", "1-x", "go", true, &rules);
        let at = allowing.iter().position(|arg| arg == "--allowedTools").unwrap();
        assert_eq!(&allowing[at + 1..at + 3], rules);
    }

    #[test]
    fn says_what_each_session_does() {
        let run = |state, pid| Run {
            dir: PathBuf::from("/r/p-1"),
            name: "1-x".into(),
            state,
            since: 0,
            pid,
            lines: 1,
            denials: Vec::new(),
            code: None,
        };
        let mut table = Table::default();
        table.runs.insert(
            "running".into(),
            run(State::Running, Some(i64::from(std::process::id() as i32))),
        );
        table
            .runs
            .insert("gone".into(), run(State::Running, Some(i64::from(i32::MAX))));
        table.runs.insert("idle".into(), run(State::Idle, None));
        table.runs.insert("asks".into(), run(State::Asks, None));
        let queue = Queue {
            events: vec![event("idle", "x", 5), event("new", "y", 6), event("running", "z", 7)],
        };
        let states = states_of(&table, &queue);
        let state = |session: &str| states[session].state;
        assert_eq!(state("running"), State::Running);
        assert_eq!(state("gone"), State::Idle);
        assert_eq!(state("idle"), State::Queued);
        assert_eq!(state("new"), State::Queued);
        assert_eq!(state("asks"), State::Asks);
        assert_eq!(State::Asks.status(), "blocked");
    }

    #[test]
    fn queues_under_its_lock() {
        let scratch = crate::lock::scratch::Scratch::new();
        let main = scratch.path();
        enqueue(main, event("a", "one", 1)).unwrap();
        enqueue(main, event("a", "two", 2)).unwrap();
        let taken = change_queue(main, |queue| std::mem::take(&mut queue.events)).unwrap();
        assert_eq!(taken.len(), 2);
        let left: Queue = read_or_default(&queue_file(main));
        assert!(left.events.is_empty());
    }
}
