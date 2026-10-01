//! An orchestration's part of GitLab (gitlab.mjs `readRemoteOfOrchestration`):
//! the MR of each of its branches with its pipeline, the issue of each with
//! its boxes and prerequisites, the jobs of a failed pipeline, and the state
//! of every MR a wait or a handover names. In lists, side by side: open MRs,
//! the MRs updated lately, the last pipelines, the issues by iid; then the
//! links of the open issues, the jobs of failed pipelines and what the lists
//! didn't have. Read only, nothing cached on disk (the cache comes with
//! core's GitLab client).

use crate::cli::read::gitlab;
use crate::cli::read::issues::{Box as IssueBox, boxes_with_headings};
use breadboard_core::git::issue_of_branch;
use breadboard_core::json::{int_of, str_of};
use breadboard_core::time::now_ms;
use serde_json::Value;
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
}

impl Pipeline {
    fn from_value(value: &Value) -> Option<Pipeline> {
        Some(Pipeline {
            id: int_of(value, "id").unwrap_or(0),
            status: str_of(value, "status")?.to_string(),
            url: str_of(value, "web_url").map(String::from),
            sha: str_of(value, "sha").map(String::from),
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
    })
}

/// A read's lists side by side; Err the first reason one failed.
fn lists(main: &Path, endpoints: &[String]) -> Result<Vec<Vec<Value>>, String> {
    gitlab::lists(main, endpoints).into_iter().collect()
}

/// Reads GitLab for `branches` and the MRs `awaited` names: `Err` the reason
/// when its lists can't be read.
pub fn read(main: &Path, branches: &[String], awaited: &[i64]) -> Result<Remote, String> {
    let mut numbers: Vec<i64> = branches
        .iter()
        .filter_map(|branch| issue_of_branch(branch).map(|number| number as i64))
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    let mut endpoints = vec![
        "projects/:fullpath/merge_requests?state=opened&per_page=100".to_string(),
        "projects/:fullpath/merge_requests?state=all&per_page=100&order_by=updated_at".to_string(),
        "projects/:fullpath/pipelines?per_page=100".to_string(),
    ];
    for chunk in numbers.chunks(100) {
        let iids: Vec<String> = chunk.iter().map(|number| format!("iids[]={number}")).collect();
        endpoints.push(format!(
            "projects/:fullpath/issues?state=all&per_page=100&{}",
            iids.join("&")
        ));
    }
    let mut answers = lists(main, &endpoints)?.into_iter();
    let opened = answers.next().unwrap_or_default();
    let recent = answers.next().unwrap_or_default();
    let pipelines = answers.next().unwrap_or_default();
    let issues: Vec<Value> = answers.flatten().collect();

    // Newest first: the first of each is the one that counts.
    let mut pipeline_of: HashMap<String, Pipeline> = HashMap::new();
    for pipeline in pipelines.iter().filter_map(Pipeline::from_value) {
        if let Some(sha) = pipeline.sha.clone() {
            pipeline_of.entry(sha).or_insert(pipeline);
        }
    }
    let mut mr_of: HashMap<String, Mr> = HashMap::new();
    let mut state_of: HashMap<i64, String> = HashMap::new();
    for (list, is_open) in [(&opened, true), (&recent, false)] {
        for mr in list.iter().filter_map(Mr::from_value) {
            state_of.entry(mr.iid).or_insert_with(|| mr.state.clone());
            // An open one before any other of its branch; else the newest.
            let is_better = match mr_of.get(&mr.source_branch) {
                None => true,
                Some(known) => is_open && !known.is_open(),
            };
            if is_better {
                mr_of.insert(mr.source_branch.clone(), mr);
            }
        }
    }
    let mut found_mrs: Vec<Mr> = branches
        .iter()
        .filter_map(|branch| mr_of.get(branch).cloned())
        .map(|mut mr| {
            // The head pipeline by the head commit, as the list of pipelines has it.
            if let Some(listed) = mr.sha.as_ref().and_then(|sha| pipeline_of.get(sha)) {
                mr.pipeline = Some(listed.clone());
            }
            mr
        })
        .collect();

    // What the lists didn't answer: the links of the open issues, the jobs of
    // a failed pipeline, an MR's pipeline the last 100 don't hold, the state
    // of an awaited MR none of the lists has.
    let open_issues: Vec<i64> = issues
        .iter()
        .filter(|issue| str_of(issue, "state") == Some("opened"))
        .filter_map(|issue| int_of(issue, "iid"))
        .collect();
    let failed: Vec<i64> = found_mrs
        .iter()
        .filter(|mr| mr.is_open() && mr.status() == Some("failed"))
        .filter_map(|mr| mr.pipeline.as_ref().map(|pipeline| pipeline.id))
        .collect();
    let missing: Vec<i64> = found_mrs
        .iter()
        .filter(|mr| mr.pipeline.is_none())
        .map(|mr| mr.iid)
        .collect();
    let unknown: Vec<i64> = awaited
        .iter()
        .copied()
        .filter(|iid| !state_of.contains_key(iid))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut more: Vec<String> = Vec::new();
    more.extend(
        open_issues
            .iter()
            .map(|iid| format!("projects/:fullpath/issues/{iid}/links")),
    );
    more.extend(
        failed
            .iter()
            .map(|id| format!("projects/:fullpath/pipelines/{id}/jobs?per_page=100&include_retried=false")),
    );
    let singles: Vec<i64> = missing.iter().chain(&unknown).copied().collect();
    let answers = thread::scope(|scope| {
        let singles = scope.spawn(|| {
            singles
                .chunks(10)
                .flat_map(|chunk| {
                    thread::scope(|inner| {
                        let handles: Vec<_> = chunk
                            .iter()
                            .map(|iid| {
                                inner.spawn(move || {
                                    gitlab::get(main, &format!("projects/:fullpath/merge_requests/{iid}")).ok()
                                })
                            })
                            .collect();
                        handles
                            .into_iter()
                            .map(|handle| handle.join().ok().flatten())
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>()
        });
        let lists = gitlab::lists(main, &more);
        (lists, singles.join().unwrap_or_default())
    });
    let (more_lists, single_answers) = answers;
    let mut more_lists = more_lists.into_iter();
    let links: HashMap<i64, Vec<Value>> = open_issues
        .iter()
        .map(|iid| (*iid, more_lists.next().and_then(Result::ok).unwrap_or_default()))
        .collect();
    let jobs: HashMap<i64, Vec<Job>> = failed
        .iter()
        .map(|id| {
            let mut jobs = more_lists.next().and_then(Result::ok).unwrap_or_default();
            jobs.sort_by_key(|job| int_of(job, "id").unwrap_or(0));
            let jobs = jobs
                .iter()
                .map(|job| Job {
                    name: str_of(job, "name").unwrap_or_default().to_string(),
                    status: str_of(job, "status").unwrap_or_default().to_string(),
                    url: str_of(job, "web_url").map(String::from),
                })
                .collect();
            (*id, jobs)
        })
        .collect();
    for single in single_answers.iter().flatten() {
        let Some(read) = Mr::from_value(single) else { continue };
        state_of.insert(read.iid, read.state.clone());
        if let Some(mr) = found_mrs.iter_mut().find(|mr| mr.iid == read.iid)
            && mr.pipeline.is_none()
        {
            mr.pipeline = read.pipeline;
        }
    }

    let issue_by: HashMap<i64, Issue> = issues
        .iter()
        .filter_map(|issue| {
            let iid = int_of(issue, "iid")?;
            issue_of(issue, links.get(&iid).map(Vec::as_slice).unwrap_or_default()).map(|issue| (iid, issue))
        })
        .collect();
    let mut of = HashMap::new();
    for branch in branches {
        let mr = found_mrs.iter().find(|mr| &mr.source_branch == branch).cloned();
        let jobs = mr
            .as_ref()
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
}
