//! Whether a `try` can be tried now (try-state.mjs, #237). One on the
//! worktree's app is ready once its commits are pushed; one on the test
//! instance (`wt:action try --on test`) once its MR is merged and the develop
//! pipeline of the merge commit deployed it: `waits for merge !N` →
//! `deploying (publish, 3 min)` → `ready (deploy:develop passed 2 min ago)`,
//! or `deploy failed (<job>)`. When the merge commit's own pipeline failed,
//! a later develop pipeline that contains it decides (#275).

use super::remote::Job;
use crate::cli::read::gitlab;
use breadboard_core::actions::Action;
use breadboard_core::json::{int_of, str_of};
use breadboard_core::time::{parse_iso, span};
use serde_json::Value;
use std::path::Path;

/// How long Coolify gets after `deploy:develop` passed to pull and start the image.
const DEPLOY_GRACE_MS: i64 = 90_000;
/// The jobs of the develop pipeline that deploy: the others don't hold a try.
const DEPLOY_JOBS: [&str; 3] = ["build:develop", "publish:develop", "deploy:develop"];
const DEPLOY_JOB: &str = "deploy:develop";
/// Without a deploy job, publishing is the last step.
const PUBLISH_JOB: &str = "publish:develop";
/// A pipeline that no longer runs.
const DONE: [&str; 4] = ["success", "failed", "canceled", "skipped"];

/// A develop pipeline that deploys a merge commit, with its jobs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Deploy {
    /// None while GitLab has none for the commit yet.
    pub pipeline: Option<DeployPipeline>,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeployPipeline {
    pub id: i64,
    pub status: String,
    pub url: Option<String>,
    pub created_at: Option<i64>,
}

/// Where a deployment stands (try-state.mjs `deployStateOf`).
#[derive(Debug, Clone, PartialEq)]
enum DeployState {
    /// At `job` (the one running, else the first to come), since the pipeline's start;
    /// `passed` when the last job passed and Coolify is still starting it.
    Deploying {
        job: Option<Job>,
        since: Option<i64>,
        passed: Option<i64>,
    },
    /// The last deploying job passed DEPLOY_GRACE_MS ago, at `passed`.
    Deployed { job: Job, passed: i64 },
    /// A deploying job failed or was cancelled, or a job before them.
    Failed { job: Job },
}

fn is_failed(job: &Job) -> bool {
    matches!(job.status.as_str(), "failed" | "canceled")
}

fn deploy_state_of(deploy: Option<&Deploy>, now: i64) -> DeployState {
    let Some((deploy, pipeline)) = deploy.and_then(|deploy| Some((deploy, deploy.pipeline.as_ref()?))) else {
        return DeployState::Deploying {
            job: None,
            since: None,
            passed: None,
        };
    };
    let jobs: Vec<&Job> = deploy
        .jobs
        .iter()
        .filter(|job| DEPLOY_JOBS.contains(&job.name.as_str()))
        .collect();
    let since = pipeline.created_at;
    if let Some(failed) = jobs.iter().find(|job| is_failed(job)) {
        return DeployState::Failed { job: (*failed).clone() };
    }
    let is_done = DONE.contains(&pipeline.status.as_str());
    let last = jobs.iter().find(|job| job.name == DEPLOY_JOB).or_else(|| {
        if is_done {
            jobs.iter().find(|job| job.name == PUBLISH_JOB)
        } else {
            None
        }
    });
    if let Some(last) = last.filter(|job| job.status == "success") {
        let passed = last.finished_at.as_deref().and_then(parse_iso).unwrap_or(now);
        return if now - passed >= DEPLOY_GRACE_MS {
            DeployState::Deployed {
                job: (*last).clone(),
                passed,
            }
        } else {
            DeployState::Deploying {
                job: Some((*last).clone()),
                since,
                passed: Some(passed),
            }
        };
    }
    // Ended without the deploying jobs passing, none failed: a job before them did.
    if is_done && pipeline.status != "success" {
        let job = deploy
            .jobs
            .iter()
            .find(|job| is_failed(job))
            .or(jobs.last().copied())
            .cloned()
            .unwrap_or_else(|| Job {
                name: "pipeline".into(),
                url: pipeline.url.clone(),
                ..Job::default()
            });
        return DeployState::Failed { job };
    }
    let job = jobs
        .iter()
        .find(|job| job.status == "running")
        .or_else(|| jobs.iter().find(|job| job.status != "success"))
        .map(|job| (*job).clone());
    DeployState::Deploying {
        job,
        since,
        passed: None,
    }
}

/// Whether a try can be tried now, and what its head says: `tone` one of
/// `ready`, `waits`, `failed`.
#[derive(Debug, Clone, PartialEq)]
pub struct TryState {
    pub is_ready: bool,
    pub label: String,
    pub tone: &'static str,
}

fn ready(label: String) -> TryState {
    TryState {
        is_ready: true,
        label,
        tone: "ready",
    }
}

fn waits(label: String) -> TryState {
    TryState {
        is_ready: false,
        label,
        tone: "waits",
    }
}

fn failed(label: String) -> TryState {
    TryState {
        is_ready: false,
        label,
        tone: "failed",
    }
}

/// The MR of a try's worktree, as far as its state needs it.
pub struct TryMr<'a> {
    pub iid: i64,
    pub state: &'a str,
    pub deploy: Option<&'a Deploy>,
}

/// The state of the try `action` (try-state.mjs `tryStateOf`): `unpushed`
/// the worktree's commits origin doesn't have, `mr` its MR, `origin` the
/// test instance's, `has_remote` whether GitLab answered.
pub fn try_state_of(
    action: &Action,
    unpushed: u64,
    mr: Option<TryMr>,
    origin: Option<&str>,
    has_remote: bool,
    now: i64,
) -> TryState {
    if action.on.as_deref() != Some("test") {
        return if unpushed > 0 {
            waits("not pushed".into())
        } else {
            ready("ready".into())
        };
    }
    if origin.is_none() {
        return failed("no test instance: VITA_TEST_ORIGIN unset".into());
    }
    if !has_remote {
        return waits("GitLab …".into());
    }
    let Some(mr) = mr else {
        return waits("waits for its MR".into());
    };
    if mr.state == "closed" {
        return failed(format!("!{} closed", mr.iid));
    }
    if mr.state != "merged" {
        return waits(format!("waits for merge !{}", mr.iid));
    }
    match deploy_state_of(mr.deploy, now) {
        DeployState::Failed { job } => failed(format!("deploy failed ({})", job.name)),
        DeployState::Deployed { job, passed } => {
            ready(format!("ready ({} passed {} ago)", job.name, span(now - passed)))
        }
        DeployState::Deploying { job, since, passed } => {
            let at = match (&job, passed) {
                (Some(job), Some(_)) => format!("{} passed, Coolify starts it", job.name),
                (Some(job), None) => job.name.split(':').next().unwrap_or_default().to_string(),
                (None, _) => "pipeline".into(),
            };
            let since = since
                .map(|since| format!(", {}", span(now - since)))
                .unwrap_or_default();
            waits(format!("deploying ({at}{since})"))
        }
    }
}

/// The origin of the test instance (try.mjs `testOrigin`): `$VITA_TEST_ORIGIN`,
/// else `VITA_TEST_ORIGIN` in vita's `.env.local` of the main checkout; no
/// slash at its end.
pub fn test_origin(main: &Path) -> Option<String> {
    let from_env = std::env::var("VITA_TEST_ORIGIN")
        .ok()
        .filter(|origin| !origin.is_empty());
    let origin = from_env.or_else(|| {
        let env = std::fs::read_to_string(main.join("products/vita/web/.env.local")).ok()?;
        env.lines().find_map(|line| {
            let value = line.trim().strip_prefix("VITA_TEST_ORIGIN=")?.trim();
            Some(value.trim_matches(|c| c == '"' || c == '\'').to_string())
        })
    })?;
    let origin = origin.trim().trim_end_matches('/');
    (!origin.is_empty()).then(|| origin.to_string())
}

fn job_of(value: &Value) -> Job {
    Job {
        name: str_of(value, "name").unwrap_or_default().to_string(),
        status: str_of(value, "status").unwrap_or_default().to_string(),
        url: str_of(value, "web_url").map(String::from),
        finished_at: str_of(value, "finished_at").map(String::from),
    }
}

/// A develop pipeline as GitLab lists it, with its jobs (gitlab.mjs `deployFrom`).
fn deploy_from(main: &Path, found: Option<&Value>) -> Deploy {
    let Some(found) = found else { return Deploy::default() };
    let id = int_of(found, "id").unwrap_or(0);
    let jobs = gitlab::list(
        main,
        &format!("projects/:fullpath/pipelines/{id}/jobs?per_page=100&include_retried=false"),
    )
    .unwrap_or_default();
    Deploy {
        pipeline: Some(DeployPipeline {
            id,
            status: str_of(found, "status").unwrap_or_default().to_string(),
            url: str_of(found, "web_url").map(String::from),
            created_at: str_of(found, "created_at").and_then(parse_iso),
        }),
        jobs: jobs.iter().map(job_of).collect(),
    }
}

/// The develop pipeline that deploys the merge commit `sha` with its jobs
/// (gitlab.mjs `deployOf`): its own, or when that failed the newest develop
/// pipeline after it that contains the commit (#275). Read only for a merged
/// MR whose try on the test instance is open: a print asks nothing else.
pub fn deploy_of(main: &Path, sha: &str, now: i64) -> Option<Deploy> {
    let own = gitlab::list(
        main,
        &format!("projects/:fullpath/pipelines?ref=develop&sha={sha}&per_page=1"),
    )
    .ok()?;
    let own = deploy_from(main, own.first());
    if !matches!(deploy_state_of(Some(&own), now), DeployState::Failed { .. }) {
        return Some(own);
    }
    let after = own.pipeline.as_ref().map_or(0, |pipeline| pipeline.id);
    for filter in ["&status=success", ""] {
        let Ok(found) = gitlab::list(
            main,
            &format!("projects/:fullpath/pipelines?ref=develop{filter}&per_page=1"),
        ) else {
            continue;
        };
        let Some(found) = found.first().filter(|found| int_of(found, "id").unwrap_or(0) > after) else {
            continue;
        };
        let head = str_of(found, "sha").unwrap_or_default();
        let base = gitlab::get(
            main,
            &format!("projects/:fullpath/repository/merge_base?refs%5B%5D={sha}&refs%5B%5D={head}"),
        );
        if base.ok().as_ref().and_then(|base| str_of(base, "id")) == Some(sha) {
            return Some(deploy_from(main, Some(found)));
        }
    }
    Some(own)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000_000;

    fn job(name: &str, status: &str, finished: Option<i64>) -> Job {
        Job {
            name: name.into(),
            status: status.into(),
            url: Some(format!("https://x/{name}")),
            finished_at: finished.map(breadboard_core::time::to_iso),
        }
    }

    fn deploy(status: &str, jobs: Vec<Job>) -> Deploy {
        Deploy {
            pipeline: Some(DeployPipeline {
                id: 1,
                status: status.into(),
                url: None,
                created_at: Some(NOW - 180_000),
            }),
            jobs,
        }
    }

    fn action(on: Option<&str>) -> Action {
        Action {
            kind: "try".into(),
            on: on.map(String::from),
            ..Action::default()
        }
    }

    fn merged(deploy: Option<&Deploy>) -> Option<TryMr<'_>> {
        Some(TryMr {
            iid: 7,
            state: "merged",
            deploy,
        })
    }

    #[test]
    fn a_try_on_its_app_is_ready_once_pushed() {
        assert_eq!(try_state_of(&action(None), 0, None, None, true, NOW).label, "ready");
        let state = try_state_of(&action(None), 2, None, None, false, NOW);
        assert_eq!(
            (state.is_ready, state.label.as_str(), state.tone),
            (false, "not pushed", "waits")
        );
    }

    #[test]
    fn a_try_on_the_test_instance_waits_for_its_deploy() {
        let test = action(Some("test"));
        let origin = Some("https://test.example");
        assert_eq!(
            try_state_of(&test, 0, None, None, true, NOW).label,
            "no test instance: VITA_TEST_ORIGIN unset"
        );
        assert_eq!(try_state_of(&test, 0, None, origin, false, NOW).label, "GitLab …");
        assert_eq!(
            try_state_of(&test, 0, None, origin, true, NOW).label,
            "waits for its MR"
        );
        let open = Some(TryMr {
            iid: 7,
            state: "opened",
            deploy: None,
        });
        assert_eq!(
            try_state_of(&test, 0, open, origin, true, NOW).label,
            "waits for merge !7"
        );
        assert_eq!(
            try_state_of(&test, 0, merged(None), origin, true, NOW).label,
            "deploying (pipeline)"
        );
        let running = deploy(
            "running",
            vec![
                job("build:develop", "success", None),
                job("publish:develop", "running", None),
            ],
        );
        assert_eq!(
            try_state_of(&test, 0, merged(Some(&running)), origin, true, NOW).label,
            "deploying (publish, 3 min)"
        );
        let starting = deploy("success", vec![job("deploy:develop", "success", Some(NOW - 30_000))]);
        assert_eq!(
            try_state_of(&test, 0, merged(Some(&starting)), origin, true, NOW).label,
            "deploying (deploy:develop passed, Coolify starts it, 3 min)"
        );
        let done = deploy("success", vec![job("deploy:develop", "success", Some(NOW - 120_000))]);
        let state = try_state_of(&test, 0, merged(Some(&done)), origin, true, NOW);
        assert!(state.is_ready);
        assert_eq!(state.label, "ready (deploy:develop passed 2 min ago)");
        let broken = deploy(
            "failed",
            vec![job("lint", "failed", None), job("build:develop", "skipped", None)],
        );
        assert_eq!(
            try_state_of(&test, 0, merged(Some(&broken)), origin, true, NOW).label,
            "deploy failed (lint)"
        );
    }
}
