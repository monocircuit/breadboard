//! A run cut off (#445): a crash, an OOM kill or a reboot ended its claude
//! while the status table still says `running`. The run engine picks each
//! up once by itself, a few at a time (`[agents] max_runs`), instead of the
//! orchestrator waking every agent by hand. A small session is resumed
//! with one line; one the fresh-start rule retires (its cache is gone after
//! a reboot) is not resumed at all: a fresh session takes its worktree with
//! a handoff breadboard writes from what is on disk (its last answer, the
//! worktree's git state), so a restart costs a brief, not a history.

use crate::conversations::{Claude, bound_session, new_session_id, read_claude, session_file};
use crate::fresh::Fresh;
use crate::runs::{Event, Queue, Run, State, Table};
use crate::time::hm;
use serde_json::{Map, Value, json};
use std::path::Path;

/// Where the handoff of a cut-off agent is written for the fresh one.
pub const HANDOFF_CUT: &str = ".claude/handoff-cut.md";

/// The sessions whose run was cut off: `running` in the table, its claude
/// gone, not one this engine runs (`flying`).
pub fn cut_off_of(table: &Table, is_flying: impl Fn(&str) -> bool) -> Vec<String> {
    table
        .runs
        .iter()
        .filter(|(session, run)| {
            run.state == State::Running && !is_flying(session) && !run.pid.is_some_and(crate::lock::is_alive)
        })
        .map(|(session, _)| session.clone())
        .collect()
}

/// The line a small cut-off session is resumed with.
pub fn resume_line(since: i64) -> String {
    format!(
        "[breadboard] your last run was cut off at {} (a crash, an OOM kill or a reboot); nothing on disk is lost. Check where you are (git status, your MR, your issue's boxes) and go on; push what is committed.",
        hm(since)
    )
}

/// The first line of the fresh session that takes a cut-off agent's worktree.
pub fn fresh_line(since: i64, why: &str) -> String {
    format!(
        "[breadboard] fresh start: the agent of this worktree before you was cut off at {} (a crash, an OOM kill or a reboot) and is not resumed: {why}. You take its work over. Read .claude/brief.md, then {HANDOFF_CUT} (what it did last, the worktree's state), check your MR and your issue's boxes, and go on.",
        hm(since)
    )
}

/// The handoff breadboard writes for the fresh session: the cut-off one's
/// last prompt and answer, the worktree's uncommitted and unpushed work.
pub fn handoff_of(claude: Option<&Claude>, status: &str, unpushed: &str) -> String {
    let said = |text: Option<&str>| {
        text.map(|text| crate::said::sentences(text, 1200))
            .unwrap_or_else(|| "none".into())
    };
    format!(
        "# Handoff of a cut-off agent\n\nWritten by breadboard from what is on disk (#445), not by the agent: where it and the code, the MR or the issue differ, those win.\n\n## Its last prompt\n\n{}\n\n## Its last answer\n\n{}\n\n## Uncommitted (git status --short)\n\n```\n{}\n```\n\n## Committed, not pushed\n\n```\n{}\n```\n",
        said(claude.and_then(|claude| claude.prompt.as_deref())),
        said(claude.and_then(|claude| claude.answer.as_deref())),
        if status.trim().is_empty() {
            "nothing"
        } else {
            status.trim_end()
        },
        if unpushed.trim().is_empty() {
            "nothing"
        } else {
            unpushed.trim_end()
        },
    )
}

/// What picking up `session` (its row `run`) at `now` does: the line queued,
/// a fresh session in place of it when `fresh` retires it. Its row is set
/// `idle`. A session whose worktree is gone is only set idle.
pub fn pick_up(main: &Path, session: &str, run: &mut Run, fresh: &Fresh, now: i64) -> Option<Event> {
    let since = run.since;
    run.state = State::Idle;
    run.pid = None;
    run.since = now;
    if !run.dir.is_dir() {
        return None;
    }
    let claude = read_claude(&run.dir).filter(|claude| claude.id == session);
    let why = claude.as_ref().and_then(|claude| fresh.why(&claude.context(), now));
    let Some(why) = why else {
        return Some(event(session, run, resume_line(since), now));
    };
    // Fresh: its handoff from the disk, a new session bound, the record's agent moved to it.
    let git = |args: &[&str]| crate::git::git(&run.dir, args).out().to_string();
    let handoff = handoff_of(
        claude.as_ref(),
        &git(&["status", "--short"]),
        &git(&["log", "--oneline", "@{upstream}..HEAD"]),
    );
    if std::fs::write(run.dir.join(HANDOFF_CUT), handoff).is_err() {
        return Some(event(session, run, resume_line(since), now));
    }
    let id = new_session_id();
    if std::fs::write(session_file(&run.dir), format!("{id}\n")).is_err() {
        return Some(event(session, run, resume_line(since), now));
    }
    move_worker(
        main,
        &run.dir,
        session,
        &id,
        claude.as_ref().map_or(0, |claude| claude.used),
        now,
    );
    Some(event(&id, run, fresh_line(since, &why), now))
}

/// The record's agent of `old` goes on as `new` (its name, person, issues
/// and files); `old` is kept as retired, at `used` tokens.
fn move_worker(main: &Path, dir: &Path, old: &str, new: &str, used: u64, now: i64) {
    let Some(origin) = crate::record::origin_of(dir) else {
        return;
    };
    let raw = crate::json::read(&crate::record::record_file(main, &origin.name));
    let known = raw
        .as_ref()
        .and_then(|raw| raw.get("workers")?.get(old))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut moved: Map<String, Value> = known
        .into_iter()
        .filter(|(key, _)| ["name", "display_name", "issues", "files", "mrs"].contains(&key.as_str()))
        .collect();
    moved.insert("history".into(), json!([]));
    let _ = crate::record::write_worker(main, &origin.name, new, moved);
    let retired = json!({
        "retired": {
            "at": crate::time::to_iso(now),
            "from": crate::git::feature_of(main, dir),
            "tokens": used,
            "handoff": null,
            "cut_off": true,
        },
    });
    let _ = crate::record::write_worker(
        main,
        &origin.name,
        old,
        retired.as_object().cloned().unwrap_or_default(),
    );
}

fn event(session: &str, run: &Run, line: String, now: i64) -> Event {
    Event {
        session: session.to_string(),
        dir: run.dir.clone(),
        name: run.name.clone(),
        line,
        at: now,
        allowed: Vec::new(),
        is_notice: false,
    }
}

/// Whether `session` is bound to its worktree still: one handed over or freed meanwhile is not picked up.
pub fn is_bound(run: &Run, session: &str) -> bool {
    bound_session(&run.dir).as_deref() == Some(session)
}

/// `the run of 7-x was cut off: picked up (resumed)`, for the log.
pub fn said_of(run: &Run, event: &Event, session: &str) -> String {
    if event.session == session {
        format!("the run of {} was cut off: resumed once", run.name)
    } else {
        format!("the run of {} was cut off: a fresh session takes it over", run.name)
    }
}

/// The queue with the picked up lines first (their sessions waited longest).
pub fn queue_first(queue: &mut Queue, mut events: Vec<Event>) {
    events.append(&mut queue.events);
    queue.events = events;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn run(state: State, pid: Option<i64>) -> Run {
        Run {
            dir: PathBuf::from("/nonexistent/platform-7-x"),
            name: "7-x".into(),
            state,
            since: 0,
            pid,
            lines: 1,
            denials: Vec::new(),
            code: None,
        }
    }

    #[test]
    fn finds_the_runs_whose_claude_is_gone() {
        let table = Table {
            runs: BTreeMap::from([
                ("dead".to_string(), run(State::Running, Some(999_999_999))),
                (
                    "alive".to_string(),
                    run(State::Running, Some(i64::from(std::process::id()))),
                ),
                ("flying".to_string(), run(State::Running, None)),
                ("idle".to_string(), run(State::Idle, None)),
            ]),
        };
        assert_eq!(cut_off_of(&table, |session| session == "flying"), ["dead"]);
    }

    #[test]
    fn a_gone_worktree_is_only_set_idle() {
        let mut gone = run(State::Running, Some(999_999_999));
        assert_eq!(
            pick_up(Path::new("/nonexistent"), "s", &mut gone, &Fresh::default(), 5),
            None
        );
        assert_eq!((gone.state, gone.since), (State::Idle, 5));
    }

    #[test]
    fn the_fresh_one_gets_a_handoff_of_what_is_on_disk() {
        let handoff = handoff_of(None, " M src/a.rs\n", "abc1234 feat: x\n");
        assert!(handoff.contains("not by the agent"));
        assert!(handoff.contains("```\n M src/a.rs\n```"));
        assert!(handoff.contains("```\nabc1234 feat: x\n```"));
        assert!(handoff_of(None, "", "").contains("```\nnothing\n```"));
        assert!(fresh_line(0, "idle 2 h with 120 k tokens").contains(&format!("then {HANDOFF_CUT}")));
        assert!(resume_line(0).starts_with("[breadboard] your last run was cut off at "));
    }
}
