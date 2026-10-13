//! Headless task agents keep auto mode (#360). A run is `claude -p` with the
//! same permission mode as a tab: the auto-mode classifier decides each tool
//! call, the repo's allow and deny lists and hooks apply unchanged. What a tab
//! would stop and ask the user can't be asked without a terminal: the run
//! gets it denied (`--permission-prompts none`), and the daemon turns that
//! denial into a `permission` action. `allow` runs the agent again with the
//! call allowed for that run (`--allowedTools`), `always` also writes it to
//! the worktree's own settings.
//!
//! A run's denials come in its stream (`--output-format stream-json`): the
//! `result` event lists them (`permission_denials`), and the tool result the
//! agent got says why. Three kinds, told apart by that text and the deny
//! lists (a probe of `claude -p --permission-mode auto` on 2026-10-11):
//! - the auto-mode classifier blocked it: "… denied by the Claude Code auto
//!   mode classifier. Reason: […]" — a tab blocks it the same way, no question;
//! - a deny rule matched: "Permission to use Bash with command … has been
//!   denied." — a tab refuses it too, no question;
//! - nothing decided it, so a tab would have asked: the same text without a
//!   matching deny rule. Only these become a `permission` action.

use crate::actions::{Action, NewAction, actions_file, add_action};
use crate::json::{self, str_of};
use serde_json::{Map, Value, json};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Tools a run never gets: a question goes to the overview as `wt:action decide`.
pub const NEVER_IN_RUNS: [&str; 1] = ["AskUserQuestion"];

/// The answers of a `permission` action.
pub const ALLOW_ONCE: &str = "allow once";
pub const ALLOW_ALWAYS: &str = "allow always";
pub const DENY: &str = "deny";
pub const PERMISSION_ANSWERS: [&str; 3] = [ALLOW_ONCE, ALLOW_ALWAYS, DENY];

/// The permission flags of a headless run: auto mode as in a tab, what would
/// ask denied, `AskUserQuestion` off, and the rules the user allowed for
/// this run (an `allow` of a `permission` action).
pub fn run_flags(allowed: &[String]) -> Vec<String> {
    let mut flags: Vec<String> = [
        "--permission-mode",
        "auto",
        "--permission-prompts",
        "none",
        "--disallowedTools",
    ]
    .iter()
    .map(|flag| flag.to_string())
    .collect();
    flags.extend(NEVER_IN_RUNS.iter().map(|tool| tool.to_string()));
    if !allowed.is_empty() {
        flags.push("--allowedTools".into());
        flags.extend(allowed.iter().cloned());
    }
    flags
}

/// Why a run's tool call was denied.
#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    /// No rule and no classifier decided it: a tab would have asked the user.
    Asked,
    /// A deny rule matched: a tab refuses it as well.
    Rule,
    /// The auto-mode classifier blocked it, with its reason.
    Classifier(String),
}

/// A tool call a run was denied.
#[derive(Debug, Clone, PartialEq)]
pub struct Denial {
    pub tool: String,
    pub input: Value,
    pub tool_use_id: String,
    /// The rule that allows exactly this call (`Bash(git tag v1)`).
    pub rule: String,
    pub why: Why,
}

const CLASSIFIER: &str = "denied by the Claude Code auto mode classifier";

/// The denials of a run, from the lines of its stream-json output; `deny`
/// the deny rules of its settings. A line that isn't JSON is passed over.
pub fn denials_of(lines: &[String], deny: &[String]) -> Vec<Denial> {
    let events: Vec<Value> = lines
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let said = |id: &str| -> String {
        events
            .iter()
            .filter(|event| str_of(event, "type") == Some("user"))
            .filter_map(|event| event.get("message")?.get("content")?.as_array())
            .flatten()
            .find(|part| str_of(part, "type") == Some("tool_result") && str_of(part, "tool_use_id") == Some(id))
            .map(|part| match part.get("content") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Array(parts)) => parts
                    .iter()
                    .filter_map(|part| str_of(part, "text"))
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            })
            .unwrap_or_default()
    };
    events
        .iter()
        .filter(|event| str_of(event, "type") == Some("result"))
        .filter_map(|event| event.get("permission_denials")?.as_array())
        .flatten()
        .map(|denial| {
            let tool = str_of(denial, "tool_name").unwrap_or_default().to_string();
            let input = denial.get("tool_input").cloned().unwrap_or(Value::Null);
            let tool_use_id = str_of(denial, "tool_use_id").unwrap_or_default().to_string();
            let text = said(&tool_use_id);
            let why = if text.contains(CLASSIFIER) {
                Why::Classifier(reason_of(&text))
            } else if deny.iter().any(|rule| matches(rule, &tool, &input)) {
                Why::Rule
            } else {
                Why::Asked
            };
            Denial {
                rule: rule_of(&tool, &input),
                tool,
                input,
                tool_use_id,
                why,
            }
        })
        .collect()
}

/// The classifier's reason in its denial: `Reason: [Irreversible Local Destruction]`.
fn reason_of(text: &str) -> String {
    text.split_once("Reason: [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(reason, _)| reason.to_string())
        .unwrap_or_default()
}

/// The tools a rule on files covers: an `Edit` rule is one for every edit.
const EDITS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// The rule that allows exactly this call, as `--allowedTools` and the
/// settings take it: a command as itself, a file by its absolute path
/// (`//` starts one), a fetch by its host, any other tool by its name.
pub fn rule_of(tool: &str, input: &Value) -> String {
    let field = |key| str_of(input, key).filter(|value| !value.is_empty());
    if tool == "Bash"
        && let Some(command) = field("command")
    {
        return format!("Bash({command})");
    }
    if EDITS.contains(&tool)
        && let Some(file) = field("file_path").or_else(|| field("notebook_path"))
    {
        let file = file.strip_prefix('/').unwrap_or(file);
        return format!("Edit(//{file})");
    }
    if tool == "WebFetch"
        && let Some(host) = field("url").and_then(host_of)
    {
        return format!("WebFetch(domain:{host})");
    }
    tool.to_string()
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

/// Whether a permission rule (`Bash(git push --force:*)`, `Bash(glab mr
/// update * --ready)`, `WebFetch`) covers this call: a command by its prefix
/// (`:*`), its glob (`*`) or as a whole; any other rule only by the tool's
/// name alone. Rules on paths aren't matched: a deny list of commands is
/// what decides here, and a wrong "no match" only offers an `allow` that the
/// deny rule then refuses again.
pub fn matches(rule: &str, tool: &str, input: &Value) -> bool {
    let (name, spec) = match rule.split_once('(') {
        Some((name, rest)) => (name, rest.strip_suffix(')')),
        None => (rule, None),
    };
    let covers = name == tool || (name == "Edit" && EDITS.contains(&tool));
    if !covers {
        return false;
    }
    let Some(spec) = spec else {
        return true;
    };
    if tool != "Bash" {
        return false;
    }
    let Some(command) = str_of(input, "command") else {
        return false;
    };
    if let Some(prefix) = spec.strip_suffix(":*") {
        return command == prefix || command.starts_with(&format!("{prefix} "));
    }
    if spec.contains('*') {
        return glob(spec, command);
    }
    command == spec
}

/// `*` matches any text, everything else itself.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return text.is_empty();
    };
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    for (at, part) in parts.iter().enumerate() {
        if at == parts.len() - 1 {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(found) => rest = &rest[found + part.len()..],
            None => return false,
        }
    }
    rest.is_empty()
}

/// The settings files that apply to a run in `dir`: the user's, the
/// repo's and the worktree's own.
pub fn settings_files(dir: &Path, home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".claude").join("settings.json"),
        dir.join(".claude").join("settings.json"),
        local_settings(dir),
    ]
}

/// The worktree's own settings, where `allow always` writes.
pub fn local_settings(dir: &Path) -> PathBuf {
    dir.join(".claude").join("settings.local.json")
}

/// The deny rules of the settings that apply to a run in `dir`.
pub fn deny_rules(dir: &Path, home: &Path) -> Vec<String> {
    settings_files(dir, home)
        .iter()
        .flat_map(|file| {
            let settings = Value::Object(json::read_object(file));
            json::strs_of(settings.get("permissions").unwrap_or(&Value::Null), "deny")
        })
        .collect()
}

/// `rule` allowed in the worktree's own settings for good (`allow always`),
/// the file's other settings kept; nothing written when it is there already.
pub fn allow_always(dir: &Path, rule: &str) -> io::Result<()> {
    let file = local_settings(dir);
    let mut settings = json::read_object(&file);
    let permissions = settings
        .entry("permissions")
        .or_insert_with(|| Value::Object(Map::new()));
    if !permissions.is_object() {
        *permissions = Value::Object(Map::new());
    }
    let allow = permissions
        .as_object_mut()
        .map(|permissions| permissions.entry("allow").or_insert_with(|| json!([])))
        .filter(|allow| allow.is_array());
    let Some(Value::Array(allow)) = allow else {
        return Err(io::Error::other(format!(
            "{}: permissions.allow is no list",
            file.display()
        )));
    };
    if allow.iter().any(|known| known.as_str() == Some(rule)) {
        return Ok(());
    }
    allow.push(json!(rule));
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&file, format!("{}\n", serde_json::to_string_pretty(&settings)?))
}

/// The text of a `permission` action: `may 360-… run Bash: git tag v1?`.
pub fn question_of(agent: &str, denial: &Denial) -> String {
    let about = ["command", "file_path", "notebook_path", "url"]
        .iter()
        .find_map(|key| str_of(&denial.input, key))
        .map(|about| about.lines().next().unwrap_or_default().to_string())
        .unwrap_or_default();
    if about.is_empty() {
        format!("may {agent} use {}?", denial.tool)
    } else {
        format!("may {agent} run {}: {about}?", denial.tool)
    }
}

/// What the agent's next run is told after the user answered a `permission`
/// action, and the rules that run may use: `allow` the call with its rule,
/// `deny` nothing.
pub fn answer_run(answer: &str, rule: &str) -> (String, Vec<String>) {
    match answer {
        ALLOW_ONCE | ALLOW_ALWAYS => (
            format!("[overview] the user allowed {rule}, go on: it is allowed in this run."),
            vec![rule.to_string()],
        ),
        _ => (
            format!(
                "[overview] the user denied {rule}: don't run it, go on without it or leave an action for the user."
            ),
            Vec::new(),
        ),
    }
}

/// A `permission` action in the worktree's actions file for each call of a
/// run that a tab would have asked about (`Why::Asked`); the same question
/// again replaces the older one. `agent` names the worktree in the question.
pub fn post_denials(dir: &Path, agent: &str, denials: &[Denial]) -> io::Result<Vec<Action>> {
    denials
        .iter()
        .filter(|denial| denial.why == Why::Asked)
        .map(|denial| {
            add_action(
                &actions_file(dir),
                NewAction {
                    kind: "permission".into(),
                    text: question_of(agent, denial),
                    options: PERMISSION_ANSWERS.iter().map(|answer| answer.to_string()).collect(),
                    rule: Some(denial.rule.clone()),
                    ..NewAction::default()
                },
            )
        })
        .collect()
}

/// What an answered `permission` action gives its agent's next run: the line
/// and the rules allowed in that run. `allow always` writes the rule to the
/// worktree's settings first. None for any other action, or one unanswered.
pub fn delivery_of(dir: &Path, action: &Action) -> io::Result<Option<(String, Vec<String>)>> {
    let (Some(answer), Some(rule)) = (&action.answer, &action.rule) else {
        return Ok(None);
    };
    if action.kind != "permission" {
        return Ok(None);
    }
    if answer == ALLOW_ALWAYS {
        allow_always(dir, rule)?;
    }
    Ok(Some(answer_run(answer, rule)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{actions_of, answer_action};

    /// The deny list of the repo's `.claude/settings.json`, the part a probe touches.
    fn deny() -> Vec<String> {
        [
            "Bash(git push --force:*)",
            "Bash(git tag:*)",
            "Bash(git reset --hard:*)",
            "Bash(glab mr merge:*)",
        ]
        .iter()
        .map(|rule| rule.to_string())
        .collect()
    }

    /// A run's stream as the probe of 2026-10-11 got it (shortened): calls,
    /// the tool results the agent got, the result with its denials.
    fn stream(calls: &[(&str, &str, &str)]) -> Vec<String> {
        let mut lines = vec![json!({"type": "system", "subtype": "init"}).to_string()];
        let mut denials = Vec::new();
        for (id, command, said) in calls {
            lines.push(
                json!({"type": "assistant", "message": {"content": [
                    {"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}
                ]}})
                .to_string(),
            );
            let is_denied = said.contains("denied");
            lines.push(
                json!({"type": "user", "message": {"content": [
                    {"type": "tool_result", "tool_use_id": id, "content": said, "is_error": is_denied}
                ]}})
                .to_string(),
            );
            if is_denied {
                denials.push(json!({"tool_name": "Bash", "tool_use_id": id, "tool_input": {"command": command}}));
            }
        }
        lines.push("not json".into());
        lines.push(json!({"type": "result", "permission_denials": denials}).to_string());
        lines
    }

    fn rule_text(command: &str) -> String {
        format!("Permission to use Bash with command {command} has been denied.")
    }

    #[test]
    fn tells_a_question_from_a_deny_rule_and_the_classifier() {
        let classifier = "Permission for this action was denied by the Claude Code auto mode classifier. Reason: [Irreversible Local Destruction]. If you have other tasks …";
        let lines = stream(&[
            ("t1", "git status --short", "(Bash completed with no output)"),
            ("t2", "git tag probe-v0", &rule_text("git tag probe-v0")),
            ("t3", "curl -s https://example.com", classifier),
            ("t4", "docker system prune", &rule_text("docker system prune")),
        ]);
        let denials = denials_of(&lines, &deny());
        let whys: Vec<_> = denials
            .iter()
            .map(|d| (d.tool_use_id.as_str(), d.why.clone()))
            .collect();
        assert_eq!(
            whys,
            vec![
                ("t2", Why::Rule),
                ("t3", Why::Classifier("Irreversible Local Destruction".into())),
                ("t4", Why::Asked),
            ]
        );
        assert_eq!(denials[2].rule, "Bash(docker system prune)");
    }

    #[test]
    fn a_run_without_denials_has_none() {
        assert!(denials_of(&stream(&[("t1", "ls", "a.txt")]), &deny()).is_empty());
        assert!(denials_of(&[], &deny()).is_empty());
    }

    #[test]
    fn runs_in_auto_mode_without_questions() {
        assert_eq!(
            run_flags(&[]),
            vec![
                "--permission-mode",
                "auto",
                "--permission-prompts",
                "none",
                "--disallowedTools",
                "AskUserQuestion"
            ]
        );
        assert_eq!(
            run_flags(&["Bash(docker system prune)".into()])[6..],
            ["--allowedTools", "Bash(docker system prune)"]
        );
    }

    #[test]
    fn matches_rules_as_claude_code_writes_them() {
        let bash = |command: &str| json!({ "command": command });
        assert!(matches(
            "Bash(git push --force:*)",
            "Bash",
            &bash("git push --force origin x")
        ));
        assert!(matches("Bash(git tag:*)", "Bash", &bash("git tag")));
        assert!(!matches("Bash(git tag:*)", "Bash", &bash("git tagger")));
        assert!(matches(
            "Bash(glab mr update * --ready)",
            "Bash",
            &bash("glab mr update 12 --ready")
        ));
        assert!(!matches(
            "Bash(glab mr update * --ready)",
            "Bash",
            &bash("glab mr update 12 --draft")
        ));
        assert!(matches("Bash(git status)", "Bash", &bash("git status")));
        assert!(!matches("Bash(git status)", "Bash", &bash("git status -s")));
        assert!(matches("WebFetch", "WebFetch", &json!({"url": "https://a.b"})));
        assert!(matches("Edit", "Write", &json!({"file_path": "/x"})));
        assert!(!matches("Edit(//x)", "Write", &json!({"file_path": "/x"})));
        assert!(!matches("Bash(git tag:*)", "Edit", &json!({})));
    }

    #[test]
    fn the_rule_that_allows_exactly_the_call() {
        assert_eq!(
            rule_of("Bash", &json!({"command": "rm -rf build"})),
            "Bash(rm -rf build)"
        );
        assert_eq!(
            rule_of("Write", &json!({"file_path": "/home/a/.bashrc"})),
            "Edit(//home/a/.bashrc)"
        );
        assert_eq!(
            rule_of("WebFetch", &json!({"url": "https://docs.rs/serde?x=1"})),
            "WebFetch(domain:docs.rs)"
        );
        assert_eq!(rule_of("Task", &json!({})), "Task");
    }

    #[test]
    fn asks_in_one_line_about_the_call() {
        let denial = Denial {
            tool: "Bash".into(),
            input: json!({"command": "docker system prune\n-f"}),
            tool_use_id: "t".into(),
            rule: "Bash(docker system prune)".into(),
            why: Why::Asked,
        };
        assert_eq!(
            question_of("360-x", &denial),
            "may 360-x run Bash: docker system prune?"
        );
        let task = Denial {
            input: Value::Null,
            tool: "Task".into(),
            ..denial
        };
        assert_eq!(question_of("360-x", &task), "may 360-x use Task?");
    }

    #[test]
    fn an_allow_runs_again_with_the_rule() {
        let (line, allowed) = answer_run(ALLOW_ONCE, "Bash(x)");
        assert!(line.contains("allowed Bash(x)"));
        assert_eq!(allowed, vec!["Bash(x)"]);
        assert_eq!(answer_run(ALLOW_ALWAYS, "Bash(x)").1, vec!["Bash(x)"]);
        let (line, allowed) = answer_run(DENY, "Bash(x)");
        assert!(line.contains("denied Bash(x)"));
        assert!(allowed.is_empty());
    }

    #[test]
    fn allow_always_writes_the_worktrees_own_settings_once() {
        let dir = std::env::temp_dir().join(format!("permissions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claude")).unwrap();
        fs::write(
            local_settings(&dir),
            r#"{"statusLine": {"type": "command"}, "permissions": {"allow": ["Bash(ls)"]}}"#,
        )
        .unwrap();
        allow_always(&dir, "Bash(docker system prune)").unwrap();
        allow_always(&dir, "Bash(docker system prune)").unwrap();
        let settings: Value = serde_json::from_str(&fs::read_to_string(local_settings(&dir)).unwrap()).unwrap();
        assert_eq!(
            settings["permissions"]["allow"],
            json!(["Bash(ls)", "Bash(docker system prune)"])
        );
        assert_eq!(settings["statusLine"]["type"], "command");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_question_becomes_a_permission_action_and_allow_always_its_rule() {
        let dir = std::env::temp_dir().join(format!("permissions-post-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claude")).unwrap();
        let lines = stream(&[
            ("t1", "git tag v1", &rule_text("git tag v1")),
            ("t2", "docker system prune", &rule_text("docker system prune")),
        ]);
        let posted = post_denials(&dir, "360-x", &denials_of(&lines, &deny())).unwrap();
        assert_eq!(posted.len(), 1, "the deny rule's refusal asks nothing");
        let actions = actions_of(&actions_file(&dir));
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        assert_eq!(action.kind, "permission");
        assert_eq!(action.text, "may 360-x run Bash: docker system prune?");
        assert_eq!(crate::actions::answers_of(action), PERMISSION_ANSWERS);
        assert_eq!(action.rule.as_deref(), Some("Bash(docker system prune)"));
        assert_eq!(delivery_of(&dir, action).unwrap(), None, "not answered yet");

        // The same question again replaces it.
        post_denials(&dir, "360-x", &denials_of(&lines, &deny())).unwrap();
        assert_eq!(actions_of(&actions_file(&dir)).len(), 1);

        let id = actions_of(&actions_file(&dir))[0].id;
        answer_action(&actions_file(&dir), id, ALLOW_ALWAYS).unwrap();
        let answered = &actions_of(&actions_file(&dir))[0];
        let (line, allowed) = delivery_of(&dir, answered).unwrap().unwrap();
        assert!(line.starts_with("[overview] the user allowed Bash(docker system prune), go on"));
        assert_eq!(allowed, vec!["Bash(docker system prune)"]);
        let settings: Value = serde_json::from_str(&fs::read_to_string(local_settings(&dir)).unwrap()).unwrap();
        assert_eq!(settings["permissions"]["allow"], json!(["Bash(docker system prune)"]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_the_deny_rules_of_every_settings_file() {
        let root = std::env::temp_dir().join(format!("permissions-deny-{}", std::process::id()));
        let (dir, home) = (root.join("tree"), root.join("home"));
        for at in [&dir, &home] {
            fs::create_dir_all(at.join(".claude")).unwrap();
        }
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"permissions": {"deny": ["Bash(a:*)"]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join(".claude/settings.json"),
            r#"{"permissions": {"deny": ["Bash(b:*)"]}}"#,
        )
        .unwrap();
        assert_eq!(deny_rules(&dir, &home), vec!["Bash(a:*)", "Bash(b:*)"]);
        fs::remove_dir_all(&root).unwrap();
    }
}
