//! What the agents cost (#372): the `usage` of every answer in claude's
//! transcripts, as the context share is read (`conversations`), priced per
//! model. claude writes one line per part of an answer, each with the whole
//! answer's usage: an answer counts once, by its message id. A subagent's
//! answers (`<session>/subagents/*.jsonl`) count for the session that started
//! it. A transcript is read once and then only what was appended to it.
//!
//! - `today_of`: the repo's agents today (local midnight on), all and per session;
//! - `of_session`: one session's, between two times (an issue's share of it);
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

/// What a model costs per million tokens, in dollars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub read: f64,
}

impl Price {
    /// `usage` at this price: a 5-minute cache write is 1.25 times the input,
    /// a 1-hour one twice.
    pub fn of(&self, usage: &Usage) -> f64 {
        (usage.input as f64 * self.input
            + usage.write_5m as f64 * self.input * 1.25
            + usage.write_1h as f64 * self.input * 2.0
            + usage.read as f64 * self.read
            + usage.output as f64 * self.output)
            / 1_000_000.0
    }
}

/// The price of a model, by its id (`claude-opus-5-5`, `claude-sonnet-4-6-20260101`):
/// Anthropic's list prices of 2026-09; an unknown model at Opus 5.5's.
pub fn price_of(model: &str) -> Price {
    let price = |input: f64, output: f64, read: f64| Price { input, output, read };
    let model = model.trim_start_matches("claude-");
    let is = |prefix: &str| model.starts_with(prefix);
    if is("fable") || is("mythos") {
        price(10.0, 50.0, if is("fable-5-1") || is("mythos-5-1") { 0.25 } else { 1.0 })
    } else if is("opus-5-5") {
        price(4.0, 20.0, 0.2)
    // Opus 4 and 4.1 (`opus-4-20250514`, `opus-4-1-…`), Opus 3.
    } else if is("opus-4-1") || is("opus-4-0") || model == "opus-4" || is("opus-4-2025") || is("3-opus") {
        price(15.0, 75.0, 1.5)
    } else if is("opus") {
        price(5.0, 25.0, 0.5)
    } else if is("sonnet-5") {
        price(2.0, 10.0, 0.2)
    } else if is("sonnet") || is("3-7-sonnet") || is("3-5-sonnet") {
        price(3.0, 15.0, 0.3)
    } else if is("haiku-4") {
        price(1.0, 5.0, 0.1)
    } else if is("haiku") || is("3-5-haiku") {
        price(0.8, 4.0, 0.08)
    } else {
        price(4.0, 20.0, 0.2)
    }
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
            format!("{:.1} M", millions)
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
    let mut spend = Spend::default();
    for file in files_of_session(id) {
        for answer in answers_of_file(&file) {
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
/// worktrees' (`<main>-<feature>`, removed ones included).
pub fn projects_of(main: &Path) -> Vec<PathBuf> {
    let name = project_name(main);
    let prefix = format!("{name}-");
    fs::read_dir(claude_dir().join("projects"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    let found = entry.file_name().to_string_lossy().into_owned();
                    found == name || found.starts_with(&prefix)
                })
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
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
        assert_eq!(Limit::Tokens(20_000_000).against(&spend), "4.2 M of 20.0 M tokens");
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
    fn names_projects_as_claude_does() {
        assert_eq!(
            project_name(Path::new("/home/a.b/Repositories/platform")),
            "-home-a-b-Repositories-platform"
        );
    }
}
