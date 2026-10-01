//! An orchestration's worktrees as the view reads them (view.mjs `readView`):
//! git, the actions file, the wait, the requested change, what isn't pushed
//! and whether its HEAD is tested, each worktree side by side. And its
//! issues (gitlab.mjs `issuesOf`): the record's entries, each with its
//! worktree while that is there, and a worktree the record doesn't list.

use crate::cli::read::list::{offset_of, port_answers};
use breadboard_core::actions::{Action, Change, actions_file, actions_of, answered_of, change_in};
use breadboard_core::git::{self, Worktree, git, issue_of_branch};
use breadboard_core::json::{self, int_of, ints_of, str_of, strs_of};
use breadboard_core::lock::is_alive;
use breadboard_core::record::{IssueEntry, origin_of};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::thread;

/// Whether a worktree's HEAD is tested (#298, check.mjs `checkStateOf`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Check {
    /// wt:check runs.
    Checking,
    /// The last check that passed tested HEAD's tree.
    Tested { sha: String },
    /// The last check of HEAD's tree failed: what failed, in one line.
    Failed { what: String, is_test: bool },
    /// Commits since the last pass (since develop without one).
    Untested { count: u64 },
    /// Nothing of its own yet.
    #[default]
    None,
}

/// A worktree of the orchestration with what the view shows of it.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub dir: PathBuf,
    /// Its feature: the directory's name after the main checkout's.
    pub name: String,
    pub branch: String,
    pub actions: Vec<Action>,
    /// The answered and delivered ones: a `broken` try holds the merge (#280).
    pub answered: Vec<Action>,
    /// The MRs it waits for (wt:wait).
    pub wait: Option<Vec<i64>>,
    pub change: Option<Change>,
    /// origin's commit of its branch, None without a remote branch.
    pub pushed: Option<String>,
    pub has_remote_branch: bool,
    pub unpushed: u64,
    pub unpushed_at: Option<i64>,
    pub check: Check,
    /// What its work touches, for the merge order.
    pub files: BTreeSet<String>,
    pub behind: u64,
    pub ahead: u64,
    pub changed: usize,
    /// Commits develop doesn't have (`git cherry`), what keeps a merged one.
    pub own_commits: usize,
    /// Its app: whether it has a `.env.local`, the port offset of its own stack, whether that answers.
    pub has_env: bool,
    pub offset: i64,
    pub is_up: bool,
}

/// The worktrees of `worktrees` (not the main checkout) whose origin is `name`, read side by side.
pub fn trees_of(main: &Path, worktrees: &[Worktree], name: &str) -> Vec<Tree> {
    let ours: Vec<&Worktree> = worktrees
        .iter()
        .skip(1)
        .filter(|tree| origin_of(&tree.dir).is_some_and(|origin| origin.name == name))
        .collect();
    thread::scope(|scope| {
        let handles: Vec<_> = ours
            .iter()
            .map(|tree| scope.spawn(move || tree_of(main, tree)))
            .collect();
        handles.into_iter().filter_map(|handle| handle.join().ok()).collect()
    })
}

fn tree_of(main: &Path, worktree: &Worktree) -> Tree {
    let dir = &worktree.dir;
    let branch = worktree.branch.clone().unwrap_or_else(|| "(detached)".into());
    let file = json::read(&actions_file(dir)).unwrap_or(Value::Null);
    let push = git::push_state(dir, worktree.branch.as_deref());
    let ahead = git::count(dir, "origin/develop..HEAD").unwrap_or(0);
    // Read only: the move of an old `.env.local` is wt:list's (a print changes nothing).
    let env_file = dir.join("products/vita/web/.env.local");
    let has_env = env_file.exists();
    let offset = offset_of(&env_file);
    Tree {
        has_env,
        offset,
        is_up: has_env && offset > 0 && port_answers(3000 + offset),
        name: git::feature_of(main, dir),
        actions: actions_of(&actions_file(dir)),
        answered: answered_of(&actions_file(dir)),
        wait: wait_of(dir),
        change: change_in(&file),
        pushed: push.pushed,
        has_remote_branch: push.has_remote_branch,
        unpushed: push.unpushed,
        unpushed_at: push.unpushed_at,
        check: check_of(dir, ahead),
        files: git::work_files(dir),
        behind: git::count(dir, "HEAD..origin/develop").unwrap_or(0),
        ahead,
        changed: git::changed_count(dir),
        own_commits: if worktree.branch.is_some() {
            git::own_commits(dir, &branch)
        } else {
            0
        },
        branch,
        dir: dir.clone(),
    }
}

/// The MRs the wait of `dir` names (`.claude/wait.json`), None without one.
pub fn wait_of(dir: &Path) -> Option<Vec<i64>> {
    let mrs = ints_of(&json::read(&dir.join(".claude").join("wait.json"))?, "mrs");
    (!mrs.is_empty()).then_some(mrs)
}

/// checkStateOf for the worktree `dir` (worktree/check.mjs `checkOf`):
/// `.claude/checked.json` against HEAD's tree. `ahead`: its commits after develop.
pub fn check_of(dir: &Path, ahead: u64) -> Check {
    let Some(head) = git::head_of(dir) else {
        return Check::None;
    };
    let record = json::read(&dir.join(".claude").join("checked.json")).unwrap_or(Value::Null);
    let part = |key: &str| record.get(key).filter(|value| value.is_object());
    if part("running")
        .and_then(|running| int_of(running, "pid"))
        .is_some_and(is_alive)
    {
        return Check::Checking;
    }
    let tree = git::tree_of(dir, "HEAD");
    let is_this = |value: &&Value| tree.is_some() && str_of(value, "tree") == tree.as_deref();
    if part("passed").filter(is_this).is_some() {
        return Check::Tested { sha: head };
    }
    if let Some(failed) = part("failed").filter(is_this) {
        let failures: Vec<&Value> = failed
            .get("failures")
            .and_then(Value::as_array)
            .map(|list| list.iter().collect())
            .unwrap_or_default();
        let is_test = failures.iter().any(|failure| str_of(failure, "task") == Some("test"));
        return Check::Failed {
            what: failures_text(&failures),
            is_test,
        };
    }
    let passed = part("passed");
    if passed.is_none() && ahead == 0 {
        return Check::None;
    }
    let since = passed.and_then(|passed| {
        let sha = str_of(passed, "sha").filter(|sha| !sha.is_empty())?;
        git(dir, &["merge-base", "--is-ancestor", sha, "HEAD"])
            .is_ok
            .then(|| git::count(dir, &format!("{sha}..HEAD")))
            .flatten()
    });
    Check::Untested {
        count: since.unwrap_or(ahead),
    }
}

/// `@monocircuit/vita (frame.test.ts > keeps …, +2 more)`: what failed, in one line (check.mjs `failuresText`).
fn failures_text(failures: &[&Value]) -> String {
    failures
        .iter()
        .map(|failure| {
            let name = str_of(failure, "package").unwrap_or_default();
            let task = str_of(failure, "task").unwrap_or_default();
            let tests = strs_of(failure, "tests");
            if task != "test" || tests.is_empty() {
                return format!("{name} {task}");
            }
            let shown = tests.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
            let rest = if tests.len() > 2 {
                format!(", +{} more", tests.len() - 2)
            } else {
                String::new()
            };
            format!("{name} ({shown}{rest})")
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// An issue of the orchestration: the record's entry, with its worktree
/// (an index into the trees) while that is there.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub entry: IssueEntry,
    pub feature: String,
    pub tree: Option<usize>,
}

impl Item {
    pub fn number(&self) -> Option<i64> {
        self.entry.number
    }
    pub fn branch(&self) -> Option<&str> {
        self.entry.branch.as_deref()
    }
}

/// The issues of the record, each with its worktree, and a worktree the
/// record doesn't list as one more; by number, one without last.
pub fn items_of(entries: &[IssueEntry], trees: &[Tree]) -> Vec<Item> {
    let mut items: Vec<Item> = entries
        .iter()
        .map(|entry| Item {
            feature: entry.feature.clone().unwrap_or_default(),
            tree: trees
                .iter()
                .position(|tree| entry.branch.as_deref() == Some(tree.branch.as_str())),
            entry: entry.clone(),
        })
        .collect();
    for (at, tree) in trees.iter().enumerate() {
        if items.iter().any(|item| item.tree == Some(at)) {
            continue;
        }
        items.push(Item {
            entry: IssueEntry {
                number: issue_of_branch(&tree.branch).map(|number| number as i64),
                branch: Some(tree.branch.clone()),
                ..IssueEntry::default()
            },
            feature: tree.name.clone(),
            tree: Some(at),
        });
    }
    items.sort_by_key(|item| item.number().unwrap_or(i64::MAX));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(number: i64, branch: &str) -> IssueEntry {
        IssueEntry {
            number: Some(number),
            branch: Some(branch.into()),
            feature: Some(format!("{number}-x")),
            ..IssueEntry::default()
        }
    }

    #[test]
    fn every_issue_with_its_worktree_and_an_unlisted_worktree_too() {
        let trees = vec![
            Tree {
                name: "5-b".into(),
                branch: "feature/5-b".into(),
                ..Tree::default()
            },
            Tree {
                name: "3-a".into(),
                branch: "feature/3-a".into(),
                ..Tree::default()
            },
        ];
        let items = items_of(&[entry(5, "feature/5-b"), entry(1, "feature/1-z")], &trees);
        let shown: Vec<(Option<i64>, Option<usize>, &str)> = items
            .iter()
            .map(|item| (item.number(), item.tree, item.feature.as_str()))
            .collect();
        assert_eq!(
            shown,
            vec![
                (Some(1), None, "1-x"),
                (Some(3), Some(1), "3-a"),
                (Some(5), Some(0), "5-x")
            ]
        );
    }

    #[test]
    fn names_what_failed_in_one_line() {
        let failures = [
            serde_json::json!({"package": "@m/vita", "task": "test", "tests": ["a", "b", "c"]}),
            serde_json::json!({"package": "@m/ui", "task": "lint"}),
        ];
        let failures: Vec<&Value> = failures.iter().collect();
        assert_eq!(failures_text(&failures), "@m/vita (a, b, +1 more) · @m/ui lint");
    }
}
