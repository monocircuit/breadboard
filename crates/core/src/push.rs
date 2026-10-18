//! Unpushed commits pushed by breadboard (#450): a run cut off (herdr
//! killed, OOM) or a wt:check that waited long on the build lock leaves
//! commits unpushed while the agent idles, and pushing needs no LLM. So
//! `breadboard serve` takes them (its `push`): a headless agent's worktree
//! with unpushed commits, no run of its agent in flight, no wt:check
//! running and nothing uncommitted gets `wt:check` run by breadboard, one
//! at a time; green it is pushed, a draft MR opened when it has none. A red
//! check tells the agent once, with the failing task and the log's end; a
//! second red after its fix, or a push git refused, is the user's: a
//! `look` action saying why. What it is at stands in `.claude/pushing.json`
//! of the worktree, for its node (`5 commits not pushed · wt:check
//! running, then push`). Pure here: the daemon reads, runs and writes.

use crate::check::{Failure, failures_text};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `git config breadboard.push off`: no push by breadboard in this repo, its agents push.
pub const MODE_KEY: &str = "breadboard.push";
/// How long an agent's run must have ended before its commits are taken: its own push comes first.
pub const IDLE_MS: i64 = 60_000;
/// How many lines of wt:check's output end its line to the agent.
pub const TAIL_LINES: usize = 20;

/// `IDLE_MS`, or `BREADBOARD_PUSH_IDLE_MS` (the gauntlet's short one).
pub fn idle_ms() -> i64 {
    std::env::var("BREADBOARD_PUSH_IDLE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(IDLE_MS)
}

pub fn pushing_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("pushing.json")
}

/// Where breadboard's push of a worktree's HEAD stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Its wt:check runs; green, it is pushed.
    Checking,
    /// Its wt:check failed: the agent was told, its fix comes as a new HEAD.
    Told,
    /// The user was asked (a second red, a push refused): nothing more at this HEAD.
    Asked,
    /// Uncommitted changes would be part of the check: the agent was told.
    Dirty,
}

/// `.claude/pushing.json`: the HEAD it is about, its stage since `at`, the
/// red checks since the last push, why it is stuck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pushing {
    pub head: String,
    pub stage: Stage,
    pub at: i64,
    #[serde(default)]
    pub reds: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

pub fn read(dir: &Path) -> Option<Pushing> {
    serde_json::from_value(crate::json::read(&pushing_file(dir))?).ok()
}

pub fn write(dir: &Path, pushing: &Pushing) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(pushing).map_err(std::io::Error::other)?;
    crate::json::write_whole(&pushing_file(dir), &format!("{text}\n"))
}

pub fn clear(dir: &Path) {
    let _ = std::fs::remove_file(pushing_file(dir));
}

/// What the daemon knows of a worktree for its step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    pub head: String,
    pub unpushed: u64,
    /// Its agent's run in flight or queued, a claude open in it, a wt:check running, a wake pending.
    pub is_busy: bool,
    /// Changes not committed: wt:check would test them with the commits.
    pub is_dirty: bool,
    /// The last wt:check that passed tested HEAD's tree.
    pub is_tested: bool,
}

/// What to do with a worktree now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing, for now.
    Wait,
    /// Everything is pushed: the record goes.
    Clear,
    /// Uncommitted changes: the agent is told, once per HEAD.
    TellDirty,
    /// Tested: push.
    Push,
    /// wt:check first, then push.
    Check,
}

pub fn step_of(look: &Look, record: Option<&Pushing>) -> Step {
    if look.unpushed == 0 {
        return if record.is_some() { Step::Clear } else { Step::Wait };
    }
    if look.is_busy {
        return Step::Wait;
    }
    // Told or asked at this HEAD: its fix, or the user, comes as a new one.
    match record
        .filter(|record| record.head == look.head)
        .map(|record| record.stage)
    {
        Some(Stage::Told | Stage::Asked) => return Step::Wait,
        Some(Stage::Dirty) if look.is_dirty => return Step::Wait,
        _ => {}
    }
    if look.is_dirty {
        Step::TellDirty
    } else if look.is_tested {
        Step::Push
    } else {
        Step::Check
    }
}

/// The red checks after this one, since the last push: a second goes to the user.
pub fn reds_after(record: Option<&Pushing>) -> u32 {
    record.map_or(0, |record| record.reds) + 1
}

/// `3 commits`, `2 commits + a merge of develop`: what HEAD holds that its
/// upstream doesn't (#450: a merge of develop brings develop's commits too).
pub fn commits_text(unpushed: u64, own: u64, merged_base: Option<&str>) -> String {
    let counted = |count: u64| format!("{count} commit{}", if count == 1 { "" } else { "s" });
    match merged_base {
        Some(base) if own == 0 => format!("a merge of {base}"),
        Some(base) => format!("{} + a merge of {base}", counted(own)),
        None => counted(unpushed),
    }
}

/// The line a red check tells the agent: what failed, the log's end.
pub fn red_line(commits: &str, head: &str, failures: &[Failure], output: &str, run: &str) -> String {
    let failed = if failures.is_empty() {
        String::new()
    } else {
        format!(": {}", failures_text(failures))
    };
    let lines: Vec<&str> = output.trim_end().lines().collect();
    let tail = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
    format!(
        "[overview] your {commits} {} not pushed, and breadboard's wt:check of {} failed{failed}. Fix it, commit, {run} wt:check, git push in this run (when develop fails the same, it isn't yours: say so in the MR and go on). The end of its log:\n{tail}",
        if commits == "1 commit" { "is" } else { "are" },
        short(head)
    )
}

/// The line of a worktree whose commits wait on uncommitted changes.
pub fn dirty_line(commits: &str, run: &str) -> String {
    format!(
        "[overview] your {commits} {} not pushed, and the worktree has uncommitted changes breadboard won't check with them: commit them (or drop what isn't work), {run} wt:check, git push.",
        if commits == "1 commit" { "is" } else { "are" }
    )
}

/// The user's `look` after a second red: its agent's fix didn't make it green.
pub fn second_red_text(branch: &str, head: &str, failures: &[Failure]) -> String {
    let failed = if failures.is_empty() {
        String::new()
    } else {
        format!(" ({})", failures_text(failures))
    };
    format!(
        "{branch} stays unpushed: breadboard's wt:check of {} failed again after its agent's fix{failed}. Look at it (pnpm wt:check there), or tell its agent",
        short(head)
    )
}

/// The user's `look` after a push git refused.
pub fn refused_text(branch: &str, why: &str) -> String {
    let why = why.trim().lines().last().unwrap_or("").trim();
    format!("{branch} passed wt:check but git refused its push ({why}): look at it")
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look {
            head: "b".into(),
            unpushed: 2,
            ..Default::default()
        }
    }
    fn at(stage: Stage, head: &str) -> Pushing {
        Pushing {
            head: head.into(),
            stage,
            at: 0,
            reds: 1,
            why: None,
        }
    }

    #[test]
    fn checks_then_pushes_an_idle_agents_commits() {
        assert_eq!(step_of(&look(), None), Step::Check);
        assert_eq!(
            step_of(
                &Look {
                    is_tested: true,
                    ..look()
                },
                None
            ),
            Step::Push
        );
        // A check cut off (the daemon gone): checked again.
        assert_eq!(step_of(&look(), Some(&at(Stage::Checking, "b"))), Step::Check);
    }

    #[test]
    fn never_while_its_agent_runs_or_with_uncommitted_changes() {
        assert_eq!(
            step_of(
                &Look {
                    is_busy: true,
                    ..look()
                },
                None
            ),
            Step::Wait
        );
        let dirty = Look {
            is_dirty: true,
            ..look()
        };
        assert_eq!(step_of(&dirty, None), Step::TellDirty);
        assert_eq!(step_of(&dirty, Some(&at(Stage::Dirty, "b"))), Step::Wait);
        // Committed since: checked.
        assert_eq!(step_of(&look(), Some(&at(Stage::Dirty, "b"))), Step::Check);
    }

    #[test]
    fn a_told_or_asked_head_waits_for_a_new_one() {
        assert_eq!(step_of(&look(), Some(&at(Stage::Told, "b"))), Step::Wait);
        assert_eq!(step_of(&look(), Some(&at(Stage::Asked, "b"))), Step::Wait);
        assert_eq!(step_of(&look(), Some(&at(Stage::Told, "a"))), Step::Check);
        // Its second red goes to the user.
        assert_eq!(reds_after(None), 1);
        assert_eq!(reds_after(Some(&at(Stage::Told, "a"))), 2);
    }

    #[test]
    fn pushed_by_anyone_the_record_goes() {
        let pushed = Look { unpushed: 0, ..look() };
        assert_eq!(step_of(&pushed, Some(&at(Stage::Told, "b"))), Step::Clear);
        assert_eq!(step_of(&pushed, None), Step::Wait);
    }

    #[test]
    fn names_a_merge_of_the_base_apart_from_the_own_commits() {
        assert_eq!(commits_text(1, 1, None), "1 commit");
        assert_eq!(commits_text(77, 2, Some("develop")), "2 commits + a merge of develop");
        assert_eq!(commits_text(30, 0, Some("develop")), "a merge of develop");
    }

    #[test]
    fn tells_the_failing_task_and_the_logs_end() {
        let failures = [Failure {
            package: "@monocircuit/breadboard".into(),
            task: "test".into(),
            tests: vec!["the overview draws".into()],
        }];
        let output = (1..=30).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        let line = red_line("2 commits", "0123456789ab", &failures, &output, "pnpm");
        assert!(line.starts_with("[overview] your 2 commits are not pushed, and breadboard's wt:check of 01234567 failed: @monocircuit/breadboard"), "{line}");
        assert!(line.contains("the overview draws"), "{line}");
        assert!(line.ends_with("line 11\nline 12\nline 13\nline 14\nline 15\nline 16\nline 17\nline 18\nline 19\nline 20\nline 21\nline 22\nline 23\nline 24\nline 25\nline 26\nline 27\nline 28\nline 29\nline 30"), "{line}");
        assert!(!line.contains("line 10\n"));
        assert_eq!(
            refused_text(
                "feature/9-x",
                "To origin\n ! [rejected] HEAD -> feature/9-x (fetch first)\n"
            ),
            "feature/9-x passed wt:check but git refused its push (! [rejected] HEAD -> feature/9-x (fetch first)): look at it"
        );
    }
}
