//! What the printed views know of the agents, read only: herdr's state of
//! each (auto-actions.mjs `agentsOf`: one `herdr pane list`, inside herdr
//! only) and the conversation bound to a worktree (claude.mjs `readClaude`):
//! its last answer, the context that answer had, a tool call it waits at,
//! and the claudes still standing in a worktree after a handover.
//!
//! The transcript half is a trimmed copy of core's `conversations.rs`, which
//! the core snapshot doesn't compile yet; it goes once core exports it.

use breadboard_core::json::{self, int_of, str_of};
use breadboard_core::lock::is_alive;
use breadboard_core::record::is_session_id;
use breadboard_core::time::parse_iso;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How much of a transcript's end is read: the last answer is in there.
const TAIL_BYTES: u64 = 1 << 19;
/// How much of its end a pending tool call is looked for in: it is the last entry.
const PENDING_BYTES: u64 = 1 << 16;

/// An agent herdr runs: its pane and tab, and what herdr says it does
/// (`working`, `blocked`: it asks you, `idle`, `done`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Agent {
    pub pane: String,
    pub tab: String,
    pub status: String,
}

/// The agents herdr runs, by their conversation's id.
pub type Agents = HashMap<String, Agent>;

/// Whether this runs inside herdr: only then is there a herdr to ask.
pub fn is_in_herdr() -> bool {
    std::env::var("HERDR_ENV").is_ok_and(|value| value == "1")
}

/// The agents herdr runs (one `herdr pane list`); none outside herdr or when herdr doesn't answer.
pub fn agents_of() -> Agents {
    if !is_in_herdr() {
        return Agents::new();
    }
    let output = Command::new("herdr")
        .args(["pane", "list"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => serde_json::from_slice::<Value>(&output.stdout)
            .map(|answer| agents_in(&answer))
            .unwrap_or_default(),
        _ => Agents::new(),
    }
}

/// The agents of an answer of `herdr pane list`: the panes with an agent and its conversation.
pub fn agents_in(answer: &Value) -> Agents {
    let panes = answer
        .get("result")
        .and_then(|result| result.get("panes"))
        .and_then(Value::as_array);
    panes
        .into_iter()
        .flatten()
        .filter(|pane| str_of(pane, "agent").is_some_and(|agent| !agent.is_empty()))
        .filter_map(|pane| {
            let id = pane.get("agent_session").and_then(|session| str_of(session, "value"))?;
            Some((
                id.to_string(),
                Agent {
                    pane: str_of(pane, "pane_id").unwrap_or_default().to_string(),
                    tab: str_of(pane, "tab_id").unwrap_or_default().to_string(),
                    status: str_of(pane, "agent_status").unwrap_or_default().to_string(),
                },
            ))
        })
        .collect()
}

/// The conversation bound to a worktree, as the views use it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Claude {
    pub id: String,
    /// The last answer, None before the first.
    pub answer: Option<String>,
    /// The last prompt someone typed (a harness prompt is none).
    pub prompt: Option<String>,
    /// Where `claude --resume` finds it: the directory it last worked in, else the worktree.
    pub cwd: PathBuf,
    pub transcript: PathBuf,
    pub model: Option<String>,
    /// When it last answered.
    pub at: Option<i64>,
    /// The context of that answer: the prompt it read plus what it wrote, of its window.
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

/// claude's config directory: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude"))
}

/// The conversation bound to a worktree's space (`.claude/herdr-session`).
fn bound_session(dir: &Path) -> Option<String> {
    let id = fs::read_to_string(dir.join(".claude").join("herdr-session")).ok()?;
    let id = id.trim();
    is_session_id(id).then(|| id.to_string())
}

/// The transcript of a conversation, in whichever project it was started.
fn transcript_of(id: &str) -> Option<PathBuf> {
    fs::read_dir(claude_dir().join("projects"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(format!("{id}.jsonl")))
        .find(|file| file.exists())
}

/// The last `max` bytes of a file as lines, the first one dropped when it was cut.
fn tail_lines(file: &Path, max: u64) -> Option<Vec<String>> {
    let mut opened = fs::File::open(file).ok()?;
    let size = opened.metadata().ok()?.len();
    let length = size.min(max);
    opened.seek(SeekFrom::Start(size - length)).ok()?;
    let mut buffer = Vec::with_capacity(length as usize);
    opened.take(length).read_to_end(&mut buffer).ok()?;
    let mut lines: Vec<String> = String::from_utf8_lossy(&buffer).split('\n').map(String::from).collect();
    if length < size && !lines.is_empty() {
        lines.remove(0);
    }
    Some(lines)
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

/// The last text of `kind` in `lines` (oldest first): `assistant` an
/// answer, `user` a prompt someone typed (a tool's result or a harness
/// prompt, `<command-…>`, is none); a subagent's left out.
fn last_text_of(lines: &[String], kind: &str) -> Option<String> {
    let needle = format!("\"type\":\"{kind}\"");
    lines
        .iter()
        .rev()
        .filter(|line| line.contains(&needle) && !(kind == "user" && line.contains("\"tool_result\"")))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| {
            str_of(entry, "type") == Some(kind)
                && !json::is_truthy(entry.get("isSidechain"))
                && !json::is_truthy(entry.get("isMeta"))
        })
        .map(|entry| text_of(entry.get("message").and_then(|message| message.get("content"))))
        .map(|text| text.trim().to_string())
        .find(|text| !text.is_empty() && !(kind == "user" && text.starts_with('<')))
}

/// The context of the last answer in `lines`: `(model, at, used)`, `used`
/// the prompt it read (cached or not) plus what it wrote.
fn usage_of(lines: &[String]) -> (Option<String>, Option<i64>, u64) {
    let last = lines
        .iter()
        .rev()
        .filter(|line| line.contains("\"usage\""))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|entry| {
            let message = entry.get("message");
            str_of(entry, "type") == Some("assistant")
                && !json::is_truthy(entry.get("isSidechain"))
                && message
                    .and_then(|message| message.get("usage"))
                    .is_some_and(|usage| !usage.is_null())
                && message.and_then(|message| str_of(message, "model")) != Some("<synthetic>")
        });
    let Some(last) = last else { return (None, None, 0) };
    let message = &last["message"];
    let used = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ]
    .iter()
    .map(|key| message["usage"].get(key).and_then(Value::as_u64).unwrap_or(0))
    .sum();
    (
        str_of(message, "model").map(String::from),
        str_of(&last, "timestamp").and_then(parse_iso),
        used,
    )
}

/// The window an answer of `used` tokens had: 1M with a `[1m]` model in
/// claude's settings (or past 200k, which only 1M holds), 200k otherwise.
fn window_of(used: u64) -> u64 {
    let model = json::read(&claude_dir().join("settings.json"))
        .and_then(|settings| settings.get("model").map(Value::to_string))
        .unwrap_or_default();
    if used > 200_000 || model.contains("[1m]") {
        1_000_000
    } else {
        200_000
    }
}

/// The conversation bound to the worktree `dir`, None without one or its transcript.
pub fn read_claude(dir: &Path) -> Option<Claude> {
    let id = bound_session(dir)?;
    let transcript = transcript_of(&id)?;
    let lines = tail_lines(&transcript, TAIL_BYTES)?;
    let (model, at, used) = usage_of(&lines);
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
    Some(Claude {
        answer: last_text_of(&lines, "assistant"),
        prompt: last_text_of(&lines, "user"),
        cwd,
        transcript: fs::canonicalize(&transcript).unwrap_or(transcript),
        model,
        at,
        used,
        window: window_of(used),
        id,
    })
}

/// The share of its window (0 to 1) the conversation `id` had used at its last answer.
pub fn context_share_of(id: &str) -> Option<f64> {
    let lines = tail_lines(&transcript_of(id)?, TAIL_BYTES)?;
    let (_, _, used) = usage_of(&lines);
    Some((used as f64 / window_of(used) as f64).min(1.0))
}

/// `200k`, `1M`, `1.5M`: a count of tokens (render-worktree.mjs `tokens`).
pub fn tokens(count: u64) -> String {
    if count >= 1_000_000 {
        let millions = format!("{:.1}", count as f64 / 1_000_000.0);
        format!("{}M", millions.strip_suffix(".0").unwrap_or(&millions))
    } else {
        format!("{}k", (count as f64 / 1000.0).round() as u64)
    }
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
/// paragraph (auto-actions.mjs `questionOf`). An agent that stops on one waits for you.
pub fn question_of(answer: &str) -> Option<String> {
    let lines: Vec<&str> = answer.trim().split('\n').collect();
    // The last paragraph: the lines after the last blank one.
    let start = lines
        .iter()
        .rposition(|line| line.trim().is_empty())
        .map_or(0, |at| at + 1);
    lines[start..]
        .iter()
        .rev()
        .find(|line| line.contains('?'))
        .map(|line| first_line(line))
        .filter(|line| !line.is_empty())
}

/// The tool call a conversation's last answer ends with, unanswered, as a
/// line (auto-actions.mjs `pendingOf`): an `AskUserQuestion`'s question,
/// else the tool and what it is about (a permission prompt).
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
        let content = entry.get("message")?.get("content")?.as_array()?;
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
            .find_map(|key| input.get(key).filter(|value| !value.is_null()))
            .map(|value| match value {
                Value::String(text) => first_line(text),
                other => first_line(&other.to_string()),
            })
            .unwrap_or_default();
        return Some(if about.is_empty() {
            format!("may it run {name}?")
        } else {
            format!("may it run {name}: {about}?")
        });
    }
    None
}

/// pending_of of a transcript file.
pub fn pending_of_file(transcript: &Path) -> Option<String> {
    pending_of(&tail_lines(transcript, PENDING_BYTES)?)
}

/// A claude running on this machine (`~/.claude/sessions/<pid>.json`), where its process stands now.
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
            let pid = int_of(&session, "pid").filter(|pid| is_alive(*pid))?;
            let written = str_of(&session, "cwd").map(PathBuf::from);
            let cwd = fs::read_link(format!("/proc/{pid}/cwd")).ok().or(written)?;
            Some(Running {
                pid,
                id: str_of(&session, "sessionId").unwrap_or_default().to_string(),
                name: str_of(&session, "name").map(String::from),
                cwd,
            })
        })
        .collect()
}

/// The claudes standing in the worktree `dir` without being its agent: a
/// handover bound them elsewhere and they haven't left yet (#252), by name.
pub fn left_in(dir: &Path, claudes: &[Running], bound: Option<&str>) -> Vec<String> {
    claudes
        .iter()
        .filter(|claude| Some(claude.id.as_str()) != bound && claude.cwd.starts_with(dir))
        .map(|claude| claude.name.clone().unwrap_or_else(|| format!("claude {}", claude.pid)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn knows_the_agents_herdr_runs_by_their_conversation() {
        let answer = json!({"result": {"panes": [
            {"pane_id": "w1:p1", "tab_id": "w1:t1", "agent_status": "unknown"},
            {"pane_id": "w1:p2", "tab_id": "w1:t2", "agent": "claude", "agent_status": "working",
             "agent_session": {"value": "c1"}},
            {"pane_id": "w1:p3", "agent": "claude", "agent_status": "idle"}
        ]}});
        let agents = agents_in(&answer);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents["c1"].status, "working");
        assert_eq!(agents["c1"].tab, "w1:t2");
        assert!(agents_in(&json!({"result": {}})).is_empty());
    }

    #[test]
    fn reads_the_last_answer_and_its_context() {
        let transcript = lines(&[
            r#"{"type":"assistant","message":{"model":"claude-x","content":[{"type":"text","text":"an answer"}],"usage":{"input_tokens":10,"cache_read_input_tokens":5,"output_tokens":5}},"timestamp":"2026-10-01T10:01:00.000Z"}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":"a subagent's"}}"#,
        ]);
        assert_eq!(last_text_of(&transcript, "assistant").as_deref(), Some("an answer"));
        assert_eq!(last_text_of(&transcript, "user"), None);
        let (model, at, used) = usage_of(&transcript);
        assert_eq!((model.as_deref(), used), (Some("claude-x"), 20));
        assert!(at.is_some());
        assert_eq!(usage_of(&[]), (None, None, 0));
    }

    #[test]
    fn counts_tokens_as_node_writes_them() {
        assert_eq!(tokens(200_000), "200k");
        assert_eq!(tokens(1_000_000), "1M");
        assert_eq!(tokens(1_540_000), "1.5M");
        assert_eq!(tokens(499), "0k");
    }

    #[test]
    fn finds_the_question_an_answer_ends_on() {
        assert_eq!(
            question_of("Done.\n\nShall I merge it?").as_deref(),
            Some("Shall I merge it?")
        );
        assert_eq!(question_of("Is it?\n\nDone."), None);
        assert_eq!(question_of("Which one?\nA or B"), Some("Which one?".into()));
        assert_eq!(question_of("- **Which?** A"), Some("Which?** A".into()));
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
        let running = |pid, id: &str, name: Option<&str>, cwd: &str| Running {
            pid,
            id: id.into(),
            name: name.map(String::from),
            cwd: cwd.into(),
        };
        let claudes = vec![
            running(1, "a", Some("244-x"), "/r/p-1/products"),
            running(2, "b", None, "/r/p-1"),
            running(3, "c", None, "/r/p-10"),
        ];
        assert_eq!(left_in(Path::new("/r/p-1"), &claudes, Some("b")), vec!["244-x"]);
        assert_eq!(left_in(Path::new("/r/p-1"), &claudes, None), vec!["244-x", "claude 2"]);
    }
}
