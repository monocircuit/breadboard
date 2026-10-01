//! The view of everything (no flag, render.mjs) as data: every worktree with
//! its stack and MR, the stacks without a worktree, the open MRs and issues.

use super::{Paint, Section, Span, Tree, section};
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

/// A checkout's stack as a cell (boxes.mjs `stackCell`): `+2 running(5)`, `default down`, `shared`.
pub fn stack_cell(tree: &Tree) -> Span {
    if !tree.has_env {
        return Span::dim("no .env.local");
    }
    if !tree.has_own_port {
        return Span::dim("shared");
    }
    let label = if tree.offset > 0 {
        format!("+{}", tree.offset)
    } else {
        "default".into()
    };
    match &tree.stack {
        None => Span::dim(format!("{label} down")),
        Some(state) => Span::new(
            format!("{label} {state}"),
            if state.starts_with("running") {
                Paint::Green
            } else {
                Paint::Yellow
            },
        ),
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
                tree: tree.clone(),
            })
            .collect()
    });
    let known: Vec<&str> = local.trees.iter().map(|tree| tree.project.as_str()).collect();
    let orphans = local
        .projects
        .iter()
        .flatten()
        .filter(|(name, _)| name.starts_with("vita") && !known.contains(&name.as_str()))
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
        assert_eq!(stack_cell(&rows[0].tree).text, "default running(5)");
        assert_eq!(stack_cell(&rows[1].tree).text, "no .env.local");
        let own = Tree {
            has_env: true,
            has_own_port: true,
            offset: 2,
            ..Default::default()
        };
        assert_eq!(stack_cell(&own).text, "+2 down");
    }
}
