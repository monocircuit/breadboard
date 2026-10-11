//! The view of everything (no flag, render.mjs) as data: every worktree with
//! its stack and MR, the stacks without a worktree, the open MRs and issues.

use super::{Docker, Paint, Section, Span, Tree, section};
use crate::gitlab::read::{EverythingRemote, ListedMr};
use std::collections::BTreeMap;

/// What the local read found: every worktree, the stacks docker runs.
#[derive(Debug, Clone, Default)]
pub struct EverythingLocal {
    pub trees: Vec<Tree>,
    /// docker's projects by name → status, None when docker doesn't answer.
    pub projects: Option<BTreeMap<String, String>>,
}

/// A row of WORKTREES: the worktree and its open MR.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub tree: Tree,
    pub mr: Option<ListedMr>,
    /// The branch, when it says more than the name (`feature/<name>` says nothing new).
    pub branch: Option<String>,
}

/// The view of everything.
#[derive(Debug, Clone)]
pub struct EverythingModel {
    pub trees: Section<Vec<TreeRow>>,
    /// vita stacks no worktree has: `(name, status)`.
    pub orphans: Vec<(String, String)>,
    pub has_docker: bool,
    /// The main checkout's stack is down: its start button shows.
    pub can_start_main: bool,
    pub remote: Option<EverythingRemote>,
}

/// A checkout's app as a cell (render.mjs): `● :3002` up (its page linked),
/// `○ :3002` down, `⇒ :3000` on the shared one, `—` without .env.local.
pub fn app_cell(tree: &Tree) -> Span {
    let port = tree.app_port + tree.offset;
    if !tree.has_env {
        Span::dim("—")
    } else if !tree.has_own_port {
        Span::dim(format!("⇒ :{}", tree.app_port))
    } else if tree.is_up {
        Span {
            url: Some(format!("http://localhost:{port}")),
            ..Span::new(format!("● :{port}"), Paint::Green)
        }
    } else {
        Span::dim(format!("○ :{port}"))
    }
}

pub fn everything_of(local: &EverythingLocal, remote: Option<EverythingRemote>) -> EverythingModel {
    let trees = section("WORKTREES", || {
        local
            .trees
            .iter()
            .map(|tree| TreeRow {
                mr: remote
                    .as_ref()
                    .and_then(|remote| remote.mrs.iter().find(|mr| mr.branch == tree.branch).cloned()),
                branch: (tree.branch.rsplit('/').next() != Some(tree.name.as_str())).then(|| tree.branch.clone()),
                // docker didn't answer: every stack is unknown, never `down` (#331).
                tree: Tree {
                    docker: if local.projects.is_some() {
                        tree.docker
                    } else {
                        Docker::Silent
                    },
                    ..tree.clone()
                },
            })
            .collect()
    });
    let known: Vec<&str> = local.trees.iter().map(|tree| tree.project.as_str()).collect();
    // The stacks of the repo's project no worktree claims: `<project>`, `<project>-<feature>`.
    let shared = local
        .trees
        .iter()
        .find(|tree| tree.is_main)
        .map(|tree| tree.project.as_str());
    let is_ours = |name: &str| {
        shared.is_some_and(|shared| !shared.is_empty() && (name == shared || name.starts_with(&format!("{shared}-"))))
    };
    let orphans = local
        .projects
        .iter()
        .flatten()
        .filter(|(name, _)| is_ours(name) && !known.contains(&name.as_str()))
        .map(|(name, status)| (name.clone(), status.clone()))
        .collect();
    let main = local.trees.iter().find(|tree| tree.is_main);
    EverythingModel {
        trees,
        orphans,
        has_docker: local.projects.is_some(),
        can_start_main: local.projects.is_some()
            && main.is_some_and(|main| {
                main.has_env && !main.stack.as_deref().is_some_and(|stack| stack.contains("running"))
            }),
        remote,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_every_worktree_with_its_mr_and_finds_stacks_without_one() {
        let tree = |name: &str, branch: &str, project: &str| Tree {
            name: name.into(),
            branch: branch.into(),
            project: project.into(),
            ..Default::default()
        };
        let local = EverythingLocal {
            trees: vec![
                Tree {
                    is_main: true,
                    has_env: true,
                    app_port: 3000,
                    has_own_port: true,
                    stack: Some("running(5)".into()),
                    ..tree("platform", "develop", "vita")
                },
                tree("1-x", "feature/1-x", "vita-1"),
                tree("2-y", "fix/9-other", "vita"),
            ],
            projects: Some(
                [
                    ("vita".to_string(), "running(5)".to_string()),
                    ("vita-7".to_string(), "exited(3)".to_string()),
                ]
                .into(),
            ),
        };
        let remote = EverythingRemote {
            mrs: vec![ListedMr {
                iid: 3,
                branch: "feature/1-x".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let model = everything_of(&local, Some(remote));
        let rows = model.trees.unwrap();
        assert_eq!(rows[1].mr.as_ref().map(|mr| mr.iid), Some(3));
        assert_eq!(rows[1].branch, None);
        assert_eq!(rows[2].branch.as_deref(), Some("fix/9-other"));
        assert_eq!(model.orphans, [("vita-7".to_string(), "exited(3)".to_string())]);
        assert!(model.has_docker && !model.can_start_main);
        assert_eq!(rows[0].tree.stack.as_deref(), Some("running(5)"));
        let silent = everything_of(
            &EverythingLocal {
                projects: None,
                ..local.clone()
            },
            None,
        );
        assert_eq!(silent.trees.unwrap()[0].tree.docker, Docker::Silent);
        let own = Tree {
            has_env: true,
            app_port: 3000,
            has_own_port: true,
            offset: 2,
            ..Default::default()
        };
        assert_eq!(app_cell(&own).text, "○ :3002");
        let up = app_cell(&Tree { is_up: true, ..own });
        assert_eq!(
            (up.text.as_str(), up.url.as_deref()),
            ("● :3002", Some("http://localhost:3002"))
        );
        assert_eq!(app_cell(&rows[1].tree).text, "—");
        let shared = Tree {
            has_env: true,
            app_port: 3000,
            ..Default::default()
        };
        assert_eq!(app_cell(&shared).text, "⇒ :3000");
    }
}
