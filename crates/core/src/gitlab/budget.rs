//! GitLab's per-user budget, shared by every process of the user (#462):
//! GitLab.com locks a token that asks too much (a 429 with `retry-after`,
//! for an hour once). One 429 anywhere pauses every caller: the pause is a
//! file in the runtime dir, and nothing asks GitLab before it ends: not the
//! daemon's feeds, not its CI tasks, not a command's glab, not an agent's
//! glab (the pre-bash hook refuses it). Every request is a line of the
//! request log, so who asks how much is measured (`breadboard gitlab:usage`).

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The pause after a 429 that names no end (a glab's: it prints no headers):
/// GitLab's burst window is a minute.
pub const UNKNOWN_MS: i64 = 60_000;
/// The request log is started anew past this size; the one before stays as `.1`.
const LOG_MAX: u64 = 2 << 20;

/// GitLab said 429: nothing is asked of it before `until`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Pause {
    pub until: i64,
    /// When the 429 came.
    pub at: i64,
    /// Which limit (`ratelimit-name`), or who saw it (`glab`).
    pub why: String,
}

impl Pause {
    /// The one line every caller shows: `GitLab's rate limit (…): nothing is asked until 16:45`.
    pub fn text(&self) -> String {
        let limit = if self.why.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.why)
        };
        format!(
            "GitLab's rate limit{limit}: nothing is asked until {} (in {})",
            crate::time::clock(self.until, crate::time::now_ms()),
            crate::time::span((self.until - crate::time::now_ms()).max(0))
        )
    }
}

/// The runtime dir: every process of the user sees it, a gauntlet world has its own.
/// `$BREADBOARD_BUDGET_DIR` instead (the unit tests': none to be had).
fn dir() -> PathBuf {
    match std::env::var_os("BREADBOARD_BUDGET_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => crate::local::runtime_dir(),
    }
}

fn pause_file() -> PathBuf {
    dir().join("gitlab-pause.json")
}

/// The request log, a JSON line per request.
pub fn log_file() -> PathBuf {
    dir().join("gitlab-requests.jsonl")
}

/// The pause in force, None when GitLab may be asked.
pub fn paused() -> Option<Pause> {
    paused_at(crate::time::now_ms())
}

fn paused_at(now: i64) -> Option<Pause> {
    let text = std::fs::read_to_string(pause_file()).ok()?;
    let pause: Pause = serde_json::from_str(&text).ok()?;
    (pause.until > now).then_some(pause)
}

/// GitLab answered 429: every caller pauses for `ms`; a pause already
/// longer stays.
pub fn pause(ms: i64, why: &str) -> Pause {
    let now = crate::time::now_ms();
    let fresh = Pause {
        until: now + ms.max(1_000),
        at: now,
        why: why.to_string(),
    };
    if let Some(known) = paused_at(now).filter(|known| known.until >= fresh.until) {
        return known;
    }
    if crate::local::own_dir(&dir())
        && let Ok(text) = serde_json::to_string(&fresh)
    {
        let _ = crate::json::write_whole(&pause_file(), &text);
    }
    fresh
}

/// How long a 429 asks to wait, in ms: its `retry-after` (seconds), else
/// until its `ratelimit-reset` (epoch seconds), else UNKNOWN_MS.
pub fn wait_of(retry_after: Option<&str>, reset: Option<&str>, now: i64) -> i64 {
    if let Some(seconds) = retry_after.and_then(|value| value.trim().parse::<i64>().ok()) {
        return seconds.max(1) * 1_000;
    }
    if let Some(reset) = reset.and_then(|value| value.trim().parse::<i64>().ok()) {
        let ms = reset * 1_000 - now;
        if ms > 0 {
            return ms;
        }
    }
    UNKNOWN_MS
}

/// Whether what a failed glab printed is GitLab's 429.
pub fn is_rate_limited(said: &str) -> bool {
    said.lines().any(|line| {
        let line = line.trim();
        let line = line.strip_prefix("glab: ").unwrap_or(line);
        line.starts_with("429 ") || line.contains("HTTP 429") || line.to_lowercase().contains("too many requests")
    })
}

/// Who this process is in the request log (`serve`, `overview`, `wt:wait`): set once at its start.
pub fn name_process(name: &str) {
    let _ = PROCESS.set(name.to_string());
}

static PROCESS: OnceLock<String> = OnceLock::new();

/// One line of the request log.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Logged {
    pub at: i64,
    /// The process (`serve`, `overview`, `wt:wait`, `agent`).
    pub by: String,
    /// Its part that asked (`feed orchestration breadboard`, `ci queue`, `glab`).
    pub task: String,
    pub what: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

/// A request asked of GitLab, into the request log.
pub fn note(task: &str, what: &str, status: Option<u16>) {
    let by = PROCESS.get().map(String::as_str).unwrap_or("breadboard");
    note_by(by, task, what, status);
}

pub fn note_by(by: &str, task: &str, what: &str, status: Option<u16>) {
    if !crate::local::own_dir(&dir()) {
        return;
    }
    let line = Logged {
        at: crate::time::now_ms(),
        by: by.to_string(),
        task: task.to_string(),
        what: what.chars().take(160).collect(),
        status,
    };
    let Ok(mut text) = serde_json::to_string(&line) else {
        return;
    };
    text.push('\n');
    let file = log_file();
    if std::fs::metadata(&file).is_ok_and(|meta| meta.len() > LOG_MAX) {
        let _ = std::fs::rename(&file, file.with_extension("jsonl.1"));
    }
    // One write of a short line in append mode: lines of several processes never mix.
    if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        let _ = out.write_all(text.as_bytes());
    }
}

/// A glab call of `task` may run: None, and it is logged; else the pause's
/// line, and it isn't run.
pub fn glab_may(task: &str, args: &[&str]) -> Option<String> {
    if let Some(pause) = paused() {
        return Some(pause.text());
    }
    note(task, &format!("glab {}", args.join(" ")), None);
    None
}

/// What a failed glab printed: a 429 pauses every caller (UNKNOWN_MS, glab
/// prints no `retry-after`); the line to show in its place, None for any other failure.
pub fn glab_failed(said: &str) -> Option<String> {
    is_rate_limited(said).then(|| pause(UNKNOWN_MS, "glab saw HTTP 429").text())
}

/// The request log since `since`, both files.
pub fn logged_since(since: i64) -> Vec<Logged> {
    let file = log_file();
    [file.with_extension("jsonl.1"), file]
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .flat_map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str::<Logged>(line).ok())
                .collect::<Vec<_>>()
        })
        .filter(|logged| logged.at >= since)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_what_gitlab_asks() {
        assert_eq!(wait_of(Some("3350"), Some("9"), 0), 3_350_000);
        assert_eq!(wait_of(None, Some("100"), 40_000), 60_000);
        assert_eq!(wait_of(Some("Thu, 22 Oct"), Some("100"), 90_000), 10_000);
        assert_eq!(wait_of(None, Some("1"), 90_000), UNKNOWN_MS);
        assert_eq!(wait_of(None, None, 0), UNKNOWN_MS);
        assert_eq!(wait_of(Some("0"), None, 0), 1_000);
    }

    #[test]
    fn knows_glabs_429() {
        assert!(is_rate_limited("glab: 429 Too Many Requests (HTTP 429)"));
        assert!(is_rate_limited("glab: 429 Retry later"));
        assert!(!is_rate_limited("glab: 404 Not Found (HTTP 404)"));
        assert!(!is_rate_limited("merge_requests/429: HTTP 404"));
    }
}
