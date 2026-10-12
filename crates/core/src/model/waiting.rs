//! Which open merge requests a worktree waits for (waiting.mjs): the branches
//! its work builds on (a stack, !194 on !193 on !192), in the order they have
//! to be merged. Read from git, nobody writes the stack down.

use crate::git::{count, git};
use crate::gitlab::read::OpenMr;
use std::path::Path;

/// An open MR the worktree builds on, with how many of its commits are in HEAD and not on its base.
#[derive(Debug, Clone, PartialEq)]
pub struct Waiting {
    pub mr: OpenMr,
    pub count: u64,
}

/// The open MRs whose branch the worktree `dir` builds on: commits of the
/// branch (its `origin/` ref) are in HEAD and not in its base (`develop`, the remote ref) yet, through
/// their merge base, so a base that got more commits since still counts. The
/// base of the stack first: that is the merge order.
pub fn waiting_for(dir: &Path, branch: Option<&str>, mrs: &[OpenMr], develop: &str) -> Vec<Waiting> {
    let mut waiting: Vec<Waiting> = mrs
        .iter()
        .filter(|mr| Some(mr.branch.as_str()) != branch)
        .filter_map(|mr| {
            let reference = git(
                dir,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/remotes/origin/{}", mr.branch),
                ],
            );
            if !reference.is_ok || reference.out().is_empty() {
                return None;
            }
            let base = git(dir, &["merge-base", reference.out(), "HEAD"]);
            if !base.is_ok || base.out().is_empty() {
                return None;
            }
            let count = count(dir, &format!("{develop}..{}", base.out())).unwrap_or(0);
            (count > 0).then(|| Waiting { mr: mr.clone(), count })
        })
        .collect();
    waiting.sort_by_key(|found| found.count);
    waiting
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::run;
    use crate::lock::scratch::Scratch;

    fn commit(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), name).unwrap();
        run(dir, &["add", name]);
        run(dir, &["commit", "-q", "-m", name]);
    }
    fn publish(dir: &Path, branch: &str) {
        run(dir, &["update-ref", &format!("refs/remotes/origin/{branch}"), branch]);
    }

    #[test]
    fn lists_the_branches_the_work_builds_on_base_first() {
        let scratch = Scratch::new();
        let dir = scratch.path();
        run(dir, &["init", "-q", "-b", "develop"]);
        commit(dir, "base");
        publish(dir, "develop");
        for (branch, from, commits) in [
            ("a", None, vec!["a1", "a2"]),
            ("b", None, vec!["b1"]),
            ("c", None, vec!["c1"]),
            ("x", Some("develop"), vec!["x1"]),
        ] {
            match from {
                Some(from) => run(dir, &["switch", "-q", "-c", branch, from]),
                None => run(dir, &["switch", "-q", "-c", branch]),
            };
            for name in commits {
                commit(dir, name);
            }
            publish(dir, branch);
        }
        run(dir, &["switch", "-q", "c"]);
        let mrs: Vec<OpenMr> = ["x", "c", "b", "a", "gone"]
            .iter()
            .enumerate()
            .map(|(i, branch)| OpenMr {
                iid: 10 + i as i64,
                branch: branch.to_string(),
                ..Default::default()
            })
            .collect();
        let found = |branch: &str| -> Vec<(String, u64)> {
            waiting_for(dir, Some(branch), &mrs, "origin/develop")
                .into_iter()
                .map(|found| (found.mr.branch, found.count))
                .collect()
        };
        assert_eq!(found("c"), [("a".to_string(), 2), ("b".to_string(), 3)]);
        run(dir, &["switch", "-q", "a"]);
        commit(dir, "a3");
        publish(dir, "a");
        run(dir, &["switch", "-q", "c"]);
        assert_eq!(
            found("c").into_iter().map(|(branch, _)| branch).collect::<Vec<_>>(),
            ["a", "b"]
        );
        run(
            dir,
            &["update-ref", "refs/remotes/origin/develop", "refs/remotes/origin/b"],
        );
        assert!(found("c").is_empty());
        run(dir, &["update-ref", "refs/remotes/origin/develop", "develop"]);
        run(dir, &["switch", "-q", "x"]);
        assert!(found("x").is_empty());
    }
}
