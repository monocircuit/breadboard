//! `pnpm wt:wait !213 !217 --then "…"` (commands/wait.mjs, worktree/wait.mjs),
//! meant for `run_in_background`: waits until every named MR is merged,
//! asking GitLab every minute, and ends with the line to go on with (Claude
//! Code wakes the agent when a background command exits). It ends early when
//! one is closed, stays in conflict, or the limit passes. The wait is
//! recorded in `.claude/wait.json` for the overview, which delivers the line
//! itself when this command is gone. A headless agent's (#359) wait returns
//! at once: the overview queues the line as its next run.

use super::common::{Argv, Here, Outcome, Takes, help, say};
use super::glab;
use breadboard_core::git::{Base, base_of, git};
use breadboard_core::json::{self, int_of, ints_of, str_of};
use breadboard_core::runs;
use breadboard_core::time::{now_ms, to_iso};
use serde_json::{Map, Number, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// How often it asks GitLab.
const POLL: Duration = Duration::from_secs(60);
/// How often it looks between asks whether the wait is still its own.
const LOOK: Duration = Duration::from_secs(1);
/// How long it waits by default before it gives up, in hours.
const DEFAULT_HOURS: f64 = 8.0;
/// How long an awaited MR may have conflicts with develop before the wait ends.
const CONFLICT_MS: i64 = 30 * 60_000;

const USAGE: &str = "usage: pnpm wt:wait !<mr> [!<mr>…] [--then \"<what to do>\"] [--hours <h>]   (run_in_background)\n       pnpm wt:wait --cancel";
const ABOUT: &str = "pnpm wt:wait waits until every named MR is merged, asking GitLab every minute, and ends with
the line to go on with. It ends early when one is closed or stays in conflict for 30 min, or after
--hours (default 8). The overview shows the wait; --cancel ends it.";
/// Only to hear `--help`: parse_wait refuses everything else in its own words.
const TAKES: Takes = Takes {
    valued: &["--then", "--hours"],
    switches: &["--cancel"],
    words: None,
    usage: USAGE,
    command: "wt:wait",
};

fn wait_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("wait.json")
}

/// The wait of a worktree as stored, None when it waits for nothing (no `mrs`).
fn wait_of(dir: &Path) -> Option<Value> {
    json::read(&wait_file(dir)).filter(|wait| {
        wait.get("mrs")
            .and_then(Value::as_array)
            .is_some_and(|mrs| !mrs.is_empty())
    })
}

fn clear_wait(dir: &Path) {
    let _ = std::fs::remove_file(wait_file(dir));
}

/// `!213`, `!213 and !217`, `!213, !217 and !219`.
fn list_of(mrs: &[i64]) -> String {
    let names: Vec<String> = mrs.iter().map(|iid| format!("!{iid}")).collect();
    match names.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// `waits for !213, !217`.
fn waits_text(mrs: &[i64]) -> String {
    let names: Vec<String> = mrs.iter().map(|iid| format!("!{iid}")).collect();
    format!("waits for {}", names.join(", "))
}

/// What wt:wait was given.
#[derive(Debug, Clone, PartialEq)]
struct Parsed {
    mrs: Vec<i64>,
    then: Option<String>,
    hours: f64,
}

/// The command line after `wait`: MRs as `!213` or `213`, each once, in order (wait.mjs `parseWait`).
fn parse_wait(argv: &[String]) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        mrs: Vec::new(),
        then: None,
        hours: DEFAULT_HOURS,
    };
    let mut at = 0;
    while at < argv.len() {
        let arg = argv[at].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if arg.starts_with("--") => (flag, Some(value)),
            _ => (arg, None),
        };
        if flag == "--then" || flag == "--hours" {
            let value = match inline {
                Some(value) => value,
                None => {
                    at += 1;
                    match argv.get(at) {
                        Some(value) if !value.starts_with("--") => value.as_str(),
                        _ => return Err(format!("{flag} needs a value")),
                    }
                }
            };
            if flag == "--then" {
                parsed.then = Some(value.trim().to_string()).filter(|then| !then.is_empty());
            } else {
                // Number(value), finite: an endless wait has no `until` to write.
                let trimmed = value.trim();
                let hours = if trimmed.is_empty() {
                    0.0
                } else {
                    trimmed.parse::<f64>().unwrap_or(f64::NAN)
                };
                let is_plain = trimmed
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
                if !(hours > 0.0 && hours.is_finite() && is_plain) {
                    return Err(format!("--hours takes a number above 0, not {value}"));
                }
                parsed.hours = hours;
            }
        } else if let Some(iid) = mr_of(arg) {
            if !parsed.mrs.contains(&iid) {
                parsed.mrs.push(iid);
            }
        } else if arg.starts_with("--") {
            return Err(format!("unknown flag {flag}"));
        } else {
            return Err(format!("{arg} is no MR (!213 or 213)"));
        }
        at += 1;
    }
    if parsed.mrs.is_empty() {
        return Err("name the MRs to wait for: pnpm wt:wait !213 !217".into());
    }
    Ok(parsed)
}

/// `!213` or `213` (`/^!?\d+$/`).
fn mr_of(arg: &str) -> Option<i64> {
    let digits = arg.strip_prefix('!').unwrap_or(arg);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// An awaited MR as GitLab has it.
#[derive(Debug, Clone, PartialEq)]
struct Facts {
    iid: i64,
    state: String,
    has_conflicts: bool,
}

fn facts_of(mr: &Value) -> Option<Facts> {
    Some(Facts {
        iid: int_of(mr, "iid")?,
        state: str_of(mr, "state").unwrap_or_default().to_string(),
        has_conflicts: json::is_truthy(mr.get("has_conflicts")),
    })
}

/// Where a wait stands (wait.mjs `verdictOf`).
#[derive(Debug, Clone, PartialEq)]
enum Verdict {
    Merged,
    Closed(i64),
    Conflicts(i64),
    Timeout(Vec<i64>),
    Waiting,
}

/// From the awaited MRs as GitLab has them (one missing counts as not
/// answered yet), at `now`. `conflicts_since`: iid → since when it has
/// conflicts, kept between asks.
fn verdict_of(mrs: &[i64], until: i64, found: &[Facts], now: i64, conflicts_since: &mut HashMap<i64, i64>) -> Verdict {
    let state = |iid: i64| found.iter().find(|mr| mr.iid == iid);
    if let Some(&closed) = mrs
        .iter()
        .find(|&&iid| state(iid).is_some_and(|mr| mr.state == "closed"))
    {
        return Verdict::Closed(closed);
    }
    let open: Vec<i64> = mrs
        .iter()
        .copied()
        .filter(|&iid| state(iid).is_none_or(|mr| mr.state != "merged"))
        .collect();
    if open.is_empty() {
        return Verdict::Merged;
    }
    for &iid in &open {
        if !state(iid).is_some_and(|mr| mr.has_conflicts) {
            conflicts_since.remove(&iid);
        } else if let Some(&since) = conflicts_since.get(&iid) {
            if now - since >= CONFLICT_MS {
                return Verdict::Conflicts(iid);
            }
        } else {
            conflicts_since.insert(iid, now);
        }
    }
    if now >= until {
        return Verdict::Timeout(open);
    }
    Verdict::Waiting
}

/// The line the agent goes on with (wait.mjs `lineOf`). `behind`: how many
/// commits of its base (`develop`, an epic) the worktree lacks, None unknown.
fn line_of(parsed: &Parsed, verdict: &Verdict, base: &str, behind: Option<u64>) -> String {
    let all = list_of(&parsed.mrs);
    match verdict {
        Verdict::Merged => {
            let merge = match behind {
                Some(0) => format!("{base} is in already"),
                Some(behind) => {
                    format!("{base} is {behind} behind: merge it in only if your work needs it or it conflicts")
                }
                None => format!("merge {base} in only if your work needs it or it conflicts"),
            };
            let then = parsed
                .then
                .as_deref()
                .map(|then| format!(", then {then}"))
                .unwrap_or_default();
            format!(
                "{all} {} merged: {merge}{then}",
                if parsed.mrs.len() > 1 { "are" } else { "is" }
            )
        }
        Verdict::Closed(mr) => format!(
            "!{mr} was closed without a merge: the wait for {all} ends, find out what replaces it (glab mr view {mr}) before you go on"
        ),
        Verdict::Conflicts(mr) => format!(
            "!{mr} has had conflicts with {base} for {} min: the wait for {all} ends, tell the orchestrator (SendMessage) instead of waiting on",
            CONFLICT_MS / 60_000
        ),
        Verdict::Timeout(open) => format!(
            "waited {} h for {all}, {} not merged yet: the wait ends, tell the orchestrator or wait again ({run} wt:wait)",
            hours_text(parsed),
            list_of(open),
            run = breadboard_core::config::run_word()
        ),
        Verdict::Waiting => String::new(),
    }
}

/// The hours as JavaScript prints the number: `8`, `0.0001`, `1e-7`.
fn hours_text(parsed: &Parsed) -> String {
    let hours = parsed.hours;
    if hours != 0.0 && !(1e-6..1e21).contains(&hours.abs()) {
        let text = format!("{hours:e}");
        return match text.split_once('e') {
            Some((mantissa, exponent)) if !exponent.starts_with('-') => format!("{mantissa}e+{exponent}"),
            _ => text,
        };
    }
    format!("{hours}")
}

/// The hours as JSON writes the number: a whole one without `.0`.
fn hours_json(hours: f64) -> Value {
    if hours.fract() == 0.0 && hours.abs() < 9e15 {
        Value::Number(Number::from(hours as i64))
    } else {
        Number::from_f64(hours).map(Value::Number).unwrap_or(Value::Null)
    }
}

/// How many commits of its base the worktree lacks, fetched first; None when git can't say.
fn behind_of(dir: &Path, base: &Base) -> Option<u64> {
    git(dir, &["fetch", "--quiet", "origin", &base.local]);
    let count = git(dir, &["rev-list", "--count", &format!("HEAD..{}", base.remote)]);
    if count.is_ok { count.out().parse().ok() } else { None }
}

/// Whether the wait recorded is still this process's: a newer wt:wait took over otherwise.
fn is_ours(dir: &Path) -> bool {
    wait_of(dir).and_then(|wait| int_of(&wait, "pid")) == Some(i64::from(std::process::id()))
}

/// What GitLab said of an awaited MR.
#[derive(Debug, Clone, PartialEq)]
enum Answer {
    Found(Facts),
    /// GitLab has no such MR (404).
    Missing,
    /// glab couldn't ask, or GitLab refused: `glab failed: …` / `GitLab answered …`.
    Failed(String),
}

fn answer_of(ran: &glab::Ran) -> Answer {
    match ran.json().as_ref().and_then(facts_of) {
        Some(facts) => Answer::Found(facts),
        None if ran.is_not_there() => Answer::Missing,
        None => Answer::Failed(ran.failure()),
    }
}

/// Every awaited MR asked side by side.
fn ask(dir: &Path, mrs: &[i64]) -> Vec<Answer> {
    let calls: Vec<_> = mrs
        .iter()
        .map(|iid| glab::start(dir, &["api", &format!("projects/:fullpath/merge_requests/{iid}")]))
        .collect();
    calls.into_iter().map(|call| answer_of(&glab::finish(call))).collect()
}

/// The first `glab failed: …` of the answers.
fn failure_of(answers: &[Answer]) -> Option<&str> {
    answers.iter().find_map(|answer| match answer {
        Answer::Failed(failure) => Some(failure.as_str()),
        _ => None,
    })
}

/// Sleeps until the next ask, at most until `until`, looking every second
/// whether the wait is still this process's; false when it isn't (cancelled
/// or taken over), so a stale wait doesn't poll GitLab for another minute.
fn pause(dir: &Path, until: i64) -> bool {
    let end = (now_ms() + POLL.as_millis() as i64).min(until);
    loop {
        let left = end - now_ms();
        if left <= 0 {
            return true;
        }
        std::thread::sleep(LOOK.min(Duration::from_millis(left as u64)));
        if !is_ours(dir) {
            return false;
        }
    }
}

/// Whether the agent of `dir` runs this open in a tab, not in a headless
/// run: the nearest claude among this process's ancestors is interactive
/// and runs the conversation bound to `dir` (its session file says which).
/// Any other caller (a headless run, a shell, another claude) counts as headless.
fn is_interactive_caller(dir: &Path) -> bool {
    let Some(bound) = breadboard_core::conversations::bound_session(dir) else {
        return false;
    };
    let claude = crate::cli::lifecycle::herdr::ancestors_of(i64::from(std::process::id()))
        .into_iter()
        .skip(1)
        .find_map(|pid| {
            let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
            Some((pid, breadboard_core::conversations::claude_print_in(&cmdline)?))
        });
    let Some((pid, false)) = claude else {
        return false;
    };
    breadboard_core::conversations::running_claudes()
        .iter()
        .any(|claude| claude.pid == pid && claude.id == bound)
}

pub fn run(argv: &Argv) -> Outcome {
    match argv.check(&TAKES) {
        Ok(true) => return help(ABOUT, USAGE),
        // A flag it doesn't take is refused as every command refuses it, `--cancel` too.
        Err(problem) if problem.starts_with("unknown flag") => return Err(problem),
        _ => {}
    }
    let here = Here::find();
    let Some(dir) = here.worktree() else {
        return Err(
            "wt:wait is for a worktree's agent: run it in your worktree (cd <worktree>, then pnpm wt:wait !<mr>)"
                .into(),
        );
    };
    if argv.has("--cancel") {
        let wait = wait_of(dir);
        clear_wait(dir);
        say(&match wait {
            Some(wait) => format!("\n✓ no longer {}\n", waits_text(&ints_of(&wait, "mrs"))),
            None => "\n  no wait\n".into(),
        });
        return Ok(ExitCode::SUCCESS);
    }
    let parsed = parse_wait(argv.all()).map_err(|error| format!("{error}\n{USAGE}"))?;
    // Asked once before anything is written or said: a typo or a glab that
    // can't reach GitLab ends it at once, not after 8 h of silent asking
    // (Node wrote the wait and printed its start line first).
    let first = ask(dir, &parsed.mrs);
    let unknown: Vec<i64> = parsed
        .mrs
        .iter()
        .zip(&first)
        .filter(|(_, answer)| **answer == Answer::Missing)
        .map(|(&iid, _)| iid)
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "GitLab has no MR {}: glab mr list shows the open ones",
            list_of(&unknown)
        ));
    }
    if let Some(failure) = failure_of(&first) {
        return Err(failure.to_string());
    }
    let since = now_ms();
    // A century at most: a date past year 9999 is none the readers parse (Node threw on it).
    let until = since + (parsed.hours * 3_600_000.0).min(100.0 * 365.25 * 86_400_000.0) as i64;
    let mut wait = Map::new();
    wait.insert("mrs".into(), parsed.mrs.iter().map(|&iid| Value::from(iid)).collect());
    wait.insert(
        "then".into(),
        parsed.then.clone().map(Value::String).unwrap_or(Value::Null),
    );
    wait.insert("hours".into(), hours_json(parsed.hours));
    wait.insert("since".into(), Value::String(to_iso(since)));
    wait.insert("until".into(), Value::String(to_iso(until)));
    // A headless agent (#359) is no process to wake: the wait is recorded
    // without a command, and the overview queues its line as a run. Called
    // from a claude open in a tab it waits as before: its queue is held
    // while that claude runs, the line would wait for its end (#361).
    let is_headless =
        here.main.as_deref().is_some_and(|main| runs::is_headless(main, dir)) && !is_interactive_caller(dir);
    let base = base_of(dir);
    if is_headless {
        // Settled already (merged, closed): said at once, nothing waits (#361).
        let found: Vec<Facts> = first
            .iter()
            .filter_map(|answer| match answer {
                Answer::Found(facts) => Some(facts.clone()),
                _ => None,
            })
            .collect();
        let verdict = verdict_of(&parsed.mrs, until, &found, now_ms(), &mut HashMap::new());
        if verdict != Verdict::Waiting {
            let behind = (verdict == Verdict::Merged).then(|| behind_of(dir, &base)).flatten();
            say(&line_of(&parsed, &verdict, &base.local, behind));
            return Ok(if verdict == Verdict::Merged {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
    } else {
        wait.insert("pid".into(), Value::from(std::process::id()));
    }
    let file = wait_file(dir);
    json::write_pretty(&file, &Value::Object(wait))
        .map_err(|error| format!("{} could not be written: {error}", file.display()))?;
    if is_headless {
        say(&format!(
            "▸ {}: the overview wakes you with its line when it ends; stop now",
            waits_text(&parsed.mrs)
        ));
        return Ok(ExitCode::SUCCESS);
    }
    say(&format!("▸ {}, asking GitLab every minute", waits_text(&parsed.mrs)));

    let mut conflicts_since = HashMap::new();
    let mut first = Some(first);
    let mut is_failing = false;
    loop {
        // A newer wt:wait in this worktree took over: this one steps back quietly.
        if !is_ours(dir) {
            return Ok(ExitCode::SUCCESS);
        }
        let answers = first.take().unwrap_or_else(|| ask(dir, &parsed.mrs));
        // glab failing later (a login expired, the network gone) is said once
        // on stderr, and once more when GitLab answers again; the wait goes on.
        match failure_of(&answers) {
            Some(failure) if !is_failing => {
                eprintln!("! {failure}: asks again every minute");
                is_failing = true;
            }
            None if is_failing => {
                eprintln!("  GitLab answers again");
                is_failing = false;
            }
            _ => {}
        }
        let found: Vec<Facts> = answers
            .into_iter()
            .filter_map(|answer| match answer {
                Answer::Found(facts) => Some(facts),
                _ => None,
            })
            .collect();
        let verdict = verdict_of(&parsed.mrs, until, &found, now_ms(), &mut conflicts_since);
        if verdict != Verdict::Waiting {
            let behind = if verdict == Verdict::Merged {
                behind_of(dir, &base)
            } else {
                None
            };
            let line = line_of(&parsed, &verdict, &base.local, behind);
            if is_ours(dir) {
                clear_wait(dir);
            }
            say(&line);
            return Ok(if verdict == Verdict::Merged {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        if !pause(dir, until) {
            return Ok(ExitCode::SUCCESS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn parses_the_mrs_once_each_and_the_flags() {
        let parsed = parse_wait(&args(&["!213", "217", "!213", "--then", " go on ", "--hours=0.5"])).unwrap();
        assert_eq!(parsed.mrs, [213, 217]);
        assert_eq!(parsed.then.as_deref(), Some("go on"));
        assert_eq!(parsed.hours, 0.5);
        assert_eq!(
            parse_wait(&args(&["!1", "--then", "--hours", "2"])),
            Err("--then needs a value".into())
        );
        assert_eq!(
            parse_wait(&args(&["!1", "--hours"])),
            Err("--hours needs a value".into())
        );
        assert_eq!(
            parse_wait(&args(&["!1", "--then=--x"])).unwrap().then.as_deref(),
            Some("--x")
        );
        assert_eq!(
            parse_wait(&args(&["!1", "--hours", "0"])),
            Err("--hours takes a number above 0, not 0".into())
        );
        assert_eq!(
            parse_wait(&args(&["!1", "--hours", "inf"])),
            Err("--hours takes a number above 0, not inf".into())
        );
        assert_eq!(parse_wait(&args(&["!1", "--fast"])), Err("unknown flag --fast".into()));
        assert_eq!(
            parse_wait(&args(&["!1", "x2"])),
            Err("x2 is no MR (!213 or 213)".into())
        );
        assert_eq!(
            parse_wait(&args(&["--then", "x"])),
            Err("name the MRs to wait for: pnpm wt:wait !213 !217".into())
        );
        assert_eq!(parse_wait(&args(&["!1", "--then="])).unwrap().then, None);
    }

    #[test]
    fn names_a_list_of_mrs() {
        assert_eq!(list_of(&[1]), "!1");
        assert_eq!(list_of(&[1, 2]), "!1 and !2");
        assert_eq!(list_of(&[1, 2, 3]), "!1, !2 and !3");
    }

    fn mr(iid: i64, state: &str, has_conflicts: bool) -> Facts {
        Facts {
            iid,
            state: state.into(),
            has_conflicts,
        }
    }

    #[test]
    fn judges_the_wait_from_gitlab_s_answers() {
        let mut since = HashMap::new();
        assert_eq!(
            verdict_of(
                &[1, 2],
                100,
                &[mr(1, "merged", false), mr(2, "merged", false)],
                0,
                &mut since
            ),
            Verdict::Merged
        );
        assert_eq!(
            verdict_of(
                &[1, 2],
                100,
                &[mr(1, "merged", false), mr(2, "closed", false)],
                0,
                &mut since
            ),
            Verdict::Closed(2)
        );
        assert_eq!(
            verdict_of(&[1, 2], 100, &[mr(1, "merged", false)], 0, &mut since),
            Verdict::Waiting
        );
        assert_eq!(
            verdict_of(&[1, 2], 100, &[mr(1, "merged", false)], 100, &mut since),
            Verdict::Timeout(vec![2])
        );
        assert_eq!(
            verdict_of(&[3], i64::MAX, &[mr(3, "opened", true)], 0, &mut since),
            Verdict::Waiting
        );
        assert_eq!(
            verdict_of(&[3], i64::MAX, &[mr(3, "opened", true)], CONFLICT_MS - 1, &mut since),
            Verdict::Waiting
        );
        assert_eq!(
            verdict_of(&[3], i64::MAX, &[mr(3, "opened", true)], CONFLICT_MS, &mut since),
            Verdict::Conflicts(3)
        );
        assert_eq!(
            verdict_of(&[3], i64::MAX, &[mr(3, "opened", false)], CONFLICT_MS, &mut since),
            Verdict::Waiting
        );
        assert!(since.is_empty());
    }

    #[test]
    fn ends_with_the_line_to_go_on_with() {
        let parsed = parse_wait(&args(&["!313", "!315", "--then", "go on"])).unwrap();
        assert_eq!(
            line_of(&parsed, &Verdict::Merged, "develop", Some(3)),
            "!313 and !315 are merged: develop is 3 behind: merge it in only if your work needs it or it conflicts, then go on"
        );
        let one = parse_wait(&args(&["!313"])).unwrap();
        assert_eq!(
            line_of(&one, &Verdict::Merged, "develop", Some(0)),
            "!313 is merged: develop is in already"
        );
        assert_eq!(
            line_of(&one, &Verdict::Merged, "develop", None),
            "!313 is merged: merge develop in only if your work needs it or it conflicts"
        );
        let short = parse_wait(&args(&["!316", "--hours", "0.0001"])).unwrap();
        assert_eq!(
            line_of(&short, &Verdict::Timeout(vec![316]), "develop", None),
            "waited 0.0001 h for !316, !316 not merged yet: the wait ends, tell the orchestrator or wait again (pnpm wt:wait)"
        );
        assert_eq!(
            line_of(&one, &Verdict::Timeout(vec![313]), "develop", None)
                .split(" h ")
                .next(),
            Some("waited 8")
        );
    }

    #[test]
    fn tells_a_missing_mr_from_glab_failing() {
        let ran = |is_ok: bool, stdout: &str, stderr: &str| glab::Ran {
            is_ok,
            stdout: stdout.into(),
            stderr: stderr.into(),
            is_unstarted: false,
        };
        assert_eq!(
            answer_of(&ran(true, "{\"iid\":3,\"state\":\"merged\"}", "")),
            Answer::Found(mr(3, "merged", false))
        );
        assert_eq!(answer_of(&ran(false, "", "glab: 404 Not found\n")), Answer::Missing);
        let failed = Answer::Failed("GitLab does not take glab's login (401 Unauthorized): glab auth login".into());
        assert_eq!(answer_of(&ran(false, "", "glab: 401 Unauthorized\n")), failed);
        assert_eq!(
            failure_of(&[Answer::Missing, failed]),
            Some("GitLab does not take glab's login (401 Unauthorized): glab auth login")
        );
        assert_eq!(failure_of(&[Answer::Missing]), None);
    }

    #[test]
    fn writes_whole_hours_as_json_does() {
        assert_eq!(hours_json(8.0).to_string(), "8");
        assert_eq!(hours_json(0.5).to_string(), "0.5");
    }
}
