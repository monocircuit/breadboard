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
use super::claude::{bound_session, context_share_of, is_conversation_running, session_file};
use super::gitlab;
use super::herdr;
use super::plugin;
use super::proc::{Done, Fail, Wait, as_step, done, exe, fail, output_within, step, text};
use super::repo::Repo;
use breadboard_core::conversations::interactive_pids_of;
use breadboard_core::git::branch_files;
use breadboard_core::json::{self, int_of, str_of};
use breadboard_core::record::{self, is_name, origin_of};
use breadboard_core::runs;
use breadboard_core::setups::{Claim, claim_setup, setup_text};
use breadboard_core::time::{now_ms, to_iso};
pub use brief::brief_of;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

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
/// The one reading of an issue for a fresh start (the setup group's) and a
/// handover: both name its branch and worktree the same.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub number: i64,
    pub title: String,
    /// Its description: what the start's memory estimate reads.
    pub description: String,
    pub feature: String,
    pub is_fix: bool,
    pub dir: PathBuf,
    /// Why it can't get a worktree: closed, there already, claimed on origin.
    pub why: Option<String>,
    pub branch: String,
    /// Its milestone's title: which epic it belongs to (#350).
    pub milestone: Option<String>,
}

const STOPWORDS: [&str; 15] = [
    "a", "an", "the", "of", "in", "on", "for", "to", "and", "or", "with", "by", "at", "from", "is",
];

/// A combining mark (`\p{M}`), as NFKD leaves it after the letter it was part of.
static MARKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{M}").expect("a valid pattern"));

/// What NFKD leaves of a lowercase letter once the marks go, for the
/// letters titles use (Latin-1, Latin Extended-A, the compatibility forms
/// of ASCII); any other character stays and splits the word, as there.
/// `æ`, `ø`, `ß`, `ł` have no decomposition: they split it too.
fn base_of(c: char, plain: &mut String) {
    let folded = match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' | 'ǎ' | 'ª' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ǐ' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ō' | 'ŏ' | 'ő' | 'ǒ' | 'º' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' | 'ſ' => 's',
        'ţ' | 'ť' | 'ț' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'ǔ' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        '¹' => '1',
        '²' => '2',
        '³' => '3',
        'ĳ' => return plain.push_str("ij"),
        'ﬁ' => return plain.push_str("fi"),
        'ﬂ' => return plain.push_str("fl"),
        // Fullwidth ASCII (`ｆｉｘ`, `１２`).
        '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
        other => other,
    };
    plain.push(folded);
}

/// `Crème brûlée: the fix!` → `creme-brulee-fix`: up to five words without
/// the small ones, at most 40 characters (orchestrator.mjs `slugOf`: lower
/// case, NFKD, the marks dropped, anything but `a-z0-9` a space).
pub fn slug_of(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut folded = String::new();
    for c in MARKS.replace_all(&lower, "").chars() {
        base_of(c, &mut folded);
    }
    let plain: String = folded
        .to_lowercase()
        .chars()
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

/// The branch on origin that claims the issue of `feature` already
/// (`feature/34-…`, `fix/34-…`, never `…/134-…` or `chore/34-…`): Ok(None)
/// when none does or `feature` names no issue, Err(why) when origin did not
/// answer. wt:new's claim check and the issues of wt:orchestrate, one.
pub fn claim_on_origin(main: &Path, feature: &str) -> Result<Option<String>, String> {
    let number: String = feature.chars().take_while(char::is_ascii_digit).collect();
    if number.is_empty() || !feature[number.len()..].starts_with('-') {
        return Ok(None);
    }
    let heads = output_within(
        "git",
        &[
            "ls-remote",
            "--heads",
            "origin",
            &format!("feature/{number}-*"),
            &format!("fix/{number}-*"),
        ],
        main,
        &Wait::new(
            "origin (git ls-remote, the branches of the issue)",
            Duration::from_secs(30),
        ),
    );
    if heads.stderr.starts_with("timeout") {
        return Err("origin did not answer whether a branch claims it (git ls-remote)".into());
    }
    Ok(first_claim(&heads.stdout, &number))
}

/// The first branch of `ls-remote` output that claims the issue `number`.
fn first_claim(ls_remote: &str, number: &str) -> Option<String> {
    ls_remote.lines().find_map(|line| {
        let (_, branch) = line.split_once("refs/heads/")?;
        let (kind, rest) = branch.split_once('/')?;
        let is_claim =
            matches!(kind, "feature" | "fix") && rest.strip_prefix(number).is_some_and(|after| after.starts_with('-'));
        is_claim.then(|| branch.trim().to_string())
    })
}

/// Why the issue of `feature` is not free: a branch on origin claims it
/// already, or origin did not say (nothing is set up on a claim not known).
fn claimed_elsewhere(repo: &Repo, feature: &str) -> Option<String> {
    match claim_on_origin(&repo.main, feature) {
        Ok(claim) => claim.map(|branch| format!("claimed by {branch}")),
        Err(why) => Some(why),
    }
}

/// Why the issue `number` could not be read, and what to do.
fn unread_issue(number: &str, unread: gitlab::Unread) -> Fail {
    Fail(match unread {
        gitlab::Unread::Missing => format!("#{number} is no issue of this project (glab issue list)"),
        gitlab::Unread::Unanswered => format!(
            "issue #{number} could not be read: GitLab did not answer; again once it does (glab issue view {number} says why)"
        ),
    })
}

/// The issue `arg` (its number, or `<n>-<feature>`) as GitLab has it, or
/// why it can't be read (orchestrator.mjs `readIssue`).
pub fn read_issue(repo: &Repo, arg: &str) -> Result<Issue, Fail> {
    let number: String = arg.chars().take_while(char::is_ascii_digit).collect();
    let found = as_step(&format!("GitLab: issue #{number}"), || {
        gitlab::view(&repo.main, &["issue", "view", &number], &format!("issue #{number}"))
    })
    .map_err(|unread| unread_issue(&number, unread))?;
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
        claimed_elsewhere(repo, &feature)
    };
    Ok(Issue {
        number: number
            .parse()
            .map_err(|_| unread_issue(&number, gitlab::Unread::Missing))?,
        title,
        description: str_of(&found, "description").unwrap_or_default().to_string(),
        branch: format!("{}/{feature}", if is_fix { "fix" } else { "feature" }),
        feature,
        is_fix,
        dir,
        why,
        milestone: found
            .get("milestone")
            .and_then(|milestone| str_of(milestone, "title"))
            .map(String::from),
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

/// The pane herdr runs the conversation `id` in, None for none, or why
/// herdr could not tell (it hung or failed: not the agent's refusal).
pub fn pane_of_conversation(id: &str) -> Result<Option<Value>, String> {
    Ok(herdr::all_panes_answered()?.into_iter().find(|pane| {
        pane.get("agent_session")
            .and_then(|session| session.get("value"))
            .and_then(Value::as_str)
            == Some(id)
    }))
}

/// What the agent `id` does: the run engine's status table (#359), where no
/// row is an idle session that never ran; one an interactive claude runs (a
/// task agent still in its tab, #361) as herdr's pane of it says.
fn agent_of(main: &Path, id: &str) -> Result<Option<Value>, String> {
    if !interactive_pids_of(id).is_empty() {
        return pane_of_conversation(id);
    }
    let status = runs::agents_of(main)
        .remove(id)
        .and_then(|agent| agent.status)
        .unwrap_or_else(|| "idle".into());
    Ok(Some(json!({ "pane_id": "", "agent_status": status })))
}

/// A headless agent's next run (#359): bound to the issue's worktree, its
/// line queued there. An idle claude of it still in a tab (#361, judged idle
/// above) ends first, SIGTERM and waited for: from here on it is headless.
/// No other claude may hold the conversation: a run that started since the
/// check would write over where it stands.
fn queue_next_run(main: &Path, worker: &Worker, issue: &Issue, line: &str) -> Result<(), String> {
    for pid in interactive_pids_of(&worker.id) {
        if !end_claude(pid) {
            return Err(format!("its claude in a tab (pid {pid}) did not end"));
        }
    }
    if is_conversation_running(&worker.id) {
        return Err("a claude runs its conversation".into());
    }
    bind(worker, issue);
    if !crate::tui::overview::actor::herdr::enqueue(main, &issue.dir, line) {
        return Err("its line could not be queued".into());
    }
    Ok(())
}

/// The claude `pid` asked to end (SIGTERM); whether it is gone within 15 s.
pub fn end_claude(pid: i64) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: a signal to one process this user runs.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let started = std::time::Instant::now();
    while breadboard_core::lock::is_alive(i64::from(pid)) {
        if started.elapsed() >= Duration::from_secs(15) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    true
}

/// A handover herdr could not judge: nothing of the agent is refused or
/// recorded, the same command again does it once herdr answers.
fn herdr_unanswered(why: &str, worker: &str) -> Fail {
    Fail(format!(
        "{why}: can't tell whether {worker} is idle; nothing changed, run the same command again once herdr answers"
    ))
}

/// Why an agent can't take a next issue, or None: no pane runs it (a free
/// agent sleeps without one, #345), it works or waits, herdr can't tell, or
/// its context `share` is above `limit`.
pub fn refusal_of(pane: Option<&Value>, is_free: bool, share: Option<f64>, limit: f64) -> Option<String> {
    let status = match pane {
        Some(pane) => pane.get("agent_status").and_then(Value::as_str).unwrap_or("unknown"),
        None if is_free => "idle",
        None => return Some("no claude runs it (herdr knows no pane with its conversation)".into()),
    };
    match status {
        "working" => return Some("it is working".into()),
        "blocked" => return Some("it waits for an answer".into()),
        "queued" => return Some("lines wait for its next run".into()),
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

/// The first prompt of an agent resumed in the worktree of its next issue;
/// its new name comes with it (#251, #335).
pub fn next_issue_line(number: i64, feature: &str, dir: &Path) -> String {
    format!(
        "[overview] next issue #{number}, your name is {feature} now: your conversation runs again in its worktree {}, read .claude/brief.md",
        text(dir)
    )
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
        "You are a reused agent: you worked on {} before. Treat this issue on its own, from its base as it is now: nothing of that work carries over but what is merged.",
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
/// feature's name; `renamed` (when) is tell.rs's, once its `/rename` took (#313).
fn record_handover(repo: &Repo, name: &str, worker: &Worker, issue: &Issue) {
    let raw = raw_record(repo, name);
    let known = worker_entry(&raw, &worker.id);
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
    let renamed = known.get("renamed").cloned().unwrap_or(Value::Null);
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

/// What wt:new left (orchestrator.mjs `makeWorktree`): whether the worktree
/// is there, the reason it failed with (`✗ …`), its exit code.
pub struct Made {
    pub is_there: bool,
    pub failed: Option<String>,
    pub status: Option<i32>,
}

impl Made {
    /// Made, but wt:new failed after the setup (#130): the worktree counts, said with a note.
    pub fn failed_after_setup(&self) -> bool {
        self.is_there && (self.failed.is_some() || self.status != Some(0))
    }
}

/// `wt:new <feature> --origin <name>` for the issue, its output shown; it
/// may fail after the setup (#130), so the worktree counts when it is there.
/// The one for a fresh start (the setup group's) and a handover.
pub fn make_worktree(repo: &Repo, issue: &Issue, name: &str, is_shared: bool) -> Made {
    // One issue, one worktree, one MR (#349): never a branch that claims another issue.
    let claimed = breadboard_core::git::issue_of_branch(&issue.branch);
    if claimed.and_then(|claimed| i64::try_from(claimed).ok()) != Some(issue.number) {
        return Made {
            is_there: false,
            failed: Some(format!(
                "#{} gets a worktree of its own, {} claims another issue",
                issue.number, issue.branch
            )),
            status: None,
        };
    }
    let mut args = vec!["wt:new", issue.feature.as_str(), "--origin", name];
    if issue.is_fix {
        args.push("--fix");
    }
    if is_shared {
        args.push("--shared");
    }
    let _ = std::io::stdout().flush();
    let made = herdr::pass_silence(&mut Command::new(exe()))
        .args(&args)
        .current_dir(&repo.main)
        // This process checked the herdr plugin already (#230): the child asks herdr nothing about it.
        .env(super::super::setup::PLUGIN_CHECKED, "1")
        .stdin(Stdio::null())
        // Its steps show as they run; its failure is read from stderr.
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .output();
    let (errors, status) = match made {
        Ok(made) => (String::from_utf8_lossy(&made.stderr).into_owned(), made.status.code()),
        Err(error) => (format!("\n✗ wt:new did not start: {error}\n"), None),
    };
    if !errors.trim().is_empty() {
        let _ = std::io::stderr().write_all(errors.as_bytes());
    }
    Made {
        is_there: repo.worktrees().iter().any(|tree| tree.dir == issue.dir),
        // wt:new's failure says `✗ <reason>`.
        failed: errors
            .lines()
            .find_map(|line| line.strip_prefix("✗ "))
            .map(String::from),
        status,
    }
}

/// A start that failed once its worktree was made (#272, orchestrator.mjs
/// `undoStart`): no worktree stays without its agent, `wt:remove` takes it
/// and its empty claim; a queued issue goes back to the queue. `release`
/// lets the claim go first, or wt:remove refuses. The record's entry of the
/// issue is replaced by the undone one with its why: today's `writeRecord`
/// kept the entry the start wrote before (no why) over it, so the record
/// said the worktree was gone but not why. Fresh starts and handovers, one.
/// Whether the worktree is gone after it, read from git, never assumed:
/// wt:remove refuses a worktree with changes (a setup that left a file), and
/// a child that did not start removed nothing; its ✗ then says where it stays.
pub fn undo_start(
    repo: &Repo,
    name: &str,
    issue: &Issue,
    known: Option<&Value>,
    why: &str,
    release: impl FnOnce(),
) -> bool {
    release();
    let is_there = || repo.worktrees().iter().any(|tree| tree.dir == issue.dir);
    if is_there() {
        let _ = std::io::stdout().flush();
        let removed = herdr::pass_silence(&mut Command::new(exe()))
            .args(["wt:remove", &issue.feature])
            .current_dir(&repo.main)
            .env(super::super::setup::PLUGIN_CHECKED, "1")
            .stdin(Stdio::null())
            .status();
        if let Err(error) = removed {
            eprintln!("✗ wt:remove {} did not start: {error}", issue.feature);
        }
    }
    let is_gone = !is_there();
    let entry = start_entry(issue, known, Some(why));
    let raw = json::read(&record::record_file(&repo.main, name));
    let issues = with_entry(
        raw.as_ref()
            .and_then(|raw| raw.get("issues"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        entry,
    );
    let mut fields = Map::new();
    fields.insert("issues".into(), Value::Array(issues));
    let _ = record::update_record(&repo.main, name, fields);
    is_gone
}

/// What became of the worktree of an undone start, for its ✗: removed
/// again, or where it stays (the run again takes it as it is, or `pnpm
/// wt:remove <feature>` once what wt:remove said is dealt with).
pub fn undone(is_gone: bool, issue: &Issue) -> String {
    if is_gone {
        format!("the worktree of #{} is removed again", issue.number)
    } else {
        format!(
            "the worktree of #{} stays at {}, not removed (see above why; {run} wt:remove {} takes it)",
            issue.number,
            text(&issue.dir),
            issue.feature,
            run = breadboard_core::config::run_word()
        )
    }
}

/// `issues` with `entry` in place of the one of its number (or added at the end).
fn with_entry(mut issues: Vec<Value>, entry: Value) -> Vec<Value> {
    match issues
        .iter()
        .position(|known| known.get("number") == entry.get("number"))
    {
        Some(at) => issues[at] = entry,
        None => issues.push(entry),
    }
    issues
}

/// What the command line gives a handover.
pub struct Request<'a> {
    pub name: &'a str,
    pub issue: &'a str,
    pub to: &'a str,
    pub limit: Option<&'a str>,
    /// This run's `--peer`; without one the issue's kept peer, then the orchestration's.
    pub peer: Option<String>,
    /// The text of this run's `--brief`; without one the issue's kept brief.
    pub extra: Option<String>,
    /// The file of this run's `--brief`, for what a dry run says.
    pub brief_file: Option<&'a str>,
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
    // A --brief applies to the issues of the run that passes it only (#247).
    let extra = match brief_file {
        Some(file) => {
            Some(fs::read_to_string(file).map_err(|error| Fail(format!("{file} could not be read: {error}")))?)
        }
        None => None,
    };
    let peer = args.value("--peer").filter(|peer| !peer.is_empty());
    if let Some(after) = after {
        if !is_dry {
            plugin::heal(&repo.main);
        }
        let given = Given {
            brief: brief_file.zip(extra.as_deref()),
            peer,
        };
        return announce(&repo, name, issues[0], to, &after, &given, is_dry);
    }
    hand_over(
        &repo,
        &Request {
            name,
            issue: issues[0],
            to,
            limit: args.value("--limit"),
            peer: peer.map(String::from),
            extra,
            brief_file,
            is_shared: args.has("--shared"),
            is_dry,
        },
    )
}

/// What an earlier run kept for the issue `number` under `field` of the
/// record (#247: `briefs`, written by wt:orchestrate; `peers`, by an
/// announced handover).
fn kept(raw: &Value, field: &str, number: i64) -> Option<String> {
    raw.get(field)
        .and_then(|kept| kept.get(number.to_string()))
        .and_then(Value::as_str)
        .map(String::from)
}

/// The brief an earlier run gave the issue `number`, "" without one.
fn kept_brief(raw: &Value, number: i64) -> String {
    kept(raw, "briefs", number).unwrap_or_default()
}

/// The peer of the issue `number`: this run's, the one its announcement
/// kept, the orchestration's; None for none.
fn peer_of(given: Option<&str>, raw: &Value, number: i64) -> Option<String> {
    given
        .map(String::from)
        .or_else(|| kept(raw, "peers", number))
        .or_else(|| str_of(raw, "peer").map(String::from))
        .filter(|peer| !peer.is_empty())
}

/// `text` kept under `field` (`briefs`, `peers`) for the issue `number`'s later start (#247).
fn keep(repo: &Repo, name: &str, field: &str, number: i64, text: &str) {
    let mut kept = raw_record(repo, name)
        .get(field)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    kept.insert(number.to_string(), json!(text));
    let mut fields = Map::new();
    fields.insert(field.into(), Value::Object(kept));
    let _ = record::update_record(&repo.main, name, fields);
}

/// What an announcement was given to keep for its handover (#247).
pub struct Given<'a> {
    /// `--brief`: its file and text.
    pub brief: Option<(&'a str, &'a str)>,
    /// `--peer`.
    pub peer: Option<&'a str>,
}

/// What an announced handover will start with, for its dry run and its ✓:
/// the brief and the peer, given now or kept from before.
fn announced_with(given: &Given, raw: &Value, number: i64) -> String {
    let brief = brief_source(given.brief.map(|(file, _)| file), raw, number);
    let peer = peer_of(given.peer, raw, number)
        .map(|peer| format!(", peer {peer}"))
        .unwrap_or_default();
    format!("{brief}{peer}")
}

/// Which brief of its own the issue `number` starts with (#247): this run's
/// `--brief` file, the one kept for it, or none.
fn brief_source(file: Option<&str>, raw: &Value, number: i64) -> String {
    match file {
        Some(file) => format!("the brief of {file}"),
        None if !kept_brief(raw, number).trim().is_empty() => "the brief kept for it".into(),
        None => "no brief of its own".into(),
    }
}

/// What a dry run says under the next-issue line: the brief below, and which
/// brief of its own is in it (#247; without one the section is left out).
fn dry_brief_line(source: &str) -> String {
    if source.starts_with("no ") {
        format!("  with the brief below, {source}")
    } else {
        format!("  with the brief below, {source} under ## From the orchestrator")
    }
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
    // herdr hung or failed: no refusal of the agent, nothing read or written further.
    let idle = format!("herdr: is {} idle?", worker.name);
    let pane =
        as_step(&idle, || agent_of(&repo.main, &worker.id)).map_err(|why| herdr_unanswered(&why, &worker.name))?;
    let share = context_share_of(&worker.id);
    let refusal = refusal_of(pane.as_ref(), worker.is_free, share, limit);
    let issue = read_issue(repo, request.issue)?;
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
    // Once nothing refused it: a refusal costs no extra herdr call (#230).
    if !request.is_dry {
        plugin::heal(&repo.main);
    }
    // One start of an issue at a time (#272).
    let claim = if request.is_dry {
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
    let peer = peer_of(request.peer.as_deref(), &raw, number);
    let brief = brief_of(
        &issue,
        name,
        &label,
        peer.as_deref(),
        &extra,
        before.as_deref(),
        &repo.main,
    );
    if request.is_dry {
        step(&format!("nothing was done (--dry-run). {} would get", worker.name));
        // An empty one adds no section either.
        let source = if extra.trim().is_empty() {
            "no brief of its own".to_string()
        } else {
            brief_source(request.brief_file, &raw, number)
        };
        println!(
            "  {}\n{}\n",
            next_issue_line(number, &issue.feature, &issue.dir),
            dry_brief_line(&source)
        );
        println!("{brief}");
        return Ok(());
    }

    // wt:new may fail after its setup (#130): the handover goes on, its ✓ says so.
    let mut setup_note = "";
    if !is_made {
        let _ = record::write_record(&repo.main, name, &[start_entry(&issue, known.as_ref(), None)]);
        step(&format!(
            "{run} wt:new {} --origin {name}",
            issue.feature,
            run = breadboard_core::config::run_word()
        ));
        let made = make_worktree(repo, &issue, name, request.is_shared);
        if made.failed_after_setup() {
            setup_note = " (wt:new failed after the setup, see above)";
        }
        if !made.is_there {
            let reason = made.failed.unwrap_or_else(|| "no worktree".into());
            let why = format!("its worktree failed: {reason}");
            undo_start(repo, name, &issue, known.as_ref(), &why, || claim.release());
            record_failure(repo, name, &worker.id, number, &why);
            alarm(repo, name, &format!("#{number} did not go to {}: {why}", worker.name));
            return fail(format!("#{number} {}: {reason}", issue.feature));
        }
    }
    // Again: the setup took minutes, the agent may have started on something
    // meanwhile, or gone. Its worktree goes again then (#272).
    let queued = if known.as_ref().is_some_and(|known| json::is_truthy(known.get("queued"))) {
        ", the issue is back in the queue"
    } else {
        ""
    };
    let pane = match as_step(&idle, || agent_of(&repo.main, &worker.id)) {
        Ok(pane) => pane,
        Err(why) => {
            // Not the agent's failure: its history stays as it was.
            let is_gone = undo_start(repo, name, &issue, known.as_ref(), &why, || claim.release());
            return fail(format!(
                "{why}: can't tell whether {} is still idle after the setup; {}{queued}; run the same command again once herdr answers",
                worker.name,
                undone(is_gone, &issue)
            ));
        }
    };
    if let Some(now) = refusal_of(pane.as_ref(), worker.is_free, None, limit) {
        let why = format!(
            "after the setup, {} {}",
            worker.name,
            now.strip_prefix("it ").unwrap_or(&now)
        );
        let is_gone = undo_start(repo, name, &issue, known.as_ref(), &why, || claim.release());
        record_failure(repo, name, &worker.id, number, &format!("after the setup, {now}"));
        return fail(format!(
            "{why}: {}{queued}; run the same command again once it is idle",
            undone(is_gone, &issue)
        ));
    }
    if pane.is_none() && !worker.is_free {
        return fail("herdr lost the agent's pane");
    }
    if let Err(error) = record::write_brief(&issue.dir, &brief) {
        return fail(format!("the brief of #{number} could not be written: {error}"));
    }
    let feature = issue.feature.as_str();
    let line = next_issue_line(number, feature, &issue.dir);
    // A headless agent (#359, #361): its next run, in the worktree under its new name; nothing is typed.
    let queued_run = as_step(&format!("{} queued for its next run in {feature}", worker.name), || {
        queue_next_run(&repo.main, &worker, &issue, &line)
    });
    if let Err(why) = &queued_run {
        // Its conversation stays where it was: its worktree goes again, the issue may be handed over anew.
        unbind(&worker, &issue);
        let why = format!("{} got no run in its worktree: {why}", worker.name);
        let is_gone = undo_start(repo, name, &issue, known.as_ref(), &why, || claim.release());
        record_failure(repo, name, &worker.id, number, &why);
        alarm(repo, name, &format!("#{number} did not go to {why}"));
        return fail(format!("{why}; {}{queued}", undone(is_gone, &issue)));
    }
    record_handover(repo, name, &worker, &issue);
    let _ = record::write_record(&repo.main, name, &[record_entry(&issue, None)]);
    claim.release();
    let is_announced = record::record_of(&repo.main, name)
        .is_some_and(|record| record.handovers.iter().any(|handover| handover.number == number));
    record_start(
        repo,
        name,
        &worker.id,
        number,
        if is_announced { "announced" } else { "handover" },
        share,
    );
    let _ = record::drop_handover(&repo.main, name, number);
    let renamed = if worker.name == feature {
        String::new()
    } else {
        format!(", renamed from {}", worker.name)
    };
    done(&format!(
        "#{number} went to the agent {feature}{renamed}: its next run is in its worktree, the next-issue line its prompt{setup_note}; breadboard runs shows it"
    ));
    Ok(())
}

/// A handover that failed after its start, in ACTIONS of the orchestration
/// (#335): a ✗ of a background run is easily missed, a `look` stays.
fn alarm(repo: &Repo, name: &str, text: &str) {
    let action = breadboard_core::actions::NewAction {
        kind: "look".into(),
        text: text.into(),
        ..Default::default()
    };
    let _ = breadboard_core::actions::add_action(&record::record_file(&repo.main, name), action);
}

/// [`bind`] undone: the conversation stays bound where it runs.
fn unbind(worker: &Worker, issue: &Issue) {
    let _ = fs::remove_file(session_file(&issue.dir));
    if !worker.is_free {
        let _ = fs::write(session_file(&worker.dir), format!("{}\n", worker.id));
    }
}

/// The binding follows the conversation: a restart resumes it in its new
/// worktree; a free agent's checkout is the orchestration's, no binding of its own there.
fn bind(worker: &Worker, issue: &Issue) {
    let _ = fs::write(session_file(&issue.dir), format!("{}\n", worker.id));
    if !worker.is_free {
        let _ = fs::remove_file(session_file(&worker.dir));
    }
}

/// The handover announced, not done (handover.mjs `announceCommand`): the
/// record keeps it, the issue queued; the overview does it once the MRs are
/// merged and the agent is idle.
pub fn announce(repo: &Repo, name: &str, arg: &str, to: &str, after: &[i64], given: &Given, is_dry: bool) -> Done {
    let label = repo.orchestrator_label(name);
    let workers = workers_of(repo, name);
    let Some(worker) = find_worker(&workers, to) else {
        return Err(no_agent(&label, to, &workers));
    };
    let issue = read_issue(repo, arg)?;
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
    let with = announced_with(given, &raw, number);
    if is_dry {
        step(&format!(
            "nothing was done (--dry-run). {} would take #{number} once {mrs} {verb} merged, with {with}.",
            worker.name
        ));
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
    // Its --brief and --peer wait with it for the handover (#247), next to the record's issues.
    if let Some((_, text)) = given.brief {
        keep(repo, name, "briefs", number, text);
    }
    if let Some(peer) = given.peer {
        keep(repo, name, "peers", number, peer);
    }
    let _ = record::announce_handover(&repo.main, name, number, &worker.name, after, &to_iso(now_ms()));
    done(&format!(
        "#{number} {} goes to {} once {mrs} {verb} merged, with {with}: the overview of {label} hands it over then (its worktree is kept until), shown on {}'s node as → next #{number}",
        issue.feature, worker.name, worker.name
    ));
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
    fn slugs_as_nfkd_without_the_marks() {
        // Node: `æ` has no decomposition (a space), a mark written apart goes,
        // a typographic apostrophe splits: the fresh start and a handover agree.
        assert_eq!(
            format!("230-{}", slug_of("Cæsar\u{2019}s re\u{301}sume\u{301} of the plugin")),
            "230-c-sar-s-resume-plugin"
        );
        assert_eq!(slug_of("Café für Über-Größen"), "cafe-fur-uber-gro-en");
        assert_eq!(slug_of("ＦＩＸ ﬁle ĳs șir"), "fix-file-ijs-sir");
    }

    #[test]
    fn a_claim_is_of_that_issue_only() {
        let out = "a\trefs/heads/fix/305-overview-gitlab-refresh-makes-312\nb\trefs/heads/feature/123-x\nc\trefs/heads/chore/5-x\n";
        assert_eq!(first_claim(out, "5"), None);
        assert_eq!(first_claim(out, "23"), None);
        assert_eq!(first_claim(out, "12"), None);
        assert_eq!(first_claim(out, "123").as_deref(), Some("feature/123-x"));
        assert_eq!(
            first_claim(out, "305").as_deref(),
            Some("fix/305-overview-gitlab-refresh-makes-312")
        );
    }

    #[test]
    fn an_undone_start_replaces_its_entry() {
        let issues = vec![
            json!({ "number": 5, "why": null, "removedAt": "x" }),
            json!({ "number": 6 }),
        ];
        let entry = json!({ "number": 5, "why": "failed: its claude did not start" });
        assert_eq!(with_entry(issues, entry.clone()), vec![entry, json!({ "number": 6 })]);
        assert_eq!(with_entry(vec![], json!({ "number": 7 })), vec![json!({ "number": 7 })]);
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
            refusal_of(None, false, None, 0.5).unwrap(),
            "no claude runs it (herdr knows no pane with its conversation)"
        );
        assert_eq!(
            refusal_of(Some(&pane("working")), false, None, 0.5).unwrap(),
            "it is working"
        );
        assert_eq!(
            refusal_of(Some(&pane("blocked")), false, None, 0.5).unwrap(),
            "it waits for an answer"
        );
        assert_eq!(
            refusal_of(Some(&json!({})), false, None, 0.5).unwrap(),
            "herdr can't tell what it does (unknown)"
        );
        assert_eq!(
            refusal_of(Some(&pane("idle")), false, Some(0.62), 0.5).unwrap(),
            "its context is at 62 %, above the limit of 50 %"
        );
        assert_eq!(refusal_of(Some(&pane("done")), false, Some(0.4), 0.5), None);
        // A free agent sleeps without a pane (#345): it takes one, unless full.
        assert_eq!(refusal_of(None, true, Some(0.4), 0.5), None);
        assert!(refusal_of(None, true, Some(0.62), 0.5).is_some());
    }

    #[test]
    fn a_queued_start_stays_queued() {
        let issue = Issue {
            number: 5,
            title: "t".into(),
            description: String::new(),
            feature: "5-t".into(),
            is_fix: false,
            dir: PathBuf::from("/r/p-5-t"),
            why: None,
            branch: "feature/5-t".into(),
            milestone: None,
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
    fn an_undone_start_says_removed_again_only_when_the_worktree_is_gone() {
        let issue = Issue {
            number: 247,
            title: "t".into(),
            description: String::new(),
            feature: "247-t".into(),
            is_fix: true,
            dir: PathBuf::from("/r/p-247-t"),
            why: None,
            branch: "fix/247-t".into(),
            milestone: None,
        };
        assert_eq!(undone(true, &issue), "the worktree of #247 is removed again");
        let stays = undone(false, &issue);
        assert!(!stays.contains("removed again"), "{stays}");
        assert_eq!(
            stays,
            "the worktree of #247 stays at /r/p-247-t, not removed (see above why; pnpm wt:remove 247-t takes it)"
        );
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
    fn a_dry_run_says_which_brief_applies() {
        let raw = json!({ "briefs": { "5": "mind the cache" } });
        assert_eq!(brief_source(Some("b.md"), &raw, 9), "the brief of b.md");
        assert_eq!(brief_source(None, &raw, 5), "the brief kept for it");
        assert_eq!(brief_source(None, &raw, 9), "no brief of its own");
        assert_eq!(
            dry_brief_line("no brief of its own"),
            "  with the brief below, no brief of its own"
        );
        assert_eq!(
            dry_brief_line("the brief kept for it"),
            "  with the brief below, the brief kept for it under ## From the orchestrator"
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
            "You are a reused agent: you worked on #3 (a), #4 before. Treat this issue on its own, from its base as it is now: nothing of that work carries over but what is merged."
        );
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
