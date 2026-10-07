//! `pnpm wt:action change "<what>"` (commands/change.mjs, #228): a change is
//! requested for an MR. Before any code is written the MR goes back to
//! draft, the merge actions of its worktree are withdrawn and the overview
//! shows `change requested` until a commit after it is pushed and green. In
//! a worktree: its own MR; the orchestrator names it (`--orchestration <name>
//! --mr !N`). A merged MR takes no change: that needs its own issue.

use super::common::{Argv, Here, Outcome, say};
use super::glab;
use breadboard_core::actions::{actions_file, request_change};
use breadboard_core::git::{git, worktrees};
use breadboard_core::time::{now_ms, to_iso};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::ExitCode;

/// What to do when the MR took its last commit already: the change is work of its own.
fn late_text(iid: &str, state: &str, text: &str) -> String {
    format!(
        "!{iid} is {state} already: \"{text}\" can't go into it any more. It gets its own at once, pushed and visible:
  glab issue create --title \"<what>\" --description \"<why, Related: !{iid}>\"
  pnpm wt:new <n>-<feature>   (with --origin <orchestration> for its overview)
  commit, push, pnpm wt:mr    the draft MR from the first commit
  pnpm wt:wait !N … --then \"…\" when it has to wait for others: pushed first, waiting second"
    )
}

pub fn run(argv: &Argv, here: &Here, text: &str) -> Outcome {
    let main = here.main.clone().unwrap_or_else(|| here.dir().to_path_buf());
    let (dir, mr): (Option<PathBuf>, Option<Value>) = if argv.given("--orchestration").is_some() {
        let Some(iid) = argv
            .given("--mr")
            .map(|mr| mr.strip_prefix('!').unwrap_or(mr))
            .filter(|iid| !iid.is_empty())
        else {
            return Err("which MR? --mr !N".into());
        };
        // glab failing is no "no MR": the change would block nothing (rule 6).
        let Some(mr) = glab::view(&main, "mr", iid)? else {
            return Err(format!("GitLab knows no MR !{iid}"));
        };
        let source = glab::text_of(&mr, "source_branch");
        let dir = worktrees(&main)
            .into_iter()
            .skip(1)
            .find(|tree| tree.branch.as_deref() == Some(source))
            .map(|tree| tree.dir);
        (dir, Some(mr))
    } else {
        let Some(dir) = here.worktree() else {
            return Err("run it inside a worktree, or name the MR: --orchestration <name> --mr !N".into());
        };
        let branch = here.branch();
        let mr = match Some(branch).filter(|b| !b.is_empty()) {
            Some(branch) => glab::view(&main, "mr", &branch)?,
            None => None,
        };
        (Some(dir.to_path_buf()), mr)
    };
    if let Some(dir) = &dir {
        super::action::intact(&actions_file(dir), here)?;
    }
    let iid = mr.as_ref().map(|mr| glab::shown(mr.get("iid")));
    if let (Some(mr), Some(iid)) = (&mr, &iid) {
        let state = glab::text_of(mr, "state");
        if state != "opened" {
            return Err(late_text(iid, &glab::shown(mr.get("state")), text));
        }
    }

    // First blocked, then built: the user can't merge it meanwhile.
    let drafted = iid
        .as_ref()
        .map(|iid| glab::glab(&main, &["mr", "update", iid, "--draft"]));
    let withdrawn = match &dir {
        Some(dir) => {
            let branch = match &mr {
                Some(mr) => glab::text_of(mr, "source_branch").to_string(),
                None => git(dir, &["branch", "--show-current"]).out().to_string(),
            };
            let head = git(dir, &["ls-remote", "--heads", "origin", &branch]);
            let head = head
                .out()
                .split_whitespace()
                .next()
                .filter(|sha| !sha.is_empty())
                .map(String::from);
            let change = json!({
                "text": text,
                "at": to_iso(now_ms()),
                "mr": mr.as_ref().and_then(|mr| mr.get("iid").cloned()).unwrap_or(Value::Null),
                "head": head,
            });
            let file = actions_file(dir);
            super::action::announce_lock(&file);
            request_change(&file, change)
                .map_err(|error| format!("{} could not be written: {error}", file.display()))?
        }
        None => Vec::new(),
    };

    let source = mr.as_ref().map(|mr| glab::text_of(mr, "source_branch")).unwrap_or("");
    let ids: Vec<i64> = withdrawn.iter().map(|action| action.id).collect();
    let draft = drafted.map(|ran| if ran.is_ok { Ok(()) } else { Err(ran.first_line()) });
    let (lines, failure) = report(text, iid.as_deref(), draft, &ids, dir.is_some(), source);
    say(&lines);
    failure.map_or(Ok(ExitCode::SUCCESS), Err)
}

/// What `change` says once the change is written, and the failure that
/// follows it when the MR could not be made a draft: the change stays
/// written (the overview shows it), but the MR can still be merged, so the
/// command ends with `✗` and exit 1 and says how to block it by hand.
fn report(
    text: &str,
    iid: Option<&str>,
    drafted: Option<Result<(), String>>,
    withdrawn: &[i64],
    has_dir: bool,
    source: &str,
) -> (String, Option<String>) {
    let mut lines = format!(
        "\n✓ change requested for {}: {text}\n",
        iid.map(|iid| format!("!{iid}"))
            .unwrap_or_else(|| "this worktree (no MR yet)".into())
    );
    match (iid, &drafted) {
        (Some(iid), Some(Ok(()))) => {
            lines += &format!("  !{iid} is a draft now: it can't be merged until the change is pushed and green\n")
        }
        (Some(_), _) => {}
        (None, _) => lines += "  no MR to block yet: open it with the first commit (pnpm wt:mr)\n",
    }
    if !withdrawn.is_empty() {
        let merges: Vec<String> = withdrawn.iter().map(|id| format!("merge #{id}")).collect();
        lines += &format!("  withdrawn: {}\n", merges.join(", "));
    }
    if has_dir {
        lines += "  the overview shows it on the MR and the worktree until a commit after it is pushed and green; then post the merge again (pnpm wt:action merge …)\n";
    } else {
        lines +=
            &format!("  no worktree of this orchestration has {source}: the MR is blocked, the change needs one\n");
    }
    let failure = match (iid, drafted) {
        (Some(iid), Some(Err(why))) => Some(format!(
            "glab could not make !{iid} a draft ({why}): do it by hand before the change is built (glab mr update {iid} --draft)"
        )),
        _ => None,
    };
    (lines, failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_the_mr_is_a_draft_and_what_was_withdrawn() {
        let (lines, failure) = report("rename it", Some("12"), Some(Ok(())), &[3, 5], true, "b");
        assert!(lines.starts_with("\n✓ change requested for !12: rename it\n"));
        assert!(lines.contains("  !12 is a draft now"));
        assert!(lines.contains("  withdrawn: merge #3, merge #5\n"));
        assert_eq!(failure, None);
    }

    #[test]
    fn a_draft_that_failed_keeps_the_change_and_ends_with_what_to_do() {
        let (lines, failure) = report("x", Some("12"), Some(Err("403 Forbidden".into())), &[], true, "b");
        // The change is written and said, but not the draft that didn't happen.
        assert!(lines.contains("✓ change requested for !12: x"));
        assert!(!lines.contains("is a draft now"));
        assert_eq!(
            failure.as_deref(),
            Some(
                "glab could not make !12 a draft (403 Forbidden): do it by hand before the change is built (glab mr update 12 --draft)"
            )
        );
    }

    #[test]
    fn without_an_mr_or_a_worktree_says_what_is_missing() {
        let (lines, failure) = report("x", None, None, &[], true, "");
        assert!(lines.contains("this worktree (no MR yet)"));
        assert!(lines.contains("no MR to block yet"));
        assert_eq!(failure, None);
        let (lines, _) = report("x", Some("4"), Some(Ok(())), &[], false, "feature/4-x");
        assert!(lines.contains("no worktree of this orchestration has feature/4-x"));
    }
}
