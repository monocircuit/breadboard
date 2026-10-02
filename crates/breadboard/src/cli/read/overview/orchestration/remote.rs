//! An orchestration's part of GitLab (gitlab.mjs `readRemoteOfOrchestration`):
//! the MR of each of its branches with its pipeline, the issue of each with
//! its boxes and prerequisites, the jobs of a failed pipeline, and the state
//! of every MR a wait or a handover names. Lists side by side, the links of
//! the open issues in one GraphQL query, what can't change any more from the
//! disk cache Node's overview shares (`read`).

use super::try_state::Deploy;
use crate::cli::read::cache::Cache;
use crate::cli::read::gitlab;
use crate::cli::read::issues::{Box as IssueBox, boxes_with_headings};
use breadboard_core::git::issue_of_branch;
use breadboard_core::json::{int_of, str_of};
use breadboard_core::time::now_ms;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;
use std::thread;

/// A pipeline as the view uses it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pipeline {
    pub id: i64,
    pub status: String,
    pub url: Option<String>,
    pub sha: Option<String>,
    /// When it finished, else when it last changed: the age of an action about it.
    pub finished_at: Option<String>,
    /// When it last changed: with the id, the key of its jobs in the cache.
    pub updated_at: Option<String>,
}

impl Pipeline {
    fn from_value(value: &Value) -> Option<Pipeline> {
        Some(Pipeline {
            id: int_of(value, "id").unwrap_or(0),
            status: str_of(value, "status")?.to_string(),
            url: str_of(value, "web_url").map(String::from),
            sha: str_of(value, "sha").map(String::from),
            finished_at: str_of(value, "finished_at")
                .or_else(|| str_of(value, "updated_at"))
                .map(String::from),
            updated_at: str_of(value, "updated_at").map(String::from),
        })
    }
}

/// A merge request as the view uses it (read.mjs `mrOf`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mr {
    pub iid: i64,
    /// Without its `Draft:`: the state says it.
    pub title: String,
    pub is_draft: bool,
    pub state: String,
    pub merged_at: Option<String>,
    pub has_conflicts: bool,
    /// GitLab's `detailed_merge_status`: only `mergeable` can be merged.
    pub merge_status: Option<String>,
    /// GitLab's `merge_status`: `checking` after a push.
    pub merge_check: Option<String>,
    pub sha: Option<String>,
    pub url: Option<String>,
    pub source_branch: String,
    pub pipeline: Option<Pipeline>,
    /// The commit a merged one put on develop.
    pub merge_sha: Option<String>,
    /// Merged: the develop pipeline that deploys it, read for a try on the test instance only.
    pub deploy: Option<Deploy>,
}

impl Mr {
    pub fn from_value(value: &Value) -> Option<Mr> {
        let title = str_of(value, "title").unwrap_or_default();
        let title = title
            .strip_prefix("Draft:")
            .map(str::trim_start)
            .unwrap_or(title)
            .to_string();
        Some(Mr {
            iid: int_of(value, "iid")?,
            title,
            is_draft: value.get("draft").and_then(Value::as_bool).unwrap_or(false),
            state: str_of(value, "state").unwrap_or_default().to_string(),
            merged_at: str_of(value, "merged_at").map(String::from),
            has_conflicts: value.get("has_conflicts").and_then(Value::as_bool).unwrap_or(false),
            merge_status: str_of(value, "detailed_merge_status").map(String::from),
            merge_check: str_of(value, "merge_status").map(String::from),
            sha: str_of(value, "sha").map(String::from),
            url: str_of(value, "web_url").map(String::from),
            source_branch: str_of(value, "source_branch").unwrap_or_default().to_string(),
            pipeline: value.get("head_pipeline").and_then(Pipeline::from_value),
            merge_sha: str_of(value, "merge_commit_sha")
                .or_else(|| str_of(value, "squash_commit_sha"))
                .map(String::from),
            deploy: None,
        })
    }

    pub fn is_open(&self) -> bool {
        self.state == "opened"
    }

    pub fn is_merged(&self) -> bool {
        self.state == "merged"
    }

    /// The pipeline's status, None without one.
    pub fn status(&self) -> Option<&str> {
        self.pipeline.as_ref().map(|pipeline| pipeline.status.as_str())
    }
}

/// A job of a pipeline.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Job {
    pub name: String,
    pub status: String,
    pub url: Option<String>,
    pub finished_at: Option<String>,
}

/// An issue as the view uses it (read.mjs `issueOf`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Issue {
    pub iid: i64,
    pub title: String,
    pub url: Option<String>,
    pub state: String,
    pub boxes: Vec<IssueBox>,
    /// The issues it names as prerequisites or is blocked by.
    pub needs: Vec<i64>,
    /// The files it names: the agent that knows them is suggested for it (fit.mjs `pathsOf`).
    pub paths: Vec<String>,
}

/// What GitLab has for one branch.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub mr: Option<Mr>,
    pub jobs: Vec<Job>,
    pub issue: Option<Issue>,
}

/// The orchestration's part of GitLab.
#[derive(Debug, Clone, Default)]
pub struct Remote {
    /// By branch.
    pub of: HashMap<String, Found>,
    /// The MRs waits and handovers name, by iid: their state.
    pub awaited: HashMap<i64, String>,
    /// When it was read.
    pub at: i64,
    /// What was read only in part, in words: the view says it, its advice may miss it.
    pub warnings: Vec<String>,
}

impl Remote {
    pub fn of(&self, branch: Option<&str>) -> Option<&Found> {
        self.of.get(branch?)
    }
}

/// How an issue names a prerequisite: `after #180`, `needs #180 and #181`
/// (merge-order.mjs `PREREQUISITE`).
static PREREQUISITE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(?:after|needs|builds on|depends on|requires|blocked by)\s+(#\d+(?:(?:\s*,\s*|\s+and\s+|\s*&\s*|\s*/\s*)#\d+)*)",
    )
    .expect("the prerequisite pattern")
});

/// A file path an issue names: with a folder or not, with an extension of the repo's files.
static PATH: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?:[\w@.-]+/)*[\w@-][\w@.-]*\.(?:mjs|cjs|js|jsx|ts|tsx|json|md|css|scss|sql|toml|ya?ml)\b")
        .expect("the path pattern")
});

/// The file paths a description names, each once (fit.mjs `pathsOf`).
pub fn paths_of(text: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for found in PATH.find_iter(text) {
        if !paths.iter().any(|path| path == found.as_str()) {
            paths.push(found.as_str().to_string());
        }
    }
    paths
}

/// The issues a description names as prerequisites, each once.
pub fn needs_of(text: &str) -> Vec<i64> {
    let mut needs = Vec::new();
    for found in PREREQUISITE.captures_iter(text) {
        for number in found[1].split('#').filter_map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        }) {
            if !needs.contains(&number) {
                needs.push(number);
            }
        }
    }
    needs
}

fn issue_of(value: &Value, links: &[Value]) -> Option<Issue> {
    let description = str_of(value, "description").unwrap_or_default();
    let mut needs = needs_of(description);
    for link in links {
        if str_of(link, "link_type") == Some("is_blocked_by")
            && let Some(iid) = int_of(link, "iid")
            && !needs.contains(&iid)
        {
            needs.push(iid);
        }
    }
    Some(Issue {
        iid: int_of(value, "iid")?,
        title: str_of(value, "title").unwrap_or_default().to_string(),
        url: str_of(value, "web_url").map(String::from),
        state: str_of(value, "state").unwrap_or_default().to_string(),
        boxes: boxes_with_headings(description),
        needs,
        paths: paths_of(description),
    })
}

/// A read's lists side by side; Err the first reason one failed.
fn lists(main: &Path, endpoints: &[String]) -> Result<Vec<Vec<Value>>, String> {
    gitlab::lists(main, endpoints).into_iter().collect()
}

/// The GraphQL query of the links of `issues`: the issues that block each, as
/// real GitLab has them (`blockedByIssues` is a connection there).
fn links_query(issues: &[i64]) -> String {
    let iids: Vec<String> = issues.iter().map(|iid| format!("\"{iid}\"")).collect();
    format!(
        "issues(iids: [{}], state: all, first: 100) {{ nodes {{ iid blockedByIssues {{ nodes {{ iid }} }} }} }}",
        iids.join(", ")
    )
}

/// The links of each issue GraphQL answered, in the shape of REST's
/// `issues/<n>/links` (`{ link_type, iid }`) that `issue_of` reads.
fn links_of(project: &Value) -> HashMap<i64, Vec<Value>> {
    let nodes = |value: &Value, field: &str| -> Vec<Value> {
        value
            .pointer(&format!("/{field}/nodes"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let iid_of = |value: &Value| match value.get("iid") {
        Some(Value::String(iid)) => iid.parse().ok(),
        Some(iid) => iid.as_i64(),
        None => None,
    };
    nodes(project, "issues")
        .iter()
        .filter_map(|issue| {
            let links = nodes(issue, "blockedByIssues")
                .iter()
                .filter_map(iid_of)
                .map(|iid| json!({ "link_type": "is_blocked_by", "iid": iid }))
                .collect();
            Some((iid_of(issue)?, links))
        })
        .collect()
}

/// The links of the open `issues`: one GraphQL query; when GitLab doesn't
/// answer it (an older GitLab, a proxy that only passes REST), REST's
/// `issues/<n>/links` of each, side by side (Node's route). What neither
/// could read comes back as a warning: a missing "blocked by" turns the
/// merge advice wrong, so the view must not keep quiet about it.
fn links_of_issues(main: &Path, issues: &[i64]) -> (HashMap<i64, Vec<Value>>, Option<String>) {
    if issues.is_empty() {
        return (HashMap::new(), None);
    }
    let graphql_error = match gitlab::graphql(main, &links_query(issues)) {
        Ok(project) => return (links_of(&project), None),
        Err(error) => error,
    };
    let endpoints: Vec<String> = issues
        .iter()
        .map(|iid| format!("projects/:fullpath/issues/{iid}/links"))
        .collect();
    let mut links = HashMap::new();
    let mut unread: Vec<i64> = Vec::new();
    let mut rest_error = None;
    for (iid, listed) in issues.iter().zip(gitlab::lists(main, &endpoints)) {
        match listed {
            Ok(listed) => {
                links.insert(*iid, listed);
            }
            Err(error) => {
                unread.push(*iid);
                rest_error.get_or_insert(error);
            }
        }
    }
    unread.sort_unstable();
    let unread: Vec<String> = unread.iter().map(|iid| format!("#{iid}")).collect();
    let warning = rest_error.map(|rest_error| {
        let reason = |error: &str| gitlab::explain(error).0;
        format!(
            "the \"blocked by\" links of {} can't be read: the merge order may miss one (GraphQL: {}; REST: {})",
            unread.join(", "),
            reason(&graphql_error),
            reason(&rest_error)
        )
    });
    (links, warning)
}

/// An MR as the Node overview caches a final one (read.mjs `mrOf`, rule 5):
/// `{ mr, mergeSha }`, the pipeline in GitLab's REST shape.
fn mr_to_cache(mr: &Mr) -> Value {
    let pipeline = mr.pipeline.as_ref().map(|pipeline| {
        json!({
            "id": pipeline.id, "status": pipeline.status, "web_url": pipeline.url, "sha": pipeline.sha,
            "finished_at": pipeline.finished_at, "updated_at": pipeline.updated_at,
        })
    });
    json!({
        "mr": {
            "iid": mr.iid, "title": mr.title, "isDraft": mr.is_draft, "state": mr.state,
            "mergedAt": mr.merged_at, "hasConflicts": mr.has_conflicts, "mergeStatus": mr.merge_status,
            "mergeCheck": mr.merge_check, "sha": mr.sha, "url": mr.url, "pipeline": pipeline, "deploy": null,
        },
        "mergeSha": mr.merge_sha,
    })
}

/// A final MR of `branch` from the cache, None in another shape.
fn mr_from_cache(value: &Value, branch: &str) -> Option<Mr> {
    let mr = value.get("mr")?;
    let text = |key: &str| str_of(mr, key).map(String::from);
    let is = |key: &str| mr.get(key).and_then(Value::as_bool).unwrap_or(false);
    Some(Mr {
        iid: int_of(mr, "iid")?,
        title: text("title").unwrap_or_default(),
        is_draft: is("isDraft"),
        state: text("state")?,
        merged_at: text("mergedAt"),
        has_conflicts: is("hasConflicts"),
        merge_status: text("mergeStatus"),
        merge_check: text("mergeCheck"),
        sha: text("sha"),
        url: text("url"),
        source_branch: branch.to_string(),
        pipeline: mr.get("pipeline").and_then(Pipeline::from_value),
        merge_sha: str_of(value, "mergeSha").map(String::from),
        deploy: None,
    })
}

/// A closed issue as the Node overview caches it (read.mjs `issueOf`).
fn issue_to_cache(issue: &Issue) -> Value {
    let boxes: Vec<Value> = issue
        .boxes
        .iter()
        .map(|found| json!({ "line": found.line, "isTicked": found.is_ticked, "text": found.text, "heading": found.heading }))
        .collect();
    json!({
        "iid": issue.iid, "title": issue.title, "url": issue.url, "state": issue.state,
        "boxes": boxes, "needs": issue.needs, "paths": issue.paths,
    })
}

/// A closed issue from the cache, None in another shape.
fn issue_from_cache(value: &Value) -> Option<Issue> {
    let list = |key: &str| value.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    Some(Issue {
        iid: int_of(value, "iid")?,
        title: str_of(value, "title").unwrap_or_default().to_string(),
        url: str_of(value, "url").map(String::from),
        state: str_of(value, "state")?.to_string(),
        boxes: list("boxes")
            .iter()
            .map(|found| IssueBox {
                line: found.get("line").and_then(Value::as_u64).unwrap_or(0) as usize,
                is_ticked: found.get("isTicked").and_then(Value::as_bool).unwrap_or(false),
                text: str_of(found, "text").unwrap_or_default().to_string(),
                heading: str_of(found, "heading").map(String::from),
            })
            .collect(),
        needs: list("needs").iter().filter_map(Value::as_i64).collect(),
        paths: list("paths")
            .iter()
            .filter_map(|path| path.as_str().map(String::from))
            .collect(),
    })
}

/// The jobs of a finished pipeline the Node overview cached (gitlab.mjs `jobsOf`).
fn jobs_from_cache(cache: &Cache, pipeline: &Pipeline) -> Option<Vec<Job>> {
    let key = format!("{}@{}", pipeline.id, pipeline.updated_at.as_deref().unwrap_or_default());
    let jobs = cache.get("jobs", &key)?.as_array()?;
    Some(
        jobs.iter()
            .map(|job| Job {
                name: str_of(job, "name").unwrap_or_default().to_string(),
                status: str_of(job, "status").unwrap_or_default().to_string(),
                url: str_of(job, "url").map(String::from),
                finished_at: str_of(job, "finishedAt").map(String::from),
            })
            .collect(),
    )
}

fn job_of(job: &Value) -> Job {
    Job {
        name: str_of(job, "name").unwrap_or_default().to_string(),
        status: str_of(job, "status").unwrap_or_default().to_string(),
        url: str_of(job, "web_url").map(String::from),
        finished_at: str_of(job, "finished_at").map(String::from),
    }
}

fn is_final(state: &str) -> bool {
    state == "merged" || state == "closed"
}

/// Reads GitLab for `branches` and the MRs `awaited` names: `Err` the reason
/// when its lists can't be read.
///
/// What can't change any more comes from the disk cache and goes into it
/// (cache.rs): a closed issue, a merged or closed MR of a branch, an awaited
/// MR's final state. The rest in at most three rounds of calls side by side,
/// four calls or five whatever the number of branches: (1) the open MRs, the
/// last 100 pipelines (an MR's head pipeline by its commit), the issues not
/// known as closed; (2) the MRs updated lately, only for a branch neither
/// open nor known as final, the links of the open issues in one GraphQL
/// query, the jobs of a failed open pipeline; (3) single MRs, only for what
/// the lists didn't hold.
pub fn read(main: &Path, branches: &[String], awaited: &[i64]) -> Result<Remote, String> {
    let mut cache = Cache::open(main);
    let mut numbers: Vec<i64> = branches
        .iter()
        .filter_map(|branch| issue_of_branch(branch).map(|number| number as i64))
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    let mut issue_by: HashMap<i64, Issue> = numbers
        .iter()
        .filter_map(|number| Some((*number, issue_from_cache(cache.get("issues", &number.to_string())?)?)))
        .collect();
    let final_of: HashMap<&String, Mr> = branches
        .iter()
        .filter_map(|branch| {
            let iid = cache.get("branches", branch)?.as_i64()?;
            Some((branch, mr_from_cache(cache.get("mrs", &iid.to_string())?, branch)?))
        })
        .collect();

    // (1) What moves.
    let asked: Vec<i64> = numbers
        .iter()
        .copied()
        .filter(|number| !issue_by.contains_key(number))
        .collect();
    let mut endpoints = vec![
        "projects/:fullpath/merge_requests?state=opened&per_page=100".to_string(),
        "projects/:fullpath/pipelines?per_page=100".to_string(),
    ];
    for chunk in asked.chunks(100) {
        let iids: Vec<String> = chunk.iter().map(|number| format!("iids[]={number}")).collect();
        endpoints.push(format!(
            "projects/:fullpath/issues?state=all&per_page=100&{}",
            iids.join("&")
        ));
    }
    let mut answers = lists(main, &endpoints)?.into_iter();
    let opened = answers.next().unwrap_or_default();
    let pipelines = answers.next().unwrap_or_default();
    let issues: Vec<Value> = answers.flatten().collect();

    // Newest first: the first of each is the one that counts.
    let mut pipeline_of: HashMap<String, Pipeline> = HashMap::new();
    for pipeline in pipelines.iter().filter_map(Pipeline::from_value) {
        if let Some(sha) = pipeline.sha.clone() {
            pipeline_of.entry(sha).or_insert(pipeline);
        }
    }
    let with_listed_pipeline = |mut mr: Mr| {
        // The head pipeline by the head commit, as the list of pipelines has it.
        if let Some(listed) = mr.sha.as_ref().and_then(|sha| pipeline_of.get(sha)) {
            mr.pipeline = Some(listed.clone());
        }
        mr
    };
    let mut mr_of: HashMap<String, Mr> = HashMap::new();
    let mut state_of: HashMap<i64, String> = HashMap::new();
    for mr in opened.iter().filter_map(Mr::from_value) {
        state_of.entry(mr.iid).or_insert_with(|| mr.state.clone());
        if !mr_of.contains_key(&mr.source_branch) {
            mr_of.insert(mr.source_branch.clone(), with_listed_pipeline(mr));
        }
    }
    for (branch, mr) in &final_of {
        state_of.entry(mr.iid).or_insert_with(|| mr.state.clone());
        if !mr_of.contains_key(*branch) {
            mr_of.insert((*branch).clone(), mr.clone());
        }
    }
    for iid in awaited {
        if let Some(state) = cache.get("states", &iid.to_string()).and_then(Value::as_str) {
            state_of.entry(*iid).or_insert_with(|| state.to_string());
        }
    }

    // (2) What the first round didn't settle.
    let is_unsettled = branches.iter().any(|branch| !mr_of.contains_key(branch));
    let open_issues: Vec<i64> = issues
        .iter()
        .filter(|issue| str_of(issue, "state") == Some("opened"))
        .filter_map(|issue| int_of(issue, "iid"))
        .collect();
    let mut jobs: HashMap<i64, Vec<Job>> = HashMap::new();
    let mut failed: Vec<i64> = Vec::new();
    for mr in mr_of
        .values()
        .filter(|mr| mr.is_open() && mr.status() == Some("failed"))
    {
        let Some(pipeline) = &mr.pipeline else { continue };
        match jobs_from_cache(&cache, pipeline) {
            Some(cached) => {
                jobs.insert(pipeline.id, cached);
            }
            None if !failed.contains(&pipeline.id) => failed.push(pipeline.id),
            None => {}
        }
    }
    let job_endpoints: Vec<String> = failed
        .iter()
        .map(|id| format!("projects/:fullpath/pipelines/{id}/jobs?per_page=100&include_retried=false"))
        .collect();
    let (recent, links, job_lists) = thread::scope(|scope| {
        let recent = scope.spawn(|| {
            is_unsettled.then(|| {
                gitlab::list(
                    main,
                    "projects/:fullpath/merge_requests?state=all&per_page=100&order_by=updated_at",
                )
            })
        });
        let links = scope.spawn(|| links_of_issues(main, &open_issues));
        let job_lists = gitlab::lists(main, &job_endpoints);
        (
            recent.join().unwrap_or(None),
            links.join().unwrap_or_default(),
            job_lists,
        )
    });
    let recent = recent.transpose()?.unwrap_or_default();
    let (links, links_warning) = links;
    for (id, listed) in failed.iter().zip(job_lists) {
        let mut listed = listed.unwrap_or_default();
        listed.sort_by_key(|job| int_of(job, "id").unwrap_or(0));
        jobs.insert(*id, listed.iter().map(job_of).collect());
    }
    // Newly found: what may be final, written to the cache once its pipeline is known.
    let mut found_final: Vec<String> = Vec::new();
    for mr in recent.iter().filter_map(Mr::from_value) {
        state_of.entry(mr.iid).or_insert_with(|| mr.state.clone());
        if branches.contains(&mr.source_branch) && !mr_of.contains_key(&mr.source_branch) {
            if is_final(&mr.state) {
                found_final.push(mr.source_branch.clone());
            }
            mr_of.insert(mr.source_branch.clone(), with_listed_pipeline(mr));
        }
    }

    // (3) Single MRs: a pipeline the last 100 don't hold, the state of an
    // awaited MR none of the lists has.
    let mut singles: Vec<i64> = mr_of
        .iter()
        .filter(|(branch, mr)| mr.pipeline.is_none() && !final_of.contains_key(branch))
        .map(|(_, mr)| mr.iid)
        .collect();
    singles.extend(awaited.iter().filter(|iid| !state_of.contains_key(iid)));
    singles.sort_unstable();
    singles.dedup();
    let endpoints: Vec<String> = singles
        .iter()
        .map(|iid| format!("projects/:fullpath/merge_requests/{iid}"))
        .collect();
    let mut is_read: HashSet<i64> = HashSet::new();
    for single in gitlab::gets(main, &endpoints).into_iter().flatten() {
        let Some(read) = Mr::from_value(&single) else { continue };
        is_read.insert(read.iid);
        state_of.insert(read.iid, read.state.clone());
        if let Some(mr) = mr_of.values_mut().find(|mr| mr.iid == read.iid)
            && mr.pipeline.is_none()
        {
            mr.pipeline = read.pipeline;
        }
    }

    // What became final this read goes into the cache.
    for branch in &found_final {
        if let Some(mr) = mr_of.get(branch)
            && (mr.pipeline.is_some() || is_read.contains(&mr.iid))
        {
            cache.set("mrs", &mr.iid.to_string(), mr_to_cache(mr));
            cache.set("branches", branch, json!(mr.iid));
        }
    }
    for iid in awaited {
        if let Some(state) = state_of.get(iid).filter(|state| is_final(state)) {
            cache.set("states", &iid.to_string(), json!(state));
        }
    }
    for value in &issues {
        let Some(iid) = int_of(value, "iid") else { continue };
        let Some(issue) = issue_of(value, links.get(&iid).map(Vec::as_slice).unwrap_or_default()) else {
            continue;
        };
        if issue.state == "closed" {
            cache.set("issues", &iid.to_string(), issue_to_cache(&issue));
        }
        issue_by.insert(iid, issue);
    }
    cache.save();

    let mut of = HashMap::new();
    for branch in branches {
        let mr = mr_of.get(branch).cloned();
        let jobs = mr
            .as_ref()
            .filter(|mr| mr.is_open())
            .and_then(|mr| mr.pipeline.as_ref())
            .and_then(|pipeline| jobs.get(&pipeline.id).cloned())
            .unwrap_or_default();
        let issue = issue_of_branch(branch).and_then(|number| issue_by.get(&(number as i64)).cloned());
        of.insert(branch.clone(), Found { mr, jobs, issue });
    }
    let awaited = awaited
        .iter()
        .filter_map(|iid| state_of.get(iid).map(|state| (*iid, state.clone())))
        .collect();
    Ok(Remote {
        of,
        awaited,
        at: now_ms(),
        warnings: links_warning.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_prerequisites_an_issue_names() {
        assert_eq!(
            needs_of("Builds on #180 and #181, after #12 / #13."),
            vec![180, 181, 12, 13]
        );
        assert_eq!(needs_of("needs #5, needs #5"), vec![5]);
        assert_eq!(needs_of("no #4 here"), Vec::<i64>::new());
    }

    #[test]
    fn finds_the_files_an_issue_names() {
        assert_eq!(
            paths_of("see src/overview/tree.mjs and tree.mjs, README.md, not v1.2 or a.rs"),
            vec!["src/overview/tree.mjs", "tree.mjs", "README.md"]
        );
    }

    #[test]
    fn reads_an_mr_as_the_view_uses_it() {
        let mr = Mr::from_value(&json!({
            "iid": 3, "title": "Draft: feat: x", "draft": true, "state": "opened",
            "detailed_merge_status": "draft_status", "merge_status": "checking",
            "head_pipeline": {"id": 9, "status": "success", "sha": "abc"}
        }))
        .unwrap();
        assert_eq!(mr.title, "feat: x");
        assert!(mr.is_draft && mr.is_open());
        assert_eq!(mr.status(), Some("success"));
        assert_eq!(mr.merge_check.as_deref(), Some("checking"));
        assert_eq!(Mr::from_value(&json!({"title": "no iid"})), None);
    }

    #[test]
    fn an_issue_needs_what_it_names_and_what_blocks_it() {
        let issue = issue_of(
            &json!({"iid": 7, "state": "opened", "description": "after #3\n## Work\n- [x] a\n- [ ] b"}),
            &[
                json!({"link_type": "is_blocked_by", "iid": 4}),
                json!({"link_type": "relates_to", "iid": 5}),
            ],
        )
        .unwrap();
        assert_eq!(issue.needs, vec![3, 4]);
        assert_eq!(issue.boxes.len(), 2);
        assert_eq!(issue.boxes[1].heading.as_deref(), Some("Work"));
    }

    #[test]
    fn asks_the_links_of_every_open_issue_at_once() {
        assert_eq!(
            links_query(&[302, 304]),
            r#"issues(iids: ["302", "304"], state: all, first: 100) { nodes { iid blockedByIssues { nodes { iid } } } }"#
        );
        let links = links_of(&json!({ "issues": { "nodes": [
            { "iid": "302", "blockedByIssues": { "nodes": [{ "iid": "300" }] } },
            { "iid": "304", "blockedByIssues": { "nodes": [] } },
        ] } }));
        assert_eq!(links[&302], vec![json!({ "link_type": "is_blocked_by", "iid": 300 })]);
        assert!(links[&304].is_empty());
        let issue = issue_of(&json!({ "iid": 302, "state": "opened" }), &links[&302]).unwrap();
        assert_eq!(issue.needs, vec![300]);
    }

    #[test]
    fn a_final_mr_comes_back_from_the_cache_as_it_went_in() {
        let mut mr = Mr::from_value(&json!({
            "iid": 249, "title": "feat: x", "state": "merged", "merged_at": "2026-09-30T15:27:33Z",
            "detailed_merge_status": "not_open", "merge_status": "can_be_merged", "sha": "abc",
            "web_url": "https://x/249", "source_branch": "feature/234-x", "merge_commit_sha": "def",
            "head_pipeline": { "id": 9, "status": "success", "sha": "abc", "updated_at": "2026-09-30T15:00:00Z" }
        }))
        .unwrap();
        let cached = mr_to_cache(&mr);
        // Node's shape (read.mjs `mrOf`), which the Node overview reads.
        assert_eq!(cached["mergeSha"], json!("def"));
        assert_eq!(cached["mr"]["mergeStatus"], json!("not_open"));
        assert_eq!(cached["mr"]["pipeline"]["web_url"], Value::Null);
        assert_eq!(cached["mr"]["pipeline"]["status"], json!("success"));
        mr.deploy = None;
        assert_eq!(mr_from_cache(&cached, "feature/234-x"), Some(mr));
        assert_eq!(mr_from_cache(&json!({ "iid": 1 }), "x"), None);
    }

    #[test]
    fn a_closed_issue_comes_back_from_the_cache_as_it_went_in() {
        let issue = issue_of(
            &json!({ "iid": 234, "title": "t", "web_url": "https://x/234", "state": "closed",
                     "description": "after #3, see tree.mjs\n## Done\n- [x] a\n- [ ] b" }),
            &[],
        )
        .unwrap();
        let cached = issue_to_cache(&issue);
        assert_eq!(
            cached["boxes"][0],
            json!({ "line": 2, "isTicked": true, "text": "a", "heading": "Done" })
        );
        assert_eq!(issue_from_cache(&cached), Some(issue));
    }
}
