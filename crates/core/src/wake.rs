//! A headless run that ended while nothing will wake its agent (#417): in
//! `claude -p` a background command (Bash `run_in_background`, Monitor)
//! is stopped with the run and its notification never comes, and an agent
//! that ends saying "I'll wait for …" without a `wt:wait` waits for nothing.
//! The run engine reads the run's stream when it ends (`wake_of`), writes the
//! wake into the worktree (`.claude/wake.json`) and queues its line once it
//! is due (`due_line`): a background command's when its processes left
//! behind are gone, at the latest after `DELAY_MS`; a run that says it waits
//! after `DELAY_MS`. The overview shows the second as `waits on itself since
//! 00:25`.

use crate::json::str_of;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// How long a wake waits at the latest: 15 min, `BREADBOARD_WAKE_MS` (the gauntlet's short one).
pub const DELAY_MS: i64 = 15 * 60_000;
/// The variable every process of a run carries: its session (`leftover_of`).
pub const RUN_ENV: &str = "BREADBOARD_RUN";
/// The words that start a background command's line.
pub const BACKGROUND_LINE: &str = "[breadboard] your background command";
/// The line of a run that said it waits.
pub const ITSELF_LINE: &str = "[breadboard] your last run ended saying you wait, but nothing will wake you: no wt:wait is recorded and no background command runs.";

pub fn delay_ms() -> i64 {
    std::env::var("BREADBOARD_WAKE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(DELAY_MS)
}

pub fn wake_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("wake.json")
}

/// Why a wake was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The run ended with background commands running.
    Background,
    /// The run's last answer says it waits, and nothing is recorded that would end it.
    Itself,
}

/// A background command a run ended with: what it ran, how claude said it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub command: String,
    /// `stopped` (claude ends them with the run), `completed`, …; None when nothing was said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// A wake of the session `session`: since when (the run's end, ms), when it is due at the latest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wake {
    pub kind: Kind,
    pub session: String,
    pub since: i64,
    pub due: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<Command>,
    /// The processes of the run left behind: the wake is due once they are gone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pids: Vec<i64>,
}

pub fn wake_of_dir(dir: &Path) -> Option<Wake> {
    serde_json::from_str(&std::fs::read_to_string(wake_file(dir)).ok()?).ok()
}

pub fn write_wake(dir: &Path, wake: &Wake) -> std::io::Result<()> {
    crate::runs::write_whole(&wake_file(dir), wake)
}

pub fn clear_wake(dir: &Path) {
    let _ = std::fs::remove_file(wake_file(dir));
}

/// The background tasks of a run's stream that still ran when its result
/// came (started, no end said before it), each with what it ran and the
/// status claude gave it after the result.
pub fn open_tasks_of(lines: &[String]) -> Vec<Command> {
    let events: Vec<Value> = lines
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let input_of = |id: &str| -> Option<&Value> {
        events
            .iter()
            .filter(|event| str_of(event, "type") == Some("assistant"))
            .filter_map(|event| event.get("message")?.get("content")?.as_array())
            .flatten()
            .find(|part| str_of(part, "type") == Some("tool_use") && str_of(part, "id") == Some(id))?
            .get("input")
    };
    let is_end = |event: &Value| match str_of(event, "subtype") {
        Some("task_notification") => true,
        Some("task_updated") => event
            .get("patch")
            .and_then(|patch| str_of(patch, "status"))
            .is_some_and(|status| status != "running" && status != "pending"),
        _ => false,
    };
    let mut open: Vec<(String, Command)> = Vec::new();
    let mut is_after = false;
    for event in &events {
        if str_of(event, "type") == Some("result") {
            is_after = true;
            continue;
        }
        if str_of(event, "type") != Some("system") {
            continue;
        }
        let Some(task) = str_of(event, "task_id") else { continue };
        match str_of(event, "subtype") {
            Some("task_started") if !is_after => {
                let input = str_of(event, "tool_use_id").and_then(input_of);
                let command = input
                    .and_then(|input| str_of(input, "command").or_else(|| str_of(input, "description")))
                    .or_else(|| str_of(event, "description"))
                    .unwrap_or("a background command");
                open.push((
                    task.to_string(),
                    Command {
                        command: command.to_string(),
                        status: None,
                    },
                ));
            }
            _ if is_end(event) && !is_after => open.retain(|(id, _)| id != task),
            Some("task_notification") => {
                if let Some((_, command)) = open.iter_mut().find(|(id, _)| id == task) {
                    command.status = str_of(event, "status").map(String::from);
                }
            }
            _ => {}
        }
    }
    open.into_iter().map(|(_, command)| command).collect()
}

/// The text of a run's result, None without one.
pub fn result_of(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| str_of(event, "type") == Some("result"))
        .find_map(|event| str_of(&event, "result").map(String::from))
}

static WAITS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(i'?ll|i will|i'?m going to|let me|i'?ll now) (now )?(wait|be waiting)\b|\b(i'?m|i am|now|still) waiting\b|\bwaiting (for|on|until) (the|a|it|its|my)\b|\b(i'?ll|i will) (continue|resume|go on|check back|report back|pick (it|this) up)\b[^.]*\b(once|when|after)\b|\buntil (it|the \w+) (finishes|completes|is done|ends)\b|\b(i'?ll|i will) be notified\b",
    )
    .expect("the pattern of a run that says it waits")
});

/// Whether an answer says its agent waits for something to finish: `I'll
/// wait for the test run to finish`, `waiting on a breadboard build`.
pub fn says_it_waits(text: &str) -> bool {
    WAITS.is_match(&text.replace('’', "'"))
}

/// The processes still alive that a run of `session` left behind: the ones
/// carrying `BREADBOARD_RUN=<session>`, not breadboard itself (a daemon
/// an agent's command started). Linux only (`/proc`); elsewhere none.
pub fn leftover_of(session: &str) -> Vec<i64> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let marker = format!("{RUN_ENV}={session}");
    let own = std::env::current_exe().ok();
    let me = i64::from(std::process::id());
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i64>().ok())
        .filter(|pid| *pid != me)
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/environ"))
                .is_ok_and(|environ| environ.split(|byte| *byte == 0).any(|pair| pair == marker.as_bytes()))
        })
        .filter(|pid| {
            let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
            own.is_none() || exe != own
        })
        .collect()
}

/// What a run that just ended (`lines` its stream, `prompt` its lines) leaves
/// to wake its agent, None when something else will: a background command
/// still running at its end, else an answer that says it waits while the
/// worktree records no `wt:wait` and no open action, and the run wasn't
/// such a wake itself (once in a row: an agent that waits for a line that
/// comes by itself isn't woken every `DELAY_MS`).
pub fn wake_of(lines: &[String], prompt: &str, session: &str, dir: &Path, now: i64, pids: &[i64]) -> Option<Wake> {
    let commands = open_tasks_of(lines);
    let wake = |kind, commands, pids: &[i64]| Wake {
        kind,
        session: session.to_string(),
        since: now,
        due: now + delay_ms(),
        commands,
        pids: pids.to_vec(),
    };
    if !commands.is_empty() {
        return Some(wake(Kind::Background, commands, pids));
    }
    let is_waiting = crate::wait::wait_of(dir).is_some();
    let is_asking = crate::actions::actions_of(&crate::actions::actions_file(dir))
        .iter()
        .any(|action| action.answer.is_none());
    let was_woken = prompt.contains(ITSELF_LINE);
    (!is_waiting && !is_asking && !was_woken && result_of(lines).is_some_and(|text| says_it_waits(&text)))
        .then(|| wake(Kind::Itself, Vec::new(), &[]))
}

/// The line of `wake` once it is due at `now` (its processes gone, or its
/// time up), None while it isn't.
pub fn due_line(wake: &Wake, now: i64, alive: impl Fn(i64) -> bool) -> Option<String> {
    let is_gone = wake.pids.iter().all(|pid| !alive(*pid));
    match wake.kind {
        Kind::Background if is_gone || now >= wake.due => Some(background_line(&wake.commands, is_gone)),
        Kind::Itself if now >= wake.due => Some(format!(
            "{ITSELF_LINE} Go on now: run long commands (wt:check, the gauntlet, builds) in the foreground; to wait for MRs use pnpm wt:wait !N --then \"…\". A pipeline's end or the user's answer comes as a line by itself: then just stop."
        )),
        _ => None,
    }
}

/// `[breadboard] your background command finished: pnpm wt:check (stopped), continue …`
fn background_line(commands: &[Command], is_gone: bool) -> String {
    let named: Vec<String> = commands
        .iter()
        .map(|command| match &command.status {
            Some(status) => format!("{} ({status})", command.command),
            None => command.command.clone(),
        })
        .collect();
    let ended = match (is_gone, commands.len() > 1) {
        (true, false) => " finished",
        (true, true) => "s finished",
        (false, false) => " still runs",
        (false, true) => "s still run",
    };
    format!(
        "{BACKGROUND_LINE}{ended}: {}, continue. A headless run stops its background commands when it ends and their notification never comes: run them again in the foreground; to wait for something external use pnpm wt:wait.",
        named.join(", ")
    )
}

/// The sessions whose wakes are kept by the engine: `dirs` with a wake of their session.
pub fn wakes_in<'a>(dirs: impl IntoIterator<Item = (&'a str, &'a Path)>) -> Vec<(PathBuf, Wake)> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    dirs.into_iter()
        .filter(|(_, dir)| seen.insert(dir.to_path_buf()))
        .filter_map(|(session, dir)| {
            let wake = wake_of_dir(dir)?;
            (wake.session == session).then(|| (dir.to_path_buf(), wake))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stream(events: &[Value]) -> Vec<String> {
        events.iter().map(Value::to_string).collect()
    }

    fn background_run(ended_before: bool) -> Vec<String> {
        let mut events = vec![
            json!({ "type": "system", "subtype": "init" }),
            json!({ "type": "assistant", "message": { "content": [
                { "type": "tool_use", "id": "toolu_1", "name": "Bash",
                  "input": { "command": "pnpm wt:check", "run_in_background": true } } ] } }),
            json!({ "type": "system", "subtype": "task_started", "task_id": "b1", "tool_use_id": "toolu_1",
                    "description": "Run the checks", "task_type": "local_bash" }),
            json!({ "type": "system", "subtype": "task_started", "task_id": "b2", "description": "Watch the log" }),
        ];
        if ended_before {
            events.push(
                json!({ "type": "system", "subtype": "task_notification", "task_id": "b1", "status": "completed" }),
            );
            events.push(json!({ "type": "system", "subtype": "task_updated", "task_id": "b2", "patch": { "status": "killed" } }));
        }
        events.push(
            json!({ "type": "result", "subtype": "success", "result": "I'll wait for the wt:check notification." }),
        );
        events.push(
            json!({ "type": "system", "subtype": "task_updated", "task_id": "b1", "patch": { "status": "killed" } }),
        );
        events.push(json!({ "type": "system", "subtype": "task_notification", "task_id": "b1", "status": "stopped" }));
        stream(&events)
    }

    #[test]
    fn finds_the_background_commands_a_run_ended_with() {
        assert_eq!(
            open_tasks_of(&background_run(false)),
            [
                Command {
                    command: "pnpm wt:check".into(),
                    status: Some("stopped".into()),
                },
                Command {
                    command: "Watch the log".into(),
                    status: None,
                },
            ]
        );
        assert!(open_tasks_of(&background_run(true)).is_empty());
    }

    #[test]
    fn hears_a_run_that_says_it_waits() {
        for text in [
            "Committed. I'll wait for the test run to finish.",
            "I’ll wait for the wt:check notification",
            "Fix committed; waiting on a breadboard build.",
            "I'm waiting for the gauntlet.",
            "I'll continue once the build is done.",
            "I will be notified when it completes.",
        ] {
            assert!(says_it_waits(text), "{text}");
        }
        for text in [
            "Done: pushed, the MR waits for !412.",
            "Pushed. The others wait for the fix in !403.",
            "wt:wait recorded: waits for !12, its line starts my next run.",
        ] {
            assert!(!says_it_waits(text), "{text}");
        }
    }

    #[test]
    fn wakes_a_background_run_and_one_that_says_it_waits_once() {
        let scratch = crate::lock::scratch::Scratch::new();
        let dir = scratch.path();
        let wake = wake_of(&background_run(false), "go", "s1", dir, 1_000, &[7]).unwrap();
        assert_eq!(wake.kind, Kind::Background);
        assert_eq!(
            (wake.since, wake.due, wake.pids.clone()),
            (1_000, 1_000 + DELAY_MS, vec![7])
        );
        let says = stream(&[json!({ "type": "result", "result": "I'll wait for the pipeline." })]);
        let itself = wake_of(&says, "go", "s1", dir, 1_000, &[]).unwrap();
        assert_eq!(itself.kind, Kind::Itself);
        assert!(wake_of(&says, &format!("{ITSELF_LINE} …"), "s1", dir, 1_000, &[]).is_none());
        let done = stream(&[json!({ "type": "result", "result": "Done." })]);
        assert!(wake_of(&done, "go", "s1", dir, 1_000, &[]).is_none());
        // A wt:wait recorded: its line wakes it.
        crate::wait::record_wait(dir, &json!({ "mrs": [12] })).unwrap();
        assert!(wake_of(&says, "go", "s1", dir, 1_000, &[]).is_none());
    }

    #[test]
    fn is_due_once_its_processes_are_gone_or_its_time_is_up() {
        let wake = Wake {
            kind: Kind::Background,
            session: "s1".into(),
            since: 0,
            due: 100,
            commands: vec![Command {
                command: "sleep 9".into(),
                status: None,
            }],
            pids: vec![7],
        };
        assert_eq!(due_line(&wake, 50, |_| true), None);
        assert_eq!(
            due_line(&wake, 50, |_| false).unwrap(),
            "[breadboard] your background command finished: sleep 9, continue. A headless run stops its background commands when it ends and their notification never comes: run them again in the foreground; to wait for something external use pnpm wt:wait."
        );
        assert!(
            due_line(&wake, 100, |_| true)
                .unwrap()
                .contains("command still runs: sleep 9")
        );
        let itself = Wake {
            kind: Kind::Itself,
            pids: Vec::new(),
            commands: Vec::new(),
            ..wake
        };
        assert_eq!(due_line(&itself, 50, |_| false), None);
        assert!(due_line(&itself, 100, |_| false).unwrap().starts_with(ITSELF_LINE));
    }

    #[test]
    fn keeps_a_wake_of_the_session_it_names() {
        let scratch = crate::lock::scratch::Scratch::new();
        let dir = scratch.path();
        let wake = Wake {
            kind: Kind::Itself,
            session: "s1".into(),
            since: 0,
            due: 1,
            commands: Vec::new(),
            pids: Vec::new(),
        };
        write_wake(dir, &wake).unwrap();
        assert_eq!(wakes_in([("s1", dir), ("s1", dir)]), [(dir.to_path_buf(), wake)]);
        assert!(wakes_in([("s2", dir)]).is_empty());
        clear_wake(dir);
        assert!(wakes_in([("s1", dir)]).is_empty());
    }
}
