//! Room made for an MR ahead (#419): a young pipeline of an MR behind it cancelled.

use super::first_line;
use crate::daemon::glab::{glab_get, glab_post};
use breadboard_core::ci::{self, Asked, Preempt, Preemption};
use breadboard_core::time::now_ms;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long after making room for an MR ahead the queue makes room again:
/// the runner's containers take a while to go, the slot shows free only then.
const PREEMPT_EVERY: Duration = Duration::from_secs(30);

/// When the queue last made room, for `PREEMPT_EVERY`.
#[derive(Default)]
pub struct Preempter {
    at: Option<Instant>,
}

impl Preempter {
    /// Whether it may make room now.
    pub fn is_due(&self) -> bool {
        self.at.is_none_or(|at| at.elapsed() >= PREEMPT_EVERY)
    }

    /// It makes room now.
    pub fn mark(&mut self) {
        self.at = Some(Instant::now());
    }
}

/// Cancels `preempt`'s pipeline while it is young and keeps it in `asked`: what the queue says it did.
pub fn step(main: &Path, preempt: Preempt, asked: &mut Asked, now: i64) -> String {
    let said = cancel_young(main, "glab", &preempt, now_ms());
    let Preempt { iid, pipeline, sha, by } = preempt;
    match said {
        Ok(true) => {
            asked.preempted.insert(iid, sha);
            asked.preempts.insert(iid, Preemption { by, at: now });
            asked.add_cancelled(pipeline);
            format!("!{iid} preempted by !{by}")
        }
        Ok(false) => format!("!{iid} kept, it finishes"),
        Err(error) => format!("!{iid} not held: {}", first_line(&error)),
    }
}

/// The pipeline to preempt read back first (#419): cancelled only while it
/// is still young (`ci::PREEMPT_GRACE_MS`) and none of its jobs went past
/// its first stage; true when it was cancelled.
fn cancel_young(main: &Path, glab: &str, preempt: &Preempt, now: i64) -> Result<bool, String> {
    let id = preempt.pipeline;
    let pipeline = glab_get(main, glab, &format!("projects/:fullpath/pipelines/{id}"))?;
    let status = pipeline
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let started = pipeline
        .get("started_at")
        .and_then(serde_json::Value::as_str)
        .and_then(breadboard_core::time::parse_iso);
    let is_young = match started {
        Some(started) => now - started < ci::PREEMPT_GRACE_MS,
        None => status != "running",
    };
    if !ci::is_live(status) || !is_young {
        return Ok(false);
    }
    let jobs = glab_get(
        main,
        glab,
        &format!("projects/:fullpath/pipelines/{id}/jobs?per_page=100"),
    )?;
    let jobs: Vec<(i64, String, String)> = jobs
        .as_array()
        .into_iter()
        .flatten()
        .map(|job| {
            let text = |key: &str| {
                job.get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            (
                job.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0),
                text("stage"),
                text("status"),
            )
        })
        .collect();
    if ci::is_past_first_stage(&jobs) {
        return Ok(false);
    }
    glab_post(main, glab, &format!("projects/:fullpath/pipelines/{id}/cancel"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::glab::fake_glab;

    #[test]
    fn preempts_only_a_pipeline_still_young_and_in_its_first_stage() {
        // #419: GitLab read back right before the cancel; the queue's read may be old.
        let scratch = std::env::temp_dir().join(format!("ci-preempt-test-{}", std::process::id()));
        let now = 1_790_000_000_000;
        let preempt = Preempt {
            iid: 381,
            pipeline: 42,
            sha: "b".into(),
            by: 371,
        };
        const PIPELINE: &str = "api projects/:fullpath/pipelines/42";
        const JOBS: &str = "api projects/:fullpath/pipelines/42/jobs?per_page=100";
        const CANCEL: &str = "api --method POST projects/:fullpath/pipelines/42/cancel";
        let started = |ago: i64| {
            format!(
                r#"{{"status":"running","started_at":"{}"}}"#,
                breadboard_core::time::to_iso(now - ago)
            )
        };
        let check = r#"[{"id":1,"stage":"check","status":"running"},{"id":2,"stage":"build","status":"created"}]"#;
        let build = r#"[{"id":1,"stage":"check","status":"success"},{"id":2,"stage":"build","status":"running"}]"#;
        for (pipeline, jobs, is_cancelled) in [
            (started(30_000), check, true),
            (started(30_000), build, false),
            (started(ci::PREEMPT_GRACE_MS), check, false),
            (r#"{"status":"success"}"#.to_string(), check, false),
        ] {
            let _ = std::fs::remove_file(scratch.join("glab.log"));
            let glab = fake_glab(&scratch, &[(PIPELINE, &pipeline), (JOBS, jobs)]);
            assert_eq!(
                cancel_young(&scratch, &glab, &preempt, now),
                Ok(is_cancelled),
                "{pipeline} {jobs}"
            );
            let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
            assert_eq!(log.contains(CANCEL), is_cancelled, "{log}");
        }
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
