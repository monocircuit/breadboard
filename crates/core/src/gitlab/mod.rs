//! GitLab as the views read it (#305): an orchestration's MRs, pipelines,
//! jobs and issues in one GraphQL request where it can, REST where GraphQL
//! lacks something, what can't change any more cached on disk for every
//! overview of the checkout (cache.rs). The shapes are the Node overview's
//! (read.mjs `mrOf`, `issueOf`, gitlab.mjs `jobsOf`): the cache file is
//! shared with it while both run (rewrite rule 5).

pub mod cache;
pub mod client;
pub mod read;
pub mod recheck;
mod rest;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

/// A pipeline as GitLab's REST lists it, the fields the views read: the
/// head pipeline of an MR (`head_pipeline`) is cached in this shape.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Pipeline {
    pub id: i64,
    pub status: String,
    pub web_url: Option<String>,
    pub sha: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub created_at: Option<String>,
    /// When its first job started: none while it waits for the runner.
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    pub finished_at: Option<String>,
}

/// A job of a pipeline (gitlab.mjs `jobsOf`): a failed one with the last lines of its log.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Job {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub url: Option<String>,
    pub finished_at: Option<String>,
    pub log: Option<Vec<String>>,
}

/// The develop pipeline of a merge commit (try-state.mjs), `{ id, status, url, createdAt }`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DeployPipeline {
    pub id: i64,
    pub status: String,
    pub url: Option<String>,
    pub created_at: Option<String>,
}

/// Where the deployment of a merged MR stands: the develop pipeline of its
/// commit and its jobs; `pipeline` None while GitLab has none yet.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Deploy {
    pub pipeline: Option<DeployPipeline>,
    pub jobs: Vec<Job>,
}

/// An MR as the views use it (read.mjs `mrOf`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mr {
    pub iid: i64,
    /// Without `Draft:`.
    pub title: String,
    pub is_draft: bool,
    /// `opened`, `merged`, `closed`, `locked`.
    pub state: String,
    pub merged_at: Option<String>,
    pub has_conflicts: bool,
    /// GitLab's `detailed_merge_status`: only `mergeable` can be merged.
    pub merge_status: Option<String>,
    /// GitLab's `merge_status`: `checking` after a push (resolve.mjs).
    pub merge_check: Option<String>,
    /// The head GitLab has.
    pub sha: Option<String>,
    pub url: String,
    pub pipeline: Option<Pipeline>,
    /// Merged: the develop pipeline of the merge commit (try-state.mjs).
    pub deploy: Option<Deploy>,
    /// Its source branch; not in Node's shape, which ignores it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
    /// The branch it merges into (#349): develop, or an epic; empty when
    /// GitLab didn't say, which means develop.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// The boxes of its own description (its Tests), apart from its issue's (#346).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub boxes: Vec<IssueBox>,
}

impl Mr {
    /// The status of its pipeline, None without one.
    pub fn pipeline_status(&self) -> Option<&str> {
        self.pipeline
            .as_ref()
            .map(|pipeline| pipeline.status.as_str())
            .filter(|status| !status.is_empty())
    }
    pub fn is_open(&self) -> bool {
        self.state == "opened"
    }
    /// The branch it merges into: develop unless GitLab names another.
    pub fn target(&self) -> &str {
        if self.target.is_empty() {
            crate::git::default_branch()
        } else {
            &self.target
        }
    }
    pub fn is_merged(&self) -> bool {
        self.state == "merged"
    }
}

/// A checkbox of an issue's description (worktree/issues.mjs `boxesOf`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IssueBox {
    pub line: usize,
    pub is_ticked: bool,
    pub text: String,
    pub heading: Option<String>,
}

/// An issue as the views use it (read.mjs `issueOf`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Issue {
    pub iid: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub boxes: Vec<IssueBox>,
    /// The issues it names as prerequisites, or is blocked by.
    pub needs: Vec<i64>,
    /// The files it names: the agent that knows them is suggested for it.
    pub paths: Vec<String>,
    /// What is wrong or wanted, from its `## Goal` / `## Bug` (#341).
    pub problem: Option<String>,
}

impl Issue {
    /// An issue from its description and the issues GitLab says block it.
    pub fn of(iid: i64, title: &str, url: &str, state: &str, description: &str, blocked_by: &[i64]) -> Issue {
        let mut needs = needs_of(description);
        for iid in blocked_by {
            if !needs.contains(iid) {
                needs.push(*iid);
            }
        }
        Issue {
            iid,
            title: title.to_string(),
            url: url.to_string(),
            state: state.to_string(),
            boxes: boxes_of(description),
            needs,
            paths: paths_of(description),
            problem: crate::said::problem_of(description),
        }
    }
}

/// One branch's part of GitLab: its MR (open, else the latest), the jobs of
/// its pipeline, its issue.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BranchRemote {
    pub mr: Option<Mr>,
    pub jobs: Vec<Job>,
    pub issue: Option<Issue>,
}

/// develop's last finished pipeline; its jobs only when it failed (a job
/// failing there too is not an MR's doing).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DevelopPipeline {
    pub id: i64,
    pub status: String,
    pub url: Option<String>,
    pub jobs: Vec<Job>,
}

/// What a read of an orchestration found (gitlab.mjs `readRemoteOfOrchestration`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Remote {
    /// branch → its MR, jobs and issue.
    pub of: HashMap<String, BranchRemote>,
    /// iid → state of the MRs handovers, waits and queued issues wait for.
    pub awaited: HashMap<i64, String>,
    pub develop: Option<DevelopPipeline>,
    /// Its epic branch's last pipeline and open MR into develop (#350).
    #[serde(default)]
    pub epic: Option<Box<EpicRemote>>,
    /// When it was read, ms since the epoch.
    pub at: i64,
}

/// The GitLab side of an orchestration's epic branch (#350): its last
/// finished pipeline (jobs only when it failed) and its open MR into develop.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EpicRemote {
    pub branch: String,
    pub pipeline: Option<DevelopPipeline>,
    pub mr: Option<Mr>,
    /// The task MRs merged into it, newest first (#414): what a batch ships.
    pub tasks: Vec<BatchTask>,
    /// The open MRs into develop of the product's batches shipped before it (#414).
    pub shipping: Vec<Mr>,
}

/// A task MR merged into a batch (#414): the fold under BATCH, the ship suggestion.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchTask {
    pub iid: i64,
    /// Without `Draft:`.
    pub title: String,
    pub url: String,
    pub branch: String,
    pub merged_at: Option<String>,
    pub labels: Vec<String>,
}

impl Remote {
    /// The part of a branch, empty for one GitLab knows nothing of.
    pub fn of(&self, branch: &str) -> &BranchRemote {
        static NONE: LazyLock<BranchRemote> = LazyLock::new(BranchRemote::default);
        self.of.get(branch).unwrap_or(&NONE)
    }
}

// ---------------------------------------------------------------------------
// What a view says about its GitLab data (#305, gitlab-status.mjs)
// ---------------------------------------------------------------------------

/// In place of the data while the read runs: `GitLab: 4/7 · pipelines`.
pub fn waiting_text(progress: &client::Progress) -> String {
    if progress.asked == 0 {
        return "GitLab …".into();
    }
    let phase = progress
        .phase
        .as_deref()
        .map(|phase| format!(" · {phase}"))
        .unwrap_or_default();
    format!("GitLab: {}/{}{phase}", progress.answered, progress.asked)
}

/// The line of a read that failed: when, why, the age of the data shown in
/// its place; `, tried again` only where it is (a print asks once).
pub fn error_text(reason: &str, at: Option<i64>, data_at: Option<i64>, will_retry: bool) -> String {
    let when = at.map(|at| format!(" {}", crate::time::hm(at))).unwrap_or_default();
    let kept = data_at
        .map(|at| format!(", data from {}", crate::time::hm(at)))
        .unwrap_or_default();
    let again = if will_retry { ", tried again" } else { "" };
    format!(
        "GitLab didn't answer{when} ({}){kept}{again}",
        reason.lines().next().unwrap_or("")
    )
}

/// The age of the data and GitLab's minute, all clients of the user: `GitLab 14:57 · 122/2000 a minute`.
pub fn fresh_text(data_at: i64, rate: Option<client::Rate>) -> String {
    let rate = rate
        .map(|rate| format!(" · {}/{} a minute", rate.observed, rate.limit))
        .unwrap_or_default();
    format!("GitLab {}{rate}", crate::time::hm(data_at))
}

// ---------------------------------------------------------------------------
// What an issue's description says
// ---------------------------------------------------------------------------

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*```").unwrap());
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,6}\s+(.*)$").unwrap());
static BOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+\[([ xX])\]\s+(.*)$").unwrap());

/// The checkboxes of an issue description, in order: its line, whether it is
/// ticked, its text and the heading it stands under. A box in fenced code is
/// an example, not a task.
pub fn boxes_of(text: &str) -> Vec<IssueBox> {
    let mut boxes = Vec::new();
    let mut heading = None;
    let mut is_in_code = false;
    for (line, raw) in text.split('\n').enumerate() {
        if FENCE.is_match(raw) {
            is_in_code = !is_in_code;
            continue;
        }
        if is_in_code {
            continue;
        }
        if let Some(title) = HEADING.captures(raw) {
            heading = Some(title[1].trim().to_string());
        }
        if let Some(found) = BOX.captures(raw) {
            boxes.push(IssueBox {
                line,
                is_ticked: &found[1] != " ",
                text: found[2].trim().to_string(),
                heading: heading.clone(),
            });
        }
    }
    boxes
}

/// How an issue names a prerequisite: `after #180`, `needs #180 and #181`, `builds on #180`.
static PREREQUISITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:after|needs|builds on|depends on|requires|blocked by)\s+(#\d+(?:(?:\s*,\s*|\s+and\s+|\s*&\s*|\s*/\s*)#\d+)*)")
        .unwrap()
});
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#(\d+)").unwrap());

/// The issues a description names as prerequisites, each once (merge-order.mjs `needsOf`).
pub fn needs_of(text: &str) -> Vec<i64> {
    let mut needs = Vec::new();
    for found in PREREQUISITE.captures_iter(text) {
        for number in NUMBER.captures_iter(&found[1]) {
            if let Ok(number) = number[1].parse::<i64>()
                && !needs.contains(&number)
            {
                needs.push(number);
            }
        }
    }
    needs
}

/// A file path an issue names, with an extension of the repo's files (fit.mjs
/// `PATH`; Rust and shell since breadboard is, #381: an issue on them shares
/// files with the agent that knows them).
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:[A-Za-z0-9_@.-]+/)*[A-Za-z0-9_@-][A-Za-z0-9_@.-]*\.(?:mjs|cjs|js|jsx|ts|tsx|json|md|css|scss|sql|toml|ya?ml|rs|sh)\b").unwrap()
});

/// The file paths a description names, each once.
pub fn paths_of(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    PATH.find_iter(text)
        .map(|found| found.as_str().to_string())
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// The issue a branch claims: `feature/34-x` → 34.
pub fn number_of(branch: &str) -> Option<i64> {
    crate::git::issue_of_branch(branch)
        .map(|number| number as i64)
        .filter(|number| *number > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_rust_and_shell_files_too() {
        assert_eq!(
            paths_of("fix `crates/core/src/git.rs` and herdr/cargo.sh, not rust.rsx"),
            ["crates/core/src/git.rs", "herdr/cargo.sh"]
        );
    }

    #[test]
    fn reads_every_box_with_its_state_text_and_heading() {
        let text =
            "## Phase 1 — Move\n\n- [ ] move the scripts\n- [x] the paths\n\n## Acceptance criteria\n* [X] lint green";
        let boxes = boxes_of(text);
        assert_eq!(
            boxes,
            vec![
                IssueBox {
                    line: 2,
                    is_ticked: false,
                    text: "move the scripts".into(),
                    heading: Some("Phase 1 — Move".into())
                },
                IssueBox {
                    line: 3,
                    is_ticked: true,
                    text: "the paths".into(),
                    heading: Some("Phase 1 — Move".into())
                },
                IssueBox {
                    line: 6,
                    is_ticked: true,
                    text: "lint green".into(),
                    heading: Some("Acceptance criteria".into())
                },
            ]
        );
        assert_eq!(
            boxes_of("```md\n- [ ] an example\n```\n- [ ] real")
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>(),
            ["real"]
        );
        assert_eq!(boxes_of("- [ ] first")[0].heading, None);
    }

    #[test]
    fn finds_the_prerequisites_an_issue_names() {
        assert_eq!(
            needs_of("Builds on #180. Needs #181 and #183, after #186 / #187.\nRelated: #179"),
            vec![180, 181, 183, 186, 187]
        );
        assert!(needs_of("Related: #179, #182").is_empty());
        assert!(needs_of("").is_empty());
    }

    #[test]
    fn finds_the_files_an_issue_names() {
        assert_eq!(
            paths_of("see `src/overview/tree.mjs` and tree.mjs, also README.md and tree.mjs"),
            vec!["src/overview/tree.mjs", "tree.mjs", "README.md"]
        );
        assert!(paths_of("no files, v1.2 only").is_empty());
    }

    #[test]
    fn says_where_the_gitlab_data_stands() {
        let progress = client::Progress {
            asked: 7,
            answered: 4,
            phase: Some("pipelines".into()),
        };
        assert_eq!(waiting_text(&progress), "GitLab: 4/7 · pipelines");
        assert_eq!(waiting_text(&client::Progress::default()), "GitLab …");
        assert!(
            error_text("timeout\nmore", None, None, true).starts_with("GitLab didn't answer (timeout), tried again")
        );
        assert_eq!(
            error_text("timeout", None, None, false),
            "GitLab didn't answer (timeout)"
        );
        assert!(
            fresh_text(
                0,
                Some(client::Rate {
                    limit: 2000,
                    observed: 122,
                    remaining: 1878
                })
            )
            .ends_with(" · 122/2000 a minute")
        );
    }

    #[test]
    fn reads_the_shapes_node_caches() {
        let cached = r#"{"iid":249,"title":"t","isDraft":false,"state":"merged","mergedAt":"2026-09-30T10:00:00.000Z","hasConflicts":false,"mergeStatus":"not_open","mergeCheck":"can_be_merged","sha":"abc","url":"https://gitlab.com/x/-/merge_requests/249","pipeline":{"id":5,"iid":3,"status":"success","ref":"refs/merge-requests/249/head","web_url":"https://gitlab.com/x/-/pipelines/5"},"deploy":null}"#;
        let mr: Mr = serde_json::from_str(cached).unwrap();
        assert_eq!(
            mr.pipeline.as_ref().unwrap().reference.as_deref(),
            Some("refs/merge-requests/249/head")
        );
        assert_eq!(mr.pipeline_status(), Some("success"));
        assert!(mr.is_merged());
        let back = serde_json::to_value(&mr).unwrap();
        assert_eq!(back["isDraft"], false);
        assert_eq!(back["pipeline"]["web_url"], "https://gitlab.com/x/-/pipelines/5");
        assert!(back.get("branch").is_none());
        let issue: Issue = serde_json::from_str(
            r#"{"iid":234,"title":"t","url":"u","state":"closed","boxes":[{"line":2,"isTicked":true,"text":"a","heading":null}],"needs":[],"paths":[]}"#,
        )
        .unwrap();
        assert!(issue.boxes[0].is_ticked);
    }
}
