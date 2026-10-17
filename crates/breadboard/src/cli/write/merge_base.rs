//! `pnpm wt:merge-base [--resolve]` (#396): the worktree's base merged in,
//! never left half-merged. `git merge --no-commit` of `origin/<base>`; clean,
//! it commits; on a conflict it lists the files and runs `git merge --abort`,
//! so the worktree is as before, and says how to resolve them. `--resolve`
//! leaves the conflicted merge open for the agent to finish in the same run
//! (the overview flags one left open for long: model/half_merged.rs).

use super::common::{Argv, Here, Outcome, Takes, help, plural, say};
use breadboard_core::git::{self, git};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: pnpm wt:merge-base [--resolve]";
const ABOUT: &str = "pnpm wt:merge-base merges the worktree's base (origin/<base>, fetched first) in: a clean merge is
committed, a conflicting one is aborted with its files listed, so the worktree is never left half-merged.
--resolve leaves a conflicting merge open, to resolve, git add and git commit --no-edit in the same run.";
const TAKES: Takes = Takes {
    valued: &[],
    switches: &["--resolve"],
    words: Some(0),
    usage: USAGE,
    command: "wt:merge-base",
};

pub fn run(argv: &Argv) -> Outcome {
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let here = Here::find();
    let Some(dir) = here.worktree() else {
        return Err("not in a worktree: run it in the worktree whose base to merge in".into());
    };
    if is_merging(dir) {
        return Err(format!("a merge is open here already: {}", FINISH_OR_ABORT));
    }
    if has_changes(dir) {
        return Err("uncommitted changes: commit them first, a merge that has to be aborted would mix them in".into());
    }
    let base = git::base_of(dir);
    let fetched = git(dir, &["fetch", "--quiet", "origin", &base.local]);
    if !fetched.is_ok {
        return Err(format!(
            "git fetch origin {} failed: {}",
            base.local,
            fetched.stderr.trim()
        ));
    }
    match merge_in(dir, &base.remote, argv.has("--resolve")) {
        Merged::UpToDate => say(&format!("\n✓ {} is in already: nothing to merge\n", base.remote)),
        Merged::Done { commits } => say(&format!(
            "\n✓ {} merged in ({}): {run} wt:check, then push\n",
            base.remote,
            plural(commits as usize, "commit"),
            run = breadboard_core::config::run_word()
        )),
        Merged::Open { files } => say(&format!(
            "\n⚠ conflicts in {}: the merge stays open. Resolve them now, in this run: fix the files, git add them, git commit --no-edit, {run} wt:check, push (or git merge --abort)\n",
            files.join(", "),
            run = breadboard_core::config::run_word()
        )),
        Merged::Aborted { files } => {
            return Err(format!(
                "conflicts in {} with {}: the merge is aborted, the worktree is as before. Resolve by {run} wt:merge-base --resolve, fixing the files, git add, git commit --no-edit, {run} wt:check and push, all in one run",
                files.join(", "),
                base.remote,
                run = breadboard_core::config::run_word()
            ));
        }
        Merged::Failed(why) => return Err(format!("git merge {} failed: {why}", base.remote)),
    }
    Ok(ExitCode::SUCCESS)
}

const FINISH_OR_ABORT: &str =
    "finish it (resolve the files, git add, git commit --no-edit) or abort it (git merge --abort)";

/// How a merge of the base went.
#[derive(Debug, PartialEq, Eq)]
enum Merged {
    /// HEAD has every commit of it.
    UpToDate,
    /// Merged and committed: the commits it brought.
    Done { commits: u64 },
    /// Conflicts, the merge left open (`--resolve`).
    Open { files: Vec<String> },
    /// Conflicts, the merge aborted: no MERGE_HEAD, the files as before.
    Aborted { files: Vec<String> },
    /// git refused or failed without a conflict (what it said); nothing left open.
    Failed(String),
}

/// `git merge --no-commit <reference>` in `dir`, committed when clean; on a
/// conflict aborted unless `should_resolve`. The fetch is the caller's.
fn merge_in(dir: &Path, reference: &str, should_resolve: bool) -> Merged {
    let Some(commits) = git::count(dir, &format!("HEAD..{reference}")) else {
        return Merged::Failed(format!("{reference} is no commit"));
    };
    if commits == 0 {
        return Merged::UpToDate;
    }
    let merge = git(dir, &["merge", "--no-commit", "--no-edit", reference]);
    if merge.is_ok {
        // A fast-forward has no merge to commit.
        if is_merging(dir) {
            let commit = git(dir, &["commit", "--no-edit", "--quiet"]);
            if !commit.is_ok {
                let why = said(&commit);
                abort(dir);
                return Merged::Failed(format!("the merge commit failed: {why}"));
            }
        }
        return Merged::Done { commits };
    }
    let files: Vec<String> = git(dir, &["diff", "--name-only", "--diff-filter=U"])
        .lines()
        .map(String::from)
        .collect();
    if files.is_empty() {
        // Refused before it began (files it would overwrite), or broke off.
        abort(dir);
        return Merged::Failed(said(&merge));
    }
    if should_resolve {
        return Merged::Open { files };
    }
    abort(dir);
    if is_merging(dir) {
        return Merged::Failed(format!("git merge --abort left MERGE_HEAD: {FINISH_OR_ABORT}"));
    }
    Merged::Aborted { files }
}

/// What git said, its error first.
fn said(out: &git::Out) -> String {
    let text = if out.stderr.trim().is_empty() {
        out.out()
    } else {
        out.stderr.trim()
    };
    text.lines().next().unwrap_or("").to_string()
}

fn abort(dir: &Path) {
    if is_merging(dir) {
        git(dir, &["merge", "--abort"]);
    }
}

/// Whether a merge is open in `dir`: its MERGE_HEAD.
fn is_merging(dir: &Path) -> bool {
    git::merge_head_at(dir).is_some()
}

/// Whether tracked files have uncommitted changes.
fn has_changes(dir: &Path) -> bool {
    !git(dir, &["status", "--porcelain", "--untracked-files=no"])
        .out()
        .is_empty()
}

#[cfg(test)]
mod tests {
    use super::super::common::scratch::Scratch;
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let out = git(dir, args);
        assert!(out.is_ok, "git {args:?}: {}", out.stderr);
    }

    /// A repo whose `base` and `work` branches both changed `a.txt` (`work`
    /// checked out), and `b.txt` changed on `base` only.
    fn repo(scratch: &Scratch, is_conflicting: bool) -> &Path {
        let dir = scratch.path();
        run(dir, &["init", "--quiet", "--initial-branch=base"]);
        run(dir, &["config", "user.name", "t"]);
        run(dir, &["config", "user.email", "t@t"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        std::fs::write(dir.join("b.txt"), "one\n").unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "--quiet", "-m", "start"]);
        run(dir, &["checkout", "--quiet", "-b", "work"]);
        std::fs::write(dir.join("a.txt"), "work\n").unwrap();
        run(dir, &["commit", "--quiet", "-am", "work"]);
        run(dir, &["checkout", "--quiet", "base"]);
        let a = if is_conflicting { "base\n" } else { "one\n" };
        std::fs::write(dir.join("a.txt"), a).unwrap();
        std::fs::write(dir.join("b.txt"), "base\n").unwrap();
        run(dir, &["commit", "--quiet", "-am", "base"]);
        run(dir, &["checkout", "--quiet", "work"]);
        dir
    }

    #[test]
    fn aborts_a_conflicting_merge_and_names_its_files() {
        let scratch = Scratch::new();
        let dir = repo(&scratch, true);
        let head = git(dir, &["rev-parse", "HEAD"]).out().to_string();
        assert_eq!(
            merge_in(dir, "base", false),
            Merged::Aborted {
                files: vec!["a.txt".into()]
            }
        );
        assert!(!is_merging(dir));
        assert!(!has_changes(dir));
        assert_eq!(git(dir, &["rev-parse", "HEAD"]).out(), head);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "work\n");
    }

    #[test]
    fn leaves_a_conflicting_merge_open_to_resolve() {
        let scratch = Scratch::new();
        let dir = repo(&scratch, true);
        assert_eq!(
            merge_in(dir, "base", true),
            Merged::Open {
                files: vec!["a.txt".into()]
            }
        );
        assert!(is_merging(dir));
    }

    #[test]
    fn commits_a_clean_merge_and_knows_when_nothing_is_to_merge() {
        let scratch = Scratch::new();
        let dir = repo(&scratch, false);
        assert_eq!(merge_in(dir, "base", false), Merged::Done { commits: 1 });
        assert!(!is_merging(dir));
        assert!(!has_changes(dir));
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "base\n");
        assert_eq!(git(dir, &["rev-list", "--count", "--merges", "HEAD"]).out(), "1");
        assert_eq!(merge_in(dir, "base", false), Merged::UpToDate);
        assert!(matches!(merge_in(dir, "nothing", false), Merged::Failed(_)));
    }
}
