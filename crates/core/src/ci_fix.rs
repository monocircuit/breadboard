//! A failed MR pipeline (#371), as the CI queue of `breadboard serve` takes
//! it: a job the runner failed (its system, a timeout) is retried, a job
//! that failed its first try too (a flake passes the second), a pipeline
//! GitLab refused at its creation (its activity limit, no jobs) is asked
//! for again. A job that failed twice from its own script is the MR's
//! doing: its agent fixes it, one run per head, at most `[ci]
//! max_fix_attempts` runs per MR; past them, and past the runner's retries,
//! the user looks. Pure: the daemon reads GitLab and retries, the overview
//! tells the agent and shows `CI fix 2/3` (`Fix`, in queue.json).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How often the runner's failure of a job is retried, and a refused pipeline asked for again.
pub const RUNNER_RETRIES: u32 = 3;
/// How long a refused pipeline waits before it is asked for again: GitLab's activity limit counts a while.
pub const REFUSED_AGAIN_MS: i64 = 5 * 60_000;

/// The failure reasons that are the runner's or GitLab's, not the job's script.
const RUNNER_FAILURES: &[&str] = &[
    "runner_system_failure",
    "stuck_or_timeout_failure",
    "job_execution_timeout",
    "api_failure",
    "scheduler_failure",
    "data_integrity_failure",
    "unknown_failure",
    "runner_unsupported",
    "no_matching_runner",
];

/// Whether a job's failure is the runner's (its system, a timeout), not its script's.
pub fn is_runner_failure(reason: Option<&str>) -> bool {
    reason.is_some_and(|reason| RUNNER_FAILURES.contains(&reason))
}

/// A try of a job, as GitLab's REST lists them with `include_retried`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Try {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub failure_reason: Option<String>,
    pub allow_failure: bool,
}

/// The tries of GitLab's answer to `pipelines/:id/jobs?include_retried=true`.
pub fn tries_of(listed: &serde_json::Value) -> Vec<Try> {
    listed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|job| {
            let text = |key: &str| job.get(key).and_then(serde_json::Value::as_str).map(String::from);
            Some(Try {
                id: job.get("id")?.as_i64()?,
                name: text("name")?,
                status: text("status").unwrap_or_default(),
                failure_reason: text("failure_reason"),
                allow_failure: job
                    .get("allow_failure")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// What a failed pipeline's jobs say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A job still to finish (a retry under way): nothing to decide.
    Live,
    /// The jobs to retry (id, name): the runner failed them, or their first try failed.
    Retry(Vec<(i64, String)>),
    /// The jobs that failed twice from their script: its agent fixes them.
    Fix(Vec<String>),
    /// The job the runner failed past RUNNER_RETRIES retries: the user looks.
    Runner(String),
    /// None of its jobs failed: GitLab refused it at its creation.
    Refused,
}

/// The verdict on a failed pipeline from every try of its jobs.
pub fn verdict_of(tries: &[Try]) -> Verdict {
    let mut last: BTreeMap<&str, &Try> = BTreeMap::new();
    let mut count: BTreeMap<&str, u32> = BTreeMap::new();
    for one in tries {
        *count.entry(&one.name).or_default() += 1;
        if last.get(one.name.as_str()).is_none_or(|known| known.id < one.id) {
            last.insert(&one.name, one);
        }
    }
    if last.values().any(|one| crate::ci::is_live(&one.status)) {
        return Verdict::Live;
    }
    let failed: Vec<&Try> = last
        .values()
        .filter(|one| one.status == "failed" && !one.allow_failure)
        .copied()
        .collect();
    if failed.is_empty() {
        return Verdict::Refused;
    }
    let tries_of = |one: &Try| count.get(one.name.as_str()).copied().unwrap_or(1);
    let fix: Vec<String> = failed
        .iter()
        .filter(|one| !is_runner_failure(one.failure_reason.as_deref()) && tries_of(one) >= 2)
        .map(|one| one.name.clone())
        .collect();
    if !fix.is_empty() {
        return Verdict::Fix(fix);
    }
    if let Some(stuck) = failed.iter().find(|one| tries_of(one) > RUNNER_RETRIES) {
        return Verdict::Runner(stuck.name.clone());
    }
    Verdict::Retry(failed.iter().map(|one| (one.id, one.name.clone())).collect())
}

/// Where an MR's failed pipeline stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Its failed jobs retried: the runner's failure, or a first try's.
    #[default]
    Retrying,
    /// Refused at its creation: asked for again.
    Refused,
    /// A job failed twice: its agent fixes it.
    Fix,
    /// Its agent had its `max` runs: the user looks.
    Bound,
    /// The runner fails a job again and again (or GitLab refuses the pipeline): the user looks.
    Runner,
}

/// An MR's failed pipeline for the overview (queue.json's `fixes`): what
/// is done about it, and the fix runs of its agent so far.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Fix {
    /// The pipeline the stage is of.
    pub pipeline: i64,
    pub stage: Stage,
    /// The jobs retried, to fix, that the runner fails.
    pub jobs: Vec<String>,
    /// The runs of its agent told to fix it so far, one per head.
    pub attempt: u32,
    /// How many may be (`[ci] max_fix_attempts`).
    pub max: u32,
}

impl Fix {
    /// `CI fix 2/3` while its agent fixes or fixed; None before the first run.
    pub fn count_text(&self) -> Option<String> {
        (self.attempt > 0).then(|| format!("CI fix {}/{}", self.attempt, self.max))
    }

    /// Its mark on the MR's row: `CI fix 2/3`, `retrying check:vita`, `CI fix 3/3 used`, `runner fails check:vita`.
    pub fn mark_text(&self) -> String {
        let jobs = self.jobs.join(", ");
        match self.stage {
            Stage::Retrying if self.attempt > 0 => format!("CI fix {}/{} · retrying {jobs}", self.attempt, self.max),
            Stage::Retrying => format!("retrying {jobs}"),
            Stage::Refused => "asked again".into(),
            Stage::Fix => self.count_text().unwrap_or_else(|| "CI fix".into()),
            Stage::Bound => format!("CI fix {}/{} used", self.attempt, self.max),
            Stage::Runner if jobs.is_empty() => "GitLab refuses it".into(),
            Stage::Runner => format!("runner fails {jobs}"),
        }
    }

    /// Past what breadboard does by itself: the user looks.
    pub fn is_for_user(&self) -> bool {
        matches!(self.stage, Stage::Bound | Stage::Runner)
    }

    /// The `look` action's words for an MR past what breadboard does by itself.
    pub fn look_text(&self, iid: i64) -> Option<String> {
        let jobs = self.jobs.join(", ");
        match self.stage {
            Stage::Bound => Some(format!(
                "!{iid} {jobs} still fails after {} CI fix runs of its agent: look at it",
                self.attempt
            )),
            Stage::Runner if jobs.is_empty() => Some(format!(
                "!{iid} GitLab refused its pipeline {RUNNER_RETRIES} more times: look at it"
            )),
            Stage::Runner => Some(format!(
                "!{iid} the runner failed {jobs} {} times: look at it",
                RUNNER_RETRIES + 1
            )),
            _ => None,
        }
    }
}

/// What the queue remembers of an MR's failed pipelines (asked.json's `fixes`) until one passes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Fixes {
    /// The heads its agent was told to fix, oldest first: one run each.
    pub heads: Vec<String>,
    /// The jobs it retried: a try is retried once.
    pub retried: Vec<i64>,
    /// The commit whose refused pipeline it asked for again, how often, when last (ms).
    pub refused: Option<(String, u32, i64)>,
    /// Where its newest failed pipeline stands.
    pub now: Option<Fix>,
}

/// What the queue does about a failed pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Retry these jobs.
    Retry(Vec<i64>),
    /// Ask for a new pipeline of its commit.
    AskAgain,
}

/// The verdict on the failed pipeline `pipeline` of head `sha` taken into
/// `fixes`: its new stage, and what the queue does now, if anything.
pub fn step(fixes: &mut Fixes, pipeline: i64, sha: &str, verdict: Verdict, max: u32, now: i64) -> Option<Act> {
    let attempt = fixes.heads.len() as u32;
    let fix = |stage: Stage, jobs: Vec<String>, attempt: u32| Fix {
        pipeline,
        stage,
        jobs,
        attempt,
        max,
    };
    match verdict {
        Verdict::Live => None,
        Verdict::Retry(jobs) => {
            let fresh: Vec<(i64, String)> = jobs.into_iter().filter(|(id, _)| !fixes.retried.contains(id)).collect();
            if fresh.is_empty() {
                return None;
            }
            fixes.retried.extend(fresh.iter().map(|(id, _)| *id));
            fixes.now = Some(fix(
                Stage::Retrying,
                fresh.iter().map(|(_, name)| name.clone()).collect(),
                attempt,
            ));
            Some(Act::Retry(fresh.into_iter().map(|(id, _)| id).collect()))
        }
        Verdict::Fix(jobs) => {
            let stage = if let Some(at) = fixes.heads.iter().position(|head| head == sha) {
                fixes.now = Some(fix(Stage::Fix, jobs, at as u32 + 1));
                return None;
            } else if attempt < max {
                fixes.heads.push(sha.to_string());
                Stage::Fix
            } else {
                Stage::Bound
            };
            fixes.now = Some(fix(stage, jobs, fixes.heads.len() as u32));
            None
        }
        Verdict::Runner(job) => {
            fixes.now = Some(fix(Stage::Runner, vec![job], attempt));
            None
        }
        Verdict::Refused => {
            let (times, at) = match &fixes.refused {
                Some((asked, times, at)) if asked == sha => (*times, *at),
                _ => (0, 0),
            };
            if times >= RUNNER_RETRIES {
                fixes.now = Some(fix(Stage::Runner, Vec::new(), attempt));
                return None;
            }
            fixes.now = Some(fix(Stage::Refused, Vec::new(), attempt));
            if times > 0 && now - at < REFUSED_AGAIN_MS {
                return None;
            }
            fixes.refused = Some((sha.to_string(), times + 1, now));
            Some(Act::AskAgain)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: i64, name: &str, status: &str, reason: Option<&str>) -> Try {
        Try {
            id,
            name: name.into(),
            status: status.into(),
            failure_reason: reason.map(String::from),
            allow_failure: false,
        }
    }
    const SCRIPT: Option<&str> = Some("script_failure");
    const RUNNER: Option<&str> = Some("runner_system_failure");

    #[test]
    fn a_first_failure_is_retried_a_second_is_fixed() {
        let first = [job(1, "check:vita", "failed", SCRIPT), job(2, "lint", "success", None)];
        assert_eq!(verdict_of(&first), Verdict::Retry(vec![(1, "check:vita".into())]));
        let again = [
            job(1, "check:vita", "failed", SCRIPT),
            job(2, "lint", "success", None),
            job(3, "check:vita", "failed", SCRIPT),
        ];
        assert_eq!(verdict_of(&again), Verdict::Fix(vec!["check:vita".into()]));
        // The retry under way: nothing to decide yet.
        let live = [
            job(1, "check:vita", "failed", SCRIPT),
            job(3, "check:vita", "pending", None),
        ];
        assert_eq!(verdict_of(&live), Verdict::Live);
        // An allowed failure holds nothing up; no failed job at all is a refusal.
        let allowed = Try {
            allow_failure: true,
            ..job(4, "audit", "failed", SCRIPT)
        };
        assert_eq!(verdict_of(&[allowed]), Verdict::Refused);
        assert_eq!(verdict_of(&[]), Verdict::Refused);
    }

    #[test]
    fn the_runners_failures_are_retried_never_fixed() {
        let mut tries = vec![job(1, "check:vita", "failed", RUNNER)];
        for id in 2..=RUNNER_RETRIES as i64 + 1 {
            assert_eq!(verdict_of(&tries), Verdict::Retry(vec![(id - 1, "check:vita".into())]));
            tries.push(job(id, "check:vita", "failed", Some("stuck_or_timeout_failure")));
        }
        assert_eq!(verdict_of(&tries), Verdict::Runner("check:vita".into()));
    }

    #[test]
    fn a_fix_run_per_head_up_to_the_bound() {
        let mut fixes = Fixes::default();
        let fix = || Verdict::Fix(vec!["check:vita".into()]);
        assert_eq!(step(&mut fixes, 10, "a", fix(), 2, 0), None);
        assert_eq!(fixes.now.as_ref().map(Fix::mark_text).as_deref(), Some("CI fix 1/2"));
        // The same head again (a click's new pipeline): no second run counted.
        step(&mut fixes, 11, "a", fix(), 2, 0);
        assert_eq!((fixes.heads.len(), fixes.now.as_ref().unwrap().attempt), (1, 1));
        step(&mut fixes, 12, "b", fix(), 2, 0);
        assert_eq!(fixes.now.as_ref().unwrap().stage, Stage::Fix);
        step(&mut fixes, 13, "c", fix(), 2, 0);
        let now = fixes.now.clone().unwrap();
        assert_eq!(
            (now.stage, now.mark_text()),
            (Stage::Bound, "CI fix 2/2 used".to_string())
        );
        assert_eq!(
            now.look_text(5).as_deref(),
            Some("!5 check:vita still fails after 2 CI fix runs of its agent: look at it")
        );
    }

    #[test]
    fn a_job_is_retried_once_a_refusal_asked_again_after_a_while() {
        let mut fixes = Fixes::default();
        let retry = || Verdict::Retry(vec![(7, "check:vita".into())]);
        assert_eq!(step(&mut fixes, 10, "a", retry(), 3, 0), Some(Act::Retry(vec![7])));
        assert_eq!(step(&mut fixes, 10, "a", retry(), 3, 0), None);
        let mut fixes = Fixes::default();
        assert_eq!(step(&mut fixes, 10, "a", Verdict::Refused, 3, 0), Some(Act::AskAgain));
        assert_eq!(step(&mut fixes, 11, "a", Verdict::Refused, 3, 1000), None);
        let mut at = 0;
        for _ in 1..RUNNER_RETRIES {
            at += REFUSED_AGAIN_MS;
            assert_eq!(step(&mut fixes, 11, "a", Verdict::Refused, 3, at), Some(Act::AskAgain));
        }
        step(&mut fixes, 12, "a", Verdict::Refused, 3, at + REFUSED_AGAIN_MS);
        assert!(fixes.now.as_ref().unwrap().is_for_user());
    }

    #[test]
    fn reads_the_tries_of_gitlabs_answer() {
        let listed = serde_json::json!([
            { "id": 3, "name": "check:vita", "status": "failed", "failure_reason": "script_failure", "allow_failure": false },
            { "id": 4, "name": "lint", "status": "success" }
        ]);
        assert_eq!(
            tries_of(&listed),
            vec![job(3, "check:vita", "failed", SCRIPT), job(4, "lint", "success", None)]
        );
    }
}
