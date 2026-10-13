//! An agent that waits for other MRs (`pnpm wt:wait !213 !217 --then "…"`):
//! the wait in `.claude/wait.json` of its worktree, what GitLab's answers mean
//! for it, and the one line it goes on with (worktree/wait.mjs).

use crate::json::{self, ints_of, str_of};
use crate::lock::is_alive;
use crate::time::parse_iso;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// How long it waits by default before it gives up, in hours.
pub const DEFAULT_HOURS: f64 = 8.0;
/// How long an awaited MR may have conflicts with develop before the wait ends.
pub const CONFLICT_MS: i64 = 30 * 60_000;

/// A wait: `{ mrs, then, hours, since, until, pid }`; `raw` as stored.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Wait {
    pub mrs: Vec<i64>,
    pub then: Option<String>,
    pub hours: Option<f64>,
    pub since: Option<String>,
    pub until: Option<String>,
    /// The command that waits: gone after a restart of the agent.
    pub pid: Option<i64>,
    pub raw: Value,
}

pub fn wait_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("wait.json")
}

impl Wait {
    pub fn from_value(value: &Value) -> Option<Wait> {
        let mrs = ints_of(value, "mrs");
        if mrs.is_empty() {
            return None;
        }
        Some(Wait {
            mrs,
            then: str_of(value, "then").map(String::from),
            hours: value.get("hours").and_then(Value::as_f64),
            since: str_of(value, "since").map(String::from),
            until: str_of(value, "until").map(String::from),
            pid: json::int_of(value, "pid"),
            raw: value.clone(),
        })
    }
}

/// The wait of the worktree `dir`, None when it waits for nothing.
pub fn wait_of(dir: &Path) -> Option<Wait> {
    Wait::from_value(&json::read(&wait_file(dir))?)
}

pub fn record_wait(dir: &Path, wait: &Value) -> io::Result<()> {
    json::write_pretty(&wait_file(dir), wait)
}

pub fn clear_wait(dir: &Path) {
    let _ = fs::remove_file(wait_file(dir));
}

/// Whether the command of `wait` still runs.
pub fn is_waiting(wait: &Wait) -> bool {
    wait.pid.is_some_and(is_alive)
}

/// What `wt:wait` was given: `{ mrs, then, hours }`, or why not.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub mrs: Vec<i64>,
    pub then: Option<String>,
    pub hours: f64,
}

/// The command line after `wait`: MRs as `!213` or `213`, each once, in order.
pub fn parse_wait(argv: &[&str]) -> Result<Parsed, String> {
    let mut mrs = Vec::new();
    let mut then = None;
    let mut hours = DEFAULT_HOURS;
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i];
        let (flag, inline) = match arg.strip_prefix("--") {
            Some(_) => match arg.split_once('=') {
                Some((flag, value)) => (Some(flag), Some(value)),
                None => (Some(arg), None),
            },
            None => (None, None),
        };
        if let Some(flag @ ("--then" | "--hours")) = flag {
            let value = match inline {
                Some(value) => value,
                None => {
                    i += 1;
                    match argv.get(i) {
                        Some(value) if !value.starts_with("--") => value,
                        _ => return Err(format!("{flag} needs a value")),
                    }
                }
            };
            if flag == "--then" {
                then = Some(value.trim().to_string()).filter(|then| !then.is_empty());
            } else {
                hours = value.parse().unwrap_or(f64::NAN);
                if hours.is_nan() || hours <= 0.0 {
                    return Err(format!("--hours takes a number above 0, not {value}"));
                }
            }
        } else if let Some(iid) = arg.strip_prefix('!').unwrap_or(arg).parse::<i64>().ok().filter(|_| {
            let digits = arg.strip_prefix('!').unwrap_or(arg);
            !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
        }) {
            if !mrs.contains(&iid) {
                mrs.push(iid);
            }
        } else {
            return Err(format!("{arg} is no MR (!213) and no flag"));
        }
        i += 1;
    }
    if mrs.is_empty() {
        return Err("name the MRs to wait for: !213 !217".into());
    }
    Ok(Parsed { mrs, then, hours })
}

/// `!213`, `!213 and !217`, `!213, !217 and !219`.
pub fn list_of(mrs: &[i64]) -> String {
    let names: Vec<String> = mrs.iter().map(|iid| format!("!{iid}")).collect();
    match names.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// What the overview shows of a wait: the MRs still open, `waits for !213,
/// !217` (`· 5 merged` once some are, #262), `waits: all merged, wakes up`.
pub fn waits_text(wait: &Wait, merged: &HashSet<i64>) -> String {
    let open: Vec<i64> = wait.mrs.iter().copied().filter(|iid| !merged.contains(iid)).collect();
    if open.is_empty() {
        return "waits: all merged, wakes up".into();
    }
    let done = wait.mrs.len() - open.len();
    let names: Vec<String> = open.iter().map(|iid| format!("!{iid}")).collect();
    format!(
        "waits for {}{}",
        names.join(", "),
        if done > 0 {
            format!(" · {done} merged")
        } else {
            String::new()
        }
    )
}

/// An awaited MR as GitLab has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Awaited {
    pub iid: i64,
    pub state: String,
    pub has_conflicts: bool,
}

/// Where a wait stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Every one merged: go on.
    Merged,
    /// One closed without a merge.
    Closed(i64),
    /// One had conflicts for CONFLICT_MS.
    Conflicts(i64),
    /// Past `until`, these not merged.
    Timeout(Vec<i64>),
    /// These not merged yet.
    Waiting(Vec<i64>),
}

/// Where `wait` stands from the awaited MRs (one missing counts as not
/// answered yet), at `now`; `conflicts_since` kept by the caller between asks.
pub fn verdict_of(wait: &Wait, found: &[Awaited], now: i64, conflicts_since: &mut HashMap<i64, i64>) -> Verdict {
    let by_iid: HashMap<i64, &Awaited> = found.iter().map(|mr| (mr.iid, mr)).collect();
    if let Some(closed) = wait
        .mrs
        .iter()
        .find(|iid| by_iid.get(iid).is_some_and(|mr| mr.state == "closed"))
    {
        return Verdict::Closed(*closed);
    }
    let open: Vec<i64> = wait
        .mrs
        .iter()
        .copied()
        .filter(|iid| by_iid.get(iid).is_none_or(|mr| mr.state != "merged"))
        .collect();
    if open.is_empty() {
        return Verdict::Merged;
    }
    for iid in &open {
        if !by_iid.get(iid).is_some_and(|mr| mr.has_conflicts) {
            conflicts_since.remove(iid);
        } else if let Some(since) = conflicts_since.get(iid) {
            if now - since >= CONFLICT_MS {
                return Verdict::Conflicts(*iid);
            }
        } else {
            conflicts_since.insert(*iid, now);
        }
    }
    if wait
        .until
        .as_deref()
        .and_then(parse_iso)
        .is_some_and(|until| now >= until)
    {
        return Verdict::Timeout(open);
    }
    Verdict::Waiting(open)
}

/// The line the agent goes on with, for a verdict that ends the wait;
/// `behind` how many commits of its base (`develop`, an epic) the worktree lacks, None unknown.
pub fn line_of(wait: &Wait, verdict: &Verdict, base: &str, behind: Option<u64>) -> Option<String> {
    let all = list_of(&wait.mrs);
    let then = wait
        .then
        .as_ref()
        .map(|then| format!(", then {then}"))
        .unwrap_or_default();
    Some(match verdict {
        Verdict::Merged => {
            let merge = match behind {
                Some(0) => format!("{base} is in already"),
                Some(behind) => format!("merge {base} in ({behind} behind)"),
                None => format!("merge {base} in"),
            };
            format!(
                "{all} {} merged: {merge}{then}",
                if wait.mrs.len() > 1 { "are" } else { "is" }
            )
        }
        Verdict::Closed(mr) => format!(
            "!{mr} was closed without a merge: the wait for {all} ends, find out what replaces it (glab mr view {mr}) before you go on"
        ),
        Verdict::Conflicts(mr) => format!(
            "!{mr} has had conflicts with {base} for {} min: the wait for {all} ends, tell the orchestrator (SendMessage) instead of waiting on",
            CONFLICT_MS / 60_000
        ),
        Verdict::Timeout(open) => {
            let hours = wait.hours.unwrap_or(DEFAULT_HOURS);
            format!(
                "waited {} h for {all}, {} not merged yet: the wait ends, tell the orchestrator or wait again ({run} wt:wait)",
                format_number(hours),
                list_of(open),
                run = crate::config::run_word()
            )
        }
        Verdict::Waiting(_) => return None,
    })
}

/// A number as JavaScript prints it: `8`, `0.5`.
pub fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::scratch::Scratch;
    use serde_json::json;

    fn wait() -> Wait {
        Wait {
            mrs: vec![213, 217],
            then: Some("open the MR".into()),
            hours: Some(8.0),
            ..Default::default()
        }
    }
    fn mr(iid: i64, state: &str, has_conflicts: bool) -> Awaited {
        Awaited {
            iid,
            state: state.into(),
            has_conflicts,
        }
    }

    #[test]
    fn takes_mrs_with_and_without_the_mark_each_once() {
        assert_eq!(
            parse_wait(&["!213", "217", "!213", "--then", "go on"]),
            Ok(Parsed {
                mrs: vec![213, 217],
                then: Some("go on".into()),
                hours: DEFAULT_HOURS
            })
        );
        assert_eq!(
            parse_wait(&["!5", "--then=a b", "--hours=0.5"]),
            Ok(Parsed {
                mrs: vec![5],
                then: Some("a b".into()),
                hours: 0.5
            })
        );
    }

    #[test]
    fn refuses_what_is_no_wait() {
        assert!(parse_wait(&[]).is_err());
        assert!(parse_wait(&["--then", "x"]).is_err());
        assert!(parse_wait(&["!5", "soon"]).is_err());
        assert!(parse_wait(&["!5", "--then"]).is_err());
        assert!(parse_wait(&["!5", "--then", "--hours", "2"]).is_err());
        assert!(parse_wait(&["!5", "--hours", "0"]).is_err());
    }

    #[test]
    fn waits_while_one_is_open_and_goes_on_once_all_are_merged() {
        let mut since = HashMap::new();
        assert_eq!(
            verdict_of(
                &wait(),
                &[mr(213, "merged", false), mr(217, "opened", false)],
                0,
                &mut since
            ),
            Verdict::Waiting(vec![217])
        );
        assert_eq!(
            verdict_of(
                &wait(),
                &[mr(213, "merged", false), mr(217, "merged", false)],
                0,
                &mut since
            ),
            Verdict::Merged
        );
        assert_eq!(
            verdict_of(&wait(), &[mr(213, "merged", false)], 0, &mut since),
            Verdict::Waiting(vec![217])
        );
        assert_eq!(
            verdict_of(
                &wait(),
                &[mr(213, "merged", false), mr(217, "closed", false)],
                0,
                &mut since
            ),
            Verdict::Closed(217)
        );
    }

    #[test]
    fn waits_out_conflicts_for_a_while_then_ends() {
        let mut since = HashMap::new();
        let found = [mr(213, "opened", true), mr(217, "opened", false)];
        assert_eq!(
            verdict_of(&wait(), &found, 0, &mut since),
            Verdict::Waiting(vec![213, 217])
        );
        assert_eq!(
            verdict_of(&wait(), &found, CONFLICT_MS - 1, &mut since),
            Verdict::Waiting(vec![213, 217])
        );
        assert_eq!(
            verdict_of(&wait(), &found, CONFLICT_MS, &mut since),
            Verdict::Conflicts(213)
        );

        let mut since = HashMap::new();
        verdict_of(&wait(), &[mr(213, "opened", true)], 0, &mut since);
        verdict_of(&wait(), &[mr(213, "opened", false)], 1, &mut since);
        assert!(matches!(
            verdict_of(&wait(), &[mr(213, "opened", true)], CONFLICT_MS, &mut since),
            Verdict::Waiting(_)
        ));
    }

    #[test]
    fn ends_past_its_limit() {
        let until = Wait {
            until: Some("1970-01-01T00:00:01.000Z".into()),
            ..wait()
        };
        assert_eq!(
            verdict_of(&until, &[mr(213, "merged", false)], 1000, &mut HashMap::new()),
            Verdict::Timeout(vec![217])
        );
    }

    #[test]
    fn says_it_in_lines() {
        assert_eq!(list_of(&[1]), "!1");
        assert_eq!(list_of(&[1, 2]), "!1 and !2");
        assert_eq!(list_of(&[1, 2, 3]), "!1, !2 and !3");
        assert_eq!(waits_text(&wait(), &HashSet::new()), "waits for !213, !217");
        let six = Wait {
            mrs: vec![269, 270, 272, 273, 274, 275],
            ..Default::default()
        };
        let mut merged: HashSet<i64> = [269, 270, 272, 274, 275].into();
        assert_eq!(waits_text(&six, &merged), "waits for !273 · 5 merged");
        merged.insert(273);
        assert_eq!(waits_text(&six, &merged), "waits: all merged, wakes up");
        assert_eq!(
            line_of(&wait(), &Verdict::Merged, "develop", Some(3)).unwrap(),
            "!213 and !217 are merged: merge develop in (3 behind), then open the MR"
        );
        let five = Wait {
            mrs: vec![5],
            ..Default::default()
        };
        assert_eq!(
            line_of(&five, &Verdict::Merged, "develop", Some(0)).unwrap(),
            "!5 is merged: develop is in already"
        );
        assert_eq!(
            line_of(&five, &Verdict::Merged, "develop", None).unwrap(),
            "!5 is merged: merge develop in"
        );
        assert!(
            line_of(&wait(), &Verdict::Closed(217), "develop", None)
                .unwrap()
                .starts_with("!217 was closed without a merge")
        );
        assert!(
            line_of(&wait(), &Verdict::Conflicts(213), "develop", None)
                .unwrap()
                .starts_with("!213 has had conflicts")
        );
        assert!(
            line_of(&wait(), &Verdict::Timeout(vec![217]), "develop", None)
                .unwrap()
                .starts_with("waited 8 h for !213 and !217, !217 not merged yet")
        );
        assert_eq!(line_of(&wait(), &Verdict::Waiting(vec![217]), "develop", None), None);
    }

    #[test]
    fn is_written_read_and_cleared_in_the_worktree() {
        let scratch = Scratch::new();
        assert_eq!(wait_of(scratch.path()), None);
        record_wait(
            scratch.path(),
            &json!({ "mrs": [213, 217], "then": "x", "pid": std::process::id() }),
        )
        .unwrap();
        let read = wait_of(scratch.path()).unwrap();
        assert_eq!(read.mrs, vec![213, 217]);
        assert!(is_waiting(&read));
        clear_wait(scratch.path());
        assert_eq!(wait_of(scratch.path()), None);
        assert!(!is_waiting(&Wait {
            pid: Some((1 << 22) + 1),
            ..Default::default()
        }));
        assert!(!is_waiting(&Wait::default()));
    }
}
