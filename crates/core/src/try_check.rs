//! A try checked by a small model (#384): while its orchestration's auto
//! mode is on, the live overview starts one `claude -p --model <checker>`
//! per open try and head, in the try's worktree. The checker runs what the
//! try says (a terminal command in a pty, a page through `browser-check`),
//! keeps what it saw as evidence and answers with one line: `looks good`,
//! `broken: <reason>` or `needs a human: <why>`. The first two take the
//! user's answer path; the third leaves the try open for the user.
//!
//! The checks of a worktree are in its `.claude/try-checks.json`, the
//! evidence in `.claude/try-checks/`.

use crate::actions::Action;
use crate::json;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What a checker found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", content = "reason", rename_all = "snake_case")]
pub enum Verdict {
    /// It does what the try says: the try is answered `looks good`.
    Good(String),
    /// It doesn't: the try is answered `broken`, the reason its note.
    Broken(String),
    /// Taste or feel, or the checker couldn't run it: the user answers.
    Human(String),
}

impl Verdict {
    /// The try's answer it gives; None: the user's.
    pub fn answer(&self) -> Option<&'static str> {
        match self {
            Verdict::Good(_) => Some("looks good"),
            Verdict::Broken(_) => Some("broken"),
            Verdict::Human(_) => None,
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Verdict::Good(reason) | Verdict::Broken(reason) | Verdict::Human(reason) => reason,
        }
    }

    /// How a row says it: `looks good`, `broken`, `needs a human`.
    pub fn word(&self) -> &'static str {
        match self {
            Verdict::Good(_) => "looks good",
            Verdict::Broken(_) => "broken",
            Verdict::Human(_) => "needs a human",
        }
    }
}

/// The verdict in a checker's answer: its last line that starts with one of
/// the three, case and markdown aside. None: it answered none of them, which
/// counts as `needs a human` (the caller says so).
pub fn verdict_of(text: &str) -> Option<Verdict> {
    text.lines().rev().find_map(|line| {
        let line = line
            .trim()
            .trim_matches(|ch: char| ch == '*' || ch == '`' || ch == '>')
            .trim();
        let lower = line.to_lowercase();
        let rest = |word: &str| {
            line[word.len()..]
                .trim_start_matches([':', ' ', '-', '—', '·'])
                .trim()
                .to_string()
        };
        if lower.starts_with("looks good") {
            Some(Verdict::Good(rest("looks good")))
        } else if lower.starts_with("broken") {
            Some(Verdict::Broken(rest("broken")))
        } else if lower.starts_with("needs a human") {
            Some(Verdict::Human(rest("needs a human")))
        } else {
            None
        }
    })
}

/// Where a check stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Running,
    Done,
}

/// One checker run of a try at a head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// The try's id in the worktree's actions.
    pub id: i64,
    /// The commit it was checked at: a new push checks again.
    pub head: String,
    pub model: String,
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// What it saw: the screen, the screenshots' paths, its whole answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<PathBuf>,
    /// When it started, ISO.
    pub at: String,
    /// Its tokens as #372 counts them: input, cache writes, output (cache reads aside).
    #[serde(default)]
    pub tokens: u64,
    /// The pid of a running checker: one whose process is gone failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Its conversation: the transcript #372 prices toward the MR's cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Checks {
    checks: Vec<Check>,
}

/// How many checks a worktree keeps, the newest.
const KEPT: usize = 50;

pub fn checks_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("try-checks.json")
}

/// Where a check's evidence goes: `.claude/try-checks/<id>-<head>/`.
pub fn evidence_dir(dir: &Path, id: i64, head: &str) -> PathBuf {
    let short: String = head.chars().take(8).collect();
    dir.join(".claude").join("try-checks").join(format!("{id}-{short}"))
}

/// The checks of the worktree `dir`, oldest first.
pub fn checks_of(dir: &Path) -> Vec<Check> {
    json::read(&checks_file(dir))
        .and_then(|value| serde_json::from_value::<Checks>(value).ok())
        .map(|checks| checks.checks)
        .unwrap_or_default()
}

/// `check` written into `dir`, in place of the one of its try and head.
pub fn write_check(dir: &Path, check: &Check) -> std::io::Result<()> {
    let mut checks: Vec<Check> = checks_of(dir)
        .into_iter()
        .filter(|known| !(known.id == check.id && known.head == check.head))
        .collect();
    checks.push(check.clone());
    let skip = checks.len().saturating_sub(KEPT);
    let checks = Checks {
        checks: checks.into_iter().skip(skip).collect(),
    };
    json::write_pretty(&checks_file(dir), &serde_json::to_value(checks).unwrap_or_default())
}

/// The latest check of the try `id`.
pub fn last_check(checks: &[Check], id: i64) -> Option<&Check> {
    checks.iter().rev().find(|check| check.id == id)
}

/// Whether a checker can check this try: an open one, not answered, not
/// one that waits for the deployment (`--on test`), with something to run
/// or open.
pub fn is_checkable(action: &Action) -> bool {
    action.is_try()
        && action.answer.is_none()
        && action.on.as_deref() != Some("test")
        && (action.run.is_some() || action.url.is_some())
}

/// The tries of a worktree a checker should take now, at `head`: checkable,
/// with no check at this head (one per try and head, a new push checks again).
pub fn due<'a>(actions: &'a [Action], checks: &[Check], head: &str) -> Vec<&'a Action> {
    actions
        .iter()
        .filter(|action| is_checkable(action))
        .filter(|action| !checks.iter().any(|check| check.id == action.id && check.head == head))
        .collect()
}

/// How a breadboard try's command is run and seen: in a pty, its text
/// screen printed (and saved) after the keys.
pub const SCREEN: &str = "node packages/breadboard-gauntlet/src/screen.mjs";

/// What the checker is told: the try, how to run it, where its evidence
/// goes, and the one line it answers with.
pub fn prompt_of(action: &Action, evidence: &Path, app: Option<&str>) -> String {
    let how = match (&action.run, &action.url) {
        (Some(run), _) => format!(
            "The try is a terminal command: `{run}`. Run it in a pseudo terminal with\n\
             `{SCREEN} --save {evidence}/screen-1.txt [--keys <keys>]… [--click <x>,<y>]… -- sh -c '{run}'`\n\
             (it prints the text screen after each step; keys like `Tab`, `Enter`, `Down`, `q`, or text). Drive it as the try describes, \
             look at the screens and number the files you save (screen-2.txt, …).",
            evidence = evidence.display()
        ),
        (None, Some(url)) => format!(
            "The try is a page of this worktree's app: `{url}`{}. When the app doesn't answer, start the worktree's stack first (`{run} stack:up`, then its app's dev server in the background). \
             Check it with the `browser-check` subagent (Agent tool), tell it the worktree, the page and what to check, \
             and have it save its screenshots into `{}`.",
            app.map(|app| format!(" at {app}")).unwrap_or_default(),
            evidence.display(),
            run = crate::config::run_word()
        ),
        (None, None) => "The try names nothing to run or open.".into(),
    };
    format!(
        "You check a `try` an agent posted for its MR, in place of the user. The try:\n\n> {text}\n\n{how}\n\n\
         Judge only what the try asks, from what you saw. Change no file of the worktree, commit nothing, push nothing.\n\
         Answer with your findings, then exactly one last line:\n\
         - `looks good: <one line why>` when it does what the try says;\n\
         - `broken: <what is wrong, one line>` when it doesn't;\n\
         - `needs a human: <why>` when it is a question of taste or feel, or you couldn't run or see it.",
        text = action.text.replace('\n', "\n> "),
    )
}

/// The line a try row shows for its check: `checked by haiku · looks good · <reason>`.
pub fn row_text(check: &Check) -> String {
    match (&check.state, &check.verdict) {
        (State::Running, _) => format!("{} checks it …", check.model),
        (State::Done, Some(verdict)) if verdict.reason().is_empty() => {
            format!("checked by {} · {}", check.model, verdict.word())
        }
        (State::Done, Some(verdict)) => {
            format!("checked by {} · {} · {}", check.model, verdict.word(), verdict.reason())
        }
        (State::Done, None) => format!("checked by {} · no answer", check.model),
    }
}

/// How a checker run is started: `claude -p <prompt>` with the checker's
/// model, a conversation of its own (its transcript is the evidence too),
/// auto mode as a headless agent's (#360), its stream to read the answer and
/// the tokens from.
pub fn args_of(model: &str, session: &str, prompt: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-p".into(),
        prompt.to_string(),
        "--session-id".into(),
        session.into(),
        "--model".into(),
        model.into(),
    ];
    args.extend(crate::permissions::run_flags(&[]));
    args.extend(["--output-format".into(), "stream-json".into(), "--verbose".into()]);
    args
}

/// The answer and the tokens of a checker's stream: its `result` event's
/// text (else its last assistant text), its usage summed (input, cache, output).
pub fn answer_of(lines: &[String]) -> (String, u64) {
    let mut text = String::new();
    let mut tokens = 0;
    for line in lines {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match event.get("type").and_then(serde_json::Value::as_str) {
            Some("result") => {
                if let Some(result) = event.get("result").and_then(serde_json::Value::as_str) {
                    text = result.to_string();
                }
                if let Some(usage) = event.get("usage").and_then(serde_json::Value::as_object) {
                    tokens = ["input_tokens", "cache_creation_input_tokens", "output_tokens"]
                        .iter()
                        .filter_map(|key| usage.get(*key).and_then(serde_json::Value::as_u64))
                        .sum();
                }
            }
            Some("assistant") => {
                let said: Vec<&str> = event
                    .pointer("/message/content")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                    .collect();
                if !said.is_empty() && text.is_empty() {
                    text = said.join("\n");
                }
            }
            _ => {}
        }
    }
    (text, tokens)
}

/// The verdict of a checker's answer; one with none of the three lines
/// leaves the try to the user.
pub fn verdict_or_human(text: &str) -> Verdict {
    verdict_of(text).unwrap_or_else(|| Verdict::Human("its answer named no verdict".into()))
}

/// The note a checker's `broken` reaches the agent with.
pub fn broken_note(model: &str, reason: &str) -> String {
    format!("{reason} (checked by {model})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tried(id: i64, run: Option<&str>, url: Option<&str>) -> Action {
        Action {
            id,
            kind: "try".into(),
            text: "the overview shows the epic row".into(),
            run: run.map(String::from),
            url: url.map(String::from),
            ..Action::default()
        }
    }

    #[test]
    fn reads_the_last_verdict_line() {
        assert_eq!(
            verdict_of("I ran it.\nThe row shows.\n\nlooks good: the EPIC row names the branch"),
            Some(Verdict::Good("the EPIC row names the branch".into()))
        );
        assert_eq!(
            verdict_of("**broken: the row is missing**"),
            Some(Verdict::Broken("the row is missing".into()))
        );
        assert_eq!(
            verdict_of("Needs a human — it is about the colours"),
            Some(Verdict::Human("it is about the colours".into()))
        );
        assert_eq!(verdict_of("looks good"), Some(Verdict::Good(String::new())));
        assert_eq!(verdict_of("I think it works."), None);
        assert_eq!(Verdict::Broken("x".into()).answer(), Some("broken"));
        assert_eq!(Verdict::Human("x".into()).answer(), None);
    }

    #[test]
    fn one_check_per_try_and_head() {
        let actions = [
            tried(1, Some("pnpm overview"), None),
            tried(2, None, Some("/atlas")),
            Action {
                on: Some("test".into()),
                ..tried(3, None, Some("/"))
            },
            Action {
                answer: Some("looks good".into()),
                ..tried(4, Some("x"), None)
            },
            Action {
                kind: "look".into(),
                ..tried(5, Some("x"), None)
            },
            tried(6, None, None),
        ];
        let check = |id: i64, head: &str| Check {
            id,
            head: head.into(),
            model: "haiku".into(),
            state: State::Done,
            verdict: Some(Verdict::Human("taste".into())),
            evidence: None,
            at: String::new(),
            tokens: 0,
            pid: None,
            session: None,
        };
        let ids = |due: Vec<&Action>| due.iter().map(|action| action.id).collect::<Vec<_>>();
        assert_eq!(ids(due(&actions, &[], "a")), [1, 2]);
        assert_eq!(ids(due(&actions, &[check(1, "a")], "a")), [2], "checked at this head");
        assert_eq!(
            ids(due(&actions, &[check(1, "a")], "b")),
            [1, 2],
            "a new push checks again"
        );
    }

    #[test]
    fn keeps_its_checks_in_the_worktree() {
        let dir = std::env::temp_dir().join(format!("try-checks-{}", std::process::id()));
        let mut check = Check {
            id: 3,
            head: "abcdef123456".into(),
            model: "haiku".into(),
            state: State::Running,
            verdict: None,
            evidence: Some(evidence_dir(&dir, 3, "abcdef123456")),
            at: "2026-10-13T10:00:00.000Z".into(),
            tokens: 0,
            pid: Some(42),
            session: None,
        };
        write_check(&dir, &check).unwrap();
        assert_eq!(row_text(&checks_of(&dir)[0]), "haiku checks it …");
        check.state = State::Done;
        check.verdict = Some(Verdict::Good("the row shows".into()));
        write_check(&dir, &check).unwrap();
        let checks = checks_of(&dir);
        assert_eq!(checks.len(), 1, "one per try and head");
        assert_eq!(
            row_text(last_check(&checks, 3).unwrap()),
            "checked by haiku · looks good · the row shows"
        );
        assert!(check.evidence.unwrap().ends_with(".claude/try-checks/3-abcdef12"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_the_answer_and_the_tokens_of_its_stream() {
        let lines = [
            r#"{"type":"system","subtype":"init"}"#.to_string(),
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Ran it."}]}}"#.to_string(),
            r#"{"type":"result","result":"Ran it.\nlooks good: the row shows","usage":{"input_tokens":100,"cache_read_input_tokens":900,"output_tokens":40}}"#.to_string(),
        ];
        let (text, tokens) = answer_of(&lines);
        assert_eq!(tokens, 140, "cache reads aside, as #372 counts");
        assert_eq!(verdict_or_human(&text), Verdict::Good("the row shows".into()));
        assert_eq!(
            verdict_or_human("Hm."),
            Verdict::Human("its answer named no verdict".into())
        );
        let args = args_of("haiku", "s-1", "check it");
        assert_eq!(args[..6], ["-p", "check it", "--session-id", "s-1", "--model", "haiku"]);
        assert!(args.contains(&"stream-json".to_string()));
    }

    #[test]
    fn tells_the_checker_how_and_what_to_answer() {
        let prompt = prompt_of(&tried(1, Some("pnpm overview"), None), Path::new("/w/e"), None);
        assert!(prompt.contains("> the overview shows the epic row"));
        assert!(prompt.contains("--save /w/e/screen-1.txt"));
        assert!(prompt.contains("-- sh -c 'pnpm overview'"));
        assert!(prompt.ends_with("or you couldn't run or see it."));
        let page = prompt_of(
            &tried(2, None, Some("/atlas")),
            Path::new("/w/e"),
            Some("http://localhost:3010"),
        );
        assert!(page.contains("`/atlas` at http://localhost:3010"));
        assert!(page.contains("browser-check"));
    }
}
