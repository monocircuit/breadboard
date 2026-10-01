//! The claude conversations of the worktrees (conversations.mjs, claude.mjs):
//! the one bound to a worktree's space (`.claude/herdr-session`), its
//! transcript under claude's `projects/`, the last prompt and answer and the
//! context that answer had, and the claudes running on this machine.

use crate::json::{self, str_of};
use crate::lock::is_alive;
use crate::record::is_session_id;
use crate::time::parse_iso;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

/// How much of a transcript's end is read: the last answer is in there.
const TAIL_BYTES: u64 = 1 << 19;
/// How far back the last prompt is looked for.
const PROMPT_BYTES: u64 = 1 << 23;
/// How much of its end a pending tool call is looked for in.
const PENDING_BYTES: u64 = 1 << 16;

/// Where the conversation bound to a worktree's space is written down.
pub fn session_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("herdr-session")
}

/// The conversation bound to a worktree's space, its id or None.
pub fn bound_session(dir: &Path) -> Option<String> {
    let id = fs::read_to_string(session_file(dir)).ok()?.trim().to_string();
    is_session_id(&id).then_some(id)
}

/// claude's config directory: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude"))
}

/// The transcript of a conversation, in whichever project it was started.
pub fn transcript_of(id: &str) -> Option<PathBuf> {
    let projects = claude_dir().join("projects");
    fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(format!("{id}.jsonl")))
        .find(|file| file.exists())
}

/// The last `max` bytes of a file as lines, the first one dropped when it was cut.
fn tail_lines(file: &Path, max: u64) -> Option<(Vec<String>, u64)> {
    let mut opened = fs::File::open(file).ok()?;
    let size = opened.metadata().ok()?.len();
    let length = size.min(max);
    opened.seek(SeekFrom::Start(size - length)).ok()?;
    let mut buffer = Vec::with_capacity(length as usize);
    opened.take(length).read_to_end(&mut buffer).ok()?;
    let text = String::from_utf8_lossy(&buffer);
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    if length < size && !lines.is_empty() {
        lines.remove(0);
    }
    Some((lines, size))
}

/// The text of a message: a string, or its `text` parts.
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|part| str_of(part, "type") == Some("text"))
            .filter_map(|part| str_of(part, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The last text of `kind` (`user`: a prompt someone typed; `assistant`: an
/// answer) in `lines`, with its time; a harness prompt (`<command-…>`) is none.
pub fn last_entry_of(lines: &[String], kind: &str) -> Option<(String, Option<i64>)> {
    let needle = format!("\"type\":\"{kind}\"");
    for line in lines.iter().rev() {
        if !line.contains(&needle) || (kind == "user" && line.contains("\"tool_result\"")) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if str_of(&entry, "type") != Some(kind)
            || json::is_truthy(entry.get("isSidechain"))
            || json::is_truthy(entry.get("isMeta"))
        {
            continue;
        }
        let text = text_of(entry.get("message").and_then(|message| message.get("content")))
            .trim()
            .to_string();
        if !text.is_empty() && !(kind == "user" && text.starts_with('<')) {
            return Some((text, str_of(&entry, "timestamp").and_then(parse_iso)));
        }
    }
    None
}

/// The context of the last answer in `lines` (oldest first): `(model, at,
/// used, window)`, `used` the prompt it read (cached or not) plus what it wrote.
pub fn usage_of(lines: &[String]) -> Usage {
    let last = lines
        .iter()
        .rev()
        .filter(|line| line.contains("\"usage\""))
        .find_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            let message = entry.get("message")?;
            let is_answer = str_of(&entry, "type") == Some("assistant")
                && !json::is_truthy(entry.get("isSidechain"))
                && message.get("usage").is_some_and(|usage| !usage.is_null())
                && str_of(message, "model") != Some("<synthetic>");
            is_answer.then_some(entry)
        });
    let used = last
        .as_ref()
        .and_then(|entry| entry.get("message")?.get("usage").cloned())
        .map(|usage| {
            [
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
                "output_tokens",
            ]
            .iter()
            .map(|key| usage.get(key).and_then(Value::as_u64).unwrap_or(0))
            .sum()
        })
        .unwrap_or(0);
    Usage {
        model: last
            .as_ref()
            .and_then(|entry| str_of(entry.get("message")?, "model").map(String::from)),
        at: last
            .as_ref()
            .and_then(|entry| str_of(entry, "timestamp"))
            .and_then(parse_iso),
        used,
        // Past 200k it can only be the 1M window, whatever the settings say now.
        window: if used > 200_000 { 1_000_000 } else { context_window() },
    }
}

/// The context of a conversation's last answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub model: Option<String>,
    pub at: Option<i64>,
    pub used: u64,
    pub window: u64,
}

/// 1M with a `[1m]` model in claude's settings, 200k otherwise.
fn context_window() -> u64 {
    let model = json::read(&claude_dir().join("settings.json"))
        .and_then(|settings| settings.get("model").map(|model| model.to_string()))
        .unwrap_or_default();
    if model.contains("[1m]") { 1_000_000 } else { 200_000 }
}

/// The share of its window (0 to 1) the conversation `id` had used at its
/// last answer; read again only when its transcript changed.
pub fn context_share_of(id: &str) -> Option<f64> {
    static SHARES: Mutex<Option<HashMap<PathBuf, (Stamp, f64)>>> = Mutex::new(None);
    let transcript = transcript_of(id)?;
    let meta = fs::metadata(&transcript).ok()?;
    let stamp = (meta.len(), meta.modified().ok());
    if let Some((known, share)) = SHARES
        .lock()
        .ok()?
        .as_ref()
        .and_then(|cache| cache.get(&transcript).copied())
        && known == stamp
    {
        return Some(share);
    }
    let (lines, _) = tail_lines(&transcript, TAIL_BYTES)?;
    let usage = usage_of(&lines);
    let share = (usage.used as f64 / usage.window as f64).min(1.0);
    if let Ok(mut cache) = SHARES.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(transcript, (stamp, share));
    }
    Some(share)
}

/// The conversation bound to a worktree, as the overview reads it (claude.mjs `readClaude`).
#[derive(Debug, Clone, PartialEq)]
pub struct Claude {
    pub id: String,
    /// The last prompt someone typed, and the last answer.
    pub prompt: Option<String>,
    pub prompt_at: Option<i64>,
    pub answer: Option<String>,
    pub transcript: PathBuf,
    /// Where `claude --resume` finds it: where it last worked, else the worktree.
    pub cwd: PathBuf,
    pub model: Option<String>,
    /// When it last answered.
    pub at: Option<i64>,
    pub used: u64,
    pub window: u64,
}

impl Claude {
    /// The share of its window used, 0 to 1.
    pub fn share(&self) -> f64 {
        if self.window == 0 {
            0.0
        } else {
            (self.used as f64 / self.window as f64).min(1.0)
        }
    }
}

type Stamp = (u64, Option<SystemTime>);
static TRANSCRIPTS: Mutex<Option<HashMap<PathBuf, (Stamp, Claude)>>> = Mutex::new(None);

/// The conversation bound to `dir`; read again only when its transcript changed.
pub fn read_claude(dir: &Path) -> Option<Claude> {
    let id = bound_session(dir)?;
    let transcript = transcript_of(&id)?;
    let meta = fs::metadata(&transcript).ok()?;
    let stamp = (meta.len(), meta.modified().ok());
    if let Some((known, claude)) = TRANSCRIPTS
        .lock()
        .ok()?
        .as_ref()
        .and_then(|cache| cache.get(&transcript).cloned())
        && known == stamp
    {
        return Some(claude);
    }
    let (lines, size) = tail_lines(&transcript, TAIL_BYTES)?;
    let usage = usage_of(&lines);
    let answer = last_entry_of(&lines, "assistant");
    let mut prompt = last_entry_of(&lines, "user");
    if prompt.is_none() && size > TAIL_BYTES {
        prompt = tail_lines(&transcript, PROMPT_BYTES).and_then(|(lines, _)| last_entry_of(&lines, "user"));
    }
    let cwd = lines
        .iter()
        .rev()
        .find_map(|line| {
            let at = line.rfind("\"cwd\":\"")?;
            let rest = &line[at + 7..];
            Some(PathBuf::from(&rest[..rest.find('"')?]))
        })
        .filter(|cwd| cwd.exists())
        .unwrap_or_else(|| dir.to_path_buf());
    let claude = Claude {
        id,
        prompt_at: prompt.as_ref().and_then(|(_, at)| *at),
        prompt: prompt.map(|(text, _)| text),
        answer: answer.map(|(text, _)| text),
        transcript: fs::canonicalize(&transcript).unwrap_or(transcript.clone()),
        cwd,
        model: usage.model,
        at: usage.at,
        used: usage.used,
        window: usage.window,
    };
    if let Ok(mut cache) = TRANSCRIPTS.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(transcript, (stamp, claude.clone()));
    }
    Some(claude)
}

/// The first line of `text` that says something, its markdown marks off.
pub fn first_line(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            line.trim_start_matches(|c: char| matches!(c, '#' | '>' | '*' | '-') || c.is_whitespace())
                .trim()
        })
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// The question an answer ends on: its last line with a `?` in its last
/// paragraph (auto-actions.mjs `questionOf`).
pub fn question_of(answer: &str) -> Option<String> {
    let trimmed = answer.trim();
    let last = split_paragraphs(trimmed).pop().unwrap_or_default();
    last.split('\n')
        .rev()
        .find(|line| line.contains('?'))
        .map(first_line)
        .filter(|line| !line.is_empty())
}

/// `text.split(/\n\s*\n/)`.
fn split_paragraphs(text: &str) -> Vec<String> {
    static BLANK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\s*\n").unwrap());
    BLANK.split(text).map(String::from).collect()
}

/// The tool call a conversation's last answer ends with, unanswered, as a
/// line: an `AskUserQuestion`'s question, else the tool and what it is about
/// (a permission prompt). `lines` oldest first.
pub fn pending_of(lines: &[String]) -> Option<String> {
    for line in lines.iter().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = str_of(&entry, "type").unwrap_or_default();
        if json::is_truthy(entry.get("isSidechain")) || !matches!(kind, "user" | "assistant") {
            continue;
        }
        if kind == "user" {
            return None;
        }
        let content = entry
            .get("message")
            .and_then(|message| message.get("content"))?
            .as_array()?;
        let call = content
            .iter()
            .rev()
            .find(|part| str_of(part, "type") == Some("tool_use"))?;
        let input = call.get("input").cloned().unwrap_or(Value::Null);
        let name = str_of(call, "name").unwrap_or_default();
        if name == "AskUserQuestion" {
            let question = input
                .get("questions")
                .and_then(|questions| questions.get(0))
                .and_then(|question| str_of(question, "question"))
                .map(first_line)
                .unwrap_or_default();
            return Some(if question.is_empty() {
                "a question".into()
            } else {
                question
            });
        }
        let about = ["description", "command", "file_path", "url"]
            .iter()
            .find_map(|key| {
                input
                    .get(key)
                    .filter(|value| !value.is_null())
                    .map(|value| match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
            })
            .map(|about| first_line(&about))
            .unwrap_or_default();
        return Some(format!(
            "may it run {name}{}?",
            if about.is_empty() {
                String::new()
            } else {
                format!(": {about}")
            }
        ));
    }
    None
}

/// pendingOf of a transcript file.
pub fn pending_of_file(transcript: &Path) -> Option<String> {
    pending_of(&tail_lines(transcript, PENDING_BYTES)?.0)
}

/// A claude running on this machine, from its file in `~/.claude/sessions`:
/// `cwd` where the process stands now (it follows EnterWorktree).
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub pid: i64,
    pub id: String,
    pub name: Option<String>,
    pub cwd: PathBuf,
}

/// The claudes running on this machine.
pub fn running_claudes() -> Vec<Running> {
    let Ok(entries) = fs::read_dir(claude_dir().join("sessions")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|entry| {
            let session = json::read(&entry.path())?;
            let pid = json::int_of(&session, "pid")?;
            if !is_alive(pid) {
                return None;
            }
            let written = str_of(&session, "cwd").map(PathBuf::from);
            let cwd = fs::read_link(format!("/proc/{pid}/cwd"))
                .ok()
                .or(written)
                .unwrap_or_default();
            Some(Running {
                pid,
                id: str_of(&session, "sessionId").unwrap_or_default().to_string(),
                name: str_of(&session, "name").map(String::from),
                cwd,
            })
        })
        .collect()
}

/// The claudes that stand in the worktree `dir` without being its agent: a
/// handover bound them elsewhere and they haven't left yet (#252).
pub fn left_in(dir: &Path, claudes: &[Running], bound: Option<&str>) -> Vec<Running> {
    claudes
        .iter()
        .filter(|claude| Some(claude.id.as_str()) != bound && claude.cwd.starts_with(dir))
        .cloned()
        .collect()
}

/// `244-x`, `244-x, claude 1234`: the claudes left_in found.
pub fn left_text(left: &[Running]) -> String {
    left.iter()
        .map(|claude| claude.name.clone().unwrap_or_else(|| format!("claude {}", claude.pid)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn finds_the_last_prompt_and_answer() {
        let transcript = lines(&[
            r#"{"type":"user","message":{"content":"first prompt"},"timestamp":"2026-10-01T10:00:00.000Z"}"#,
            r#"{"type":"assistant","message":{"model":"claude-x","content":[{"type":"text","text":"an answer"}],"usage":{"input_tokens":10,"cache_read_input_tokens":5,"output_tokens":5}},"timestamp":"2026-10-01T10:01:00.000Z"}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":"a subagent's"}}"#,
        ]);
        assert_eq!(last_entry_of(&transcript, "user").unwrap().0, "first prompt");
        assert_eq!(last_entry_of(&transcript, "assistant").unwrap().0, "an answer");
        let usage = usage_of(&transcript);
        assert_eq!(usage.used, 20);
        assert_eq!(usage.model.as_deref(), Some("claude-x"));
        assert!(usage.at.is_some());
    }

    #[test]
    fn finds_the_question_an_answer_ends_on() {
        assert_eq!(
            question_of("Done.\n\nShall I merge it?").as_deref(),
            Some("Shall I merge it?")
        );
        assert_eq!(question_of("Is it?\n\nDone."), None);
        assert_eq!(
            question_of("- **Which one?** A or B"),
            Some("Which one?** A or B".into())
        );
        assert_eq!(question_of(""), None);
    }

    #[test]
    fn names_a_pending_tool_call() {
        let ask = lines(&[
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"AskUserQuestion","input":{"questions":[{"question":"Which?"}]}}]}}"#,
        ]);
        assert_eq!(pending_of(&ask).as_deref(), Some("Which?"));
        let bash = lines(&[
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"git push"}}]}}"#,
        ]);
        assert_eq!(pending_of(&bash).as_deref(), Some("may it run Bash: git push?"));
        let answered = lines(&[
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#,
        ]);
        assert_eq!(pending_of(&answered), None);
    }

    #[test]
    fn finds_the_claudes_left_in_a_worktree() {
        let claudes = vec![
            Running {
                pid: 1,
                id: "a".into(),
                name: Some("244-x".into()),
                cwd: PathBuf::from("/r/p-1/products"),
            },
            Running {
                pid: 2,
                id: "b".into(),
                name: None,
                cwd: PathBuf::from("/r/p-1"),
            },
            Running {
                pid: 3,
                id: "c".into(),
                name: None,
                cwd: PathBuf::from("/r/p-10"),
            },
        ];
        let left = left_in(Path::new("/r/p-1"), &claudes, Some("b"));
        assert_eq!(left_text(&left), "244-x");
        assert_eq!(
            left_text(&left_in(Path::new("/r/p-1"), &claudes, None)),
            "244-x, claude 2"
        );
    }
}
