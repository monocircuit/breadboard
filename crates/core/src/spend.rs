//! What the agents cost (#372): the `usage` of every answer in claude's
//! transcripts, as the context share is read (`conversations`), priced per
//! model. claude writes one line per part of an answer, each with the whole
//! answer's usage: an answer counts once, by its message id. A subagent's
//! answers (`<session>/subagents/*.jsonl`) count for the session that started
//! it. A transcript is read once and then only what was appended to it.
//!
//! - `today_of`: the repo's agents today (local midnight on), all and per session;
//! - `of_session`: one session's, between two times (an issue's share of it);
//! - `spent_of`: an orchestration's, as its overview shows them: today's,
//!   each issue's (its agents' answers while they worked on it, and its
//!   worktree's try checks, #387), its own;
//! - `Limit`: `[agents] daily_limit`, in tokens or dollars.
//!
//! Tokens are what an answer read and wrote anew: its input, what it wrote
//! into the cache and its output. What it read from the cache is priced, but
//! not counted as tokens: it is the same context read again on every answer,
//! and would make every number a hundred times the work.

use crate::conversations::claude_dir;
use crate::json::str_of;
use crate::time::parse_iso;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The tokens of one answer, or a sum of them.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Usage {
    pub input: u64,
    /// Written into the 5-minute cache, and into the 1-hour cache.
    pub write_5m: u64,
    pub write_1h: u64,
    /// Read from the cache.
    pub read: u64,
    pub output: u64,
}

impl Usage {
    /// The tokens it counts: everything but the cache reads.
    pub fn tokens(&self) -> u64 {
        self.input + self.write_5m + self.write_1h + self.output
    }

    fn of(usage: &Value) -> Usage {
        let int = |value: Option<&Value>| value.and_then(Value::as_u64).unwrap_or(0);
        let written = int(usage.get("cache_creation_input_tokens"));
        let split = usage.get("cache_creation");
        let write_1h = int(split.and_then(|split| split.get("ephemeral_1h_input_tokens")));
        Usage {
            input: int(usage.get("input_tokens")),
            // Without the split (older claudes) every write was a 5-minute one.
            write_5m: written.saturating_sub(write_1h),
            write_1h,
            read: int(usage.get("cache_read_input_tokens")),
            output: int(usage.get("output_tokens")),
        }
    }
}

/// What a model costs per million tokens, in dollars: its input, its
/// output, a write into the 5-minute and into the 1-hour cache, a cache read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub write_5m: f64,
    pub write_1h: f64,
    pub read: f64,
}

impl Price {
    /// `usage` at this price.
    pub fn of(&self, usage: &Usage) -> f64 {
        (usage.input as f64 * self.input
            + usage.write_5m as f64 * self.write_5m
            + usage.write_1h as f64 * self.write_1h
            + usage.read as f64 * self.read
            + usage.output as f64 * self.output)
            / 1_000_000.0
    }
}

/// What the dollars are: what the API bills, not what a subscription pays.
pub const BASIS: &str = "list prices";

/// A row of `PRICES`: a cache write is 1.25 times the input (5 minutes) or
/// twice it (1 hour).
const fn list(input: f64, output: f64, read: f64) -> Price {
    Price {
        input,
        output,
        write_5m: input * 1.25,
        write_1h: input * 2.0,
        read,
    }
}

const OPUS_5_5: Price = list(4.0, 20.0, 0.2);

/// Anthropic's list prices of 2026-09, by the start of a model's id without
/// `claude-` (`$` at the end: the whole id), the first that fits.
const PRICES: &[(&[&str], Price)] = &[
    (&["fable-5-1", "mythos-5-1"], list(10.0, 50.0, 0.25)),
    (&["fable", "mythos"], list(10.0, 50.0, 1.0)),
    (&["opus-5-5"], OPUS_5_5),
    // Opus 4 and 4.1 (`opus-4-20250514`, `opus-4-1-…`), Opus 3.
    (
        &["opus-4-1", "opus-4-0", "opus-4$", "opus-4-2025", "3-opus"],
        list(15.0, 75.0, 1.5),
    ),
    (&["opus"], list(5.0, 25.0, 0.5)),
    (&["sonnet-5"], list(2.0, 10.0, 0.2)),
    (&["sonnet", "3-7-sonnet", "3-5-sonnet"], list(3.0, 15.0, 0.3)),
    (&["haiku-4"], list(1.0, 5.0, 0.1)),
    (&["haiku", "3-5-haiku"], list(0.8, 4.0, 0.08)),
];

/// The price of a model, by its id (`claude-opus-5-5`, `claude-sonnet-4-6-20260101`)
/// from `PRICES`; an unknown model at Opus 5.5's.
pub fn price_of(model: &str) -> Price {
    let model = model.trim_start_matches("claude-");
    let fits = |key: &&str| match key.strip_suffix('$') {
        Some(whole) => model == whole,
        None => model.starts_with(key),
    };
    PRICES
        .iter()
        .find(|(keys, _)| keys.iter().any(fits))
        .map_or(OPUS_5_5, |(_, price)| *price)
}

/// One answer: when, what it used, what that cost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Answer {
    pub at: i64,
    pub usage: Usage,
    pub dollars: f64,
}

/// A sum of answers: their tokens and dollars.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Spend {
    pub tokens: u64,
    pub dollars: f64,
}

impl Spend {
    pub fn add(&mut self, other: Spend) {
        self.tokens += other.tokens;
        self.dollars += other.dollars;
    }

    pub fn is_empty(&self) -> bool {
        self.tokens == 0 && self.dollars == 0.0
    }

    fn of(answer: &Answer) -> Spend {
        Spend {
            tokens: answer.usage.tokens(),
            dollars: answer.dollars,
        }
    }

    /// `4.2 M tokens · ≈ $12`
    pub fn text(&self) -> String {
        format!("{} tokens · ≈ {}", tokens_text(self.tokens), dollars_text(self.dollars))
    }
}

/// `4.2 M`, `830 k`, `0`.
pub fn tokens_text(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        let millions = tokens as f64 / 1_000_000.0;
        if millions >= 100.0 {
            format!("{} M", millions.round() as u64)
        } else {
            format!("{} M", format!("{millions:.1}").trim_end_matches(".0"))
        }
    } else if tokens >= 1000 {
        format!("{} k", (tokens as f64 / 1000.0).round() as u64)
    } else {
        tokens.to_string()
    }
}

/// `$12`, `$4.20`, `$0.03`: cents below $10.
pub fn dollars_text(dollars: f64) -> String {
    if dollars >= 10.0 {
        format!("${}", dollars.round() as u64)
    } else {
        format!("${dollars:.2}")
    }
}

/// The answers in `lines` (a transcript's, oldest first) not in `seen` yet,
/// each once by its message id, which goes into `seen`. A synthetic answer
/// (claude's own, no model ran) costs nothing.
pub fn answers_in<'a>(lines: impl IntoIterator<Item = &'a str>, seen: &mut HashSet<String>) -> Vec<Answer> {
    let mut answers = Vec::new();
    for line in lines {
        if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if str_of(&entry, "type") != Some("assistant") {
            continue;
        }
        let Some(message) = entry.get("message") else { continue };
        let Some(usage) = message.get("usage").filter(|usage| usage.is_object()) else {
            continue;
        };
        let model = str_of(message, "model").unwrap_or_default();
        if model == "<synthetic>" {
            continue;
        }
        let id = str_of(message, "id")
            .or_else(|| str_of(&entry, "requestId"))
            .or_else(|| str_of(&entry, "uuid"))
            .unwrap_or_default();
        if !id.is_empty() && !seen.insert(id.to_string()) {
            continue;
        }
        let Some(at) = str_of(&entry, "timestamp").and_then(parse_iso) else {
            continue;
        };
        let usage = Usage::of(usage);
        answers.push(Answer {
            at,
            usage,
            dollars: price_of(model).of(&usage),
        });
    }
    answers
}

/// A transcript as read so far: up to where, the answers in it.
#[derive(Default)]
struct Followed {
    read: u64,
    seen: HashSet<String>,
    answers: Vec<Answer>,
}

static FOLLOWED: Mutex<Option<HashMap<PathBuf, Followed>>> = Mutex::new(None);

/// The answers of the transcript `file`: what was appended since the last
/// call is read, a file that got shorter (written anew) from its start.
fn answers_of_file(file: &Path) -> Vec<Answer> {
    let Ok(mut guard) = FOLLOWED.lock() else {
        return Vec::new();
    };
    let all = guard.get_or_insert_with(HashMap::new);
    let followed = all.entry(file.to_path_buf()).or_default();
    let Ok(mut opened) = fs::File::open(file) else {
        return followed.answers.clone();
    };
    let size = opened.metadata().map(|meta| meta.len()).unwrap_or(0);
    if size < followed.read {
        *followed = Followed::default();
    }
    if size > followed.read && opened.seek(SeekFrom::Start(followed.read)).is_ok() {
        let mut buffer = Vec::with_capacity((size - followed.read) as usize);
        if opened.take(size - followed.read).read_to_end(&mut buffer).is_ok() {
            // Only whole lines: one being written is read on the next call.
            if let Some(end) = buffer.iter().rposition(|byte| *byte == b'\n') {
                let text = String::from_utf8_lossy(&buffer[..end]);
                let found = answers_in(text.split('\n'), &mut followed.seen);
                followed.answers.extend(found);
                followed.read += end as u64 + 1;
            }
        }
    }
    followed.answers.clone()
}

/// The transcripts of a session: its own (in whichever project it was
/// started) and its subagents'.
fn files_of_session(id: &str) -> Vec<PathBuf> {
    let Some(own) = crate::conversations::transcript_of(id) else {
        return Vec::new();
    };
    let mut files = subagents_of(&own.with_extension(""));
    files.insert(0, own);
    files
}

/// The subagents' transcripts in the folder `<project>/<session>`.
fn subagents_of(folder: &Path) -> Vec<PathBuf> {
    fs::read_dir(folder.join("subagents"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
                .collect()
        })
        .unwrap_or_default()
}

/// What the session `id` spent from `from` (ms) to before `to`.
pub fn of_session(id: &str, from: i64, to: i64) -> Spend {
    of_files(&files_of_session(id), from, to)
}

/// What the answers in the transcripts `files` spent from `from` to before
/// `to`: each answer's own usage. A resumed session's run is its answers
/// since it started, never the run result's `total_cost_usd` (#444): that
/// adds up the whole session again.
fn of_files(files: &[PathBuf], from: i64, to: i64) -> Spend {
    let mut spend = Spend::default();
    for file in files {
        for answer in answers_of_file(file) {
            if answer.at >= from && answer.at < to {
                spend.add(Spend::of(&answer));
            }
        }
    }
    spend
}

/// The name claude gives the project of a folder: every character but a
/// letter or digit a `-`.
pub fn project_name(dir: &Path) -> String {
    dir.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// claude's projects of the repo of `main`: the main checkout's and its
/// worktrees' (`<main>-<feature>`, removed ones included), each once: a
/// worktree's is mostly a link to the main checkout's (a conversation
/// resumes in any of them), read through it would count it again.
pub fn projects_of(main: &Path) -> Vec<PathBuf> {
    projects_in(&claude_dir().join("projects"), main)
}

/// `projects_of` in claude's folder of projects `folder`.
fn projects_in(folder: &Path, main: &Path) -> Vec<PathBuf> {
    let name = project_name(main);
    let prefix = format!("{name}-");
    let mut projects: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        let found = entry.file_name().to_string_lossy().into_owned();
        if found != name && !found.starts_with(&prefix) {
            continue;
        }
        let Ok(real) = fs::canonicalize(entry.path()) else {
            continue;
        };
        if !projects.contains(&real) {
            projects.push(real);
        }
    }
    projects
}

/// Local midnight before `now` (ms).
pub fn midnight_of(now: i64) -> i64 {
    let seconds = now.div_euclid(1000);
    let Some(tm) = crate::time::local_tm(seconds as libc::time_t) else {
        return now - now.rem_euclid(86_400_000);
    };
    let into = i64::from(tm.tm_hour) * 3600 + i64::from(tm.tm_min) * 60 + i64::from(tm.tm_sec);
    (seconds - into) * 1000
}

/// What the repo's agents spent today: all of them, and per session.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Today {
    /// `2026-10-13`: the day it is of.
    pub day: String,
    pub total: Spend,
    pub by_session: HashMap<String, Spend>,
}

/// What the claudes in the repo of `main` spent since local midnight: every
/// transcript of its projects written to today.
pub fn today_of(main: &Path, now: i64) -> Today {
    let midnight = midnight_of(now);
    let since = UNIX_EPOCH + std::time::Duration::from_millis(midnight.max(0) as u64);
    let is_today = |path: &Path| {
        fs::metadata(path)
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified: SystemTime| modified >= since)
    };
    let mut today = Today {
        day: crate::time::day(now),
        ..Today::default()
    };
    for project in projects_of(main) {
        let Ok(entries) = fs::read_dir(&project) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let (session, files) = if path.extension().is_some_and(|ext| ext == "jsonl") {
                (
                    path.file_stem().map(|stem| stem.to_string_lossy().into_owned()),
                    vec![path],
                )
            } else if path.is_dir() {
                (
                    path.file_name().map(|name| name.to_string_lossy().into_owned()),
                    subagents_of(&path),
                )
            } else {
                continue;
            };
            let Some(session) = session else { continue };
            for file in files.into_iter().filter(|file| is_today(file)) {
                for answer in answers_of_file(&file).iter().filter(|answer| answer.at >= midnight) {
                    let spend = Spend::of(answer);
                    today.total.add(spend);
                    today.by_session.entry(session.clone()).or_default().add(spend);
                }
            }
        }
    }
    today
}

/// The times a worker worked on each of its issues (#372), from the starts
/// of its history: `(issue, from, to)`, `to` the next start's time. Its
/// first issue from the beginning of its conversation; a later start without
/// a time leaves its time with the issue before.
pub fn spans_of(worker: &crate::record::Worker) -> Vec<(i64, i64, i64)> {
    let mut spans: Vec<(i64, i64, i64)> = Vec::new();
    for (i, start) in crate::model::history::history_of(worker).iter().enumerate() {
        let at = match start.at.as_deref().and_then(parse_iso) {
            _ if i == 0 => i64::MIN,
            Some(at) => at,
            None => continue,
        };
        if let Some(last) = spans.last_mut() {
            last.2 = at;
        }
        spans.push((start.number, at, i64::MAX));
    }
    spans
}

/// What the agents of an orchestration's `record` in the repo of `main`
/// spent on each issue: every worker's answers while it worked on it
/// (`spans_of`), and the try checkers' in its worktree (`of_checks`).
pub fn by_issue_of(main: &Path, record: &crate::record::Record) -> HashMap<i64, Spend> {
    let mut by_issue: HashMap<i64, Spend> = HashMap::new();
    for worker in &record.workers {
        for (number, from, to) in spans_of(worker) {
            by_issue
                .entry(number)
                .or_default()
                .add(of_session(&worker.id, from, to));
        }
    }
    for issue in &record.issues {
        let (Some(number), Some(feature)) = (issue.number, &issue.feature) else {
            continue;
        };
        let checks = crate::try_check::checks_of(&crate::git::dir_of(main, feature));
        let spend = of_checks(&checks);
        if !spend.is_empty() {
            by_issue.entry(number).or_default().add(spend);
        }
    }
    by_issue
}

/// What the try checkers of `checks` spent (#387): each one's conversation;
/// one whose transcript is gone by the tokens its check kept, unpriced.
pub fn of_checks(checks: &[crate::try_check::Check]) -> Spend {
    of_checks_by(checks, |session| of_session(session, i64::MIN, i64::MAX))
}

/// `of_checks`, a session's spend read by `of`.
fn of_checks_by(checks: &[crate::try_check::Check], of: impl Fn(&str) -> Spend) -> Spend {
    let mut spend = Spend::default();
    let mut seen = HashSet::new();
    for check in checks {
        let read = match &check.session {
            Some(session) if !seen.insert(session.as_str()) => continue,
            Some(session) => of(session),
            None => Spend::default(),
        };
        spend.add(if read.is_empty() {
            Spend {
                tokens: check.tokens,
                dollars: 0.0,
            }
        } else {
            read
        });
    }
    spend
}

/// What an orchestration's agents spent (#372), as its overview shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spent {
    /// The repo's claudes today, all and per session: the band, AGENTS.
    pub today: Today,
    pub daily_limit: Option<Limit>,
    /// Why no run starts: the daily limit reached (`runs::held_by`).
    pub held: Option<String>,
    /// The Claude usage limit holding the runs (#393): `usage limit until 23:30`.
    pub usage: Option<String>,
    /// Each issue's, over every agent that worked on it.
    pub by_issue: HashMap<i64, Spend>,
    /// The orchestration's: its agents' and its orchestrator's, all time.
    pub total: Spend,
    pub budget: Option<Limit>,
}

/// How long `spent_of` holds before it reads the transcripts again.
const SPENT_FOR_MS: i64 = 10_000;

/// `Spent` of the orchestration `name` (its `record`) in the repo of
/// `main`, read again once SPENT_FOR_MS passed.
pub fn spent_of(main: &Path, name: &str, record: Option<&crate::record::Record>, now: i64) -> Spent {
    type Held = HashMap<(PathBuf, String), (i64, Spent)>;
    static HELD: Mutex<Option<Held>> = Mutex::new(None);
    let key = (main.to_path_buf(), name.to_string());
    // A `continue anyway` shows at once: only the transcripts wait SPENT_FOR_MS.
    let lifted = crate::runs::read_or_default(&crate::runs::limit_file(main));
    if let Some((at, mut spent)) = HELD.lock().ok().and_then(|held| held.as_ref()?.get(&key).cloned())
        && now - at < SPENT_FOR_MS
    {
        spent.held = crate::runs::held_by(spent.daily_limit, &spent.today, &lifted);
        spent.usage = crate::runs::usage_now(main, now);
        return spent;
    }
    let config = crate::config::of_main(main);
    let today = today_of(main, now);
    let mut spent = Spent {
        held: crate::runs::held_by(config.config.agents.daily_limit, &today, &lifted),
        usage: crate::runs::usage_now(main, now),
        today,
        daily_limit: config.config.agents.daily_limit,
        budget: config.config.agents.orchestration_budget,
        ..Spent::default()
    };
    if let Some(record) = record {
        spent.by_issue = by_issue_of(main, record);
        for used in spent.by_issue.values() {
            spent.total.add(*used);
        }
        if let Some(chat) = &record.chat {
            spent.total.add(of_session(&chat.session_id, i64::MIN, i64::MAX));
        }
    }
    if let Ok(mut held) = HELD.lock() {
        held.get_or_insert_with(HashMap::new).insert(key, (now, spent.clone()));
    }
    spent
}

/// The band's line of `spent` (#372): `today 4.2 M tokens · ≈ $12`, with
/// the daily limit (`of $40 a day`), the orchestration's budget
/// (`budget $80 of $200`) and the dollars' basis (`list prices`, #444);
/// None while nothing was spent and no limit is set.
pub fn band_text(spent: &Spent) -> Option<String> {
    let mut text = format!("today {}", spent.today.total.text());
    if let Some(limit) = spent.daily_limit {
        text.push_str(&format!(" of {} a day", limit.text()));
    }
    if let Some(budget) = spent.budget {
        text.push_str(&format!(" · budget {}", budget.against(&spent.total)));
    }
    text.push_str(&format!(" · {BASIS}"));
    (!spent.today.total.is_empty() || spent.daily_limit.is_some() || spent.budget.is_some()).then_some(text)
}

/// A limit in tokens or dollars (`[agents] daily_limit`, `orchestration_budget`):
/// `"20 M"`, `"500k"`, `20000000` tokens; `"$40"` dollars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Limit {
    Tokens(u64),
    Dollars(f64),
}

impl Limit {
    pub fn parse(text: &str) -> Result<Limit, String> {
        let text = text.trim();
        if let Some(dollars) = text.strip_prefix('$') {
            return dollars
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|dollars| *dollars >= 0.0 && dollars.is_finite())
                .map(Limit::Dollars)
                .ok_or_else(|| format!("{text:?} is no amount of dollars (\"$40\")"));
        }
        let lower = text.to_ascii_lowercase();
        let lower = lower.trim_end_matches("tokens").trim();
        let (number, factor) = if let Some(number) = lower.strip_suffix('m') {
            (number, 1_000_000.0)
        } else if let Some(number) = lower.strip_suffix('k') {
            (number, 1000.0)
        } else {
            (lower, 1.0)
        };
        number
            .trim()
            .replace('_', "")
            .parse::<f64>()
            .ok()
            .filter(|count| *count >= 0.0 && count.is_finite())
            .map(|count| Limit::Tokens((count * factor).round() as u64))
            .ok_or_else(|| format!("{text:?} is neither tokens (\"20 M\") nor dollars (\"$40\")"))
    }

    /// Whether `spend` reached it.
    pub fn is_reached(&self, spend: &Spend) -> bool {
        match self {
            Limit::Tokens(tokens) => spend.tokens >= *tokens,
            Limit::Dollars(dollars) => spend.dollars >= *dollars,
        }
    }

    /// `20 M tokens`, `$40`.
    pub fn text(&self) -> String {
        match self {
            Limit::Tokens(tokens) => format!("{} tokens", tokens_text(*tokens)),
            Limit::Dollars(dollars) => dollars_text(*dollars),
        }
    }

    /// `spend` against it: `$12 of $40`, `4.2 M of 20 M tokens`.
    pub fn against(&self, spend: &Spend) -> String {
        match self {
            Limit::Tokens(tokens) => format!("{} of {} tokens", tokens_text(spend.tokens), tokens_text(*tokens)),
            Limit::Dollars(dollars) => format!("{} of {}", dollars_text(spend.dollars), dollars_text(*dollars)),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Limit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Limit, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Count(u64),
            Text(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Count(count) => Ok(Limit::Tokens(count)),
            Raw::Text(text) => Limit::parse(&text).map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"{"type":"assistant","requestId":"req_1","timestamp":"2026-10-13T10:00:00.000Z","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":1000,"cache_read_input_tokens":100000,"output_tokens":500,"cache_creation":{"ephemeral_1h_input_tokens":1000,"ephemeral_5m_input_tokens":0}}}}"#;

    #[test]
    fn counts_an_answer_once_whatever_its_parts() {
        let second_part = ANSWER.replace("10:00:00", "10:00:01");
        let other = ANSWER.replace("msg_1", "msg_2").replace("req_1", "req_2");
        let synthetic = ANSWER
            .replace("msg_1", "msg_3")
            .replace("claude-opus-5-5", "<synthetic>");
        let mut seen = HashSet::new();
        let answers = answers_in(
            [ANSWER, second_part.as_str(), other.as_str(), synthetic.as_str()],
            &mut seen,
        );
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].usage.tokens(), 1502);
        assert_eq!(answers[0].usage.read, 100_000);
        assert_eq!(answers[0].usage.write_1h, 1000);
        // Read on: the same answer again counts no more.
        assert!(answers_in([ANSWER], &mut seen).is_empty());
    }

    #[test]
    fn prices_per_model() {
        let usage = Usage {
            input: 1_000_000,
            write_5m: 1_000_000,
            write_1h: 1_000_000,
            read: 1_000_000,
            output: 1_000_000,
        };
        // 4 + 5 + 8 + 0.2 + 20
        assert!((price_of("claude-opus-5-5").of(&usage) - 37.2).abs() < 1e-9);
        assert_eq!(price_of("claude-sonnet-4-6").input, 3.0);
        assert_eq!(price_of("claude-sonnet-5-5").input, 2.0);
        assert_eq!(price_of("claude-haiku-4-5-20251001").output, 5.0);
        assert_eq!(price_of("claude-fable-5-1").read, 0.25);
        assert_eq!(price_of("claude-opus-4-8").input, 5.0);
        assert_eq!(price_of("something-new").input, 4.0);
        assert_eq!(price_of("claude-opus-4").input, 15.0);
        assert_eq!(price_of("claude-opus-4-20250514").input, 15.0);
        assert_eq!(price_of("claude-3-5-haiku-20241022").input, 0.8);
        assert_eq!(price_of("claude-sonnet-4-6").write_1h, 6.0);
        let older = Usage::of(&serde_json::json!({ "input_tokens": 1, "cache_creation_input_tokens": 10 }));
        assert_eq!((older.write_5m, older.write_1h), (10, 0));
    }

    #[test]
    fn reads_a_limit_in_tokens_or_dollars() {
        assert_eq!(Limit::parse("20 M"), Ok(Limit::Tokens(20_000_000)));
        assert_eq!(Limit::parse("500k"), Ok(Limit::Tokens(500_000)));
        assert_eq!(Limit::parse("1.5M tokens"), Ok(Limit::Tokens(1_500_000)));
        assert_eq!(Limit::parse("$40"), Ok(Limit::Dollars(40.0)));
        assert!(Limit::parse("forty").is_err());
        assert!(Limit::parse("$-1").is_err());
        let spend = Spend {
            tokens: 4_200_000,
            dollars: 12.4,
        };
        assert!(!Limit::Dollars(40.0).is_reached(&spend));
        assert!(Limit::Tokens(4_000_000).is_reached(&spend));
        assert_eq!(Limit::Dollars(40.0).against(&spend), "$12 of $40");
        assert_eq!(Limit::Tokens(20_000_000).against(&spend), "4.2 M of 20 M tokens");
        assert_eq!(spend.text(), "4.2 M tokens · ≈ $12");
        assert_eq!(dollars_text(0.034), "$0.03");
        assert_eq!(tokens_text(830_400), "830 k");
    }

    #[test]
    fn reads_only_what_was_appended() {
        let scratch = crate::lock::scratch::Scratch::new();
        let file = scratch.path().join("t.jsonl");
        fs::write(&file, format!("{ANSWER}\n")).unwrap();
        assert_eq!(answers_of_file(&file).len(), 1);
        let other = ANSWER.replace("msg_1", "msg_2");
        // A line half written is read once it is whole.
        let mut opened = fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut opened, format!("{ANSWER}\n{}", &other[..20]).as_bytes()).unwrap();
        assert_eq!(answers_of_file(&file).len(), 1);
        std::io::Write::write_all(&mut opened, format!("{}\n", &other[20..]).as_bytes()).unwrap();
        assert_eq!(answers_of_file(&file).len(), 2);
    }

    #[test]
    fn a_resumed_sessions_run_costs_only_its_own_answers() {
        let scratch = crate::lock::scratch::Scratch::new();
        let file = scratch.path().join("session.jsonl");
        // The first run's answer; the second, resumed an hour later, sees it
        // in its history and reports both in its `total_cost_usd`.
        let second = ANSWER
            .replace("msg_1", "msg_2")
            .replace("req_1", "req_2")
            .replace("10:00:00", "11:00:00")
            .replace("\"output_tokens\":500", "\"output_tokens\":2000");
        let result = r#"{"type":"result","total_cost_usd":57.0,"modelUsage":{"claude-opus-5-5":{"cacheReadInputTokens":124000000,"costUSD":57.0}}}"#;
        fs::write(&file, format!("{ANSWER}\n{second}\n{result}\n")).unwrap();
        let files = [file];
        let started = parse_iso("2026-10-13T10:30:00Z").unwrap();
        let own = of_files(&files, started, i64::MAX);
        // 1000 1-hour writes at $8, 100 k reads at $0.20, 2 inputs at $4, 2000 outputs at $20.
        let dollars = (1000.0 * 8.0 + 100_000.0 * 0.2 + 2.0 * 4.0 + 2000.0 * 20.0) / 1_000_000.0;
        assert_eq!(own.tokens, 3002);
        assert!((own.dollars - dollars).abs() < 1e-9, "{own:?}");
        // Both runs: each answer once, the result's total nowhere.
        let both = of_files(&files, i64::MIN, i64::MAX);
        assert_eq!(both.tokens, 1502 + 3002);
        assert!(both.dollars < 1.0, "{both:?}");
    }

    #[test]
    fn splits_a_workers_time_by_its_issues() {
        let worker = crate::record::Worker::from_value(
            "a",
            &serde_json::json!({ "history": [
                { "number": 1, "how": "fresh", "at": "2026-10-13T08:00:00Z" },
                { "number": 2, "how": "handover" },
                { "number": 3, "how": "handover", "at": "2026-10-13T12:00:00Z" },
            ] }),
        );
        let noon = parse_iso("2026-10-13T12:00:00Z").unwrap();
        assert_eq!(spans_of(&worker), [(1, i64::MIN, noon), (3, noon, i64::MAX)]);
        let old = crate::record::Worker::from_value("b", &serde_json::json!({ "issues": [7] }));
        assert_eq!(spans_of(&old), [(7, i64::MIN, i64::MAX)]);
    }

    fn check(id: i64, session: Option<&str>, tokens: u64) -> crate::try_check::Check {
        crate::try_check::Check {
            id,
            head: "a".into(),
            model: "haiku".into(),
            state: crate::try_check::State::Done,
            verdict: None,
            evidence: None,
            at: String::new(),
            tokens,
            pid: None,
            session: session.map(String::from),
            is_overturned: false,
        }
    }

    #[test]
    fn counts_a_try_checkers_session_toward_its_issue() {
        let scratch = crate::lock::scratch::Scratch::new();
        let transcript = scratch.path().join("checker.jsonl");
        let haiku = ANSWER.replace("claude-opus-5-5", "claude-haiku-4-5-20251001");
        fs::write(&transcript, format!("{haiku}\n")).unwrap();
        let of = |session: &str| {
            let mut spend = Spend::default();
            if session == "checker" {
                answers_of_file(&transcript)
                    .iter()
                    .for_each(|answer| spend.add(Spend::of(answer)));
            }
            spend
        };
        // Its session priced from its transcript, once however often it checked;
        // one whose transcript is gone by the tokens its check kept.
        let checks = [
            check(1, Some("checker"), 9),
            check(2, Some("checker"), 9),
            check(3, Some("gone"), 40),
        ];
        let spend = of_checks_by(&checks, of);
        assert_eq!(spend.tokens, 1502 + 40);
        // 2 input + 1000 1-hour writes + 100000 reads + 500 output at Haiku 4.5's.
        assert!((spend.dollars - (2.0 + 2000.0 + 10_000.0 + 2500.0) / 1_000_000.0).abs() < 1e-12);

        let main = scratch.path().join("platform");
        let worktree = crate::git::dir_of(&main, "7-own");
        crate::try_check::write_check(&worktree, &check(1, None, 300)).unwrap();
        let record = crate::record::Record::from_value(serde_json::json!({ "issues": [
            { "number": 7, "feature": "7-own" },
            { "number": 8, "feature": "8-unchecked" },
        ] }))
        .unwrap();
        let by_issue = by_issue_of(&main, &record);
        assert_eq!(by_issue.get(&7).map(|spend| spend.tokens), Some(300));
        assert_eq!(by_issue.get(&8), None);
    }

    #[test]
    fn says_in_the_band_what_was_spent_against_the_limits() {
        let mut spent = Spent::default();
        assert_eq!(band_text(&spent), None);
        spent.today.total = Spend {
            tokens: 4_200_000,
            dollars: 12.4,
        };
        assert_eq!(
            band_text(&spent).as_deref(),
            Some("today 4.2 M tokens · ≈ $12 · list prices")
        );
        spent.daily_limit = Some(Limit::Dollars(40.0));
        spent.budget = Some(Limit::Tokens(200_000_000));
        spent.total.tokens = 80_000_000;
        assert_eq!(
            band_text(&spent).as_deref(),
            Some("today 4.2 M tokens · ≈ $12 of $40 a day · budget 80 M of 200 M tokens · list prices")
        );
    }

    #[test]
    fn reads_a_project_linked_to_another_once() {
        let scratch = crate::lock::scratch::Scratch::new();
        let projects = scratch.path().join("projects");
        let main = project_name(Path::new("/r/platform"));
        fs::create_dir_all(projects.join(&main)).unwrap();
        fs::create_dir_all(projects.join(format!("{main}-12-own"))).unwrap();
        std::os::unix::fs::symlink(projects.join(&main), projects.join(format!("{main}-11-linked"))).unwrap();
        fs::create_dir_all(projects.join(format!("{main}x"))).unwrap();
        let mut found: Vec<String> = projects_in(&projects, Path::new("/r/platform"))
            .iter()
            .map(|project| project.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, [main.clone(), format!("{main}-12-own")]);
    }

    #[test]
    fn names_projects_as_claude_does() {
        assert_eq!(
            project_name(Path::new("/home/a.b/Repositories/platform")),
            "-home-a-b-Repositories-platform"
        );
    }
}
