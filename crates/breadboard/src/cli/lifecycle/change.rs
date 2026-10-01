//! `wt:change "<what>" [--orchestration <name> --mr !N]`, today's
//! `wt:action change` (worktree/commands/change.mjs, #228): a change is
//! requested for an MR. Before any code is written the MR goes back to
//! draft, the merge actions of its worktree are withdrawn and the overview
//! shows `change requested` until a commit after it is pushed and green. A
//! merged MR takes no change: that needs its own issue.

use super::args::Args;
use super::gitlab;
use super::proc::{Done, fail, git, output};
use super::repo::Repo;
use breadboard_core::actions::{actions_file, request_change};
use breadboard_core::json::{int_of, str_of};
use breadboard_core::time::{now_ms, to_iso};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: pnpm wt:action change <what>     a change for this MR: draft at once, merge withdrawn (--orchestration <name> --mr !N: the orchestrator's)";

pub fn command(args: &Args) -> Done {
    let valued = ["--orchestration", "--mr"];
    args.check(&[], &valued, USAGE)?;
    let text = args.positionals(&valued).join(" ").trim().to_string();
    if text.is_empty() {
        return fail(USAGE);
    }
    let repo = Repo::here()?;
    let orchestration = args.value("--orchestration");
    request(&repo, &text, orchestration, args.value("--mr"))
}

/// The MR `reference` (its number or branch) as GitLab has it.
fn mr_of(repo: &Repo, reference: &str) -> Option<Value> {
    gitlab::view(&repo.main, &["mr", "view", reference])
}

/// The commit origin has of `branch` right now, None without a remote branch.
fn remote_head_of(dir: &Path, branch: &str) -> Option<String> {
    let out = git(dir, &["ls-remote", "--heads", "origin", branch], None);
    out.out().split_whitespace().next().map(String::from)
}

/// What to do when the MR took its last commit already: the change is work of its own.
pub fn late_text(iid: i64, state: &str, text: &str) -> String {
    format!(
        "!{iid} is {state} already: \"{text}\" can't go into it any more. It gets its own at once, pushed and visible:
  glab issue create --title \"<what>\" --description \"<why, Related: !{iid}>\"
  pnpm wt:new <n>-<feature>   (with --origin <orchestration> for its overview)
  commit, push, pnpm wt:mr    the draft MR from the first commit
  pnpm wt:wait !N … --then \"…\" when it has to wait for others: pushed first, waiting second"
    )
}

/// The lines after the request: what is blocked, what was withdrawn, what follows.
pub fn done_text(mr: Option<(i64, &str)>, is_draft: bool, text: &str, withdrawn: &[i64], has_dir: bool) -> String {
    let name = mr.map_or("this worktree (no MR yet)".to_string(), |(iid, _)| format!("!{iid}"));
    let mut lines = format!("\n✓ change requested for {name}: {text}\n");
    match mr {
        Some((iid, _)) if is_draft => lines.push_str(&format!(
            "  !{iid} is a draft now: it can't be merged until the change is pushed and green\n"
        )),
        Some(_) => {}
        None => lines.push_str("  no MR to block yet: open it with the first commit (pnpm wt:mr)\n"),
    }
    if !withdrawn.is_empty() {
        let merges: Vec<String> = withdrawn.iter().map(|id| format!("merge #{id}")).collect();
        lines.push_str(&format!("  withdrawn: {}\n", merges.join(", ")));
    }
    match mr {
        _ if has_dir => lines.push_str(
            "  the overview shows it on the MR and the worktree until a commit after it is pushed and green; then post the merge again (pnpm wt:action merge …)\n",
        ),
        Some((_, branch)) => lines.push_str(&format!(
            "  no worktree of this orchestration has {branch}: the MR is blocked, the change needs one\n"
        )),
        None => {}
    }
    lines
}

/// Requests the change `text`: for the MR of the worktree this runs in, or,
/// with `orchestration`, for the MR `--mr !N` (the orchestrator's).
pub fn request(repo: &Repo, text: &str, orchestration: Option<&str>, mr_flag: Option<&str>) -> Done {
    let (dir, mr): (Option<PathBuf>, Option<Value>) = if orchestration.is_some() {
        let Some(iid) = mr_flag
            .map(|iid| iid.trim_start_matches('!'))
            .filter(|iid| !iid.is_empty())
        else {
            return fail("which MR? --mr !N");
        };
        let Some(mr) = mr_of(repo, iid) else {
            return fail(format!("GitLab knows no MR !{iid}"));
        };
        let branch = str_of(&mr, "source_branch").unwrap_or_default();
        let dir = repo
            .trees()
            .into_iter()
            .find(|tree| tree.branch.as_deref() == Some(branch))
            .map(|tree| tree.dir);
        (dir, Some(mr))
    } else {
        let here = std::env::current_dir().unwrap_or_default();
        let top = git(&here, &["rev-parse", "--show-toplevel"], None);
        let dir = PathBuf::from(top.out());
        if !top.is_ok || top.out().is_empty() || dir == repo.main {
            return fail("run it inside a worktree, or name the MR: --orchestration <name> --mr !N");
        }
        let branch = git(&dir, &["branch", "--show-current"], None).out().to_string();
        let mr = if branch.is_empty() { None } else { mr_of(repo, &branch) };
        (Some(dir), mr)
    };
    let iid = mr.as_ref().and_then(|mr| int_of(mr, "iid"));
    let state = mr.as_ref().and_then(|mr| str_of(mr, "state")).unwrap_or("");
    if let (Some(iid), Some(_)) = (iid, &mr)
        && state != "opened"
    {
        return fail(late_text(iid, state, text));
    }

    // First blocked, then built: the user can't merge it meanwhile.
    let is_draft =
        iid.is_some_and(|iid| output("glab", &["mr", "update", &iid.to_string(), "--draft"], &repo.main, None).is_ok);
    let source = mr
        .as_ref()
        .and_then(|mr| str_of(mr, "source_branch"))
        .unwrap_or_default()
        .to_string();
    let withdrawn = match &dir {
        Some(dir) => {
            let branch = if mr.is_some() {
                source.clone()
            } else {
                git(dir, &["branch", "--show-current"], None).out().to_string()
            };
            let change =
                json!({ "text": text, "at": to_iso(now_ms()), "mr": iid, "head": remote_head_of(dir, &branch) });
            request_change(&actions_file(dir), change).map_err(|error| {
                super::proc::Fail(format!("{} could not be written: {error}", actions_file(dir).display()))
            })?
        }
        None => Vec::new(),
    };
    let ids: Vec<i64> = withdrawn.iter().map(|action| action.id).collect();
    println!(
        "{}",
        done_text(
            iid.map(|iid| (iid, source.as_str())),
            is_draft,
            text,
            &ids,
            dir.is_some()
        )
    );
    if let Some(iid) = iid.filter(|_| !is_draft) {
        return fail(format!(
            "glab could not make !{iid} a draft: do it by hand before the change is built (glab mr update {iid} --draft)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_what_the_request_did() {
        let text = done_text(Some((316, "feature/3-x")), true, "rename it", &[4, 7], true);
        assert_eq!(
            text,
            "\n✓ change requested for !316: rename it\n  !316 is a draft now: it can't be merged until the change is pushed and green\n  withdrawn: merge #4, merge #7\n  the overview shows it on the MR and the worktree until a commit after it is pushed and green; then post the merge again (pnpm wt:action merge …)\n"
        );
        let no_mr = done_text(None, false, "x", &[], true);
        assert!(no_mr.starts_with("\n✓ change requested for this worktree (no MR yet): x\n  no MR to block yet"));
        let no_dir = done_text(Some((9, "fix/9-y")), false, "x", &[], false);
        assert!(
            no_dir.ends_with(
                "  no worktree of this orchestration has fix/9-y: the MR is blocked, the change needs one\n"
            )
        );
    }

    #[test]
    fn a_merged_mr_takes_no_change() {
        let late = late_text(12, "merged", "add a test");
        assert!(late.starts_with("!12 is merged already: \"add a test\" can't go into it any more."));
        assert!(late.contains("--description \"<why, Related: !12>\""));
    }
}
