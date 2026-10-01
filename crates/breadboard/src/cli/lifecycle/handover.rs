//! `wt:orchestrate <name> <issue> --to <agent> [--limit <percent>] [--after
//! !<mr>…] [--dry-run] [--shared] [--peer <session>] [--brief <file>]`, the
//! one handover (cli/lifecycle.rs routes it here)
//! (worktree/commands/handover.mjs, workers.mjs, history.mjs): the issue goes
//! to an idle agent of the orchestration instead of a fresh one. Its
//! worktree is made as for any issue, the brief says what the agent had
//! before, then the agent moves in, is renamed after the feature (#251) and
//! is told in one line. With `--after` the handover is only announced: the
//! overview does it once the MRs are merged (#227).

mod brief;

use super::args::{Args, Takes};
use super::claude::{bound_session, claude_dir, context_share_of, session_file};
use super::gitlab;
use super::herdr::{self, call, did, id_at, s};
use super::layout::spaces::space_of;
use super::layout::tabs::{EDITOR, OVERVIEW, ROOT, drop_duplicates, keep_order};
use super::plugin;
use super::proc::{Done, Fail, exe, fail, git, step, text};
use super::repo::Repo;
use super::setups::{Claim, claim_setup, setup_text};
use breadboard_core::git::branch_files;
use breadboard_core::json::{self, int_of, str_of};
use breadboard_core::lock::is_alive;
use breadboard_core::record::{self, is_name, origin_of};
use breadboard_core::time::{now_ms, to_iso};
pub use brief::brief_of;
use serde_json::{Map, Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: pnpm wt:orchestrate <name> <issue> --to <agent> [--after !<mr>…] [--limit <percent>] [--shared] [--peer <session>] [--brief <file>] [--dry-run]";
const LIMIT_USAGE: &str = "usage: pnpm wt:orchestrate <name> <issue> --to <agent> [--limit <percent>]";
/// The share of its context above which an agent gets no next issue: it would run out in the middle of it.
pub const CONTEXT_LIMIT: f64 = 0.5;
/// The flags whose next word is their value, not an issue.
const VALUED: [&str; 4] = ["--peer", "--brief", "--to", "--limit"];
const TAKES: Takes = Takes {
    switches: &["--dry-run", "--shared"],
    valued: &VALUED,
    lists: &["--after"],
    // `--to hands over one issue at a time` says more than "one argument too many".
    words: usize::MAX,
    about: "Hands the issue to an idle agent of the orchestration <name> instead of a fresh one: its worktree is made, the agent moves into it, is renamed after it and told in its prompt. --after !<mr>… only announces it: the overview hands it over once those MRs are merged. --limit: the share of its context (default 50 %) above which the agent gets no next issue. --dry-run shows the brief and changes nothing.",
    usage: USAGE,
};
/// The flags of `wt:orchestrate` that start or queue fresh agents: none of them goes with `--to`.
const NOT_WITH_TO: [&str; 5] = ["--title", "--queue", "--now", "--cap", "--watch"];

/// The values of `--after` and where they stand: `--after=!232,!233` one,
/// `--after !232 !233` the `!` words behind it (handovers.mjs `afterArgs`).
/// The word right behind it is its value even without the `!` (`--after
/// 232`, a typo `x308`): parse_after says what is wrong with it, where
/// today's took it for a second issue.
pub fn after_args(words: &[String]) -> Option<(Vec<String>, Vec<usize>)> {
    let at = words
        .iter()
        .position(|word| word == "--after" || word.starts_with("--after="))?;
    if let Some(inline) = words[at].strip_prefix("--after=") {
        return Some((vec![inline.to_string()], vec![at]));
    }
    let behind: Vec<usize> = (at + 1..words.len())
        .take_while(|&i| words[i].starts_with('!') || (i == at + 1 && !words[i].starts_with("--")))
        .collect();
    let mut places = vec![at];
    places.extend(&behind);
    Some((behind.iter().map(|&i| words[i].clone()).collect(), places))
}

/// The MRs of `--after`: `!232`, `232`, `!232,!233`, each once, in order.
pub fn parse_after(values: &[String]) -> Result<Vec<i64>, Fail> {
    let mut after = Vec::new();
    for value in values.iter().flat_map(|value| value.split(',')) {
        let iid = value.trim().strip_prefix('!').unwrap_or(value.trim());
        let Some(iid) = iid
            .parse::<i64>()
            .ok()
            .filter(|_| !iid.is_empty() && iid.bytes().all(|b| b.is_ascii_digit()))
        else {
            return fail(format!("{value} is no MR (!232)"));
        };
        if !after.contains(&iid) {
            after.push(iid);
        }
    }
    if after.is_empty() {
        return fail("--after names the MRs to wait for: --after !232");
    }
    Ok(after)
}

/// An issue of `wt:orchestrate` as GitLab has it, with the worktree it gets.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub number: i64,
    pub title: String,
    pub feature: String,
    pub is_fix: bool,
    pub dir: PathBuf,
    /// Why it can't get a worktree: closed, there already, claimed on origin.
    pub why: Option<String>,
    pub branch: String,
}

const STOPWORDS: [&str; 15] = [
    "a", "an", "the", "of", "in", "on", "for", "to", "and", "or", "with", "by", "at", "from", "is",
];

/// A letter with its accent dropped (NFKD without the marks) for the Latin
/// letters titles use; None for a combining mark itself.
fn base_of(c: char) -> Option<char> {
    let folded = match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        '\u{300}'..='\u{36f}' => return None,
        other => other,
    };
    Some(folded)
}

/// `Crème brûlée: the fix!` → `creme-brulee-fix`: up to five words without
/// the small ones, at most 40 characters (orchestrator.mjs `slugOf`).
pub fn slug_of(title: &str) -> String {
    let plain: String = title
        .to_lowercase()
        .chars()
        .filter_map(base_of)
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                ' '
            }
        })
        .collect();
    let words: Vec<&str> = plain
        .split(' ')
        .filter(|word| !word.is_empty() && !STOPWORDS.contains(word))
        .collect();
    let Some(first) = words.first() else {
        return "issue".into();
    };
    let mut slug: String = first.chars().take(40).collect();
    for word in words.iter().skip(1).take(4) {
        if slug.len() + 1 + word.len() > 40 {
            break;
        }
        slug = format!("{slug}-{word}");
    }
    slug
}

/// The branch on origin that claims the issue of `feature` already, if one does.
fn claimed_elsewhere(repo: &Repo, feature: &str) -> Option<String> {
    let number: String = feature.chars().take_while(char::is_ascii_digit).collect();
    if number.is_empty() || !feature[number.len()..].starts_with('-') {
        return None;
    }
    let heads = git(
        &repo.main,
        &["ls-remote", "--heads", "origin", &format!("*{number}-*")],
        None,
    );
    heads.lines().find_map(|line| {
        line.split("refs/heads/")
            .nth(1)
            .filter(|branch| !branch.is_empty())
            .map(String::from)
    })
}

/// The issue `arg` (its number, or `<n>-<feature>`) as GitLab has it; None
/// when it can't be read (orchestrator.mjs `readIssue`).
pub fn read_issue(repo: &Repo, arg: &str) -> Option<Issue> {
    let number: String = arg.chars().take_while(char::is_ascii_digit).collect();
    let found = gitlab::view(&repo.main, &["issue", "view", &number])?;
    let title = str_of(&found, "title").unwrap_or_default().to_string();
    let feature = if number == arg {
        format!("{number}-{}", slug_of(&title))
    } else {
        arg.to_string()
    };
    let is_fix = found
        .get("labels")
        .and_then(Value::as_array)
        .is_some_and(|labels| labels.iter().any(|l| l == "bug"));
    let dir = repo.dir_of(&feature);
    let state = str_of(&found, "state").unwrap_or_default();
    let why = if state != "opened" {
        Some(format!("#{number} is {state}"))
    } else if dir.exists() {
        Some(format!(
            "{} exists already",
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))
    } else {
        claimed_elsewhere(repo, &feature).map(|branch| format!("claimed by {branch}"))
    };
    Some(Issue {
        number: number.parse().ok()?,
        title,
        branch: format!("{}/{feature}", if is_fix { "fix" } else { "feature" }),
        feature,
        is_fix,
        dir,
        why,
    })
}

/// An issue as the record keeps it.
pub fn record_entry(issue: &Issue, why: Option<&str>) -> Value {
    json!({ "number": issue.number, "title": issue.title, "feature": issue.feature, "branch": issue.branch, "why": why })
}

/// The record's entry of an issue whose start runs or failed (#272): a
/// queued one stays queued at its place, with `requeued` why it is back.
pub fn start_entry(issue: &Issue, known: Option<&Value>, why: Option<&str>) -> Value {
    if !known.is_some_and(|known| json::is_truthy(known.get("queued"))) {
        return record_entry(issue, why.map(|why| format!("failed: {why}")).as_deref());
    }
    let mut entry = record_entry(issue, None);
    let at = known
        .and_then(|known| str_of(known, "queuedAt"))
        .map(String::from)
        .unwrap_or_else(|| to_iso(now_ms()));
    if let Some(entry) = entry.as_object_mut() {
        entry.insert("queued".into(), json!(true));
        entry.insert("queuedAt".into(), json!(at));
        if let Some(why) = why {
            entry.insert("requeued".into(), json!(why));
        }
    }
    entry
}

/// An agent of the orchestration: one per worktree of it with a conversation
/// bound, then the free ones in the orchestration's checkout (workers.mjs).
#[derive(Debug, Clone, PartialEq)]
pub struct Worker {
    pub dir: PathBuf,
    pub feature: Option<String>,
    pub id: String,
    pub name: String,
    pub was: Option<String>,
    pub is_free: bool,
}

pub fn workers_of(repo: &Repo, name: &str) -> Vec<Worker> {
    let record = record::record_of(&repo.main, name);
    let known = |id: &str| record.as_ref().and_then(|record| record.worker(id).cloned());
    let bound: Vec<Worker> = repo
        .trees()
        .into_iter()
        .filter(|tree| origin_of(&tree.dir).is_some_and(|origin| origin.name == name))
        .filter_map(|tree| {
            let id = bound_session(&tree.dir)?;
            let feature = repo.feature_of(&tree.dir);
            let entry = known(&id);
            Some(Worker {
                name: entry
                    .as_ref()
                    .and_then(|entry| entry.name.clone())
                    .unwrap_or_else(|| feature.clone()),
                was: entry.and_then(|entry| entry.was),
                dir: tree.dir,
                feature: Some(feature),
                id,
                is_free: false,
            })
        })
        .collect();
    let free: Vec<Worker> = record
        .as_ref()
        .map(|record| record.free_agents())
        .unwrap_or_default()
        .into_iter()
        .filter(|worker| !bound.iter().any(|known| known.id == worker.id))
        .map(|worker| Worker {
            dir: repo.checkout_of(name),
            feature: None,
            id: worker.id.clone(),
            name: worker.name.clone().unwrap_or_default(),
            was: worker.was.clone(),
            is_free: true,
        })
        .collect();
    bound.into_iter().chain(free).collect()
}

/// The worker `who` names: its name, else its worktree's feature, else its
/// name before the last handover.
pub fn find_worker<'a>(workers: &'a [Worker], who: &str) -> Option<&'a Worker> {
    workers
        .iter()
        .find(|worker| worker.name == who)
        .or_else(|| workers.iter().find(|worker| worker.feature.as_deref() == Some(who)))
        .or_else(|| workers.iter().find(|worker| worker.was.as_deref() == Some(who)))
}

/// The pane herdr runs the conversation `id` in.
fn pane_of_conversation(id: &str) -> Option<Value> {
    herdr::all_panes().into_iter().find(|pane| {
        pane.get("agent_session")
            .and_then(|session| session.get("value"))
            .and_then(Value::as_str)
            == Some(id)
    })
}

/// Why an agent can't take a next issue, or None: no pane runs it, it works
/// or waits, herdr can't tell, or its context `share` is above `limit`.
pub fn refusal_of(pane: Option<&Value>, share: Option<f64>, limit: f64) -> Option<String> {
    let Some(pane) = pane else {
        return Some("no claude runs it (herdr knows no pane with its conversation)".into());
    };
    let status = pane.get("agent_status").and_then(Value::as_str).unwrap_or("unknown");
    match status {
        "working" => return Some("it is working".into()),
        "blocked" => return Some("it waits for an answer".into()),
        "idle" | "done" => {}
        other => return Some(format!("herdr can't tell what it does ({other})")),
    }
    let share = share.filter(|share| *share > limit)?;
    let percent = |value: f64| (value * 100.0).round() as i64;
    Some(format!(
        "its context is at {} %, above the limit of {} %",
        percent(share),
        percent(limit)
    ))
}

/// The line an agent gets its next issue with; its new name comes with it (#251).
pub fn next_issue_line(number: i64, feature: &str, dir: &Path) -> String {
    format!(
        "[overview] next issue #{number}, your name is {feature} now: EnterWorktree(path: \"{}\"), then read .claude/brief.md",
        text(dir)
    )
}

/// The line typed before it: Claude Code's own name of the conversation.
pub fn rename_line(feature: &str) -> String {
    format!("/rename {feature}")
}

/// The record of `name` as its file has it, for writes that keep the rest.
fn raw_record(repo: &Repo, name: &str) -> Value {
    json::read(&record::record_file(&repo.main, name)).unwrap_or(Value::Null)
}

fn worker_entry(raw: &Value, id: &str) -> Value {
    raw.get("workers")
        .and_then(|workers| workers.get(id))
        .cloned()
        .unwrap_or(Value::Null)
}

/// The issues `worker` worked on, as the record knows them: `(number, title)`.
pub fn issues_before(raw: &Value, worker: &Worker) -> Vec<(i64, Option<String>)> {
    let issues = raw.get("issues").and_then(Value::as_array).cloned().unwrap_or_default();
    let numbers: Vec<i64> = match worker_entry(raw, &worker.id).get("issues").and_then(Value::as_array) {
        Some(numbers) => numbers.iter().filter_map(Value::as_i64).filter(|n| *n != 0).collect(),
        None => issues
            .iter()
            .find(|issue| worker.feature.is_some() && str_of(issue, "feature") == worker.feature.as_deref())
            .and_then(|issue| int_of(issue, "number"))
            .into_iter()
            .collect(),
    };
    numbers
        .into_iter()
        .map(|number| {
            let title = issues
                .iter()
                .find(|issue| int_of(issue, "number") == Some(number))
                .and_then(|i| str_of(i, "title"));
            (number, title.map(String::from))
        })
        .collect()
}

/// What the brief of a reused agent says of its issues before, in one line.
pub fn before_line(issues: &[(i64, Option<String>)]) -> Option<String> {
    if issues.is_empty() {
        return None;
    }
    let list: Vec<String> = issues
        .iter()
        .map(|(number, title)| match title {
            Some(title) if !title.is_empty() => format!("#{number} ({title})"),
            _ => format!("#{number}"),
        })
        .collect();
    Some(format!(
        "You are a reused agent: you worked on {} before. Treat this issue on its own, from develop as it is now: nothing of that work carries over but what is merged.",
        list.join(", ")
    ))
}

/// The issues of a worker entry in order (history.mjs `historyOf`): its
/// `history`, else its `issues`, the first started fresh, the others handed over.
fn history_of(entry: &Value) -> Vec<Value> {
    if let Some(history) = entry.get("history").and_then(Value::as_array) {
        return history.clone();
    }
    let issues = entry
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    issues
        .into_iter()
        .enumerate()
        .map(|(i, number)| {
            json!({ "number": number, "how": if i == 0 { "fresh" } else { "handover" }, "at": null, "context": null })
        })
        .collect()
}

/// `entry` with the issue `start` as its latest, once (history.mjs `withStart`).
pub fn with_start(entry: &Value, start: Value) -> Map<String, Value> {
    let number = start.get("number").cloned();
    let mut history: Vec<Value> = history_of(entry)
        .into_iter()
        .filter(|known| known.get("number") != number.as_ref())
        .collect();
    let mut issues: Vec<Value> = history
        .iter()
        .filter_map(|known| known.get("number").cloned())
        .collect();
    issues.extend(number);
    history.push(start);
    let mut fields = entry.as_object().cloned().unwrap_or_default();
    fields.insert("history".into(), Value::Array(history));
    fields.insert("issues".into(), Value::Array(issues));
    fields
}

/// `failed` of `entry` with a failed handover: the same again counts its `times`.
pub fn with_failure(entry: &Value, number: i64, reason: &str, at: &str) -> Value {
    let mut failed = entry
        .get("failed")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let same = failed
        .iter_mut()
        .find(|known| int_of(known, "number") == Some(number) && str_of(known, "reason") == Some(reason));
    match same {
        Some(known) => {
            let times = int_of(known, "times").unwrap_or(1) + 1;
            if let Some(known) = known.as_object_mut() {
                known.insert("at".into(), json!(at));
                known.insert("times".into(), json!(times));
            }
        }
        None => failed.push(json!({ "number": number, "reason": reason, "at": at, "times": 1 })),
    }
    Value::Array(failed)
}

/// The record learns that the agent `id` started on `number`, `how` and on what context.
fn record_start(repo: &Repo, name: &str, id: &str, number: i64, how: &str, context: Option<f64>) {
    let entry = worker_entry(&raw_record(repo, name), id);
    let start = json!({ "number": number, "how": how, "at": to_iso(now_ms()), "context": context });
    let _ = record::write_worker(&repo.main, name, id, with_start(&entry, start));
}

/// The record learns that a handover of `number` to the agent `id` failed, and why.
fn record_failure(repo: &Repo, name: &str, id: &str, number: i64, reason: &str) {
    let raw = raw_record(repo, name);
    if record::Record::from_value(raw.clone()).is_none() {
        return;
    }
    let failed = with_failure(&worker_entry(&raw, id), number, reason, &to_iso(now_ms()));
    let mut fields = Map::new();
    fields.insert("failed".into(), failed);
    let _ = record::write_worker(&repo.main, name, id, fields);
}

/// The record learns that `worker` works on `number` in `dir` now, under its
/// feature's name; `renamed` (when) only once its `/rename` took (#313).
fn record_handover(repo: &Repo, name: &str, worker: &Worker, issue: &Issue, has_rename_taken: bool) {
    let raw = raw_record(repo, name);
    let known = worker_entry(&raw, &worker.id);
    let at = to_iso(now_ms());
    let is_renamed = issue.feature != worker.name;
    let mut files: Vec<String> = json::strs_of(&known, "files");
    if !worker.is_free {
        for file in branch_files(&worker.dir) {
            if !files.contains(&file) {
                files.push(file);
            }
        }
    }
    let mut issues: Vec<i64> = issues_before(&raw, worker)
        .into_iter()
        .map(|(number, _)| number)
        .filter(|n| *n != issue.number)
        .collect();
    issues.push(issue.number);
    let renamed = if is_renamed && has_rename_taken {
        json!(at)
    } else {
        known.get("renamed").cloned().unwrap_or(Value::Null)
    };
    let was = if is_renamed {
        json!(worker.name)
    } else {
        json!(worker.was)
    };
    let fields = json!({
        // Free no more (#289): it works on an issue again.
        "free": null, "name": issue.feature, "was": was, "renamed": renamed, "issues": issues, "files": files,
    });
    let _ = record::write_worker(
        &repo.main,
        name,
        &worker.id,
        fields.as_object().cloned().unwrap_or_default(),
    );
}

/// What wt:new left: whether the worktree is there and the reason it failed with.
struct Made {
    is_there: bool,
    failed: Option<String>,
}

/// `wt:new <feature> --origin <name>` for the issue, its output shown; it
/// may fail after the setup (#130), so the worktree counts when it is there.
fn make_worktree(repo: &Repo, issue: &Issue, name: &str, is_shared: bool) -> Made {
    let mut args = vec!["wt:new", issue.feature.as_str(), "--origin", name];
    if issue.is_fix {
        args.push("--fix");
    }
    if is_shared {
        args.push("--shared");
    }
    let _ = std::io::stdout().flush();
    let made = Command::new(exe())
        .args(&args)
        .current_dir(&repo.main)
        .stdin(Stdio::null())
        // Its steps show as they run; its failure is read from stderr.
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .output();
    let is_ok = made.as_ref().is_ok_and(|made| made.status.success());
    let errors = made
        .map(|made| String::from_utf8_lossy(&made.stderr).into_owned())
        .unwrap_or_default();
    if !errors.trim().is_empty() {
        let _ = std::io::stderr().write_all(errors.as_bytes());
    }
    Made {
        is_there: repo.worktrees().iter().any(|tree| tree.dir == issue.dir),
        // wt:new's failure says `✗ <reason>`.
        failed: errors
            .lines()
            .find_map(|line| line.strip_prefix("✗ "))
            .map(String::from)
            .or_else(|| (!is_ok).then(|| "wt:new failed".to_string())),
    }
}

/// A start that failed once its worktree was made (#272): no worktree stays
/// without its agent, `wt:remove` takes it and its empty claim; a queued
/// issue goes back to the queue. The claim goes first, or wt:remove refuses.
fn undo_start(repo: &Repo, name: &str, issue: &Issue, known: Option<&Value>, why: &str, claim: &mut Claim) {
    claim.release();
    if repo.worktrees().iter().any(|tree| tree.dir == issue.dir) {
        let _ = std::io::stdout().flush();
        let _ = Command::new(exe())
            .args(["wt:remove", &issue.feature])
            .current_dir(&repo.main)
            .stdin(Stdio::null())
            .status();
    }
    let _ = record::write_record(&repo.main, name, &[start_entry(issue, known, Some(why))]);
}

/// How long a `/rename` gets to show in the agent's session before it is
/// typed again, and once more (#313).
const RENAME_WAIT: Duration = Duration::from_secs(10);

/// What the session of the conversation `id` is named now: Some(name) from
/// its file in `~/.claude/sessions`, None when no running claude has it.
fn session_name(id: &str) -> Option<String> {
    let entries = fs::read_dir(claude_dir().join("sessions")).ok()?;
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
        .find_map(|entry| {
            let session = json::read(&entry.path())?;
            let is_it = str_of(&session, "sessionId") == Some(id) && int_of(&session, "pid").is_some_and(is_alive);
            is_it.then(|| str_of(&session, "name").unwrap_or_default().to_string())
        })
}

/// Whether a `/rename` took.
#[derive(Debug, PartialEq)]
pub enum Renamed {
    Yes,
    /// No running claude shows its session: it can't be checked.
    Unknown,
    /// Still named so after the wait.
    No(String),
}

/// Waits until the session's name reads `feature` (#313).
pub fn wait_renamed(feature: &str, wait: Duration, name_now: impl Fn() -> Option<String>) -> Renamed {
    let started = Instant::now();
    loop {
        match name_now() {
            Some(name) if name == feature => return Renamed::Yes,
            // No running claude shows the conversation: nothing to wait for.
            None => return Renamed::Unknown,
            Some(name) if started.elapsed() >= wait => return Renamed::No(name),
            Some(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Whether and how the agent was told.
#[derive(Debug, PartialEq)]
enum Told {
    Yes,
    /// herdr took no line.
    No,
    /// Its `/rename` never showed: the next-issue line was not typed (#313).
    NotRenamed(String),
}

/// Renames the agent in pane `pane` and gives it its next issue: `/rename`
/// first, typed again when its session doesn't show the name, the pane's
/// label, then the line, only once the rename is done (#313).
fn tell(pane: &str, worker: &Worker, issue: &Issue) -> Told {
    let rename = rename_line(&issue.feature);
    if !did(&["agent", "prompt", pane, &rename]) {
        return Told::No;
    }
    let name_now = || session_name(&worker.id);
    // Said at once: the wait may take its whole 2 × RENAME_WAIT.
    if name_now().is_some_and(|now| now != issue.feature) {
        step(&format!("{rename} typed, waiting for its session to show the name"));
    }
    let mut renamed = wait_renamed(&issue.feature, RENAME_WAIT, name_now);
    if let Renamed::No(now) = &renamed {
        step(&format!(
            "its session still shows {}: {rename} typed again",
            if now.is_empty() { "no name" } else { now }
        ));
        did(&["agent", "prompt", pane, &rename]);
        renamed = wait_renamed(&issue.feature, RENAME_WAIT, name_now);
    }
    match renamed {
        Renamed::No(now) => return Told::NotRenamed(now),
        // Without a session file to read it can't be checked: the line follows at once, as today's did.
        Renamed::Unknown | Renamed::Yes => {}
    }
    did(&["pane", "rename", pane, &issue.feature]);
    if did(&[
        "agent",
        "prompt",
        pane,
        &next_issue_line(issue.number, &issue.feature, &issue.dir),
    ]) {
        Told::Yes
    } else {
        Told::No
    }
}

/// The agent moves to the worktree of `issue` (made, its brief written): its
/// pane into the worktree's space as the tab `<feature>`, the conversation
/// bound to the new worktree instead of the old one, then renamed and told.
fn move_worker(repo: &Repo, worker: &Worker, pane: &Value, issue: &Issue) -> (bool, Told) {
    let feature = issue.feature.as_str();
    let mut is_moved = false;
    // A pane moved into another space gets a new id there; the old one takes no prompt (#222).
    let mut target = s(pane, "pane_id").to_string();
    if let Some(workspace) = space_of(repo, &issue.dir) {
        let moved = call(&["pane", "move", &target, "--workspace", &workspace, "--new-tab"]);
        if let Some(moved) = moved {
            target = id_at(&moved, &["move_result", "pane", "pane_id"]).unwrap_or(target);
            if let Some(created) = id_at(&moved, &["move_result", "created_tab", "tab_id"]) {
                call(&["tab", "rename", &created, feature]);
                drop_duplicates(&workspace, feature);
                keep_order(&workspace, &[ROOT, OVERVIEW, feature, EDITOR]);
                is_moved = true;
            }
        }
    }
    // The binding follows the conversation: a restart resumes it in its new space.
    let _ = fs::write(session_file(&issue.dir), format!("{}\n", worker.id));
    // A free agent's checkout is the orchestration's: no binding of its own there.
    if !worker.is_free {
        let _ = fs::remove_file(session_file(&worker.dir));
    }
    let mut told = tell(&target, worker, issue);
    if told == Told::No {
        // Without the new id in herdr's answer: the pane that runs the conversation now.
        let now = pane_of_conversation(&worker.id).map(|pane| s(&pane, "pane_id").to_string());
        if let Some(now) = now.filter(|now| *now != target) {
            told = tell(&now, worker, issue);
        }
    }
    (is_moved, told)
}

/// What the command line gives a handover.
pub struct Request<'a> {
    pub name: &'a str,
    pub issue: &'a str,
    pub to: &'a str,
    pub limit: Option<&'a str>,
    pub peer: Option<String>,
    /// The text of this run's `--brief`; without one the issue's kept brief.
    pub extra: Option<String>,
    pub is_shared: bool,
    pub is_dry: bool,
}

/// Whether `wt:orchestrate <words>` is a handover: `--to` given, and none
/// of the modes `wt:orchestrate` reads before it (`--end`, `--changelog`,
/// `--changes`).
pub fn is_handover(words: &[String]) -> bool {
    let is_given = |flag: &str| {
        words
            .iter()
            .any(|word| word == flag || word.strip_prefix(flag).is_some_and(|rest| rest.starts_with('=')))
    };
    is_given("--to") && !["--end", "--changelog", "--changes"].iter().any(|mode| is_given(mode))
}

/// `wt:orchestrate <name> <issue> --to <agent> …`: the command line held
/// against what a handover takes before anything is read or written: a
/// typo (`--dryrun`) must not hand over for real.
pub fn command(args: &Args) -> Done {
    if let Some(flag) = args.words().iter().find_map(|word| {
        let name = word.split('=').next().unwrap_or(word);
        NOT_WITH_TO.contains(&name).then_some(name)
    }) {
        return fail(format!(
            "{flag} does not go with --to: a handover gives one issue to one idle agent (--after !<mr>: once those MRs are merged)\n{USAGE}"
        ));
    }
    if args.check(&TAKES)? {
        return Ok(());
    }
    let words = args.words();
    let after = after_args(words);
    let places = after.as_ref().map(|(_, at)| at.clone()).unwrap_or_default();
    let positional: Vec<&str> = words
        .iter()
        .enumerate()
        .filter(|(i, word)| {
            !word.starts_with("--") && !places.contains(i) && !(*i > 0 && VALUED.contains(&words[i - 1].as_str()))
        })
        .map(|(_, word)| word.as_str())
        .collect();
    let Some((name, issues)) = positional
        .split_first()
        .filter(|(name, issues)| is_name(name) && !issues.is_empty())
    else {
        return fail(USAGE);
    };
    let is_issue = |arg: &str| {
        let digits = arg.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0 && (digits == arg.len() || (is_name(arg) && arg.as_bytes()[digits] == b'-'))
    };
    if let Some(issue) = issues.iter().find(|issue| !is_issue(issue)) {
        return fail(format!("{issue} is no issue (12, or 12-<feature>)\n{USAGE}"));
    }
    let brief_file = args.value("--brief");
    if let Some(file) = brief_file.filter(|file| !Path::new(file).exists()) {
        return fail(format!("{file} does not exist"));
    }
    let after = after.map(|(values, _)| parse_after(&values)).transpose()?;
    let Some(to) = args.value("--to") else {
        return fail(USAGE);
    };
    if issues.len() != 1 {
        return fail("--to hands over one issue at a time");
    }
    let repo = Repo::here()?;
    let is_dry = args.has("--dry-run");
    if !is_dry {
        plugin::heal(&repo.main);
    }
    // A --brief applies to the issues of the run that passes it only (#247).
    let extra = match brief_file {
        Some(file) => {
            Some(fs::read_to_string(file).map_err(|error| Fail(format!("{file} could not be read: {error}")))?)
        }
        None => None,
    };
    if let Some(after) = after {
        return announce(&repo, name, issues[0], to, &after, extra.as_deref(), is_dry);
    }
    let record = record::record_of(&repo.main, name);
    let peer = args
        .value("--peer")
        .map(String::from)
        .or_else(|| record.and_then(|record| record.peer));
    hand_over(
        &repo,
        &Request {
            name,
            issue: issues[0],
            to,
            limit: args.value("--limit"),
            peer: peer.filter(|peer| !peer.is_empty()),
            extra,
            is_shared: args.has("--shared"),
            is_dry,
        },
    )
}

/// The brief an earlier run gave the issue `number`: `briefs` of the record
/// (#247, written by wt:orchestrate), "" without one.
fn kept_brief(raw: &Value, number: i64) -> String {
    raw.get("briefs")
        .and_then(|briefs| briefs.get(number.to_string()))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// `text` kept as the brief of the issue `number` for its later start (#247).
fn keep_brief(repo: &Repo, name: &str, number: i64, text: &str) {
    let mut briefs = raw_record(repo, name)
        .get("briefs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    briefs.insert(number.to_string(), json!(text));
    let mut fields = Map::new();
    fields.insert("briefs".into(), Value::Object(briefs));
    let _ = record::update_record(&repo.main, name, fields);
}

/// `<label> has no agent <who>; its agents: …`.
fn no_agent(label: &str, who: &str, workers: &[Worker]) -> Fail {
    let names: Vec<&str> = workers.iter().map(|worker| worker.name.as_str()).collect();
    Fail(format!(
        "{label} has no agent {who}; its agents: {}",
        if names.is_empty() {
            "none".into()
        } else {
            names.join(", ")
        }
    ))
}

/// The issue goes to an idle agent of the orchestration (handover.mjs).
pub fn hand_over(repo: &Repo, request: &Request) -> Done {
    let name = request.name;
    let label = repo.orchestrator_label(name);
    if !herdr::is_in() {
        return fail("--to works inside herdr only: the agents are found through it");
    }
    let limit = match request.limit {
        None => CONTEXT_LIMIT,
        Some(percent) => percent
            .trim()
            .parse::<f64>()
            .map(|percent| percent / 100.0)
            .unwrap_or(f64::NAN),
    };
    if !(limit > 0.0 && limit <= 1.0) {
        return fail(LIMIT_USAGE);
    }
    let workers = workers_of(repo, name);
    let Some(worker) = find_worker(&workers, request.to).cloned() else {
        return Err(no_agent(&label, request.to, &workers));
    };
    let share = context_share_of(&worker.id);
    let refusal = refusal_of(pane_of_conversation(&worker.id).as_ref(), share, limit);
    let Some(issue) = read_issue(repo, request.issue) else {
        let number: String = request.issue.chars().take_while(char::is_ascii_digit).collect();
        return fail(format!("issue #{number} could not be read"));
    };
    let number = issue.number;
    // The handover fails: in the agent's history (not on a dry run), then said.
    let failed = |reason: &str| -> Done {
        if !request.is_dry {
            record_failure(repo, name, &worker.id, number, reason);
        }
        fail(format!("{} gets no next issue: {reason}", worker.name))
    };
    if let Some(refusal) = refusal {
        return failed(&refusal);
    }
    // One start of an issue at a time (#272).
    let mut claim = if request.is_dry {
        Claim::none()
    } else {
        let dir = text(&issue.dir);
        let from = text(&worker.dir);
        match claim_setup(&repo.main, name, number, &dir, Some(&worker.name), Some(&from)) {
            Ok(Ok(claim)) => claim,
            Ok(Err(holder)) => {
                let what = holder
                    .map(|holder| setup_text(&holder))
                    .unwrap_or_else(|| "another start".into());
                return fail(format!("#{number} is set up already: {what}"));
            }
            Err(error) => return fail(format!("the setup of #{number} could not be claimed: {error}")),
        }
    };
    // Its entry before: a queued issue stays queued until its agent moved in.
    let raw = raw_record(repo, name);
    let known = raw.get("issues").and_then(Value::as_array).and_then(|issues| {
        issues
            .iter()
            .find(|entry| int_of(entry, "number") == Some(number))
            .cloned()
    });
    // A worktree of this orchestration without an agent: an earlier try made it.
    let is_made = repo.worktrees().iter().any(|tree| tree.dir == issue.dir)
        && origin_of(&issue.dir).is_some_and(|origin| origin.name == name)
        && bound_session(&issue.dir).is_none();
    if let Some(why) = issue.why.as_ref().filter(|_| !is_made) {
        return fail(format!("#{number}: {why}"));
    }
    let before = before_line(&issues_before(&raw, &worker));
    let extra = request.extra.clone().unwrap_or_else(|| kept_brief(&raw, number));
    let brief = brief_of(
        &issue,
        name,
        &label,
        request.peer.as_deref(),
        &extra,
        before.as_deref(),
        &repo.main,
    );
    if request.is_dry {
        println!(
            "\n▸ nothing was done (--dry-run). {} would get\n  {}\n  with the brief\n",
            worker.name,
            next_issue_line(number, &issue.feature, &issue.dir)
        );
        println!("{brief}");
        return Ok(());
    }

    // wt:new may fail after its setup (#130): the handover goes on, its ✓ says so.
    let mut setup_note = "";
    if !is_made {
        let _ = record::write_record(&repo.main, name, &[start_entry(&issue, known.as_ref(), None)]);
        step(&format!("pnpm wt:new {} --origin {name}", issue.feature));
        let made = make_worktree(repo, &issue, name, request.is_shared);
        if made.is_there && made.failed.is_some() {
            setup_note = " (wt:new failed after the setup, see above)";
        }
        if !made.is_there {
            let reason = made.failed.unwrap_or_else(|| "no worktree".into());
            let why = format!("its worktree failed: {reason}");
            undo_start(repo, name, &issue, known.as_ref(), &why, &mut claim);
            record_failure(repo, name, &worker.id, number, &why);
            return fail(format!("#{number} {}: {reason}", issue.feature));
        }
    }
    // Again: the setup took minutes, the agent may have started on something
    // meanwhile, or gone. Its worktree goes again then (#272).
    let pane = pane_of_conversation(&worker.id);
    if let Some(now) = refusal_of(pane.as_ref(), None, limit) {
        let why = format!(
            "after the setup, {} {}",
            worker.name,
            now.strip_prefix("it ").unwrap_or(&now)
        );
        undo_start(repo, name, &issue, known.as_ref(), &why, &mut claim);
        record_failure(repo, name, &worker.id, number, &format!("after the setup, {now}"));
        let queued = if known.as_ref().is_some_and(|known| json::is_truthy(known.get("queued"))) {
            ", the issue is back in the queue"
        } else {
            ""
        };
        return fail(format!(
            "{why}: the worktree of #{number} is removed again{queued}; run the same command again once it is idle"
        ));
    }
    let Some(pane) = pane else {
        return fail("herdr lost the agent's pane");
    };
    if let Err(error) = record::write_brief(&issue.dir, &brief) {
        return fail(format!("the brief of #{number} could not be written: {error}"));
    }
    let (is_moved, told) = move_worker(repo, &worker, &pane, &issue);
    record_handover(repo, name, &worker, &issue, !matches!(told, Told::NotRenamed(_)));
    let _ = record::write_record(&repo.main, name, &[record_entry(&issue, None)]);
    claim.release();
    let is_announced = record::record_of(&repo.main, name)
        .is_some_and(|record| record.handovers.iter().any(|handover| handover.number == number));
    let feature = issue.feature.as_str();
    let typed = format!(
        "    {}\n    {}",
        rename_line(feature),
        next_issue_line(number, feature, &issue.dir)
    );
    if let Told::NotRenamed(_) = &told {
        // Not counted as done (#313): no start in its history, no ✓.
        record_failure(
            repo,
            name,
            &worker.id,
            number,
            &format!("its /rename to {feature} did not take"),
        );
    } else {
        record_start(
            repo,
            name,
            &worker.id,
            number,
            if is_announced { "announced" } else { "handover" },
            share,
        );
    }
    let _ = record::drop_handover(&repo.main, name, number);
    step(&if is_moved {
        format!("herdr: the tab of {} moved into the space {feature}", worker.name)
    } else {
        format!(
            "herdr: the tab of {} stayed where it is (no space {feature})",
            worker.name
        )
    });
    let renamed = if worker.name == feature {
        String::new()
    } else {
        format!(", renamed from {}", worker.name)
    };
    match told {
        Told::Yes => {
            println!(
                "\n✓ #{number} went to the agent {feature}{renamed}, told in its prompt{setup_note}; SendMessage(to: \"{feature}\")"
            );
            Ok(())
        }
        Told::No => {
            println!(
                "\n✓ #{number} {feature} is ready for {}{setup_note}, but herdr took no line: type these into its tab\n{typed}",
                worker.name
            );
            Ok(())
        }
        Told::NotRenamed(now) => fail(format!(
            "#{number} {feature} is ready for {}, but its /rename did not take (its session is still named {}): the next-issue line was not typed. Type these into its tab, one after the other:\n{typed}",
            worker.name,
            if now.is_empty() { "nothing".into() } else { now }
        )),
    }
}

/// The handover announced, not done (handover.mjs `announceCommand`): the
/// record keeps it, the issue queued; the overview does it once the MRs are
/// merged and the agent is idle.
pub fn announce(
    repo: &Repo,
    name: &str,
    arg: &str,
    to: &str,
    after: &[i64],
    extra: Option<&str>,
    is_dry: bool,
) -> Done {
    let label = repo.orchestrator_label(name);
    let workers = workers_of(repo, name);
    let Some(worker) = find_worker(&workers, to) else {
        return Err(no_agent(&label, to, &workers));
    };
    let Some(issue) = read_issue(repo, arg) else {
        let number: String = arg.chars().take_while(char::is_ascii_digit).collect();
        return fail(format!("issue #{number} could not be read"));
    };
    let number = issue.number;
    let raw = raw_record(repo, name);
    let known = raw.get("issues").and_then(Value::as_array).and_then(|issues| {
        issues
            .iter()
            .find(|entry| int_of(entry, "number") == Some(number))
            .cloned()
    });
    // Queued already is fine: it waits for an agent either way.
    let is_queued = known.as_ref().is_some_and(|known| json::is_truthy(known.get("queued")));
    if let Some(why) = issue.why.as_ref().filter(|_| !is_queued) {
        return fail(format!("#{number}: {why}"));
    }
    let mrs: Vec<String> = after.iter().map(|iid| format!("!{iid}")).collect();
    let mrs = mrs.join(", ");
    let verb = if after.len() > 1 { "are" } else { "is" };
    if is_dry {
        println!(
            "\n▸ nothing was done (--dry-run). {} would take #{number} once {mrs} {verb} merged.",
            worker.name
        );
        return Ok(());
    }
    let mut entry = record_entry(&issue, issue.why.as_deref());
    // Its place in WORKTREES' queue (#250): a queued one keeps it.
    let queued_at = known
        .as_ref()
        .and_then(|known| str_of(known, "queuedAt"))
        .map(String::from);
    if let Some(entry) = entry.as_object_mut() {
        entry.insert("queued".into(), json!(true));
        entry.insert("queuedAt".into(), json!(queued_at.unwrap_or_else(|| to_iso(now_ms()))));
    }
    let _ = record::write_record(&repo.main, name, &[entry]);
    // Its --brief waits with it for the handover (#247), next to the record's issues.
    if let Some(extra) = extra {
        keep_brief(repo, name, number, extra);
    }
    let _ = record::announce_handover(&repo.main, name, number, &worker.name, after, &to_iso(now_ms()));
    println!(
        "\n✓ #{number} {} goes to {} once {mrs} {verb} merged: the overview of {label} hands it over then (its worktree is kept until), shown on {}'s node as → next #{number}",
        issue.feature, worker.name, worker.name
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_keep_five_words_without_the_small_ones() {
        assert_eq!(
            slug_of("Split worktree.mjs and overview.mjs into modules, with tests"),
            "split-worktree-mjs-overview-mjs"
        );
        let slug = slug_of("Internationalisation of the notification preferences dialogue");
        assert_eq!(slug, "internationalisation-notification");
        assert_eq!(slug_of("Crème brûlée: the fix!"), "creme-brulee-fix");
        assert_eq!(slug_of("the — of"), "issue");
        assert_eq!(slug_of("İstanbul Straße"), "istanbul-stra-e");
    }

    #[test]
    fn after_takes_the_mrs_behind_it() {
        let words: Vec<String> = ["o", "12", "--to", "a", "--after", "!3", "!4,!3"]
            .iter()
            .map(|w| w.to_string())
            .collect();
        let (values, at) = after_args(&words).unwrap();
        assert_eq!(at, vec![4, 5, 6]);
        assert_eq!(parse_after(&values).unwrap(), vec![3, 4]);
        let inline: Vec<String> = ["--after=!7"].iter().map(|w| w.to_string()).collect();
        assert_eq!(parse_after(&after_args(&inline).unwrap().0).unwrap(), vec![7]);
        assert_eq!(parse_after(&["x".to_string()]).unwrap_err().0, "x is no MR (!232)");
        // The word right behind it is its value, a typo too; a flag is not.
        let words = |list: &[&str]| list.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let (values, at) = after_args(&words(&["o", "12", "--after", "x308", "13"])).unwrap();
        assert_eq!((values.clone(), at), (vec!["x308".to_string()], vec![2, 3]));
        assert_eq!(parse_after(&values).unwrap_err().0, "x308 is no MR (!232)");
        let (values, _) = after_args(&words(&["o", "12", "--after", "308", "!309"])).unwrap();
        assert_eq!(parse_after(&values).unwrap(), vec![308, 309]);
        assert!(
            after_args(&words(&["o", "12", "--after", "--dry-run"]))
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            parse_after(&[]).unwrap_err().0,
            "--after names the MRs to wait for: --after !232"
        );
    }

    #[test]
    fn refuses_an_agent_at_work_or_full() {
        let pane = |status: &str| json!({ "agent_status": status });
        assert_eq!(
            refusal_of(None, None, 0.5).unwrap(),
            "no claude runs it (herdr knows no pane with its conversation)"
        );
        assert_eq!(refusal_of(Some(&pane("working")), None, 0.5).unwrap(), "it is working");
        assert_eq!(
            refusal_of(Some(&pane("blocked")), None, 0.5).unwrap(),
            "it waits for an answer"
        );
        assert_eq!(
            refusal_of(Some(&json!({})), None, 0.5).unwrap(),
            "herdr can't tell what it does (unknown)"
        );
        assert_eq!(
            refusal_of(Some(&pane("idle")), Some(0.62), 0.5).unwrap(),
            "its context is at 62 %, above the limit of 50 %"
        );
        assert_eq!(refusal_of(Some(&pane("done")), Some(0.4), 0.5), None);
    }

    #[test]
    fn a_queued_start_stays_queued() {
        let issue = Issue {
            number: 5,
            title: "t".into(),
            feature: "5-t".into(),
            is_fix: false,
            dir: PathBuf::from("/r/p-5-t"),
            why: None,
            branch: "feature/5-t".into(),
        };
        let known = json!({ "number": 5, "queued": true, "queuedAt": "2026-10-01T10:00:00.000Z" });
        assert_eq!(
            start_entry(&issue, Some(&known), Some("x")),
            json!({ "number": 5, "title": "t", "feature": "5-t", "branch": "feature/5-t", "why": null, "queued": true, "queuedAt": "2026-10-01T10:00:00.000Z", "requeued": "x" })
        );
        assert_eq!(start_entry(&issue, None, Some("x"))["why"], json!("failed: x"));
        assert_eq!(start_entry(&issue, None, None)["why"], Value::Null);
    }

    #[test]
    fn history_counts_each_issue_once() {
        let entry = json!({ "name": "a", "issues": [3, 4] });
        let started = with_start(
            &entry,
            json!({ "number": 4, "how": "handover", "at": "t", "context": 0.3 }),
        );
        assert_eq!(started["issues"], json!([3, 4]));
        assert_eq!(
            started["history"][0],
            json!({ "number": 3, "how": "fresh", "at": null, "context": null })
        );
        assert_eq!(started["history"][1]["context"], json!(0.3));
        let once = with_failure(&json!({}), 9, "busy", "t1");
        let twice = with_failure(&json!({ "failed": once }), 9, "busy", "t2");
        assert_eq!(
            twice,
            json!([{ "number": 9, "reason": "busy", "at": "t2", "times": 2 }])
        );
    }

    #[test]
    fn a_brief_is_kept_per_issue() {
        let raw = json!({ "brief": "old for all", "briefs": { "12": "for #12" } });
        assert_eq!(kept_brief(&raw, 12), "for #12");
        assert_eq!(kept_brief(&raw, 13), "");
    }

    #[test]
    fn a_reused_agent_hears_its_issues() {
        assert_eq!(before_line(&[]), None);
        assert_eq!(
            before_line(&[(3, Some("a".into())), (4, None)]).unwrap(),
            "You are a reused agent: you worked on #3 (a), #4 before. Treat this issue on its own, from develop as it is now: nothing of that work carries over but what is merged."
        );
    }

    #[test]
    fn the_next_line_waits_for_the_rename() {
        let names = std::cell::RefCell::new(vec![Some("old".to_string()), Some("old".into()), Some("7-x".into())]);
        let now = || names.borrow_mut().remove(0);
        assert_eq!(wait_renamed("7-x", Duration::from_secs(5), now), Renamed::Yes);
        assert_eq!(
            wait_renamed("7-x", Duration::ZERO, || Some("old".into())),
            Renamed::No("old".into())
        );
        assert_eq!(wait_renamed("7-x", Duration::ZERO, || None), Renamed::Unknown);
    }

    #[test]
    fn finds_a_worker_by_name_feature_or_old_name() {
        let worker = |name: &str, feature: &str, was: Option<&str>| Worker {
            dir: PathBuf::from("/d"),
            feature: Some(feature.into()),
            id: name.into(),
            name: name.into(),
            was: was.map(String::from),
            is_free: false,
        };
        let workers = [worker("a", "1-a", None), worker("b", "2-b", Some("old"))];
        assert_eq!(find_worker(&workers, "b").unwrap().id, "b");
        assert_eq!(find_worker(&workers, "1-a").unwrap().id, "a");
        assert_eq!(find_worker(&workers, "old").unwrap().id, "b");
        assert!(find_worker(&workers, "c").is_none());
    }
}
