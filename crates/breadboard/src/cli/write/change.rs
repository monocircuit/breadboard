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
    let is_draft = drafted.as_ref().map(|ran| ran.is_ok);
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

    let mut lines = format!(
        "\n✓ change requested for {}: {text}\n",
        iid.as_ref()
            .map(|iid| format!("!{iid}"))
            .unwrap_or_else(|| "this worktree (no MR yet)".into())
    );
    match (&iid, is_draft) {
        (Some(iid), Some(true)) => {
            lines += &format!("  !{iid} is a draft now: it can't be merged until the change is pushed and green\n")
        }
        (Some(_), _) => {}
        (None, _) => lines += "  no MR to block yet: open it with the first commit (pnpm wt:mr)\n",
    }
    if !withdrawn.is_empty() {
        let merges: Vec<String> = withdrawn.iter().map(|action| format!("merge #{}", action.id)).collect();
        lines += &format!("  withdrawn: {}\n", merges.join(", "));
    }
    if dir.is_some() {
        lines += "  the overview shows it on the MR and the worktree until a commit after it is pushed and green; then post the merge again (pnpm wt:action merge …)\n";
    } else {
        let source = mr.as_ref().map(|mr| glab::text_of(mr, "source_branch")).unwrap_or("");
        lines +=
            &format!("  no worktree of this orchestration has {source}: the MR is blocked, the change needs one\n");
    }
    say(&lines);
    if let (Some(iid), Some(ran)) = (&iid, drafted.filter(|ran| !ran.is_ok)) {
        return Err(format!(
            "glab could not make !{iid} a draft ({}): do it by hand before the change is built (glab mr update {iid} --draft)",
            ran.first_line()
        ));
    }
    Ok(ExitCode::SUCCESS)
}
