//! The reads of the views, in as few calls as GitLab allows (#305): one
//! GraphQL request for an orchestration's MRs with their pipelines and jobs,
//! its issues, the MRs it waits for and develop's last pipeline; REST only
//! where GraphQL lacks something (a failed job's log, a merge base). What
//! can't change any more comes from the cache and is never asked again.

use super::cache::Cache;
use super::client::{Gitlab, GitlabError, Result};
use super::{
    BatchTask, BranchRemote, Deploy, DeployPipeline, DevelopPipeline, EpicRemote, Issue, Job, Mr, Pipeline, Remote,
    boxes_of, number_of,
};
use crate::time::now_ms;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

/// iids per batched call: GitLab prices a page asked by iids at five times another (#454).
const BATCH: usize = 50;
/// How many lines of a failed job's log the views show.
const LOG_LINES: usize = 6;
/// A pipeline that no longer runs: its jobs are final (a retry changes its `updatedAt`).
pub(super) const FINISHED: [&str; 4] = ["success", "failed", "canceled", "skipped"];

/// What a read keeps between reads, for the fast ones (gitlab.mjs `readMemory`):
/// the links of the open issues, the logs of failed jobs (they don't change).
#[derive(Debug, Default)]
pub struct Memory {
    pub links: HashMap<i64, Vec<i64>>,
    pub logs: HashMap<i64, Vec<String>>,
    /// GitLab answered the links' field with an error: not asked again.
    pub has_no_links: bool,
    /// GitLab answered REST but not GraphQL: read through REST, GraphQL tried again on a full read.
    pub is_rest_only: bool,
    /// The open MRs whose mergeability GitLab was asked to recheck (#391).
    pub rechecks: HashMap<i64, super::recheck::Recheck>,
}

// ---------------------------------------------------------------------------
// GraphQL's answer in the views' shapes
// ---------------------------------------------------------------------------

const PIPELINE_FIELDS: &str = "id iid status path sha ref createdAt updatedAt finishedAt";
const JOB_FIELDS: &str = "id name status webPath finishedAt";
/// The open branches' MRs per page: each brings its pipeline's jobs, 100 cost more than GitLab allows (#454).
const MRS_PAGE: usize = 50;

fn mr_fields() -> String {
    format!(
        "iid title description state draft webUrl sourceBranch targetBranch mergedAt conflicts detailedMergeStatus diffHeadSha mergeCommitSha \
         headPipeline {{ {PIPELINE_FIELDS} jobs(first: 100) {{ nodes {{ {JOB_FIELDS} }} }} }}"
    )
}

// ---------------------------------------------------------------------------
// The queries, each in one place: `queries()` lists them for the contract
// check against gitlab.com's schema (#338)
// ---------------------------------------------------------------------------

/// The develop pipeline of the merge commit `$sha`, with its jobs.
fn deploy_query() -> String {
    format!(
        "query($project: ID!, $sha: String) {{ project(fullPath: $project) {{ pipelines(ref: \"develop\", sha: $sha, first: 1) {{ nodes {{ {PIPELINE_FIELDS} jobs(first: 100) {{ nodes {{ {JOB_FIELDS} }} }} }} }} }} }}"
    )
}

/// develop's last 20 pipelines, with their jobs.
fn later_deploys_query() -> String {
    format!(
        "query($project: ID!) {{ project(fullPath: $project) {{ pipelines(ref: \"develop\", first: 20) {{ nodes {{ {PIPELINE_FIELDS} jobs(first: 100) {{ nodes {{ {JOB_FIELDS} }} }} }} }} }} }}"
    )
}

/// The issues blocking each of `$iids`.
const LINKS_QUERY: &str = "query($project: ID!, $iids: [String!]) { project(fullPath: $project) { issues(iids: $iids, state: all, first: 50) { nodes { iid blockedByIssues { nodes { iid } } } } } }";

/// develop, and where there are any the issues, the awaited MRs, the epic's
/// pipelines. GraphQL refuses a variable no field uses: each declared with
/// its field. The MRs are `mrs_query`'s: together they cost more than GitLab
/// allows one query (#454).
fn orchestration_query(has_issues: bool, has_awaited: bool, has_epic: bool) -> String {
    let mut decls = vec!["$project: ID!"];
    let mut parts = vec![format!(
        "develop: pipelines(ref: \"develop\", first: 5) {{ nodes {{ {PIPELINE_FIELDS} jobs(first: 100) {{ nodes {{ {JOB_FIELDS} }} }} }} }}"
    )];
    if has_epic {
        decls.push("$epic: String");
        parts.push(format!(
            "epicPipelines: pipelines(ref: $epic, first: 5) {{ nodes {{ {PIPELINE_FIELDS} jobs(first: 100) {{ nodes {{ {JOB_FIELDS} }} }} }} }}"
        ));
        decls.push("$epicTargets: [String!]");
        parts.push(
            "epicTasks: mergeRequests(targetBranches: $epicTargets, state: merged, first: 100, sort: MERGED_AT_DESC) { nodes { iid title webUrl sourceBranch mergedAt labels { nodes { title } } } }".into(),
        );
    }
    if has_issues {
        decls.push("$iids: [String!]");
        parts
            .push("issues(iids: $iids, state: all, first: 50) { nodes { iid title description state webUrl } }".into());
    }
    if has_awaited {
        decls.push("$awaited: [String!]");
        parts.push("awaited: mergeRequests(iids: $awaited, first: 100) { nodes { iid state } }".into());
    }
    format!(
        "query({}) {{ project(fullPath: $project) {{ {} }} }}",
        decls.join(", "),
        parts.join(" ")
    )
}

/// A page of the open branches' MRs (`$after` from the page before), and
/// with `has_epic` the MRs into develop of the epic and the batches shipped before it (#414).
fn mrs_query(has_branches: bool, has_epic: bool) -> String {
    let mut decls = vec!["$project: ID!"];
    let mut parts = Vec::new();
    if has_epic {
        decls.push("$epics: [String!]");
        parts.push(format!(
            "epicMrs: mergeRequests(sourceBranches: $epics, targetBranches: [\"develop\"], state: opened, first: 10) {{ nodes {{ {} }} }}",
            mr_fields()
        ));
    }
    if has_branches {
        decls.push("$branches: [String!]");
        decls.push("$after: String");
        parts.push(format!(
            "mrs: mergeRequests(sourceBranches: $branches, first: {MRS_PAGE}, after: $after, sort: UPDATED_DESC) {{ nodes {{ {} }} pageInfo {{ hasNextPage endCursor }} }}",
            mr_fields()
        ));
    }
    format!(
        "query({}) {{ project(fullPath: $project) {{ {} }} }}",
        decls.join(", "),
        parts.join(" ")
    )
}

/// The next batch of issues.
const ISSUES_PAGE_QUERY: &str = "query($project: ID!, $iids: [String!]) { project(fullPath: $project) { issues(iids: $iids, state: all, first: 50) { nodes { iid title description state webUrl } } } }";

/// One worktree's: the open MRs, and where it has them its branch's MRs and its issue.
fn worktree_query(has_branch: bool, has_number: bool) -> String {
    let mut decls = vec!["$project: ID!"];
    let mut parts = vec![
        "open: mergeRequests(state: opened, first: 100) { nodes { iid title draft state webUrl sourceBranch headPipeline { id iid status path sha ref } } }"
            .to_string(),
    ];
    if has_branch {
        decls.push("$branches: [String!]");
        parts.push(format!(
            "mrs: mergeRequests(sourceBranches: $branches, first: 20, sort: UPDATED_DESC) {{ nodes {{ {} }} }}",
            mr_fields()
        ));
    }
    if has_number {
        decls.push("$iid: String!");
        parts.push("issue(iid: $iid) { iid title description state webUrl }".into());
    }
    format!(
        "query({}) {{ project(fullPath: $project) {{ {} }} }}",
        decls.join(", "),
        parts.join(" ")
    )
}

/// The overview of everything: the open MRs and issues, develop's last pipeline.
const EVERYTHING_QUERY: &str = "query($project: ID!) { project(fullPath: $project) { \
    mrs: mergeRequests(state: opened, first: 50) { nodes { iid title draft webUrl sourceBranch author { username } assignees { nodes { username } } headPipeline { id iid status path sha ref } } } \
    issues(state: opened, first: 100) { nodes { iid title webUrl labels { nodes { title } } assignees { nodes { username } } } } \
    develop: pipelines(ref: \"develop\", first: 1) { nodes { id iid status path sha ref } } } }";

/// Every query breadboard sends to GitLab, each with all its parts, by name.
pub fn queries() -> Vec<(&'static str, String)> {
    vec![
        ("ci queue", CI_QUERY.into()),
        ("deploy", deploy_query()),
        ("later deploys", later_deploys_query()),
        ("links", LINKS_QUERY.into()),
        ("orchestration", orchestration_query(true, true, true)),
        ("mrs", mrs_query(true, true)),
        ("issues page", ISSUES_PAGE_QUERY.into()),
        ("worktree", worktree_query(true, true)),
        ("everything", EVERYTHING_QUERY.into()),
    ]
}

/// The CI queue's (#357): every open MR with its newest commit and its newest pipeline.
pub const CI_QUERY: &str = "query($project: ID!) { project(fullPath: $project) { mergeRequests(state: opened, first: 100) { nodes { iid draft sourceBranch targetBranch diffHeadSha labels { nodes { title } } pipelines(first: 20) { nodes { id status sha ref failureReason startedAt updatedAt } } } } } }";

/// The open MRs of CI_QUERY's answer, as the CI queue takes them.
pub fn open_mrs_of(project: &Value) -> Vec<crate::ci::OpenMr> {
    nodes(project, "mergeRequests")
        .iter()
        .map(|node| {
            // Its newest MR pipeline, of any commit: after a push that skipped CI
            // GitLab's head pipeline is none, and a red one before it matters.
            // A branch's (the `skipped` one a push with ci.skip leaves) isn't one.
            // Newest by id: in a list of MRs gitlab.com gives each one's
            // pipelines in no order of time (by sha, 2026-10-12), and the first
            // taken for the newest started its pipelines again and again (#369).
            let newest = nodes(node, "pipelines")
                .into_iter()
                .filter(|pipeline| {
                    str_at(pipeline, "ref").is_some_and(|reference| reference.starts_with("refs/merge-requests/"))
                })
                .max_by_key(|pipeline| id_of(pipeline.get("id")));
            crate::ci::OpenMr {
                blocked_by: blocked_by(node),
                iid: id_of(node.get("iid")),
                source: str_at(node, "sourceBranch").unwrap_or_default().to_string(),
                target: str_at(node, "targetBranch").unwrap_or_default().to_string(),
                is_draft: node.get("draft").and_then(Value::as_bool).unwrap_or(false),
                labels: nodes(node, "labels")
                    .iter()
                    .filter_map(|label| str_at(label, "title").map(String::from))
                    .collect(),
                sha: str_at(node, "diffHeadSha").map(String::from),
                pipeline: newest.as_ref().and_then(|pipeline| pipeline_of(pipeline, "")),
                is_refused: newest
                    .as_ref()
                    .is_some_and(|pipeline| str_at(pipeline, "failureReason").is_some()),
            }
        })
        .collect()
}

/// The newest MR pipeline of an MR's newest commit when it blocks the merge
/// (#369): cancelled, or failed at its creation (a `failureReason`: GitLab's
/// activity limit, a config error; none when its jobs failed), while an
/// older pipeline of that commit passed. GitLab takes the newest for the MR's.
fn blocked_by(node: &Value) -> Option<i64> {
    let head = str_at(node, "diffHeadSha")?;
    let mut ours: Vec<(i64, String, bool)> = nodes(node, "pipelines")
        .iter()
        .filter(|pipeline| str_at(pipeline, "sha") == Some(head))
        .filter(|pipeline| {
            str_at(pipeline, "ref").is_some_and(|reference| reference.starts_with("refs/merge-requests/"))
        })
        .map(|pipeline| {
            (
                id_of(pipeline.get("id")),
                lower(str_at(pipeline, "status")),
                str_at(pipeline, "failureReason").is_some(),
            )
        })
        .collect();
    ours.sort_by_key(|(id, _, _)| std::cmp::Reverse(*id));
    let (newest, status, is_refused) = ours.first()?;
    let is_blocking = status == "canceled" || (status == "failed" && *is_refused);
    let has_passed = ours[1..].iter().any(|(_, status, _)| status == "success");
    (is_blocking && has_passed).then_some(*newest)
}

pub(super) fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty())
}

fn nodes(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(|connection| connection.get("nodes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The number at the end of a global id (`gid://gitlab/Ci::Build/123`) or a number as text.
pub(super) fn id_of(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Value::String(text)) => text.rsplit('/').next().and_then(|id| id.parse().ok()).unwrap_or(0),
        _ => 0,
    }
}

/// `https://gitlab.com` of `https://gitlab.com/group/project/-/merge_requests/1`.
pub fn origin_of(url: &str) -> Option<String> {
    let rest = url.find("://").map(|at| at + 3)?;
    let end = url[rest..].find('/').map(|at| rest + at).unwrap_or(url.len());
    Some(url[..end].to_string())
}

pub(super) fn lower(value: Option<&str>) -> String {
    value.unwrap_or_default().to_lowercase()
}

/// A pipeline of GraphQL in REST's shape, its page under `web`.
pub fn pipeline_of(value: &Value, web: &str) -> Option<Pipeline> {
    if !value.is_object() {
        return None;
    }
    Some(Pipeline {
        id: id_of(value.get("id")),
        status: lower(str_at(value, "status")),
        web_url: str_at(value, "path").map(|path| format!("{web}{path}")),
        sha: str_at(value, "sha").map(String::from),
        reference: str_at(value, "ref").map(String::from),
        created_at: str_at(value, "createdAt").map(String::from),
        started_at: str_at(value, "startedAt").map(String::from),
        updated_at: str_at(value, "updatedAt").map(String::from),
        finished_at: str_at(value, "finishedAt").map(String::from),
    })
}

/// The jobs of a pipeline in the order they ran, the last try of each (GraphQL lists retried ones too).
pub fn jobs_of(pipeline: &Value, web: &str) -> Vec<Job> {
    let mut jobs: Vec<Job> = nodes(pipeline, "jobs")
        .iter()
        .map(|job| Job {
            id: id_of(job.get("id")),
            name: str_at(job, "name").unwrap_or_default().to_string(),
            status: lower(str_at(job, "status")),
            url: str_at(job, "webPath").map(|path| format!("{web}{path}")),
            finished_at: str_at(job, "finishedAt").map(String::from),
            log: None,
        })
        .collect();
    jobs.sort_by_key(|job| job.id);
    let mut last: HashMap<String, i64> = HashMap::new();
    for job in &jobs {
        last.insert(job.name.clone(), job.id);
    }
    jobs.retain(|job| last.get(&job.name) == Some(&job.id));
    jobs
}

/// An MR of GraphQL as the views use it, with the commit a merged one put on develop.
pub fn mr_of(value: &Value) -> (Mr, Option<String>) {
    let url = str_at(value, "webUrl").unwrap_or_default().to_string();
    let web = origin_of(&url).unwrap_or_default();
    let title = str_at(value, "title").unwrap_or_default();
    let mr = Mr {
        iid: id_of(value.get("iid")),
        title: DRAFT.replace(title, "").into_owned(),
        is_draft: value.get("draft").and_then(Value::as_bool).unwrap_or(false),
        state: str_at(value, "state").unwrap_or_default().to_string(),
        merged_at: str_at(value, "mergedAt").map(String::from),
        // GitLab's `conflict` is a conflict even where `conflicts` lags behind it (#416).
        has_conflicts: value.get("conflicts").and_then(Value::as_bool).unwrap_or(false)
            || str_at(value, "detailedMergeStatus").is_some_and(|status| status.eq_ignore_ascii_case("conflict")),
        merge_status: str_at(value, "detailedMergeStatus").map(str::to_lowercase),
        merge_check: str_at(value, "mergeStatusEnum").map(str::to_lowercase),
        sha: str_at(value, "diffHeadSha").map(String::from),
        url,
        pipeline: value
            .get("headPipeline")
            .and_then(|pipeline| pipeline_of(pipeline, &web)),
        deploy: None,
        branch: str_at(value, "sourceBranch").unwrap_or_default().to_string(),
        target: str_at(value, "targetBranch").unwrap_or_default().to_string(),
        boxes: boxes_of(str_at(value, "description").unwrap_or_default()),
    };
    // gitlab.com's GraphQL has no squash commit (#334): a squashed MR without
    // a merge commit gets it through REST, once a try waits for its deploy.
    let merge_sha = str_at(value, "mergeCommitSha").map(String::from);
    (mr, merge_sha)
}

/// A task MR merged into a batch, of GraphQL (#414).
fn batch_task_of(value: &Value) -> BatchTask {
    BatchTask {
        iid: id_of(value.get("iid")),
        title: DRAFT
            .replace(str_at(value, "title").unwrap_or_default(), "")
            .into_owned(),
        url: str_at(value, "webUrl").unwrap_or_default().to_string(),
        branch: str_at(value, "sourceBranch").unwrap_or_default().to_string(),
        merged_at: str_at(value, "mergedAt").map(String::from),
        labels: nodes(value, "labels")
            .iter()
            .filter_map(|label| str_at(label, "title").map(String::from))
            .collect(),
    }
}

pub(super) static DRAFT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Draft:\s*").unwrap());

/// An issue of GraphQL, `blocked_by` the issues GitLab links as blocking it.
pub fn issue_of(value: &Value, blocked_by: &[i64]) -> Issue {
    Issue::of(
        id_of(value.get("iid")),
        str_at(value, "title").unwrap_or_default(),
        str_at(value, "webUrl").unwrap_or_default(),
        str_at(value, "state").unwrap_or_default(),
        str_at(value, "description").unwrap_or_default(),
        blocked_by,
    )
}

// ---------------------------------------------------------------------------
// A failed job's log (REST: GraphQL has no plain text of it)
// ---------------------------------------------------------------------------

static LOG_NOISE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]|section_(start|end):\d+:[\w.-]+(\[[^\]]*\])?\r?").unwrap());
static LOG_STAMP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d\d-\d\dT[\d:.]+Z \d\d[OE]\+? ?").unwrap());
static RUNNER_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(Cleaning up |Uploading artifacts|Running after_script|Saving cache|ERROR: Job failed|WARNING: )")
        .unwrap()
});

/// The last lines of a log before what the runner writes after the error: where the error usually is.
pub fn log_lines(trace: &str) -> Vec<String> {
    let clean = LOG_NOISE.replace_all(trace, "");
    let lines: Vec<String> = clean
        .split('\n')
        .map(|line| {
            LOG_STAMP
                .replace(line.rsplit('\r').next().unwrap_or(line), "")
                .trim_end()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect();
    let Some(end) = lines.iter().rposition(|line| !RUNNER_TAIL.is_match(line)) else {
        return Vec::new();
    };
    lines[(end + 1).saturating_sub(LOG_LINES)..=end].to_vec()
}

/// The failed jobs of `jobs` with their logs: from memory, else asked once each.
async fn with_logs(gitlab: &Gitlab, memory: &mut Memory, jobs: &mut [Job]) {
    for job in jobs.iter_mut().filter(|job| job.status == "failed") {
        if let Some(log) = memory.logs.get(&job.id) {
            job.log = Some(log.clone());
            continue;
        }
        if let Ok(trace) = gitlab.get_text(&format!("jobs/{}/trace", job.id)).await {
            let log = log_lines(&trace);
            memory.logs.insert(job.id, log.clone());
            job.log = Some(log);
        }
    }
}

/// The jobs of a finished pipeline, from the cache when it has them (gitlab.mjs `jobsOf`'s key).
fn cached_jobs(cache: &Cache, pipeline: &Pipeline) -> Option<Vec<Job>> {
    if !FINISHED.contains(&pipeline.status.as_str()) {
        return None;
    }
    cache.get(
        "jobs",
        &format!("{}@{}", pipeline.id, pipeline.updated_at.as_deref().unwrap_or("")),
    )
}

/// The jobs of an open MR's pipeline as read, their logs added; a finished pipeline's kept.
pub(super) async fn settle_jobs(
    gitlab: &Gitlab,
    cache: &mut Cache,
    memory: &mut Memory,
    pipeline: &Pipeline,
    read: Vec<Job>,
) -> Vec<Job> {
    if let Some(jobs) = cached_jobs(cache, pipeline) {
        return jobs;
    }
    let mut jobs = read;
    with_logs(gitlab, memory, &mut jobs).await;
    if FINISHED.contains(&pipeline.status.as_str())
        && jobs.iter().all(|job| job.status != "failed" || job.log.is_some())
    {
        cache.set(
            "jobs",
            &format!("{}@{}", pipeline.id, pipeline.updated_at.as_deref().unwrap_or("")),
            &jobs,
        );
    }
    jobs
}

// ---------------------------------------------------------------------------
// The deployment of a merge commit (try-state.mjs): only where a try waits for it
// ---------------------------------------------------------------------------

/// A merged MR as the cache keeps it: the MR and its merge commit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Final {
    pub mr: Mr,
    pub merge_sha: Option<String>,
}

/// The commit a merged MR put on develop: GraphQL's merge commit, else REST's
/// (`merge_commit_sha`, then `squash_commit_sha`), kept in the cache once found.
async fn merge_sha_of(gitlab: &Gitlab, cache: &mut Cache, known: Final) -> Option<String> {
    if known.merge_sha.is_some() {
        return known.merge_sha;
    }
    let value = gitlab
        .get_json(&format!("merge_requests/{}", known.mr.iid))
        .await
        .ok()?;
    let merge_sha = super::rest::merge_sha_of(&value)?;
    let iid = known.mr.iid.to_string();
    cache.set(
        "mrs",
        &iid,
        &Final {
            merge_sha: Some(merge_sha.clone()),
            ..known
        },
    );
    Some(merge_sha)
}

fn deploy_from(value: &Value, web: &str) -> Deploy {
    let jobs = jobs_of(value, web);
    Deploy {
        pipeline: value.is_object().then(|| DeployPipeline {
            id: id_of(value.get("id")),
            status: lower(str_at(value, "status")),
            url: str_at(value, "path").map(|path| format!("{web}{path}")),
            created_at: str_at(value, "createdAt").map(String::from),
        }),
        jobs: if value.is_object() { jobs } else { Vec::new() },
    }
}

/// The develop pipeline that deploys the merge commit `sha`, with its jobs
/// (gitlab.mjs `deployOf`): its own, or when that failed the newest develop
/// pipeline containing it (#275). Final ones from the cache.
pub async fn deploy_of(gitlab: &Gitlab, cache: &mut Cache, sha: &str, web: &str, now: i64) -> Result<Deploy> {
    use crate::model::try_state::{DeployState, deciding_deploy_of, deploy_state_of};
    if let Some(later) = cache.get::<Deploy>("deployedBy", sha) {
        return Ok(later);
    }
    let own = match cache.get::<Deploy>("deploys", sha) {
        Some(own) => own,
        None => {
            let project = gitlab
                .graphql(&gitlab.on_branch(&deploy_query()), json!({ "sha": sha }))
                .await?;
            let found = nodes(&project, "pipelines").into_iter().next().unwrap_or(Value::Null);
            let own = deploy_from(&found, web);
            if deploy_state_of(Some(&own), now).is_final {
                cache.set("deploys", sha, &own);
            }
            own
        }
    };
    if deploy_state_of(Some(&own), now).state != DeployState::Failed {
        return Ok(own);
    }
    let after = own.pipeline.as_ref().map(|pipeline| pipeline.id).unwrap_or(0);
    let later = later_deploy_of(gitlab, sha, after, web).await?;
    let deploy = deciding_deploy_of(&own, later.as_ref(), now).clone();
    if deploy != own && deploy_state_of(Some(&deploy), now).state == DeployState::Deployed {
        cache.set("deployedBy", sha, &deploy);
    }
    Ok(deploy)
}

/// The newest develop pipeline after `after` whose commit contains `sha`: one
/// that passed, else the newest (it may still deploy).
async fn later_deploy_of(gitlab: &Gitlab, sha: &str, after: i64, web: &str) -> Result<Option<Deploy>> {
    let project = gitlab
        .graphql(&gitlab.on_branch(&later_deploys_query()), json!({}))
        .await?;
    let all = nodes(&project, "pipelines");
    let passed = all
        .iter()
        .find(|pipeline| lower(str_at(pipeline, "status")) == "success");
    for found in [passed, all.first()].into_iter().flatten() {
        if id_of(found.get("id")) <= after {
            continue;
        }
        let head = str_at(found, "sha").unwrap_or_default();
        let base = gitlab
            .get_json(&format!("repository/merge_base?refs%5B%5D={sha}&refs%5B%5D={head}"))
            .await?;
        if base.get("id").and_then(Value::as_str) == Some(sha) {
            return Ok(Some(deploy_from(found, web)));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// An orchestration (gitlab.mjs `readRemoteOfOrchestration`)
// ---------------------------------------------------------------------------

/// What an orchestration's read asks about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    /// Its branches: the record's and those of worktrees started for it.
    pub branches: Vec<String>,
    /// The MRs its handovers, queued issues and waits wait for.
    pub awaited: Vec<i64>,
    /// Branches whose merged MR a try on the test instance waits for: their deployment is read.
    pub deploys: Vec<String>,
    /// Its epic branch (#350): its pipeline and its MR into develop are read.
    #[serde(default)]
    pub epic: Option<String>,
    /// Its product's batches shipped before the epic's (#414): their MRs into develop are read.
    #[serde(default)]
    pub shipping: Vec<String>,
    /// The branches whose task closes several issues (#321): every issue, the branch's first.
    #[serde(default)]
    pub bundles: BTreeMap<String, Vec<i64>>,
}

impl Ask {
    /// Every issue of the task of `branch`, the one it carries first.
    pub fn issues_of(&self, branch: &str) -> Vec<i64> {
        match self.bundles.get(branch) {
            Some(numbers) => numbers.clone(),
            None => number_of(branch).into_iter().collect(),
        }
    }
}

/// The links GitLab has of the open issues `numbers`, in the shape real GitLab
/// has them; an error (an older GitLab, the gauntlet's fake) leaves them out.
async fn read_links(gitlab: &Gitlab, memory: &mut Memory, numbers: &[i64]) {
    if numbers.is_empty() || memory.has_no_links {
        return;
    }
    for batch in numbers.chunks(BATCH) {
        let iids: Vec<String> = batch.iter().map(i64::to_string).collect();
        match gitlab.graphql(LINKS_QUERY, json!({ "iids": iids })).await {
            Ok(project) => {
                for issue in nodes(&project, "issues") {
                    let blocked: Vec<i64> = nodes(&issue, "blockedByIssues")
                        .iter()
                        .map(|node| id_of(node.get("iid")))
                        .collect();
                    memory.links.insert(id_of(issue.get("iid")), blocked);
                }
            }
            Err(_) => {
                memory.has_no_links = true;
                return;
            }
        }
    }
}

/// A finished pipeline as an orchestration shows it: its jobs only when it failed.
fn pipeline_remote(pipeline: &Value, web: &str) -> DevelopPipeline {
    let status = lower(str_at(pipeline, "status"));
    DevelopPipeline {
        id: id_of(pipeline.get("id")),
        jobs: if status == "failed" {
            jobs_of(pipeline, web)
        } else {
            Vec::new()
        },
        status,
        url: str_at(pipeline, "path").map(|path| format!("{web}{path}")),
    }
}

/// Of every branch of `ask`: its MR (merged ones too) with its pipeline's
/// jobs, its issue; develop's last finished pipeline; the states of the
/// awaited MRs. `is_full` looks again at what moves slowly: the links of the
/// open issues. Final things come from `cache` and go into it.
pub async fn read_orchestration(
    gitlab: &Gitlab,
    cache: &mut Cache,
    memory: &mut Memory,
    ask: &Ask,
    is_full: bool,
) -> Result<Remote> {
    if memory.is_rest_only && !is_full {
        return super::rest::read_orchestration(gitlab, cache, memory, ask, is_full).await;
    }
    gitlab.phase(Some("merge requests"));
    let branches: Vec<String> = {
        let mut seen = BTreeSet::new();
        ask.branches
            .iter()
            .filter(|branch| !branch.is_empty() && seen.insert((*branch).clone()))
            .cloned()
            .collect()
    };
    let final_of = |cache: &Cache, branch: &str| -> Option<Final> {
        let iid = cache.raw("branches", branch)?.as_i64()?;
        cache.get::<Final>("mrs", &iid.to_string())
    };
    let open_branches: Vec<String> = branches
        .iter()
        .filter(|branch| final_of(cache, branch).is_none())
        .cloned()
        .collect();
    let numbers: Vec<i64> = {
        let mut seen = BTreeSet::new();
        branches
            .iter()
            .flat_map(|branch| ask.issues_of(branch))
            .filter(|number| seen.insert(*number))
            .collect()
    };
    let open_numbers: Vec<i64> = numbers
        .iter()
        .copied()
        .filter(|number| cache.get::<Issue>("issues", &number.to_string()).is_none())
        .collect();
    let awaited_unknown: Vec<i64> = ask
        .awaited
        .iter()
        .copied()
        .filter(|iid| cache.raw("states", &iid.to_string()).is_none())
        .collect();

    // Two requests at once: the issues, the awaited, develop; the MRs of the open branches and the epic.
    let first_issues: Vec<String> = open_numbers.iter().take(BATCH).map(i64::to_string).collect();
    let query = orchestration_query(
        !first_issues.is_empty(),
        !awaited_unknown.is_empty(),
        ask.epic.is_some(),
    );
    let awaited_iids: Vec<String> = awaited_unknown.iter().map(i64::to_string).collect();
    let mut variables = json!({});
    let mut mrs_variables = json!({});
    if let Some(epic) = &ask.epic {
        variables["epic"] = json!(epic);
        // `$epics`: the epic or batch, then the product's batches shipped before it (#414).
        let mut epics = vec![epic.clone()];
        epics.extend(ask.shipping.iter().filter(|branch| *branch != epic).cloned());
        mrs_variables["epics"] = json!(epics);
        variables["epicTargets"] = json!([epic]);
    }
    for (name, values) in [("iids", &first_issues), ("awaited", &awaited_iids)] {
        if !values.is_empty() {
            variables[name] = json!(values);
        }
    }
    if !open_branches.is_empty() {
        mrs_variables["branches"] = json!(open_branches);
    }
    let has_mrs = !open_branches.is_empty() || ask.epic.is_some();
    let query = gitlab.on_branch(&query);
    let first_mrs = gitlab.on_branch(&mrs_query(!open_branches.is_empty(), ask.epic.is_some()));
    let (project, mrs) = tokio::join!(gitlab.graphql(&query, variables), async {
        if has_mrs {
            gitlab.graphql(&first_mrs, mrs_variables).await
        } else {
            Ok(Value::Null)
        }
    });
    let (project, mrs) = match (project, mrs) {
        (Ok(project), Ok(mrs)) => (project, mrs),
        // GitLab answered, its GraphQL didn't (an older GitLab, a proxy): REST, Node's route.
        (Err(error), _) | (_, Err(error)) if error.status.is_some() => {
            memory.is_rest_only = true;
            return super::rest::read_orchestration(gitlab, cache, memory, ask, is_full).await;
        }
        (Err(error), _) | (_, Err(error)) => return Err(error),
    };
    memory.is_rest_only = false;
    if project.is_null() {
        return Err(GitlabError::new("GitLab knows no such project"));
    }
    let mut mr_nodes = nodes(&mrs, "mrs");
    let mut page = mrs
        .get("mrs")
        .and_then(|mrs| mrs.get("pageInfo"))
        .cloned()
        .unwrap_or(Value::Null);
    while page.get("hasNextPage").and_then(Value::as_bool) == Some(true) {
        let more = gitlab
            .graphql(
                &mrs_query(true, false),
                json!({ "branches": open_branches, "after": page.get("endCursor") }),
            )
            .await?;
        mr_nodes.extend(nodes(&more, "mrs"));
        page = more
            .get("mrs")
            .and_then(|mrs| mrs.get("pageInfo"))
            .cloned()
            .unwrap_or(Value::Null);
    }
    let mut issue_nodes = nodes(&project, "issues");
    for batch in open_numbers.chunks(BATCH).skip(1) {
        let iids: Vec<String> = batch.iter().map(i64::to_string).collect();
        let more = gitlab.graphql(ISSUES_PAGE_QUERY, json!({ "iids": iids })).await?;
        issue_nodes.extend(nodes(&more, "issues"));
    }
    // Where the pages are: GitLab's own answer says, else the API's host.
    let web = mr_nodes
        .iter()
        .chain(issue_nodes.iter())
        .find_map(|node| str_at(node, "webUrl").and_then(origin_of))
        .unwrap_or_else(|| "https://gitlab.com".into());

    // Each branch's MR: open, else the newest.
    let mut found: HashMap<String, Vec<&Value>> = HashMap::new();
    for node in &mr_nodes {
        found
            .entry(str_at(node, "sourceBranch").unwrap_or_default().to_string())
            .or_default()
            .push(node);
    }
    let mut of: HashMap<String, BranchRemote> = HashMap::new();
    gitlab.phase(Some("jobs"));
    for branch in &branches {
        let (mr, jobs) = if let Some(known) = final_of(cache, branch) {
            (Some(known.mr), Vec::new())
        } else if let Some(listed) = found.get(branch) {
            let node = listed
                .iter()
                .find(|node| str_at(node, "state") == Some("opened"))
                .unwrap_or(&listed[0]);
            let (mr, merge_sha) = mr_of(node);
            if mr.state == "merged" || mr.state == "closed" {
                let known = Final {
                    mr: mr.clone(),
                    merge_sha,
                };
                cache.set("mrs", &mr.iid.to_string(), &known);
                cache.set("branches", branch, &mr.iid);
                (Some(mr), Vec::new())
            } else {
                let read = node
                    .get("headPipeline")
                    .map(|pipeline| jobs_of(pipeline, &web))
                    .unwrap_or_default();
                let jobs = match &mr.pipeline {
                    Some(pipeline) => settle_jobs(gitlab, cache, memory, pipeline, read).await,
                    None => Vec::new(),
                };
                (Some(mr), jobs)
            }
        } else {
            (None, Vec::new())
        };
        of.insert(
            branch.clone(),
            BranchRemote {
                mr,
                jobs,
                ..BranchRemote::default()
            },
        );
    }

    // Merged, with a try on the test instance waiting: where its deployment stands.
    gitlab.phase(Some("deploys"));
    for branch in &ask.deploys {
        let Some(known) = final_of(cache, branch) else { continue };
        if !known.mr.is_merged() {
            continue;
        }
        let Some(sha) = merge_sha_of(gitlab, cache, known).await else {
            continue;
        };
        let deploy = deploy_of(gitlab, cache, &sha, &web, now_ms()).await.ok();
        if let Some(remote) = of.get_mut(branch)
            && let Some(mr) = remote.mr.as_mut()
        {
            mr.deploy = deploy;
        }
    }

    // The issues: closed ones known, the open ones' links on a full read.
    gitlab.phase(Some("issues"));
    let mut issues: HashMap<i64, Issue> = HashMap::new();
    for number in &numbers {
        if let Some(issue) = cache.get::<Issue>("issues", &number.to_string()) {
            issues.insert(*number, issue);
        }
    }
    let open_issues: Vec<i64> = issue_nodes
        .iter()
        .filter(|node| str_at(node, "state") != Some("closed"))
        .map(|node| id_of(node.get("iid")))
        .filter(|iid| is_full || !memory.links.contains_key(iid))
        .collect();
    read_links(gitlab, memory, &open_issues).await;
    for node in &issue_nodes {
        let iid = id_of(node.get("iid"));
        if str_at(node, "state") == Some("closed") {
            let done = issue_of(node, &[]);
            cache.set("issues", &iid.to_string(), &done);
            issues.insert(iid, done);
        } else {
            let links = memory.links.get(&iid).cloned().unwrap_or_default();
            issues.insert(iid, issue_of(node, &links));
        }
    }
    for (branch, remote) in of.iter_mut() {
        remote.set_issues(&ask.issues_of(branch), &issues);
    }

    // develop's last finished pipeline, its jobs only when it failed; the epic's the same way.
    let last_finished = |key: &str| {
        nodes(&project, key)
            .into_iter()
            .find(|pipeline| FINISHED.contains(&lower(str_at(pipeline, "status")).as_str()))
    };
    let epic = ask.epic.as_ref().map(|branch| {
        Box::new(EpicRemote {
            branch: branch.clone(),
            pipeline: last_finished("epicPipelines").map(|pipeline| pipeline_remote(&pipeline, &web)),
            mr: nodes(&mrs, "epicMrs")
                .iter()
                .map(|node| mr_of(node).0)
                .find(|mr| mr.branch == *branch),
            tasks: nodes(&project, "epicTasks").iter().map(batch_task_of).collect(),
            shipping: nodes(&mrs, "epicMrs")
                .iter()
                .map(|node| mr_of(node).0)
                .filter(|mr| mr.branch != *branch && ask.shipping.contains(&mr.branch))
                .collect(),
            failed_jobs: nodes(&project, "epicMrs")
                .iter()
                .filter_map(|node| {
                    let pipeline = node.get("headPipeline")?;
                    (lower(str_at(pipeline, "status")) == "failed").then(|| {
                        let jobs = jobs_of(pipeline, &web).into_iter().filter(|job| job.status == "failed");
                        (id_of(node.get("iid")), jobs.collect())
                    })
                })
                .collect(),
        })
    });
    let develop = last_finished("develop").map(|pipeline| pipeline_remote(&pipeline, &web));
    // The awaited MRs: from what was read, the cache, else the answer.
    let mut awaited = HashMap::new();
    let listed: HashMap<i64, String> = nodes(&project, "awaited")
        .iter()
        .map(|node| (id_of(node.get("iid")), lower(str_at(node, "state"))))
        .collect();
    for iid in &ask.awaited {
        let state = of
            .values()
            .filter_map(|remote| remote.mr.as_ref())
            .find(|mr| mr.iid == *iid)
            .map(|mr| mr.state.clone())
            .or_else(|| cache.get::<String>("states", &iid.to_string()))
            .or_else(|| listed.get(iid).cloned());
        if let Some(state) = state {
            if state == "merged" || state == "closed" {
                cache.set("states", &iid.to_string(), &state);
            }
            awaited.insert(*iid, state);
        }
    }
    gitlab.phase(Some("mergeability"));
    super::recheck::recheck(gitlab, &mut memory.rechecks, &mut of, now_ms()).await;
    gitlab.phase(None);
    cache.save();
    Ok(Remote {
        of,
        awaited,
        develop,
        epic,
        at: now_ms(),
    })
}

// ---------------------------------------------------------------------------
// One worktree (read.mjs `readRemoteOf`)
// ---------------------------------------------------------------------------

/// An open MR another worktree's branch is: what WAITING FOR names (waiting.mjs).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OpenMr {
    pub iid: i64,
    pub title: String,
    pub branch: String,
    pub is_draft: bool,
    pub state: String,
    pub url: String,
    pub pipeline: Option<Pipeline>,
}

/// One worktree's part of GitLab: its MR (open, else the latest) with its
/// pipeline's jobs, its issue, and the open MRs of the project (waiting.mjs
/// picks those it builds on).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorktreeRemote {
    pub mr: Option<Mr>,
    pub jobs: Vec<Job>,
    pub issue: Option<Issue>,
    pub open: Vec<OpenMr>,
    pub at: i64,
}

pub async fn read_worktree(
    gitlab: &Gitlab,
    cache: &mut Cache,
    memory: &mut Memory,
    branch: Option<&str>,
) -> Result<WorktreeRemote> {
    let number = branch.and_then(number_of);
    let query = worktree_query(branch.is_some(), number.is_some());
    let mut variables = json!({});
    if let Some(branch) = branch {
        variables["branches"] = json!([branch]);
    }
    if let Some(number) = number {
        variables["iid"] = json!(number.to_string());
    }
    let project = gitlab.graphql(&query, variables).await?;
    let listed = nodes(&project, "mrs");
    let web = listed
        .iter()
        .chain(nodes(&project, "open").iter())
        .find_map(|node| str_at(node, "webUrl").and_then(origin_of))
        .unwrap_or_else(|| "https://gitlab.com".into());
    let latest = listed
        .iter()
        .find(|node| str_at(node, "state") == Some("opened"))
        .or(listed.first());
    let (mut mr, mut jobs) = (None, Vec::new());
    if let Some(node) = latest {
        let (mut found, merge_sha) = mr_of(node);
        if let Some(pipeline) = &found.pipeline {
            let read = node
                .get("headPipeline")
                .map(|pipeline| jobs_of(pipeline, &web))
                .unwrap_or_default();
            jobs = settle_jobs(gitlab, cache, memory, pipeline, read).await;
        }
        if found.is_merged()
            && let Some(sha) = merge_sha
        {
            found.deploy = deploy_of(gitlab, cache, &sha, &web, now_ms()).await.ok();
        }
        mr = Some(found);
    }
    let issue = match project.get("issue").filter(|issue| issue.is_object()) {
        Some(node) => {
            let iid = id_of(node.get("iid"));
            if str_at(node, "state") != Some("closed") {
                read_links(gitlab, memory, &[iid]).await;
            }
            Some(issue_of(node, &memory.links.get(&iid).cloned().unwrap_or_default()))
        }
        None => None,
    };
    let open = nodes(&project, "open")
        .iter()
        .map(|node| {
            let url = str_at(node, "webUrl").unwrap_or_default().to_string();
            OpenMr {
                iid: id_of(node.get("iid")),
                title: DRAFT
                    .replace(str_at(node, "title").unwrap_or_default(), "")
                    .into_owned(),
                branch: str_at(node, "sourceBranch").unwrap_or_default().to_string(),
                is_draft: node.get("draft").and_then(Value::as_bool).unwrap_or(false),
                state: str_at(node, "state").unwrap_or_default().to_string(),
                pipeline: node
                    .get("headPipeline")
                    .and_then(|pipeline| pipeline_of(pipeline, &web)),
                url,
            }
        })
        .collect();
    cache.save();
    Ok(WorktreeRemote {
        mr,
        jobs,
        issue,
        open,
        at: now_ms(),
    })
}

// ---------------------------------------------------------------------------
// Everything (read.mjs `readRemote`)
// ---------------------------------------------------------------------------

/// An open MR in the view of everything.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ListedMr {
    pub iid: i64,
    pub title: String,
    pub branch: String,
    pub is_draft: bool,
    /// Its assignees, else its author.
    pub who: Option<String>,
    pub url: String,
    pub pipeline: Option<Pipeline>,
}

/// An open issue in the view of everything, with the branch that claims it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ListedIssue {
    pub iid: i64,
    pub title: String,
    pub url: String,
    pub labels: Vec<String>,
    pub who: Option<String>,
    pub branch: Option<String>,
    pub mr: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EverythingRemote {
    pub mrs: Vec<ListedMr>,
    /// Claimed ones first: that is the work in flight.
    pub issues: Vec<ListedIssue>,
    pub develop: Option<Pipeline>,
    /// develop's head on origin (`git ls-remote`): the repo's changelog is kept for it (#261).
    pub head: Option<String>,
    pub at: i64,
}

fn names_of(value: &Value, key: &str) -> Vec<String> {
    nodes(value, key)
        .iter()
        .filter_map(|node| str_at(node, "username").map(String::from))
        .collect()
}

/// The open MRs and issues of the project, develop's pipeline; `heads` the
/// branches on origin (`git ls-remote`), which claim the issues.
pub async fn read_everything(gitlab: &Gitlab, heads: &[String]) -> Result<EverythingRemote> {
    let project = gitlab.graphql(&gitlab.on_branch(EVERYTHING_QUERY), json!({})).await?;
    let mr_nodes = nodes(&project, "mrs");
    let web = mr_nodes
        .iter()
        .find_map(|node| str_at(node, "webUrl").and_then(origin_of))
        .unwrap_or_else(|| "https://gitlab.com".into());
    let mrs: Vec<ListedMr> = mr_nodes
        .iter()
        .map(|node| {
            let assignees = names_of(node, "assignees");
            ListedMr {
                iid: id_of(node.get("iid")),
                title: DRAFT
                    .replace(str_at(node, "title").unwrap_or_default(), "")
                    .into_owned(),
                branch: str_at(node, "sourceBranch").unwrap_or_default().to_string(),
                is_draft: node.get("draft").and_then(Value::as_bool).unwrap_or(false),
                who: if assignees.is_empty() {
                    node.get("author")
                        .and_then(|author| str_at(author, "username"))
                        .map(String::from)
                } else {
                    Some(assignees.join(","))
                },
                url: str_at(node, "webUrl").unwrap_or_default().to_string(),
                pipeline: node
                    .get("headPipeline")
                    .and_then(|pipeline| pipeline_of(pipeline, &web)),
            }
        })
        .collect();
    let claim_of = |number: i64| {
        heads
            .iter()
            .find(|branch| {
                branch
                    .rsplit('/')
                    .next()
                    .is_some_and(|last| last.starts_with(&format!("{number}-")))
            })
            .cloned()
    };
    let mut issues: Vec<ListedIssue> = nodes(&project, "issues")
        .iter()
        .map(|node| {
            let iid = id_of(node.get("iid"));
            let branch = claim_of(iid);
            let who = names_of(node, "assignees").join(",");
            ListedIssue {
                iid,
                title: str_at(node, "title").unwrap_or_default().to_string(),
                url: str_at(node, "webUrl").unwrap_or_default().to_string(),
                labels: nodes(node, "labels")
                    .iter()
                    .filter_map(|label| str_at(label, "title").map(String::from))
                    .collect(),
                who: Some(who).filter(|who| !who.is_empty()),
                mr: branch
                    .as_ref()
                    .and_then(|branch| mrs.iter().find(|mr| &mr.branch == branch))
                    .map(|mr| mr.iid),
                branch,
            }
        })
        .collect();
    issues.sort_by_key(|issue| issue.branch.is_none());
    let develop = nodes(&project, "develop")
        .first()
        .and_then(|pipeline| pipeline_of(pipeline, &web));
    Ok(EverythingRemote {
        mrs,
        issues,
        develop,
        head: None,
        at: now_ms(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The queries as the gauntlet checks them against gitlab.com's schema (#338):
    /// a query changed here and not there fails, and the file is written anew to commit.
    #[test]
    fn the_queries_the_gauntlet_checks_are_the_ones_sent() {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../breadboard-gauntlet/fixtures/gitlab-queries.json");
        let listed: serde_json::Map<String, Value> = queries()
            .into_iter()
            .map(|(name, query)| (name.to_string(), Value::String(query)))
            .collect();
        let text = format!("{}\n", serde_json::to_string_pretty(&listed).unwrap());
        if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&file, &text).unwrap();
            panic!(
                "{} was behind the queries: written anew, commit it (the gauntlet checks it against gitlab.com's schema)",
                file.display()
            );
        }
    }

    /// GitLab refuses a query above its complexity limit, and the overview reads nothing (#454):
    /// each stays under ALLOWED with every part it can have, room left for a field or two.
    #[test]
    fn every_query_costs_less_than_gitlab_allows() {
        use super::super::complexity::{ALLOWED, complexity};
        for (name, query) in queries() {
            let cost = complexity(&query);
            assert!(
                cost <= ALLOWED,
                "the {name} query costs {cost} (GitLab refuses above 250): split it or ask for less of it"
            );
        }
    }

    const MR: &str = r#"{
        "iid": "303", "title": "Draft: feat(breadboard): every drawn button works", "state": "opened", "draft": true,
        "webUrl": "https://gitlab.com/monocircuit/platform/-/merge_requests/303", "sourceBranch": "feature/303-buttons",
        "mergedAt": null, "conflicts": false, "detailedMergeStatus": "MERGEABLE", "diffHeadSha": "abc",
        "mergeCommitSha": null,
        "headPipeline": { "id": "gid://gitlab/Ci::Pipeline/2072", "iid": "9", "status": "FAILED",
          "path": "/monocircuit/platform/-/pipelines/2072", "sha": "abc", "ref": "feature/303-buttons",
          "createdAt": "2026-10-01T10:00:00Z", "updatedAt": "2026-10-01T10:05:00Z", "finishedAt": "2026-10-01T10:05:00Z",
          "jobs": { "nodes": [
            { "id": "gid://gitlab/Ci::Build/12", "name": "test", "status": "FAILED", "webPath": "/monocircuit/platform/-/jobs/12", "finishedAt": null },
            { "id": "gid://gitlab/Ci::Build/10", "name": "lint", "status": "SUCCESS", "webPath": "/monocircuit/platform/-/jobs/10", "finishedAt": null },
            { "id": "gid://gitlab/Ci::Build/11", "name": "test", "status": "FAILED", "webPath": "/monocircuit/platform/-/jobs/11", "finishedAt": null }
          ] } } }"#;

    #[test]
    fn reads_the_open_mrs_for_the_ci_queue() {
        let project: Value = serde_json::from_str(
            r#"{ "mergeRequests": { "nodes": [ { "iid": "349", "draft": true,
              "sourceBranch": "feature/357-x", "targetBranch": "develop", "diffHeadSha": "0359",
              "labels": { "nodes": [ { "title": "enhancement" } ] },
              "pipelines": { "nodes": [ { "id": "gid://gitlab/Ci::Pipeline/2936237216", "status": "SKIPPED",
                "sha": "0359", "ref": "feature/357-x" },
                { "id": "gid://gitlab/Ci::Pipeline/2936237215", "status": "PENDING",
                "sha": "0359", "ref": "refs/merge-requests/349/head" } ] } } ] } }"#,
        )
        .unwrap();
        let mrs = open_mrs_of(&project);
        assert_eq!(mrs.len(), 1);
        assert_eq!(
            (mrs[0].iid, mrs[0].is_draft, mrs[0].target.as_str()),
            (349, true, "develop")
        );
        let pipeline = mrs[0].pipeline.as_ref().unwrap();
        assert_eq!((pipeline.id, pipeline.status.as_str()), (2936237215, "pending"));
        assert_eq!(pipeline.reference.as_deref(), Some("refs/merge-requests/349/head"));
        assert_eq!(mrs[0].labels, vec!["enhancement".to_string()]);
    }

    #[test]
    fn says_which_pipeline_blocks_a_commit_that_passed() {
        let mr = |pipelines: &str| -> Option<i64> {
            let project: Value = serde_json::from_str(&format!(
                r#"{{ "mergeRequests": {{ "nodes": [ {{ "iid": "348", "diffHeadSha": "80b8",
                  "pipelines": {{ "nodes": [ {pipelines} ] }} }} ] }} }}"#
            ))
            .unwrap();
            open_mrs_of(&project)[0].blocked_by
        };
        let pipeline = |id: i64, status: &str, sha: &str, reason: &str| {
            format!(
                r#"{{ "id": "gid://gitlab/Ci::Pipeline/{id}", "status": "{status}", "sha": "{sha}",
                  "ref": "refs/merge-requests/348/head", "failureReason": {reason} }}"#
            )
        };
        let passed = pipeline(10, "SUCCESS", "80b8", "null");
        let refused = pipeline(
            12,
            "FAILED",
            "80b8",
            r#""The pipeline job activity limit was exceeded.""#,
        );
        assert_eq!(mr(&format!("{refused}, {passed}")), Some(12));
        assert_eq!(
            mr(&format!("{passed}, {}", pipeline(11, "CANCELED", "80b8", "null"))),
            Some(11)
        );
        // Its jobs failed: a real red run, not a block.
        assert_eq!(
            mr(&format!("{passed}, {}", pipeline(11, "FAILED", "80b8", "null"))),
            None
        );
        // Nothing of that commit passed, or the newest passed: nothing blocks.
        assert_eq!(mr(&refused), None);
        assert_eq!(
            mr(&format!("{}, {passed}", pipeline(9, "CANCELED", "80b8", "null"))),
            None
        );
        // An older commit's pass doesn't count.
        assert_eq!(
            mr(&format!("{refused}, {}", pipeline(10, "SUCCESS", "old", "null"))),
            None
        );
    }

    #[test]
    fn takes_the_newest_pipeline_whatever_order_gitlab_lists_them_in() {
        // !337 on 2026-10-12 as gitlab.com listed it: by sha, the newest last.
        let project: Value = serde_json::from_str(
            r#"{ "mergeRequests": { "nodes": [ { "iid": "337", "draft": true,
              "sourceBranch": "feature/337-x", "targetBranch": "develop", "diffHeadSha": "e426e44f",
              "pipelines": { "nodes": [
                { "id": "gid://gitlab/Ci::Pipeline/2936354816", "status": "CANCELED", "sha": "1277fb9d", "ref": "refs/merge-requests/337/head" },
                { "id": "gid://gitlab/Ci::Pipeline/2936044899", "status": "FAILED", "sha": "1696dc83", "ref": "refs/merge-requests/337/head" },
                { "id": "gid://gitlab/Ci::Pipeline/2939744109", "status": "RUNNING", "sha": "e426e44f", "ref": "refs/merge-requests/337/head" }
              ] } } ] } }"#,
        )
        .unwrap();
        let pipeline = open_mrs_of(&project)[0].pipeline.clone().unwrap();
        assert_eq!((pipeline.id, pipeline.sha.as_deref()), (2939744109, Some("e426e44f")));
    }

    #[test]
    fn reads_an_mr_of_graphql_as_node_shapes_it() {
        let node: Value = serde_json::from_str(MR).unwrap();
        let (mr, merge_sha) = mr_of(&node);
        assert_eq!(mr.iid, 303);
        assert_eq!(mr.title, "feat(breadboard): every drawn button works");
        assert!(mr.is_draft);
        assert_eq!(mr.merge_status.as_deref(), Some("mergeable"));
        assert_eq!(mr.branch, "feature/303-buttons");
        assert_eq!(merge_sha, None);
        let pipeline = mr.pipeline.unwrap();
        assert_eq!(pipeline.id, 2072);
        assert_eq!(pipeline.status, "failed");
        assert_eq!(
            pipeline.web_url.as_deref(),
            Some("https://gitlab.com/monocircuit/platform/-/pipelines/2072")
        );
        let jobs = jobs_of(&node["headPipeline"], "https://gitlab.com");
        assert_eq!(
            jobs.iter().map(|job| (job.id, job.name.as_str())).collect::<Vec<_>>(),
            vec![(10, "lint"), (12, "test")]
        );
        assert_eq!(
            jobs[1].url.as_deref(),
            Some("https://gitlab.com/monocircuit/platform/-/jobs/12")
        );
    }

    #[test]
    fn takes_the_lines_before_the_runner_tail_of_a_log() {
        let trace = "2026-09-28T13:02:20.886478Z 01O section_start:1:step_script\r\x1b[0K$ pnpm test\n\
                     a\nb\nc\nd\ne\nf\nError: expected 1\nCleaning up project directory\nERROR: Job failed: exit code 1\n";
        assert_eq!(log_lines(trace), vec!["b", "c", "d", "e", "f", "Error: expected 1"]);
        assert!(log_lines("").is_empty());
    }

    #[test]
    fn reads_an_issue_with_its_links() {
        let node = json!({ "iid": "296", "title": "hover", "description": "after #303\n- [ ] a box", "state": "opened", "webUrl": "u" });
        let issue = issue_of(&node, &[294, 303]);
        assert_eq!(issue.needs, vec![303, 294]);
        assert_eq!(issue.boxes.len(), 1);
        assert_eq!(
            origin_of("https://gitlab.com/a/-/issues/1").as_deref(),
            Some("https://gitlab.com")
        );
    }
}
