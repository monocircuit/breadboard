//! `wt:remove <feature> [--keep-branch] [--close-space]`
//! (worktree/commands/remove.mjs): a worktree's own stack, the worktree and
//! its merged branch away, its empty claim on origin too (#272), the record
//! of its orchestration told, its herdr space released. Refused while
//! anything of anyone's is still in it.

use super::args::Args;
use super::claude::{bound_session, left_in, left_text, running_claudes, session_file};
use super::gitlab;
use super::herdr;
use super::layout::{orchestrations::prune_orchestrators, release_space};
use super::proc::{Done, exe, fail, git, run_inherit, step, text};
use super::prune::{has_own_commits, own_files};
use super::repo::{Repo, VITA, compose_args, offset_of};
use super::setups::{setup_at, setup_text, setups_of};
use breadboard_core::git::is_inside;
use breadboard_core::record::{self, origin_of};
use breadboard_core::time::{now_ms, to_iso};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const USAGE: &str = "usage: pnpm wt:remove <feature> [--keep-branch] [--close-space]";

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `--keep-branch`: the branch stays, merged or not.
    pub is_keeping_branch: bool,
    /// `--close-space`: the whole herdr space goes, conversations too.
    pub is_closing_space: bool,
    /// wt:prune fetched a moment ago.
    pub is_fetched: bool,
}

pub fn command(args: &Args) -> Done {
    args.check(&["--keep-branch", "--close-space"], &[], USAGE)?;
    let (Some(feature), 1) = (args.first(), args.positionals(&[]).len()) else {
        return fail(USAGE);
    };
    let repo = Repo::here()?;
    let options = Options {
        is_keeping_branch: args.has("--keep-branch"),
        is_closing_space: args.has("--close-space"),
        is_fetched: false,
    };
    remove(&repo, feature, &options)
}

/// Whether the claim of `branch` on origin can go with its worktree (#272):
/// nothing on it develop lacks and no open MR builds on it; the reason it goes.
pub fn empty_claim(is_on_origin: bool, has_own: bool, open_mrs: Option<usize>) -> Option<&'static str> {
    (is_on_origin && !has_own && open_mrs == Some(0)).then_some("nothing on it develop lacks, no open MR")
}

/// `rm -rf`, again if a writer strikes once more in between.
fn remove_dir(dir: &Path) {
    for _ in 0..10 {
        if fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The MRs of `branch` in `state`, None when GitLab does not answer.
fn mrs_of(repo: &Repo, state: &str, branch: &str) -> Option<Vec<serde_json::Value>> {
    let endpoint = format!(
        "projects/:fullpath/merge_requests?state={state}&source_branch={}",
        gitlab::encode(branch)
    );
    gitlab::api(&repo.main, &endpoint)?.as_array().cloned()
}

/// Rewrites the orchestration's changelog in the background (it asks GitLab
/// per issue): a process of its own that outlives this one, so nothing waits
/// for it and nothing cuts it off. Its errors go nowhere, as today's.
fn changelog_in_background(repo: &Repo, name: &str) {
    let _ = Command::new(exe())
        .args(["wt:orchestrate", name, "--changelog"])
        .current_dir(&repo.main)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}

/// Removes the worktree of `feature` (remove.mjs), its steps said as they run.
pub fn remove(repo: &Repo, feature: &str, options: &Options) -> Done {
    let dir = repo.dir_of(feature);
    let dir_text = text(&dir);
    let Some(worktree) = repo.worktrees().into_iter().skip(1).find(|tree| tree.dir == dir) else {
        return fail(format!("{dir_text} is not a worktree (pnpm wt:list)"));
    };
    if std::env::current_dir().is_ok_and(|here| is_inside(&here, &dir)) {
        return fail(format!("run it from outside {dir_text}"));
    }
    // A handed-over agent still in it: EnterWorktree fails once its directory is gone (#252).
    let left = left_in(&dir, &running_claudes(), bound_session(&dir).as_deref());
    if !left.is_empty() {
        return fail(format!(
            "{} still stands in {feature} (it enters its next worktree, or as a free agent the orchestration's checkout): again once it left",
            left_text(&left)
        ));
    }
    // A start sets it up or moves its agent out of it: git would pull the directory from under it (#272).
    let origin = origin_of(&dir);
    if let Some(origin) = &origin {
        let setups = setups_of(&repo.main, &origin.name);
        if let Some(setup) = setup_at(&setups, &dir_text) {
            let what = if setup.dir.as_deref() == Some(dir_text.as_str()) {
                "sets up"
            } else {
                "moves its agent out of"
            };
            return fail(format!("{} {what} {feature}: again once it is done", setup_text(setup)));
        }
    }
    if !git(&dir, &["status", "--porcelain"], None).out().is_empty() {
        return fail(format!(
            "{feature} has uncommitted changes, commit or discard them first"
        ));
    }

    let branch = worktree.branch.clone();
    let mut is_merged = false;
    if let Some(branch) = &branch {
        if !options.is_fetched {
            git(&repo.main, &["fetch", "--prune"], None);
        }
        is_merged = !has_own_commits(branch, &dir);
        if !is_merged && !options.is_keeping_branch {
            return fail(format!(
                "{branch} has commits that are not in develop. Merge its MR first, or keep the branch with --keep-branch"
            ));
        }
    }

    if offset_of(&dir) != 0 {
        step(&format!("stop the own stack of {feature} and delete its data"));
        let args = compose_args(&["down", "-v"]);
        if !run_inherit("docker", &args, &dir.join(VITA)) {
            return fail(format!("docker {} failed", args.join(" ")));
        }
    }
    step(&format!("git worktree remove {dir_text}"));
    // Its binding goes first: the overview only rewrites a binding that is there.
    let _ = fs::remove_file(session_file(&dir));
    if !run_inherit("git", &["worktree", "remove", &dir_text], &repo.main) {
        // git unregisters it and deletes the files, then fails on a directory something wrote
        // into meanwhile. Nothing of anyone's work left: done; anything else stops here.
        let is_gone = !repo.worktrees().iter().any(|tree| tree.dir == dir);
        let files = if is_gone && dir.exists() {
            own_files(&repo.main, &dir)
        } else {
            Vec::new()
        };
        if !is_gone || !files.is_empty() {
            let left = if files.is_empty() {
                String::new()
            } else {
                format!(", left in it: {}", files.join(", "))
            };
            return fail(format!("git worktree remove {dir_text} failed{left}"));
        }
        remove_dir(&dir);
    }
    // What the removal left behind once git let go: ignored folders a process still wrote into (#268).
    if dir.exists() {
        let files = own_files(&repo.main, &dir);
        if files.is_empty() {
            remove_dir(&dir);
        } else {
            step(&format!("left in {dir_text}: {}", files.join(", ")));
        }
    }
    if let Some(branch) = branch.as_deref().filter(|_| is_merged && !options.is_keeping_branch) {
        step(&format!("delete {branch} (all of it is in develop)"));
        if !run_inherit("git", &["branch", "-D", branch], &repo.main) {
            return fail(format!("git branch -D {branch} failed"));
        }
        // Its claim on origin too, while empty: the issue can start again (#272).
        let remote = format!("refs/remotes/origin/{branch}");
        let is_on_origin = git(&repo.main, &["rev-parse", "--verify", "--quiet", &remote], None).is_ok;
        let claim = if is_on_origin {
            empty_claim(
                true,
                has_own_commits(&remote, &repo.main),
                mrs_of(repo, "opened", branch).map(|mrs| mrs.len()),
            )
        } else {
            None
        };
        if let Some(claim) = claim {
            step(&format!("delete {branch} on origin ({claim})"));
            run_inherit("git", &["push", "--quiet", "origin", "--delete", branch], &repo.main);
        }
    }
    // The overview of the orchestration lists it under "done and removed", with the times.
    if let (Some(origin), Some(branch)) = (&origin, &branch) {
        let merged_at = if is_merged {
            mrs_of(repo, "merged", branch)
                .and_then(|mrs| mrs.first()?.get("merged_at")?.as_str().map(String::from))
                .filter(|at| !at.is_empty())
        } else {
            None
        };
        let _ = record::mark_removed(
            &repo.main,
            &origin.name,
            branch,
            feature,
            &to_iso(now_ms()),
            merged_at.as_deref(),
        );
    }
    // Its changelog lists the merge: GitLab takes a moment per issue.
    if let Some(origin) = origin.as_ref().filter(|_| is_merged) {
        changelog_in_background(repo, &origin.name);
    }
    if let Some(line) = release_space(feature, options.is_closing_space) {
        step(&format!("herdr: {line}"));
    }
    // Its origin went with the directory; the orchestration's space goes with its last worktree.
    if herdr::is_in() {
        for line in prune_orchestrators(repo) {
            step(&format!("herdr: {line}"));
        }
    }
    let kept = match &branch {
        Some(branch) if !is_merged => format!(", {branch} kept"),
        _ => String::new(),
    };
    println!("\n✓ {feature} removed{kept}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_claim_goes_only_without_work_and_mrs() {
        assert_eq!(
            empty_claim(true, false, Some(0)),
            Some("nothing on it develop lacks, no open MR")
        );
        assert_eq!(empty_claim(true, true, Some(0)), None);
        assert_eq!(empty_claim(true, false, Some(1)), None);
        // GitLab did not answer: the claim stays.
        assert_eq!(empty_claim(true, false, None), None);
        assert_eq!(empty_claim(false, false, Some(0)), None);
    }
}
