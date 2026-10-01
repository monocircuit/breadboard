//! Whether a `try` can be tried now (#237, try-state.mjs). One on the
//! worktree's app is ready once its commits are pushed; one on the test
//! instance once its MR is merged and the develop pipeline of the merge commit
//! has deployed it: `waits for merge !N` → `deploying (publish, 3 min)` →
//! `ready`, or `deploy failed` with the job a click opens. When the merge
//! commit's own pipeline failed, a later develop pipeline that contains it
//! decides (#275). vita reports no build commit: `deploy:develop` passed plus
//! DEPLOY_GRACE_MS counts as deployed.

use super::Tree;
use crate::actions::Action;
use crate::gitlab::{Deploy, Job, Mr};
use crate::time::{parse_iso, span};

/// How long Coolify gets after `deploy:develop` passed to pull and start the image.
pub const DEPLOY_GRACE_MS: i64 = 90_000;
/// The jobs of the develop pipeline that deploy: the others don't hold a try.
const DEPLOY_JOBS: [&str; 3] = ["build:develop", "publish:develop", "deploy:develop"];
const DEPLOY_JOB: &str = "deploy:develop";
/// Without it, publishing is the last step.
const PUBLISH_JOB: &str = "publish:develop";
const FAILED: [&str; 2] = ["failed", "canceled"];
const DONE: [&str; 4] = ["success", "failed", "canceled", "skipped"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployState {
    Deploying,
    Deployed,
    Failed,
}

/// Where the deployment of a merge commit stands: `job` the one it is at
/// (the one that failed, passed, or runs), `since` the pipeline's start,
/// `passed` when the deploying job passed; `is_final`: it won't change any more.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployOf {
    pub state: DeployState,
    pub job: Option<Job>,
    pub since: Option<i64>,
    pub passed: Option<i64>,
    pub is_final: bool,
}

pub fn deploy_state_of(deploy: Option<&Deploy>, now: i64) -> DeployOf {
    let deploying = |job: Option<Job>, since: Option<i64>| DeployOf {
        state: DeployState::Deploying,
        job,
        since,
        passed: None,
        is_final: false,
    };
    let Some(pipeline) = deploy.and_then(|deploy| deploy.pipeline.as_ref()) else {
        return deploying(None, None);
    };
    let all = &deploy.map(|deploy| &deploy.jobs[..]).unwrap_or(&[]);
    let jobs: Vec<&Job> = all
        .iter()
        .filter(|job| DEPLOY_JOBS.contains(&job.name.as_str()))
        .collect();
    let since = pipeline.created_at.as_deref().and_then(parse_iso);
    if let Some(failed) = jobs.iter().find(|job| FAILED.contains(&job.status.as_str())) {
        return DeployOf {
            state: DeployState::Failed,
            job: Some((*failed).clone()),
            since,
            passed: None,
            is_final: true,
        };
    }
    let is_done = DONE.contains(&pipeline.status.as_str());
    let last = jobs
        .iter()
        .find(|job| job.name == DEPLOY_JOB)
        // No deploy job in a pipeline that has ended: publishing was the last step.
        .or_else(|| {
            if is_done {
                jobs.iter().find(|job| job.name == PUBLISH_JOB)
            } else {
                None
            }
        });
    if let Some(last) = last.filter(|job| job.status == "success") {
        let passed = last.finished_at.as_deref().and_then(parse_iso).unwrap_or(now);
        let is_deployed = now - passed >= DEPLOY_GRACE_MS;
        return DeployOf {
            state: if is_deployed {
                DeployState::Deployed
            } else {
                DeployState::Deploying
            },
            job: Some((*last).clone()),
            since,
            passed: Some(passed),
            is_final: is_deployed,
        };
    }
    // The pipeline ended without the deploying jobs passing and none failed: a job before them did.
    if is_done && pipeline.status != "success" {
        let job = all
            .iter()
            .find(|job| FAILED.contains(&job.status.as_str()))
            .or(jobs.last().copied())
            .cloned()
            .unwrap_or_else(|| Job {
                name: "pipeline".into(),
                url: pipeline.url.clone(),
                ..Default::default()
            });
        return DeployOf {
            state: DeployState::Failed,
            job: Some(job),
            since,
            passed: None,
            is_final: true,
        };
    }
    let job = jobs
        .iter()
        .find(|job| job.status == "running")
        .or_else(|| jobs.iter().find(|job| job.status != "success"))
        .map(|job| (*job).clone());
    deploying(job, since)
}

/// Which develop pipeline decides a merge's deployment (#275): its own, or,
/// when that one failed, `later`, the newest develop pipeline that contains it.
pub fn deciding_deploy_of<'a>(own: &'a Deploy, later: Option<&'a Deploy>, now: i64) -> &'a Deploy {
    match later {
        Some(later) if later.pipeline.is_some() && deploy_state_of(Some(own), now).state == DeployState::Failed => {
            later
        }
        _ => own,
    }
}

/// How a try's state shows: ready as done, waiting as working, failed as blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryTone {
    Ready,
    Waits,
    Failed,
}

/// Whether a try can be tried now, what its row says in front of the text,
/// and what a click opens in place of the spot (the failed job).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryState {
    pub is_ready: bool,
    pub label: String,
    pub tone: TryTone,
    pub url: Option<String>,
}

fn waits(label: impl Into<String>) -> TryState {
    TryState {
        is_ready: false,
        label: label.into(),
        tone: TryTone::Waits,
        url: None,
    }
}

/// tryStateOf: `tree` the worktree (none for the orchestrator's own), `mr` its
/// MR with its `deploy`, `origin` the test instance's, `has_remote` whether
/// GitLab answered yet.
pub fn try_state_of(
    action: &Action,
    tree: Option<&Tree>,
    mr: Option<&Mr>,
    origin: Option<&str>,
    has_remote: bool,
    now: i64,
) -> TryState {
    if action.on.as_deref() != Some("test") {
        if tree.is_some_and(|tree| tree.unpushed > 0) {
            return waits("not pushed");
        }
        return TryState {
            is_ready: true,
            label: "ready".into(),
            tone: TryTone::Ready,
            url: None,
        };
    }
    let failed = |label: String, url: Option<String>| TryState {
        is_ready: false,
        label,
        tone: TryTone::Failed,
        url,
    };
    if origin.is_none() {
        return failed("no test instance: VITA_TEST_ORIGIN unset".into(), None);
    }
    if !has_remote {
        return waits("GitLab …");
    }
    let Some(mr) = mr else { return waits("waits for its MR") };
    if mr.state == "closed" {
        return failed(format!("!{} closed", mr.iid), None);
    }
    if mr.state != "merged" {
        return waits(format!("waits for merge !{}", mr.iid));
    }
    let deploy = deploy_state_of(mr.deploy.as_ref(), now);
    let job_name = deploy.job.as_ref().map(|job| job.name.clone()).unwrap_or_default();
    match deploy.state {
        DeployState::Failed => failed(
            format!("deploy failed ({job_name})"),
            deploy.job.and_then(|job| job.url),
        ),
        DeployState::Deployed => TryState {
            is_ready: true,
            label: format!(
                "ready ({job_name} passed {} ago)",
                span(now - deploy.passed.unwrap_or(now))
            ),
            tone: TryTone::Ready,
            url: None,
        },
        DeployState::Deploying => {
            let at = match (&deploy.passed, &deploy.job) {
                (Some(_), _) => format!("{job_name} passed, Coolify starts it"),
                (None, Some(job)) => job.name.split(':').next().unwrap_or_default().to_string(),
                (None, None) => "pipeline".into(),
            };
            let since = deploy
                .since
                .map(|since| format!(", {}", span(now - since)))
                .unwrap_or_default();
            waits(format!("deploying ({at}{since})"))
        }
    }
}

/// The app of a checkout whose stack has `offset`: `http://localhost:3002`.
pub fn app_origin(offset: i64) -> String {
    format!("http://localhost:{}", 3000 + offset)
}

/// The page an action names: a path on the worktree's app, anything else as it is.
pub fn page_of(url: &str, offset: i64) -> String {
    if url.starts_with('/') {
        format!("{}{url}", app_origin(offset))
    } else {
        url.to_string()
    }
}

/// A page as a row shows it: `:3002/atlas` for one on localhost.
pub fn short_page(page: &str) -> String {
    let short = ["http://localhost:", "https://localhost:"]
        .iter()
        .find_map(|prefix| page.strip_prefix(prefix).map(|rest| format!(":{rest}")))
        .unwrap_or_else(|| page.to_string());
    match short.strip_suffix('/') {
        Some(port) if port.starts_with(':') && port[1..].chars().all(|c| c.is_ascii_digit()) => port.to_string(),
        _ => short,
    }
}

/// What a click on a try opens, behind its text (try.mjs `trySpot`, without
/// the state of a click under way, which is the view's).
pub fn try_spot(action: &Action, tree: Option<&Tree>) -> String {
    if action.on.as_deref() == Some("test") {
        return format!("opens {} on the test instance", action.url.as_deref().unwrap_or(""));
    }
    if let Some(run) = &action.run {
        return format!("opens a terminal: {run}");
    }
    let Some(url) = &action.url else { return String::new() };
    let Some(tree) = tree else {
        return format!("opens {}", short_page(url));
    };
    let page = page_of(url, tree.offset);
    let origin = app_origin(tree.offset);
    let is_on_app = page == origin || page.starts_with(&format!("{origin}/"));
    let will_start = tree.has_own_port && tree.has_env && !tree.is_up && is_on_app;
    format!(
        "opens {}{}",
        short_page(&page),
        if will_start { " after starting the stack" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::DeployPipeline;
    use crate::time::to_iso;

    #[test]
    fn says_what_a_click_on_a_try_opens() {
        let with = |url: Option<&str>, run: Option<&str>, on: Option<&str>| Action {
            url: url.map(String::from),
            run: run.map(String::from),
            on: on.map(String::from),
            ..Default::default()
        };
        assert_eq!(
            try_spot(&with(Some("/atlas"), None, Some("test")), None),
            "opens /atlas on the test instance"
        );
        assert_eq!(
            try_spot(&with(None, Some("cargo run"), None), None),
            "opens a terminal: cargo run"
        );
        let tree = Tree {
            offset: 2,
            has_own_port: true,
            has_env: true,
            ..Default::default()
        };
        assert_eq!(
            try_spot(&with(Some("/atlas"), None, None), Some(&tree)),
            "opens :3002/atlas after starting the stack"
        );
        assert_eq!(
            try_spot(&with(Some("http://localhost:3002/"), None, None), None),
            "opens :3002"
        );
        assert_eq!(try_spot(&with(None, None, None), None), "");
    }

    const MIN: i64 = 60_000;
    fn now() -> i64 {
        parse_iso("2026-09-30T12:00:00Z").unwrap()
    }
    fn at(ms: i64) -> String {
        to_iso(now() - ms)
    }
    fn job(name: &str, status: &str, finished_at: Option<String>) -> Job {
        Job {
            id: name.len() as i64,
            name: name.into(),
            status: status.into(),
            url: Some(format!("https://gitlab/jobs/{name}")),
            finished_at,
            log: None,
        }
    }
    fn deploy(jobs: Vec<Job>, status: &str) -> Deploy {
        Deploy {
            pipeline: Some(DeployPipeline {
                id: 1,
                status: status.into(),
                url: Some("https://gitlab/p/1".into()),
                created_at: Some(at(3 * MIN)),
            }),
            jobs,
        }
    }
    fn test_try() -> Action {
        Action {
            kind: "try".into(),
            text: "boxes".into(),
            url: Some("/atlas".into()),
            on: Some("test".into()),
            ..Default::default()
        }
    }
    fn merged(deploy: Option<Deploy>) -> Mr {
        Mr {
            iid: 7,
            state: "merged".into(),
            deploy,
            ..Default::default()
        }
    }
    fn state_of(mr: Option<&Mr>) -> TryState {
        try_state_of(&test_try(), None, mr, Some("https://test.example"), true, now())
    }

    #[test]
    fn deploys_while_gitlab_has_no_pipeline_yet() {
        assert_eq!(deploy_state_of(None, now()).state, DeployState::Deploying);
        assert_eq!(
            deploy_state_of(Some(&Deploy::default()), now()).state,
            DeployState::Deploying
        );
    }

    #[test]
    fn is_at_the_running_job_else_the_first_to_come() {
        let running = deploy_state_of(
            Some(&deploy(
                vec![
                    job("scan:deps:develop", "running", None),
                    job("build:develop", "success", None),
                    job("publish:develop", "running", None),
                    job("deploy:develop", "created", None),
                ],
                "running",
            )),
            now(),
        );
        assert_eq!(running.state, DeployState::Deploying);
        assert_eq!(running.job.unwrap().name, "publish:develop");
        assert_eq!(running.since, Some(now() - 3 * MIN));
        let pending = deploy(
            vec![
                job("build:develop", "success", None),
                job("publish:develop", "pending", None),
            ],
            "running",
        );
        assert_eq!(
            deploy_state_of(Some(&pending), now()).job.unwrap().name,
            "publish:develop"
        );
    }

    #[test]
    fn is_deployed_once_deploy_passed_the_grace_period_ago() {
        let passed = |ms| {
            deploy_state_of(
                Some(&deploy(
                    vec![
                        job("build:develop", "success", None),
                        job("deploy:develop", "success", Some(at(ms))),
                    ],
                    "running",
                )),
                now(),
            )
        };
        let young = passed(DEPLOY_GRACE_MS - 1000);
        assert_eq!((young.state, young.is_final), (DeployState::Deploying, false));
        let old = passed(DEPLOY_GRACE_MS);
        assert_eq!((old.state, old.is_final), (DeployState::Deployed, true));
    }

    #[test]
    fn takes_publish_as_the_last_step_of_an_ended_pipeline() {
        let jobs = vec![
            job("build:develop", "success", None),
            job("publish:develop", "success", Some(at(5 * MIN))),
        ];
        let published = deploy(jobs.clone(), "success");
        let state = deploy_state_of(Some(&published), now());
        assert_eq!(state.state, DeployState::Deployed);
        assert_eq!(state.job.unwrap().name, "publish:develop");
        assert_eq!(
            deploy_state_of(Some(&deploy(jobs, "running")), now()).state,
            DeployState::Deploying
        );
    }

    #[test]
    fn fails_with_the_job_that_failed_or_was_cancelled() {
        for status in ["failed", "canceled"] {
            let state = deploy_state_of(
                Some(&deploy(
                    vec![
                        job("build:develop", "success", None),
                        job("publish:develop", status, None),
                    ],
                    "running",
                )),
                now(),
            );
            assert_eq!(state.state, DeployState::Failed);
            assert_eq!(
                state.job.unwrap().url.as_deref(),
                Some("https://gitlab/jobs/publish:develop")
            );
            assert!(state.is_final);
        }
        let scan = deploy(
            vec![
                job("scan:deps:develop", "failed", None),
                job("build:develop", "running", None),
            ],
            "running",
        );
        assert_eq!(deploy_state_of(Some(&scan), now()).state, DeployState::Deploying);
    }

    #[test]
    fn a_later_develop_pipeline_deploys_a_merge_whose_own_failed() {
        let own = deploy(
            vec![
                job("check:vita", "failed", None),
                job("build:develop", "skipped", None),
                job("deploy:develop", "skipped", None),
            ],
            "failed",
        );
        let later = deploy(vec![job("deploy:develop", "success", Some(at(5 * MIN)))], "success");
        assert_eq!(deploy_state_of(Some(&own), now()).job.unwrap().name, "check:vita");
        assert_eq!(deciding_deploy_of(&own, Some(&later), now()), &later);
        assert_eq!(
            state_of(Some(&merged(Some(later.clone())))).label,
            "ready (deploy:develop passed 5 min ago)"
        );
        assert_eq!(deciding_deploy_of(&own, None, now()), &own);
        let running = deploy(vec![job("build:develop", "running", None)], "running");
        assert_eq!(deciding_deploy_of(&running, Some(&later), now()), &running);
    }

    #[test]
    fn readies_a_worktrees_try_once_its_commits_are_pushed() {
        let worktree = Action {
            kind: "try".into(),
            text: "x".into(),
            url: Some("/atlas".into()),
            ..Default::default()
        };
        let unpushed = Tree {
            unpushed: 2,
            ..Default::default()
        };
        assert_eq!(
            try_state_of(&worktree, Some(&unpushed), None, None, true, now()),
            waits("not pushed")
        );
        assert!(try_state_of(&worktree, Some(&Tree::default()), None, None, true, now()).is_ready);
        assert_eq!(try_state_of(&worktree, None, None, None, true, now()).label, "ready");
    }

    #[test]
    fn walks_a_test_try_from_the_merge_to_ready() {
        assert_eq!(state_of(None).label, "waits for its MR");
        let open = Mr {
            iid: 7,
            state: "opened".into(),
            ..Default::default()
        };
        assert_eq!(state_of(Some(&open)).label, "waits for merge !7");
        assert_eq!(state_of(Some(&merged(None))).label, "deploying (pipeline)");
        let publishing = deploy(
            vec![
                job("build:develop", "success", None),
                job("publish:develop", "running", None),
            ],
            "running",
        );
        assert_eq!(
            state_of(Some(&merged(Some(publishing)))).label,
            "deploying (publish, 3 min)"
        );
        let starting = deploy(vec![job("deploy:develop", "success", Some(at(30_000)))], "running");
        assert_eq!(
            state_of(Some(&merged(Some(starting)))).label,
            "deploying (deploy:develop passed, Coolify starts it, 3 min)"
        );
        let ready = state_of(Some(&merged(Some(deploy(
            vec![job("deploy:develop", "success", Some(at(2 * MIN)))],
            "running",
        )))));
        assert!(ready.is_ready);
        assert_eq!(ready.label, "ready (deploy:develop passed 2 min ago)");
    }

    #[test]
    fn names_the_failed_job_and_the_missing_origin() {
        let failed = state_of(Some(&merged(Some(deploy(
            vec![job("build:develop", "failed", None)],
            "running",
        )))));
        assert_eq!(
            failed,
            TryState {
                is_ready: false,
                label: "deploy failed (build:develop)".into(),
                tone: TryTone::Failed,
                url: Some("https://gitlab/jobs/build:develop".into()),
            }
        );
        let no_origin = try_state_of(&test_try(), None, Some(&merged(None)), None, true, now());
        assert_eq!(no_origin.tone, TryTone::Failed);
        let closed = Mr {
            iid: 7,
            state: "closed".into(),
            ..Default::default()
        };
        assert_eq!(state_of(Some(&closed)).label, "!7 closed");
    }
}
