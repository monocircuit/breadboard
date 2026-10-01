//! The view of one worktree (--worktree, render-worktree.mjs) as data: WORK,
//! WAITING FOR, AGENT, ISSUE, PIPELINE, DIFF, DEVELOP, WHERE, STACK.

use super::develop::{Develop, Migrations};
use super::diff::Diff;
use super::files::{LogTarget, log_targets};
use super::waiting::Waiting;
use super::{Section, Tree, section};
use crate::conversations::Claude;
use crate::gitlab::read::WorktreeRemote;
use crate::gitlab::{Issue, Job};
use crate::local::Container;
use std::path::PathBuf;

/// drizzle studio's port, and Storybook's: a worktree's move by its offset.
pub const STUDIO_PORT: i64 = 4983;
pub const STORYBOOK_PORT: i64 = 6006;
/// How many open boxes ISSUE names, the rest counted.
const MAX_OPEN: usize = 10;

/// What the local read of one worktree found (view.mjs `readView` with --worktree).
#[derive(Debug, Clone, Default)]
pub struct WorktreeLocal {
    pub dir: PathBuf,
    /// None: it is no worktree of the repo any more.
    pub tree: Option<Tree>,
    pub is_main: bool,
    /// Nothing wt:remove would refuse: no changes, no commits develop doesn't have.
    pub is_done: bool,
    pub own_commits: usize,
    pub claude: Option<Claude>,
    /// herdr's state of its agent.
    pub agent_status: Option<String>,
    pub has_docker: bool,
    pub containers: Vec<Container>,
    pub diff: Option<Diff>,
    pub develop: Option<Develop>,
    pub migrations: Option<Migrations>,
    pub is_app_up: bool,
    pub is_studio_up: bool,
    pub is_storybook_up: bool,
}

/// A heading of the issue's boxes: how many are ticked, the open ones by text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxGroup {
    pub heading: Option<String>,
    pub done: usize,
    pub total: usize,
    pub open: Vec<String>,
}

/// ISSUE: its boxes under their headings, ten open ones named, the rest counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBoxes {
    pub iid: i64,
    pub url: String,
    pub groups: Vec<BoxGroup>,
    pub more_open: usize,
    pub done: usize,
    pub total: usize,
}

pub fn issue_boxes_of(issue: &Issue) -> Option<IssueBoxes> {
    if issue.boxes.is_empty() {
        return None;
    }
    let mut headings: Vec<Option<String>> = Vec::new();
    for found in &issue.boxes {
        if !headings.contains(&found.heading) {
            headings.push(found.heading.clone());
        }
    }
    let mut shown = 0;
    let mut more_open = 0;
    let groups = headings
        .into_iter()
        .map(|heading| {
            let boxes: Vec<_> = issue.boxes.iter().filter(|found| found.heading == heading).collect();
            let mut open = Vec::new();
            for found in boxes.iter().filter(|found| !found.is_ticked) {
                if shown < MAX_OPEN {
                    open.push(found.text.clone());
                } else {
                    more_open += 1;
                }
                shown += 1;
            }
            BoxGroup {
                done: boxes.iter().filter(|found| found.is_ticked).count(),
                total: boxes.len(),
                heading,
                open,
            }
        })
        .collect();
    Some(IssueBoxes {
        iid: issue.iid,
        url: issue.url.clone(),
        groups,
        more_open,
        done: issue.boxes.iter().filter(|found| found.is_ticked).count(),
        total: issue.boxes.len(),
    })
}

/// A failed job with its log, each line with the file a click opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedLog {
    pub job: Job,
    pub lines: Vec<(String, Option<LogTarget>)>,
}

/// PIPELINE: every job of the MR's pipeline whole (#304), the failed ones' logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineSection {
    pub jobs: Vec<Job>,
    pub logs: Vec<FailedLog>,
}

pub fn pipeline_of(remote: &WorktreeRemote) -> Option<PipelineSection> {
    remote.mr.as_ref()?.pipeline.as_ref()?;
    if remote.jobs.is_empty() {
        return None;
    }
    let logs = remote
        .jobs
        .iter()
        .filter_map(|job| {
            let log = job.log.as_ref()?;
            let targets = log_targets(log);
            Some(FailedLog {
                job: job.clone(),
                lines: log.iter().cloned().zip(targets).collect(),
            })
        })
        .collect();
    Some(PipelineSection {
        jobs: remote.jobs.clone(),
        logs,
    })
}

/// A row of WHERE: what runs where, a click opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhereRow {
    pub name: &'static str,
    pub is_on: bool,
    pub url: Option<String>,
    /// Not http: nothing to open.
    pub address: Option<String>,
    /// How to start it when it is down.
    pub hint: Option<String>,
}

/// WHERE of a worktree with an .env.local.
pub fn where_of(local: &WorktreeLocal, tree: &Tree) -> Vec<WhereRow> {
    let is_shared = !tree.has_own_port;
    let offset = if is_shared { 0 } else { tree.offset };
    let port = |variable: &str, base: i64| {
        tree.env
            .get(variable)
            .and_then(|port| port.trim().parse().ok())
            .unwrap_or(base + offset)
    };
    let is_up = tree.stack.as_deref().is_some_and(|stack| stack.contains("running"));
    let row = |name, is_on, url: Option<String>, hint: Option<String>| WhereRow {
        name,
        is_on,
        url,
        address: None,
        hint,
    };
    vec![
        row(
            "app",
            local.is_app_up,
            Some(format!("http://localhost:{}", 3000 + offset)),
            if local.is_app_up {
                is_shared.then(|| "shared: whichever checkout runs pnpm dev".into())
            } else {
                Some("cd products/vita/web && pnpm dev".into())
            },
        ),
        row(
            "keycloak",
            is_up,
            Some(format!("http://localhost:{}/admin", port("KEYCLOAK_PORT", 8080))),
            None,
        ),
        row(
            "s3",
            is_up,
            Some(format!("http://localhost:{}", port("S3_PORT", 8333))),
            None,
        ),
        WhereRow {
            name: "postgres",
            is_on: is_up,
            url: None,
            address: Some(format!("localhost:{}", port("POSTGRES_PORT", 5432))),
            hint: None,
        },
        row(
            "studio",
            local.is_studio_up,
            Some(format!("https://local.drizzle.studio?port={}", STUDIO_PORT + offset)),
            (!local.is_studio_up).then(|| format!("pnpm pg:studio --port {}", STUDIO_PORT + offset)),
        ),
        row(
            "storybook",
            local.is_storybook_up,
            Some(format!("http://localhost:{}", STORYBOOK_PORT + offset)),
            (!local.is_storybook_up).then(|| {
                format!(
                    "cd packages/monolithium && pnpm exec storybook dev -p {}",
                    STORYBOOK_PORT + offset
                )
            }),
        ),
    ]
}

/// The view of one worktree.
#[derive(Debug, Clone)]
pub struct WorktreeModel {
    pub local: WorktreeLocal,
    pub remote: Option<WorktreeRemote>,
    /// The open MRs it builds on, the base first (WAITING FOR).
    pub waiting: Vec<Waiting>,
    pub issue: Section<Option<IssueBoxes>>,
    pub pipeline: Section<Option<PipelineSection>>,
    pub where_rows: Section<Vec<WhereRow>>,
    /// The project's URL, from a link GitLab gave: `https://…/platform`.
    pub project_url: Option<String>,
}

pub fn worktree_of(local: WorktreeLocal, remote: Option<WorktreeRemote>, waiting: Vec<Waiting>) -> WorktreeModel {
    let issue = section("ISSUE", || {
        remote
            .as_ref()
            .and_then(|remote| remote.issue.as_ref())
            .and_then(issue_boxes_of)
    });
    let pipeline = section("PIPELINE", || remote.as_ref().and_then(pipeline_of));
    let where_rows = section("WHERE", || {
        local
            .tree
            .as_ref()
            .filter(|tree| tree.has_env)
            .map(|tree| where_of(&local, tree))
            .unwrap_or_default()
    });
    let project_url = remote
        .as_ref()
        .and_then(|remote| {
            remote
                .issue
                .as_ref()
                .map(|issue| issue.url.clone())
                .or_else(|| remote.mr.as_ref().map(|mr| mr.url.clone()))
        })
        .and_then(|url| url.split("/-/").next().map(String::from));
    WorktreeModel {
        local,
        remote,
        waiting,
        issue,
        pipeline,
        where_rows,
        project_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::{IssueBox, Mr, Pipeline};

    #[test]
    fn groups_the_boxes_of_its_issue_under_their_headings() {
        let found = |heading: &str, text: &str, is_ticked: bool| IssueBox {
            heading: Some(heading.into()),
            text: text.into(),
            is_ticked,
            line: 0,
        };
        let issue = Issue {
            iid: 4,
            url: "u".into(),
            boxes: vec![
                found("Work", "a", true),
                found("Work", "b", false),
                found("Acceptance", "c", true),
            ],
            ..Default::default()
        };
        let boxes = issue_boxes_of(&issue).unwrap();
        assert_eq!((boxes.done, boxes.total, boxes.groups.len()), (2, 3, 2));
        assert_eq!(boxes.groups[0].open, ["b"]);
        assert!(issue_boxes_of(&Issue::default()).is_none());
    }

    #[test]
    fn shows_every_job_of_the_pipeline_and_the_failed_ones_logs() {
        let job = |name: &str, status: &str, log: Option<Vec<String>>| Job {
            name: name.into(),
            status: status.into(),
            log,
            ..Default::default()
        };
        let remote = WorktreeRemote {
            mr: Some(Mr {
                pipeline: Some(Pipeline {
                    status: "failed".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            jobs: vec![
                job("lint", "success", None),
                job("test", "failed", Some(vec!["src/a.test.mjs:3:1".into(), "boom".into()])),
            ],
            ..Default::default()
        };
        let pipeline = pipeline_of(&remote).unwrap();
        assert_eq!(pipeline.jobs.len(), 2);
        assert_eq!(pipeline.logs[0].lines[0].1.as_ref().unwrap().line, Some(3));
        assert!(pipeline.logs[0].lines[1].1.is_none());
        assert!(pipeline_of(&WorktreeRemote::default()).is_none());
    }

    #[test]
    fn names_where_things_run() {
        let tree = Tree {
            has_env: true,
            has_own_port: true,
            offset: 2,
            stack: Some("running(5)".into()),
            ..Default::default()
        };
        let rows = where_of(&WorktreeLocal::default(), &tree);
        assert_eq!(rows[0].url.as_deref(), Some("http://localhost:3002"));
        assert_eq!(rows[0].hint.as_deref(), Some("cd products/vita/web && pnpm dev"));
        assert_eq!(rows[3].address.as_deref(), Some("localhost:5434"));
        assert!(rows[1].is_on);
        let model = worktree_of(
            WorktreeLocal {
                tree: Some(tree),
                ..Default::default()
            },
            None,
            vec![],
        );
        assert_eq!(model.where_rows.unwrap().len(), 6);
    }
}
