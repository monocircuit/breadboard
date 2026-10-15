//! An epic keeps up with develop by itself (#352): the daemon merges
//! `origin/develop` into each orchestration's epic checkout once develop
//! moved, the epic's `wt:check` decides whether it is pushed. Only in a
//! checkout on its epic branch with no changes to tracked files; a conflict
//! is aborted and said, never resolved, forced or reset.
//!
//! What it tried lives in `<git common dir>/breadboard/epic-<name>.json`:
//! the develop commit it last took on and how that went, so one develop
//! commit is tried once (a failed check isn't run again every few seconds).

use crate::git::git;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How a look at an epic went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Synced {
    /// develop is in the epic, on origin too.
    UpToDate,
    /// develop merged into the checkout, `commits` of develop's came in:
    /// to check and push. Also a merge of before that isn't pushed yet.
    Merged { commits: u64 },
    /// The merge conflicts in `files`: aborted, the checkout as it was.
    Conflict { files: Vec<String> },
    /// Not touched, and why (another branch, local changes, a fetch that failed).
    Skipped { reason: String },
}

/// What was tried for an epic, kept for the next look.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tried {
    /// The develop commit it took on.
    pub develop: String,
    pub at: i64,
    /// `pushed`, `up to date`, `conflict`, `check failed`, `skipped: …`.
    pub outcome: String,
}

pub fn tried_file(store: &Path, name: &str) -> PathBuf {
    store.join(format!("epic-{name}.json"))
}

pub fn read_tried(store: &Path, name: &str) -> Tried {
    std::fs::read_to_string(tried_file(store, name))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn write_tried(store: &Path, name: &str, tried: &Tried) {
    if let Ok(text) = serde_json::to_string(tried) {
        let _ = std::fs::write(tried_file(store, name), text + "\n");
    }
}

/// origin/develop's commit as the clone knows it (the main checkout's look fetches it).
pub fn develop_head(dir: &Path) -> Option<String> {
    let out = git(
        dir,
        &["rev-parse", "--verify", "--quiet", "refs/remotes/origin/develop"],
    );
    Some(out.out().to_string()).filter(|sha| out.is_ok && !sha.is_empty())
}

/// Whether a look at the epic is due: develop moved since the last one tried.
pub fn is_due(dir: &Path, tried: &Tried) -> bool {
    develop_head(dir).is_some_and(|head| head != tried.develop)
}

fn is_ancestor(dir: &Path, older: &str, newer: &str) -> bool {
    git(dir, &["merge-base", "--is-ancestor", older, newer]).is_ok
}

/// Merges `origin/develop` into the epic checkout `dir` on `epic` (#352).
pub fn merge_develop(dir: &Path, epic: &str) -> Synced {
    let skipped = |reason: String| Synced::Skipped { reason };
    let branch = git(dir, &["branch", "--show-current"]);
    if branch.out() != epic {
        return skipped(format!(
            "the epic checkout is on {} instead of {epic}",
            if branch.out().is_empty() {
                "a detached HEAD"
            } else {
                branch.out()
            }
        ));
    }
    let changes = git(dir, &["status", "--porcelain", "--untracked-files=no"]);
    if !changes.is_ok || !changes.out().is_empty() {
        return skipped("the epic checkout has changes of its own".into());
    }
    let fetched = git(
        dir,
        &[
            "fetch",
            "--quiet",
            "origin",
            "+refs/heads/develop:refs/remotes/origin/develop",
            &format!("+refs/heads/{epic}:refs/remotes/origin/{epic}"),
        ],
    );
    if !fetched.is_ok {
        return skipped(format!("git fetch failed: {}", fetched.stderr.trim()));
    }
    let remote = format!("origin/{epic}");
    if is_ancestor(dir, "origin/develop", "HEAD") {
        // develop is in: pushed already, or a merge of an earlier look waits for its check.
        return if is_ancestor(dir, "HEAD", &remote) {
            Synced::UpToDate
        } else {
            Synced::Merged { commits: 0 }
        };
    }
    // Behind its own origin (a task merged into the epic meanwhile): catch up first, never diverge.
    if !is_ancestor(dir, &remote, "HEAD") && !git(dir, &["merge", "--quiet", "--ff-only", &remote]).is_ok {
        return skipped(format!("the epic checkout and {remote} went apart"));
    }
    let commits = git(dir, &["rev-list", "--count", "HEAD..origin/develop"])
        .out()
        .parse()
        .unwrap_or(0);
    let message = format!("Merge develop into {epic} (#352)");
    let merged = git(
        dir,
        &["merge", "--quiet", "--no-edit", "-m", &message, "origin/develop"],
    );
    if merged.is_ok {
        return Synced::Merged { commits };
    }
    let files: Vec<String> = git(dir, &["diff", "--name-only", "--diff-filter=U"])
        .lines()
        .map(String::from)
        .collect();
    let _ = git(dir, &["merge", "--abort"]);
    if files.is_empty() {
        // Refused before any file (no identity to commit as, a lock): no conflict, git's reason.
        return skipped(format!("git merge failed: {}", merged.stderr.trim()));
    }
    Synced::Conflict { files }
}

/// Pushes the epic checkout's merge to its branch on origin, or why not.
pub fn push(dir: &Path, epic: &str) -> Result<(), String> {
    let out = git(dir, &["push", "--quiet", "origin", &format!("HEAD:refs/heads/{epic}")]);
    if out.is_ok {
        Ok(())
    } else {
        Err(out.stderr.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, run};
    use crate::lock::scratch::Scratch;

    const EPIC: &str = "epic/15-breadboard";

    /// The repo, its epic branch on origin and a checkout of it, as the
    /// orchestration has them; a colleague's clone that pushes to origin.
    fn world(scratch: &Scratch) -> (PathBuf, PathBuf, PathBuf) {
        let main = repo(scratch);
        run(&main, &["config", "user.name", "Test"]);
        run(&main, &["config", "user.email", "test@example.com"]);
        run(
            &main,
            &["push", "--quiet", "origin", &format!("develop:refs/heads/{EPIC}")],
        );
        run(&main, &["fetch", "--quiet", "origin"]);
        let checkout = scratch.join("epic");
        run(
            &main,
            &[
                "worktree",
                "add",
                "--quiet",
                "--track",
                "-b",
                EPIC,
                checkout.to_str().unwrap(),
                &format!("origin/{EPIC}"),
            ],
        );
        let other = scratch.join("other");
        run(
            scratch.path(),
            &[
                "clone",
                "--quiet",
                scratch.join("origin.git").to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        run(&other, &["config", "user.name", "Test"]);
        run(&other, &["config", "user.email", "test@example.com"]);
        (main, checkout, other)
    }

    fn commit(dir: &Path, branch: &str, file: &str, text: &str) {
        run(dir, &["checkout", "--quiet", branch]);
        std::fs::write(dir.join(file), text).unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "--quiet", "-m", &format!("feat: {file}")]);
        run(dir, &["push", "--quiet", "origin", branch]);
    }

    #[test]
    fn develop_merges_into_the_epic_and_the_epics_own_work_stays() {
        let scratch = Scratch::new();
        let (_, checkout, other) = world(&scratch);
        assert_eq!(merge_develop(&checkout, EPIC), Synced::UpToDate);
        commit(&other, "develop", "hotfix.md", "fix\n");
        commit(&other, EPIC, "task.md", "task\n");
        assert_eq!(merge_develop(&checkout, EPIC), Synced::Merged { commits: 1 });
        assert!(checkout.join("hotfix.md").exists() && checkout.join("task.md").exists());
        // Merged, not pushed yet: the next look still has it to check and push.
        assert_eq!(merge_develop(&checkout, EPIC), Synced::Merged { commits: 0 });
        push(&checkout, EPIC).unwrap();
        assert_eq!(merge_develop(&checkout, EPIC), Synced::UpToDate);
        run(&other, &["fetch", "--quiet", "origin"]);
        assert!(is_ancestor(&other, "origin/develop", &format!("origin/{EPIC}")));
    }

    #[test]
    fn a_conflict_is_aborted_and_named_the_checkout_as_it_was() {
        let scratch = Scratch::new();
        let (_, checkout, other) = world(&scratch);
        commit(&other, "develop", "README.md", "develop's\n");
        commit(&other, EPIC, "README.md", "the epic's\n");
        let head = run(&other, &["rev-parse", &format!("origin/{EPIC}")]);
        assert_eq!(
            merge_develop(&checkout, EPIC),
            Synced::Conflict {
                files: vec!["README.md".into()]
            }
        );
        assert_eq!(run(&checkout, &["rev-parse", "HEAD"]), head);
        assert_eq!(run(&checkout, &["status", "--porcelain"]), "");
    }

    #[test]
    fn local_changes_or_another_branch_leave_the_checkout_alone() {
        let scratch = Scratch::new();
        let (_, checkout, other) = world(&scratch);
        commit(&other, "develop", "hotfix.md", "fix\n");
        std::fs::write(checkout.join("README.md"), "mine\n").unwrap();
        assert!(matches!(merge_develop(&checkout, EPIC), Synced::Skipped { .. }));
        assert_eq!(std::fs::read_to_string(checkout.join("README.md")).unwrap(), "mine\n");
        run(&checkout, &["checkout", "--quiet", "README.md"]);
        assert!(matches!(
            merge_develop(&checkout, "epic/16-other"),
            Synced::Skipped { .. }
        ));
    }

    #[test]
    fn one_develop_commit_is_tried_once() {
        let scratch = Scratch::new();
        let (main, _, other) = world(&scratch);
        let store = scratch.join("store");
        std::fs::create_dir_all(&store).unwrap();
        let tried = read_tried(&store, "breadboard");
        assert!(is_due(&main, &tried));
        let head = develop_head(&main).unwrap();
        write_tried(
            &store,
            "breadboard",
            &Tried {
                develop: head,
                at: 1,
                outcome: "check failed".into(),
            },
        );
        assert!(!is_due(&main, &read_tried(&store, "breadboard")));
        commit(&other, "develop", "next.md", "next\n");
        run(&main, &["fetch", "--quiet", "origin"]);
        assert!(is_due(&main, &read_tried(&store, "breadboard")));
    }
}
