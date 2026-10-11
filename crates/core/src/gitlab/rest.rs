//! An orchestration's read where GitLab answers REST only (an older GitLab,
//! a proxy that passes `/api/v4` and not `/api/graphql`): Node's route
//! (gitlab.mjs `readRemoteOfOrchestration`), so the view and its merge order
//! stay whole. Three lists (the open MRs, the last 100 pipelines, the issues
//! not known as closed), the MRs updated lately only for a branch neither
//! open nor known as final, the links of each open issue, the jobs of a
//! failed pipeline; final things from the cache and into it, as the GraphQL
//! read does. A merged MR's deployment is GraphQL's only: left out here.

use super::cache::Cache;
use super::client::{Gitlab, Result};
use super::read::{Ask, DRAFT, FINISHED, Final, Memory, id_of, lower, settle_jobs, str_at};
use super::{BranchRemote, DevelopPipeline, Issue, Job, Mr, Pipeline, Remote, number_of};
use crate::time::now_ms;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// iids per batched list: GitLab's page.
const BATCH: usize = 100;

fn list(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

fn pipeline_of(value: &Value) -> Option<Pipeline> {
    value
        .is_object()
        .then(|| serde_json::from_value(value.clone()).ok())
        .flatten()
}

/// An MR of REST as the views use it, with the commit a merged one put on develop.
fn mr_of(value: &Value) -> (Mr, Option<String>) {
    let title = str_at(value, "title").unwrap_or_default();
    let mr = Mr {
        iid: id_of(value.get("iid")),
        title: DRAFT.replace(title, "").into_owned(),
        is_draft: value.get("draft").and_then(Value::as_bool).unwrap_or(false),
        state: str_at(value, "state").unwrap_or_default().to_string(),
        merged_at: str_at(value, "merged_at").map(String::from),
        has_conflicts: value.get("has_conflicts").and_then(Value::as_bool).unwrap_or(false),
        merge_status: str_at(value, "detailed_merge_status").map(str::to_lowercase),
        merge_check: str_at(value, "merge_status").map(str::to_lowercase),
        sha: str_at(value, "sha").map(String::from),
        url: str_at(value, "web_url").unwrap_or_default().to_string(),
        pipeline: value.get("head_pipeline").and_then(pipeline_of),
        deploy: None,
        branch: str_at(value, "source_branch").unwrap_or_default().to_string(),
        target: str_at(value, "target_branch").unwrap_or_default().to_string(),
    };
    (mr, merge_sha_of(value))
}

/// The commit a merged MR of REST put on develop: its merge commit, else its squash commit.
pub(super) fn merge_sha_of(value: &Value) -> Option<String> {
    str_at(value, "merge_commit_sha")
        .or_else(|| str_at(value, "squash_commit_sha"))
        .map(String::from)
}

fn jobs_of(listed: Vec<Value>) -> Vec<Job> {
    let mut jobs: Vec<Job> = listed
        .iter()
        .map(|job| Job {
            id: id_of(job.get("id")),
            name: str_at(job, "name").unwrap_or_default().to_string(),
            status: lower(str_at(job, "status")),
            url: str_at(job, "web_url").map(String::from),
            finished_at: str_at(job, "finished_at").map(String::from),
            log: None,
        })
        .collect();
    jobs.sort_by_key(|job| job.id);
    jobs
}

async fn jobs_of_pipeline(gitlab: &Gitlab, id: i64) -> Vec<Job> {
    let endpoint = format!("pipelines/{id}/jobs?per_page=100");
    jobs_of(gitlab.get_json(&endpoint).await.map(list).unwrap_or_default())
}

/// The issues that block `iid`, as REST links them; None when GitLab didn't say.
async fn blocked_by(gitlab: &Gitlab, iid: i64) -> Option<Vec<i64>> {
    let links = gitlab.get_json(&format!("issues/{iid}/links")).await.ok()?;
    Some(
        list(links)
            .iter()
            .filter(|link| str_at(link, "link_type") == Some("is_blocked_by"))
            .map(|link| id_of(link.get("iid")))
            .collect(),
    )
}

/// What `read::read_orchestration` reads, through REST.
pub async fn read_orchestration(
    gitlab: &Gitlab,
    cache: &mut Cache,
    memory: &mut Memory,
    ask: &Ask,
    is_full: bool,
) -> Result<Remote> {
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
    let numbers: Vec<i64> = {
        let mut seen = BTreeSet::new();
        branches
            .iter()
            .filter_map(|branch| number_of(branch))
            .filter(|number| seen.insert(*number))
            .collect()
    };
    let open_numbers: Vec<i64> = numbers
        .iter()
        .copied()
        .filter(|number| cache.get::<Issue>("issues", &number.to_string()).is_none())
        .collect();

    // What moves: the open MRs, the last pipelines (an MR's head pipeline by its commit), the issues.
    let opened = list(gitlab.get_json("merge_requests?state=opened&per_page=100").await?);
    let pipelines: Vec<Pipeline> = list(gitlab.get_json("pipelines?per_page=100").await?)
        .iter()
        .filter_map(pipeline_of)
        .collect();
    let mut issue_values = Vec::new();
    for batch in open_numbers.chunks(BATCH) {
        let iids: Vec<String> = batch.iter().map(|iid| format!("iids[]={iid}")).collect();
        let endpoint = format!("issues?state=all&per_page=100&{}", iids.join("&"));
        issue_values.extend(list(gitlab.get_json(&endpoint).await?));
    }
    // Newest first: the first pipeline of a commit is the one that counts.
    let mut pipeline_by_sha: HashMap<&str, &Pipeline> = HashMap::new();
    for pipeline in &pipelines {
        if let Some(sha) = pipeline.sha.as_deref() {
            pipeline_by_sha.entry(sha).or_insert(pipeline);
        }
    }
    let with_pipeline = |(mut mr, merge_sha): (Mr, Option<String>)| {
        if mr.pipeline.is_none()
            && let Some(listed) = mr.sha.as_deref().and_then(|sha| pipeline_by_sha.get(sha))
        {
            mr.pipeline = Some((*listed).clone());
        }
        (mr, merge_sha)
    };
    let mut found: HashMap<String, (Mr, Option<String>)> = HashMap::new();
    for (mr, merge_sha) in opened.iter().map(mr_of).map(with_pipeline) {
        found.entry(mr.branch.clone()).or_insert((mr, merge_sha));
    }
    // A branch neither open nor known as final: the MRs updated lately, once.
    let is_unsettled = branches
        .iter()
        .any(|branch| !found.contains_key(branch) && final_of(cache, branch).is_none());
    let mut states: HashMap<i64, String> = found.values().map(|(mr, _)| (mr.iid, mr.state.clone())).collect();
    if is_unsettled {
        let recent = list(
            gitlab
                .get_json("merge_requests?state=all&per_page=100&order_by=updated_at")
                .await?,
        );
        for (mr, merge_sha) in recent.iter().map(mr_of).map(with_pipeline) {
            states.entry(mr.iid).or_insert_with(|| mr.state.clone());
            found.entry(mr.branch.clone()).or_insert((mr, merge_sha));
        }
    }

    let mut of: HashMap<String, BranchRemote> = HashMap::new();
    gitlab.phase(Some("jobs"));
    for branch in &branches {
        let (mr, jobs) = if let Some(known) = final_of(cache, branch) {
            (Some(known.mr), Vec::new())
        } else if let Some((mr, merge_sha)) = found.get(branch).cloned() {
            if mr.state == "merged" || mr.state == "closed" {
                cache.set(
                    "mrs",
                    &mr.iid.to_string(),
                    &Final {
                        mr: mr.clone(),
                        merge_sha,
                    },
                );
                cache.set("branches", branch, &mr.iid);
                (Some(mr), Vec::new())
            } else {
                let jobs = match mr.pipeline.as_ref().filter(|pipeline| pipeline.status == "failed") {
                    Some(pipeline) => {
                        let read = jobs_of_pipeline(gitlab, pipeline.id).await;
                        settle_jobs(gitlab, cache, memory, pipeline, read).await
                    }
                    None => Vec::new(),
                };
                (Some(mr), jobs)
            }
        } else {
            (None, Vec::new())
        };
        of.insert(branch.clone(), BranchRemote { mr, jobs, issue: None });
    }

    // The issues: closed ones known, the open ones' links on a full read.
    gitlab.phase(Some("issues"));
    let mut issues: HashMap<i64, Issue> = HashMap::new();
    for number in &numbers {
        if let Some(issue) = cache.get::<Issue>("issues", &number.to_string()) {
            issues.insert(*number, issue);
        }
    }
    for value in &issue_values {
        let iid = id_of(value.get("iid"));
        let text = |key: &str| str_at(value, key).unwrap_or_default();
        if text("state") == "closed" {
            let done = Issue::of(iid, text("title"), text("web_url"), "closed", text("description"), &[]);
            cache.set("issues", &iid.to_string(), &done);
            issues.insert(iid, done);
            continue;
        }
        if (is_full || !memory.links.contains_key(&iid))
            && let Some(links) = blocked_by(gitlab, iid).await
        {
            memory.links.insert(iid, links);
        }
        let links = memory.links.get(&iid).cloned().unwrap_or_default();
        let issue = Issue::of(
            iid,
            text("title"),
            text("web_url"),
            text("state"),
            text("description"),
            &links,
        );
        issues.insert(iid, issue);
    }
    for (branch, remote) in of.iter_mut() {
        remote.issue = number_of(branch).and_then(|number| issues.get(&number).cloned());
    }

    // develop's last finished pipeline, its jobs only when it failed.
    let develop = match pipelines.iter().find(|pipeline| {
        pipeline.reference.as_deref() == Some("develop") && FINISHED.contains(&pipeline.status.as_str())
    }) {
        Some(pipeline) => Some(DevelopPipeline {
            id: pipeline.id,
            status: pipeline.status.clone(),
            url: pipeline.web_url.clone(),
            jobs: if pipeline.status == "failed" {
                jobs_of_pipeline(gitlab, pipeline.id).await
            } else {
                Vec::new()
            },
        }),
        None => None,
    };

    // The awaited MRs: from what was read, the cache, else one by one.
    let mut awaited = HashMap::new();
    for iid in &ask.awaited {
        let mut state = states
            .get(iid)
            .cloned()
            .or_else(|| cache.get::<String>("states", &iid.to_string()));
        if state.is_none()
            && let Ok(single) = gitlab.get_json(&format!("merge_requests/{iid}")).await
        {
            state = str_at(&single, "state").map(String::from);
        }
        if let Some(state) = state {
            if state == "merged" || state == "closed" {
                cache.set("states", &iid.to_string(), &state);
            }
            awaited.insert(*iid, state);
        }
    }
    gitlab.phase(None);
    cache.save();
    Ok(Remote {
        of,
        awaited,
        develop,
        at: now_ms(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_an_mr_as_rest_lists_it() {
        let (mr, merge_sha) = mr_of(&json!({
            "iid": 311, "title": "Draft: feat(x): y", "draft": true, "state": "merged",
            "web_url": "https://gitlab.com/g/p/-/merge_requests/311", "source_branch": "feature/302-x",
            "merged_at": "2026-10-01T12:00:00Z", "detailed_merge_status": "MERGEABLE",
            "merge_status": "can_be_merged", "sha": "abc", "squash_commit_sha": "def",
        }));
        assert_eq!((mr.iid, mr.title.as_str(), mr.is_draft), (311, "feat(x): y", true));
        assert_eq!(mr.branch, "feature/302-x");
        assert_eq!(mr.merge_status.as_deref(), Some("mergeable"));
        assert_eq!(merge_sha.as_deref(), Some("def"));
        assert!(mr.pipeline.is_none());
    }

    #[test]
    fn keeps_the_jobs_in_the_order_they_ran() {
        let jobs = jobs_of(vec![
            json!({ "id": 9, "name": "b", "status": "failed", "web_url": "u9" }),
            json!({ "id": 3, "name": "a", "status": "success" }),
        ]);
        let names: Vec<&str> = jobs.iter().map(|job| job.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(jobs[1].url.as_deref(), Some("u9"));
    }
}
