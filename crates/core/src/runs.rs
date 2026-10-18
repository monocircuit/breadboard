//! Headless task agents (#358): a task agent is a Claude Code session id, not
//! a process. A line for it is queued (`enqueue`), and `breadboard serve`
//! runs `claude -p --resume <id>` in its worktree only when something waits
//! in its queue; nothing runs while it waits. Every task agent runs so
//! (#361); one is opened in a tab only on a click (the overview's
//! `open_agent`), and its queue waits while that claude runs.
//!
//! The files of `<main>/.claude/runs/`:
//! - `queue.json` (`Queue`): the lines waiting for a run, appended by every
//!   process that talks to an agent, taken by the daemon, under its lock;
//! - `status.json` (`Table`): what each session's run does, by the daemon;
//! - `log/<session>.jsonl`: the stream of its last run;
//! - `limit.json` (`Lifted`): the day a `continue anyway` lifted the daily
//!   limit for (#372);
//! - `pause.json` (`Paused`): since when the user paused every run (#397).
//! - `usage.json` (`Usage`): the Claude usage limit a run stopped at, until
//!   when it holds every run and the sessions it stopped (#393).

use crate::herdr::Agent;
use crate::lock::{is_alive, with_dir_lock};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a run needs, roughly (a claude is ~200 MB, its tools more): what
/// the memory model counts for an agent, now only while it runs.
pub const RUN: u64 = crate::model::memory::AGENT;
/// What stays free besides the runs, like `gitlab-runner-autoscale`.
pub const RESERVE: u64 = 6 << 30;
/// How long a run counts as starting: its memory isn't in `MemAvailable` yet.
pub const STARTING_MS: i64 = 15_000;
/// A lock of the queue older than this belongs to a process that died.
const STALE: Duration = Duration::from_secs(10);

/// Whether the agent of the worktree `dir` runs headless: every task agent
/// of an orchestration does (#361). A worktree of your own keeps its claude
/// in a tab.
pub fn is_headless(main: &Path, dir: &Path) -> bool {
    crate::record::is_orchestrated(main, dir)
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

pub fn pause_file(main: &Path) -> PathBuf {
    dir_of(main).join("pause.json")
}

/// The run engine paused (#397): no run starts, lines keep queueing. The
/// file is the state, so it outlives `serve`, herdr and a reboot; no file:
/// the runs go on.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Paused {
    /// Since when (ms).
    pub since: i64,
    /// Whether the pause also switched the CI queue off (`pause --ci`):
    /// `resume` switches it on again, one the user had off stays off.
    pub ci: bool,
}

/// The pause of the repo of `main`, None while the runs go on.
pub fn paused_of(main: &Path) -> Option<Paused> {
    let text = std::fs::read_to_string(pause_file(main)).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_default())
}

/// Every run of `main` paused from `now` on (an earlier pause keeps its
/// time); with `ci` the CI queue too (`git config breadboard.ci push`).
pub fn pause(main: &Path, now: i64, ci: bool) -> std::io::Result<Paused> {
    let mut paused = paused_of(main).unwrap_or(Paused { since: now, ci: false });
    let git = |args: &[&str]| crate::git::git(main, args);
    if ci && git(&["config", "--get", crate::ci::MODE_KEY]).out() != "push" {
        let set = git(&["config", crate::ci::MODE_KEY, "push"]);
        if !set.is_ok {
            return Err(std::io::Error::other(set.stderr.trim().to_string()));
        }
        paused.ci = true;
    }
    write_whole(&pause_file(main), &paused)?;
    Ok(paused)
}

/// The runs of `main` go on: the pause gone, the CI queue on again when
/// the pause switched it off. The pause that was, None when there was none.
pub fn resume(main: &Path) -> std::io::Result<Option<Paused>> {
    let Some(paused) = paused_of(main) else {
        return Ok(None);
    };
    if paused.ci {
        crate::git::git(main, &["config", "--unset", crate::ci::MODE_KEY]);
    }
    match std::fs::remove_file(pause_file(main)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    Ok(Some(paused))
}

/// `agents paused since 13:05 · 3 lines waiting`: the band's words of a pause.
pub fn paused_text(paused: &Paused, waiting: usize) -> String {
    let lines = if waiting == 1 { "line" } else { "lines" };
    let mut text = format!(
        "agents paused since {} · {waiting} {lines} waiting",
        crate::time::hm(paused.since)
    );
    if paused.ci {
        text.push_str(" · CI queue off");
    }
    text
}

/// `paused_text` of the repo of `main` now, None while the runs go on.
pub fn paused_now(main: &Path) -> Option<String> {
    let paused = paused_of(main)?;
    let queue: Queue = read_or_default(&queue_file(main));
    Some(paused_text(&paused, queue.events.len()))
}

/// The runs in flight of `main` ended (`pause --now`): SIGTERM to each
/// live claude of the status table. How many were asked to end. The run
/// engine gives their lines back to the queue (a run ended in a pause).
pub fn end_running(main: &Path) -> usize {
    let table: Table = read_or_default(&status_file(main));
    let mut ended = 0;
    for run in table.runs.values().filter(|run| run.state == State::Running) {
        if let Some(pid) = run.pid.filter(|pid| is_alive(*pid)) {
            // SAFETY: a signal to a pid, no memory involved.
            if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0 {
                ended += 1;
            }
        }
    }
    ended
}

/// The line put before the lines of a run a pause ended: they come again.
pub const ENDED_BY_PAUSE: &str =
    "[overview] your last run was ended by a pause of all agents (breadboard runs pause --now); its lines again:";
pub fn usage_file(main: &Path) -> PathBuf {
    dir_of(main).join("usage.json")
}

/// How long after the reset the stopped runs start: not a minute too early.
pub const AFTER_RESET_MS: i64 = 60_000;
/// AFTER_RESET_MS, or what `BREADBOARD_AFTER_RESET_MS` says (a test's reset that doesn't wait a minute).
fn after_reset_ms() -> i64 {
    std::env::var("BREADBOARD_AFTER_RESET_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(AFTER_RESET_MS)
}
/// Without a reset time in the result: the waits of the tries in a row, the last one repeated.
pub const BACKOFF_MS: [i64; 3] = [15 * 60_000, 30 * 60_000, 60 * 60_000];
/// The line a session stopped by the usage limit gets once it reset.
pub const RESET_LINE: &str = "[breadboard] the usage limit reset: continue where you stopped";

/// The Claude usage limit a run stopped at (#393): no run starts before
/// `until`; then each of `sessions` gets one run with RESET_LINE.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    /// When the limit resets (ms), as the result said or a backoff guesses.
    pub reset: i64,
    /// When the runs start again (ms): the reset + AFTER_RESET_MS, or a backoff's end.
    pub until: i64,
    /// How many limits in a row came without a time: the backoff's step.
    pub tries: usize,
    /// The sessions it stopped, in the order they stopped.
    pub sessions: Vec<String>,
    /// The orchestrations whose chat was told it holds: told again once it reset.
    pub told: Vec<String>,
}

impl Usage {
    /// Whether it holds the runs at `now`.
    pub fn holds(&self, now: i64) -> bool {
        now < self.until
    }

    /// The band's words while it holds: `usage limit until 23:30`.
    pub fn text(&self, now: i64) -> Option<String> {
        self.holds(now)
            .then(|| format!("usage limit until {}", crate::time::clock(self.reset, now)))
    }
}

/// A run's stream ended at the usage limit: when it resets, None when the result names no time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub reset: Option<i64>,
}

/// Whether the run of the stream `lines` stopped at the Claude usage limit
/// (#393): its result an error, a 429 or the limit's words (`You've hit
/// your session limit · resets 11:30pm (Europe/Berlin)`, older claudes
/// `Claude AI usage limit reached|1760572800`), with the reset read from
/// it. A time of day is the local clock's: claude says it in the user's zone.
pub fn hit_of(lines: &[String], now: i64) -> Option<Hit> {
    use regex::Regex;
    use std::sync::LazyLock;
    static WORDS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)hit your .*limit|usage limit|limit reached|rate limit").unwrap());
    static EPOCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\|(\d{10})\b").unwrap());
    static RESETS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\bresets\s+(?:(?P<month>[a-z]{3})[a-z]*\s+(?P<day>\d{1,2}),?\s+(?:at\s+)?)?(?P<hour>\d{1,2})(?::(?P<minute>\d{2}))?\s*(?P<half>am|pm)?",
        )
        .unwrap()
    });
    let result = lines
        .iter()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value.get("type").and_then(Value::as_str) == Some("result"))?;
    if result.get("is_error").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let text = result.get("result").and_then(Value::as_str).unwrap_or_default();
    let is_429 = result.get("api_error_status").and_then(Value::as_i64) == Some(429);
    if !is_429 && !WORDS.is_match(text) {
        return None;
    }
    let from_epoch = EPOCH
        .captures(text)
        .and_then(|found| found[1].parse::<i64>().ok())
        .map(|seconds| seconds * 1000);
    let from_clock = || {
        let found = RESETS.captures(text)?;
        let number = |name: &str| found.name(name).and_then(|part| part.as_str().parse::<i64>().ok());
        let mut hour = number("hour")?;
        if let Some(half) = found.name("half") {
            if !(1..=12).contains(&hour) {
                return None;
            }
            hour = hour % 12
                + if half.as_str().eq_ignore_ascii_case("pm") {
                    12
                } else {
                    0
                };
        }
        const MONTHS: [&str; 12] = [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ];
        let date = match (found.name("month"), number("day")) {
            (Some(month), Some(day)) => {
                let month = month.as_str().to_ascii_lowercase();
                Some((MONTHS.iter().position(|known| *known == month)? as i64 + 1, day))
            }
            _ => None,
        };
        crate::time::next_local(hour, number("minute").unwrap_or(0), date, now)
    };
    Some(Hit {
        reset: from_epoch.or_else(from_clock),
    })
}

/// `usage` after `session`'s run stopped at the limit `hit` at `now`: a
/// reset time holds the runs until a minute after it; without one the
/// runs wait the next step of BACKOFF_MS, unless a limit holds already
/// (runs that stopped together count once).
pub fn on_hit(usage: &mut Usage, session: &str, hit: Hit, now: i64) {
    if !usage.sessions.iter().any(|known| known == session) {
        usage.sessions.push(session.to_string());
    }
    match hit.reset {
        Some(reset) => {
            usage.reset = reset;
            usage.until = reset + after_reset_ms();
            usage.tries = 0;
        }
        None if usage.holds(now) => {}
        None => {
            let wait = BACKOFF_MS[usage.tries.min(BACKOFF_MS.len() - 1)];
            usage.reset = now + wait;
            usage.until = now + wait;
            usage.tries += 1;
        }
    }
}

/// `change` of `usage.json` under its lock: the daemon's runs end in tasks of their own.
pub fn change_usage<T>(main: &Path, change: impl FnOnce(&mut Usage) -> T) -> std::io::Result<T> {
    let file = usage_file(main);
    with_dir_lock(&file, STALE, || {
        let mut usage: Usage = read_or_default(&file);
        let before = usage.clone();
        let result = change(&mut usage);
        if usage != before {
            write_whole(&file, &usage)?;
        }
        Ok(result)
    })?
}

/// Once the limit reset at `now`: each session it stopped gets RESET_LINE
/// ahead of the lines queued for it meanwhile, its row `Queued`; the
/// sessions so released, none while it holds.
pub fn release(usage: &mut Usage, table: &mut Table, queue: &mut Queue, now: i64) -> Vec<String> {
    if usage.holds(now) || usage.sessions.is_empty() {
        return Vec::new();
    }
    let sessions = std::mem::take(&mut usage.sessions);
    let mut lines = Vec::new();
    for session in &sessions {
        let Some(run) = table.runs.get_mut(session) else {
            continue;
        };
        lines.push(Event {
            session: session.clone(),
            dir: run.dir.clone(),
            name: run.name.clone(),
            line: RESET_LINE.into(),
            at: now,
            allowed: Vec::new(),
        });
        run.state = State::Queued;
        run.since = now;
    }
    lines.append(&mut queue.events);
    queue.events = lines;
    sessions
}

/// The usage limit's words for the band at `now`, None while it doesn't hold.
pub fn usage_now(main: &Path, now: i64) -> Option<String> {
    read_or_default::<Usage>(&usage_file(main)).text(now)
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
    /// Its last run stopped at the Claude usage limit: it runs again once that resets (#393).
    Limit,
}

impl State {
    /// The word herdr would say of the agent, for `agent_state_of` and its callers.
    pub fn status(self) -> &'static str {
        match self {
            State::Running => "working",
            State::Idle => "idle",
            State::Asks => "blocked",
            State::Queued => "queued",
            State::Limit => "limit",
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

/// What of the user's own settings (`~/.claude/settings.json`) a lean run
/// keeps (#445): the model and effort it would have in a tab, and what
/// auto mode and the permissions go by. Its hooks, plugins and skills not.
const KEPT_SETTINGS: [&str; 5] = ["model", "effortLevel", "autoMode", "permissions", "env"];

/// The flags that keep a run's first call lean (#445): only the MCP servers
/// of `mcp` (`[agents] mcp_configs`, none by default) with
/// `--strict-mcp-config`, only the repo's settings and skills
/// (`--setting-sources project,local`: no user-level skill like a deck of
/// flash cards), and of the user's settings what a tab's run would go by
/// (`KEPT_SETTINGS`) given back with `--settings`. `user`: the user's
/// settings as read, None without them.
pub fn lean_flags(mcp: &[PathBuf], user: Option<&Value>) -> Vec<String> {
    let mut flags: Vec<String> = vec!["--strict-mcp-config".into()];
    if !mcp.is_empty() {
        flags.push("--mcp-config".into());
        flags.extend(mcp.iter().map(|file| file.display().to_string()));
    }
    flags.extend(["--setting-sources".into(), "project,local".into()]);
    let kept: serde_json::Map<String, Value> = KEPT_SETTINGS
        .iter()
        .filter_map(|key| Some((key.to_string(), user?.get(*key)?.clone())))
        .collect();
    if !kept.is_empty() {
        flags.extend(["--settings".into(), Value::Object(kept).to_string()]);
    }
    flags
}

/// `lean_flags` of the repo of `main`: its `[agents] mcp_configs` (relative
/// to the main checkout), the user's settings from claude's config directory.
pub fn lean_flags_of(main: &Path) -> Vec<String> {
    let mcp: Vec<PathBuf> = crate::config::of_main(main)
        .config
        .agents
        .mcp_configs
        .iter()
        .map(|file| main.join(file))
        .collect();
    let user = crate::json::read(&crate::conversations::claude_dir().join("settings.json"));
    lean_flags(&mcp, user.as_ref())
}

/// The command line of a run of `session` (claude's arguments): `--resume`
/// a conversation that has a transcript, else `--session-id` starts it. The
/// prompt comes right after `-p`: the tool lists take every word up to the
/// next flag, a prompt after them would be one more tool. `lean`: the
/// flags of a lean start (`lean_flags_of`, #445).
pub fn args_of(
    session: &str,
    name: &str,
    prompt: &str,
    is_known: bool,
    allowed: &[String],
    lean: &[String],
) -> Vec<String> {
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
    args.extend(lean.iter().cloned());
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

/// How long an agent's lines may wait on a claude open in a tab before the
/// overview names it (#386): such a claude holds the queue, and one herdr
/// resumed by itself after a restart is no one's to end.
pub const HELD_MS: i64 = 10 * 60_000;

/// herdr's word for an agent whose queue an open claude holds past `HELD_MS`.
pub const HELD: &str = "held";

/// `agents` (the run engine's, then herdr's panes over them) with an agent
/// open in a pane `held` when a line for it waits past `HELD_MS` (#386).
pub fn mark_held(agents: &mut HashMap<String, Agent>, queue: &Queue, now: i64) {
    for (session, agent) in agents.iter_mut() {
        let oldest = queue
            .events
            .iter()
            .filter(|event| event.session == *session)
            .map(|event| event.at)
            .min();
        if !agent.pane.is_empty() && oldest.is_some_and(|at| now - at >= HELD_MS) {
            agent.status = Some(HELD.into());
        }
    }
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

    #[test]
    fn an_open_claude_holding_lines_past_ten_minutes_is_named() {
        let agent = |pane: &str| Agent {
            pane: pane.into(),
            tab: String::new(),
            status: Some("idle".into()),
        };
        let mut agents = HashMap::from([
            ("open".to_string(), agent("w1:p2")),
            ("headless".to_string(), agent("")),
            ("fresh".to_string(), agent("w1:p3")),
        ]);
        let queue = Queue {
            events: vec![
                event("open", "a", 0),
                event("headless", "b", 0),
                event("fresh", "c", HELD_MS),
            ],
        };
        mark_held(&mut agents, &queue, HELD_MS);
        let status = |session: &str| agents[session].status.clone().unwrap();
        assert_eq!(status("open"), HELD);
        // No pane: no claude holds it, its run starts as usual.
        assert_eq!(status("headless"), "idle");
        // Its line waits less than ten minutes.
        assert_eq!(status("fresh"), "idle");
    }

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

    fn result(fields: Value) -> Vec<String> {
        let mut result = serde_json::json!({ "type": "result", "subtype": "success" });
        for (key, value) in fields.as_object().unwrap() {
            result[key] = value.clone();
        }
        vec![r#"{"type":"system"}"#.into(), result.to_string()]
    }

    #[test]
    fn reads_the_usage_limit_and_its_reset_from_the_result() {
        let now = crate::time::parse_iso("2026-10-14T20:40:00Z").unwrap();
        let session = result(serde_json::json!({
            "is_error": true,
            "api_error_status": 429,
            "result": "You've hit your session limit · resets 11:30pm (Europe/Berlin)",
        }));
        let reset = hit_of(&session, now).unwrap().reset.unwrap();
        assert_eq!(crate::time::hm(reset), "23:30");
        assert!(reset > now && reset <= now + 86_400_000);
        let at = |text: &str| {
            hit_of(&result(serde_json::json!({ "is_error": true, "result": text })), now)
                .map(|hit| hit.reset.map(crate::time::hm))
        };
        assert_eq!(at("5-hour limit reached ∙ resets 3am"), Some(Some("03:00".into())));
        assert_eq!(
            at("You've hit your weekly limit · resets Oct 16, 11am"),
            Some(Some("11:00".into()))
        );
        assert_eq!(
            at("You've hit your session limit · resets 13:05"),
            Some(Some("13:05".into()))
        );
        assert_eq!(
            hit_of(
                &result(serde_json::json!({ "is_error": true, "result": "Claude AI usage limit reached|1760572800" })),
                now
            ),
            Some(Hit {
                reset: Some(1_760_572_800_000)
            })
        );
        // No time in it: the backoff decides.
        assert_eq!(at("You've hit your session limit"), Some(None));
        assert_eq!(
            hit_of(
                &result(serde_json::json!({ "is_error": true, "api_error_status": 429, "result": "API Error" })),
                now
            ),
            Some(Hit { reset: None })
        );
        // An answer that talks about the limit, another error: no limit.
        assert_eq!(
            hit_of(
                &result(serde_json::json!({ "is_error": false, "result": "You've hit your session limit" })),
                now
            ),
            None
        );
        assert_eq!(at("API Error: 500 overloaded"), None);
        assert_eq!(hit_of(&[], now), None);
    }

    #[test]
    fn holds_the_runs_until_a_minute_after_the_reset_or_backs_off() {
        let mut usage = Usage::default();
        assert!(!usage.holds(0));
        on_hit(&mut usage, "a", Hit { reset: Some(10_000) }, 0);
        on_hit(&mut usage, "b", Hit { reset: Some(10_000) }, 5);
        on_hit(&mut usage, "a", Hit { reset: Some(10_000) }, 6);
        assert_eq!(usage.sessions, ["a", "b"]);
        assert!(usage.holds(10_000 + AFTER_RESET_MS - 1));
        assert!(!usage.holds(10_000 + AFTER_RESET_MS));
        // Without a time: 15 min, 30 min, then an hour each; together they count once.
        let mut usage = Usage::default();
        on_hit(&mut usage, "a", Hit { reset: None }, 0);
        on_hit(&mut usage, "b", Hit { reset: None }, 1);
        assert_eq!((usage.until, usage.tries), (BACKOFF_MS[0], 1));
        let mut now = usage.until;
        for wait in [BACKOFF_MS[1], BACKOFF_MS[2], BACKOFF_MS[2]] {
            on_hit(&mut usage, "a", Hit { reset: None }, now);
            assert_eq!(usage.until, now + wait);
            now = usage.until;
        }
        on_hit(&mut usage, "a", Hit { reset: Some(now + 5) }, now);
        assert_eq!(usage.tries, 0);
    }

    #[test]
    fn runs_the_stopped_sessions_once_it_reset_with_the_lines_queued_meanwhile() {
        let mut usage = Usage::default();
        on_hit(&mut usage, "a", Hit { reset: Some(1_000) }, 0);
        let mut table = Table::default();
        table.runs.insert(
            "a".into(),
            Run {
                dir: PathBuf::from("/r/p-1"),
                name: "1-x".into(),
                state: State::Limit,
                since: 0,
                pid: None,
                lines: 1,
                denials: Vec::new(),
                code: Some(0),
            },
        );
        let mut queue = Queue {
            events: vec![event("b", "other", 2), event("a", "a merge", 3)],
        };
        let states = states_of(&table, &queue);
        assert_eq!(states["a"].state, State::Limit, "a line meanwhile ran it");
        assert_eq!(release(&mut usage, &mut table, &mut queue, 1_000), Vec::<String>::new());
        let at = 1_000 + AFTER_RESET_MS;
        assert_eq!(
            usage.text(at - 1).as_deref(),
            Some(&*format!("usage limit until {}", crate::time::clock(1_000, at)))
        );
        assert_eq!(release(&mut usage, &mut table, &mut queue, at), ["a"]);
        assert_eq!(usage.text(at), None);
        assert_eq!(table.runs["a"].state, State::Queued);
        let mine: Vec<Event> = queue
            .events
            .iter()
            .filter(|event| event.session == "a")
            .cloned()
            .collect();
        assert_eq!(prompt_of(&mine), format!("{RESET_LINE}\n\na merge"));
        // Once only.
        assert_eq!(
            release(&mut usage, &mut table, &mut queue, at + 1),
            Vec::<String>::new()
        );
        assert_eq!(State::Limit.status(), "limit");
    }

    #[test]
    fn pauses_until_resumed_and_says_so() {
        let scratch = crate::lock::scratch::Scratch::new();
        let main = scratch.path();
        assert_eq!(paused_of(main), None);
        assert_eq!(paused_now(main), None);
        let first = pause(main, 1_000, false).unwrap();
        assert_eq!(
            first,
            Paused {
                since: 1_000,
                ci: false
            }
        );
        // A second pause keeps its time.
        assert_eq!(pause(main, 9_000, false).unwrap().since, 1_000);
        enqueue(main, event("a", "one", 1)).unwrap();
        enqueue(main, event("b", "two", 2)).unwrap();
        let text = paused_now(main).unwrap();
        assert!(text.starts_with("agents paused since "), "{text}");
        assert!(text.ends_with(" · 2 lines waiting"), "{text}");
        assert!(paused_text(&first, 1).ends_with("1 line waiting"));
        assert!(paused_text(&Paused { since: 0, ci: true }, 0).ends_with("· CI queue off"));
        assert_eq!(resume(main).unwrap(), Some(first));
        assert_eq!(paused_of(main), None);
        assert_eq!(resume(main).unwrap(), None);
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
        let args = args_of("abc", "1-x", "go", true, &[], &[]);
        assert_eq!(&args[..4], ["-p", "go", "--resume", "abc"]);
        assert!(args.windows(2).any(|pair| pair == ["--permission-mode", "auto"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--disallowedTools", "AskUserQuestion"])
        );
        assert!(!args.contains(&"--allowedTools".to_string()));
        assert_eq!(
            &args_of("abc", "1-x", "go", false, &[], &[])[2..4],
            ["--session-id", "abc"]
        );
        assert_eq!(args_of("abc", "1-x", "-x", true, &[], &[])[1], " -x");
        let rules = ["Bash(git push:*)".to_string(), "WebFetch".to_string()];
        let allowing = args_of("abc", "1-x", "go", true, &rules, &[]);
        let at = allowing.iter().position(|arg| arg == "--allowedTools").unwrap();
        assert_eq!(&allowing[at + 1..at + 3], rules);
    }

    #[test]
    fn a_run_starts_lean_with_the_users_model_and_auto_mode_kept() {
        let user = serde_json::json!({
            "model": "opus[1m]",
            "effortLevel": "medium",
            "autoMode": { "environment": ["a"] },
            "hooks": { "SessionStart": [] },
            "voice": { "enabled": true },
        });
        let flags = lean_flags(&[], Some(&user));
        assert_eq!(
            &flags[..3],
            ["--strict-mcp-config", "--setting-sources", "project,local"]
        );
        assert_eq!(flags[3], "--settings");
        let kept: Value = serde_json::from_str(&flags[4]).unwrap();
        assert_eq!(
            kept,
            serde_json::json!({ "model": "opus[1m]", "effortLevel": "medium", "autoMode": { "environment": ["a"] } })
        );
        // None by default; the ones the config names.
        assert!(!flags.contains(&"--mcp-config".to_string()));
        let named = lean_flags(&[PathBuf::from("/r/p/.mcp.agents.json")], None);
        assert_eq!(
            named,
            [
                "--strict-mcp-config",
                "--mcp-config",
                "/r/p/.mcp.agents.json",
                "--setting-sources",
                "project,local"
            ]
        );
        let args = args_of("abc", "1-x", "go", true, &[], &named);
        let at = args.iter().position(|arg| arg == "--strict-mcp-config").unwrap();
        assert_eq!(args[at - 2..at], ["--name", "1-x"]);
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
