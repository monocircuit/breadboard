//! `bb status` (#427): where the work of this worktree stands, on one
//! screen: the issue, branch and base, what is uncommitted or unpushed,
//! wt:check, the MR, its pipeline, the boxes left, and the one command that
//! comes next. An agent learns the workflow from it, not from the brief.

use super::boxes::{self, Box, boxes_of, tally};
use super::common::{Argv, Here, Outcome, Takes, help, plural, say};
use super::glab;
use breadboard_core::actions::{actions_file, actions_of, answered_of};
use breadboard_core::check::{CheckState, check_of};
use breadboard_core::git::{PushState, base_of, git, issue_of_branch, path_of_status, push_state};
use breadboard_core::gitlab::IssueBox;
use breadboard_core::model::merge::{is_after_deploy, open_tries_of};
use serde_json::Value;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: bb status";
const ABOUT: &str = "bb status says where the work of this worktree stands: the issue, branch and base, what is
uncommitted or unpushed, wt:check, the merge request and its pipeline, the boxes left,
and the command that comes next.";
const TAKES: Takes = Takes {
    valued: &[],
    switches: &[],
    words: Some(0),
    usage: USAGE,
    command: "status",
};

/// The merge request of the branch, as far as the next step needs it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MrFacts {
    pub iid: i64,
    /// `opened`, `merged`, `closed`.
    pub state: String,
    pub is_draft: bool,
    pub url: String,
    /// Its head pipeline's status, None without one.
    pub pipeline: Option<String>,
}

/// What `bb status` and `bb submit` know of a worktree.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// In a worktree, not the main checkout.
    pub is_worktree: bool,
    pub branch: String,
    pub base: String,
    pub issue: Option<u64>,
    pub title: String,
    /// The paths `git status` lists: uncommitted changes.
    pub changed: Vec<String>,
    pub push: PushState,
    /// Commits its base doesn't have.
    pub ahead: u64,
    pub check: CheckState,
    pub mr: Option<MrFacts>,
    /// Why GitLab couldn't be read, None when it was.
    pub gitlab: Option<String>,
    pub boxes: Vec<Box>,
    /// The tries that hold the merge.
    pub tries: Vec<i64>,
    pub is_merge_posted: bool,
    /// The orchestration that started it: breadboard marks its MR ready.
    pub orchestration: Option<String>,
    /// The MRs `wt:wait` waits for.
    pub waits: Vec<i64>,
}

impl Facts {
    /// The boxes that hold the merge: open, not one the deployment checks.
    pub fn open_boxes(&self) -> Vec<(usize, &Box)> {
        self.boxes
            .iter()
            .enumerate()
            .filter(|(_, found)| !found.is_ticked && !is_after_deploy(&issue_box(found)))
            .map(|(index, found)| (index + 1, found))
            .collect()
    }

    pub fn is_open(&self) -> bool {
        self.mr.as_ref().is_some_and(|mr| mr.state == "opened")
    }
}

fn issue_box(found: &Box) -> IssueBox {
    IssueBox {
        line: found.line,
        is_ticked: found.is_ticked,
        text: found.text.clone(),
        heading: found.heading.clone(),
    }
}

/// The facts of the checkout `here`; GitLab is asked once for the MR and once for the issue.
pub fn facts_of(here: &Here) -> Facts {
    let Some(top) = here.worktree() else {
        return Facts::default();
    };
    let branch = here.branch();
    let issue = issue_of_branch(&branch);
    let changed = git(top, &["status", "--porcelain"])
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(path_of_status)
        .collect();
    let mut facts = Facts {
        is_worktree: true,
        base: base_of(top).local,
        issue,
        changed,
        push: push_state(
            top,
            Some(&branch).filter(|branch| !branch.is_empty()).map(String::as_str),
        ),
        ahead: breadboard_core::git::ahead_of_base(top),
        check: check_of(top),
        orchestration: breadboard_core::record::origin_of(top).map(|origin| origin.name),
        waits: breadboard_core::wait::wait_of(top)
            .map(|wait| wait.mrs)
            .unwrap_or_default(),
        branch,
        ..Facts::default()
    };
    let file = actions_file(top);
    let (actions, answered) = (actions_of(&file), answered_of(&file));
    facts.tries = open_tries_of(&actions, &answered)
        .iter()
        .map(|action| action.id)
        .collect();
    facts.is_merge_posted = actions.iter().any(|action| action.kind == "merge");
    read_gitlab(top, &mut facts);
    facts
}

/// The MR of the branch and the issue's title and boxes.
fn read_gitlab(top: &Path, facts: &mut Facts) {
    let mut problems = Vec::new();
    if !facts.branch.is_empty() {
        match glab::view(top, "mr", &facts.branch) {
            Ok(mr) => facts.mr = mr.as_ref().map(mr_of),
            Err(why) => problems.push(why),
        }
    }
    if let Some(number) = facts.issue {
        match glab::view(top, "issue", &number.to_string()) {
            Ok(Some(issue)) => {
                facts.title = glab::text_of(&issue, "title").to_string();
                facts.boxes = boxes_of(glab::text_of(&issue, "description"));
            }
            Ok(None) => problems.push(format!("GitLab has no issue #{number}")),
            Err(why) => problems.push(why),
        }
    }
    facts.gitlab = (!problems.is_empty()).then(|| problems.join("; "));
}

pub fn mr_of(mr: &Value) -> MrFacts {
    let pipeline = mr
        .get("head_pipeline")
        .or_else(|| mr.get("pipeline"))
        .and_then(|pipeline| pipeline.get("status"))
        .and_then(Value::as_str)
        .map(String::from);
    MrFacts {
        iid: mr.get("iid").and_then(Value::as_i64).unwrap_or_default(),
        state: glab::text_of(mr, "state").to_string(),
        is_draft: mr.get("draft").and_then(Value::as_bool).unwrap_or_else(|| {
            let title = glab::text_of(mr, "title").to_lowercase();
            title.starts_with("draft:") || title.starts_with("[draft]")
        }),
        url: glab::text_of(mr, "web_url").to_string(),
        pipeline,
    }
}

/// What comes next: why, and the command that does it (None: someone else acts).
#[derive(Debug, Clone, PartialEq)]
pub struct Next {
    pub why: String,
    pub command: Option<String>,
}

fn next(why: impl Into<String>, command: Option<&str>) -> Next {
    Next {
        why: why.into(),
        command: command.map(String::from),
    }
}

/// The one step that comes next, the first of the workflow's order that isn't done.
pub fn next_of(facts: &Facts) -> Next {
    if !facts.is_worktree {
        return next(
            "this is the main checkout: every task works in a worktree of its own",
            Some("pnpm wt:new <n>-<feature>"),
        );
    }
    if facts.branch.is_empty() {
        return next("HEAD is detached: no branch to submit", Some("git switch <branch>"));
    }
    if let Some(mr) = facts.mr.as_ref().filter(|mr| mr.state == "merged") {
        return next(
            format!(
                "!{} is merged: this branch is done, new work goes on a new branch",
                mr.iid
            ),
            Some("pnpm wt:new <n>-<feature>"),
        );
    }
    if matches!(facts.check, CheckState::Checking { .. }) {
        return next("wt:check runs: wait for its end", None);
    }
    if !facts.changed.is_empty() {
        return next(
            format!("{} uncommitted", plural(facts.changed.len(), "file")),
            Some("git add <paths> && git commit -m \"<type>(<scope>): <what>\", then bb submit"),
        );
    }
    if let CheckState::Failed { failures, .. } = &facts.check {
        return next(
            format!(
                "wt:check failed at HEAD: {}",
                breadboard_core::check::failures_text(failures)
            ),
            Some("fix it, commit, then bb submit"),
        );
    }
    if facts.ahead == 0 && facts.mr.is_none() {
        return next(
            "nothing committed yet",
            Some("make the first change, commit it, then bb submit"),
        );
    }
    let is_tested = matches!(facts.check, CheckState::Tested { .. }) || facts.ahead == 0;
    if !is_tested || facts.push.unpushed > 0 || !facts.push.has_remote_branch {
        let why = if !is_tested {
            "HEAD isn't checked yet".to_string()
        } else {
            format!("{} unpushed", plural(facts.push.unpushed.max(1) as usize, "commit"))
        };
        return next(why, Some("bb submit"));
    }
    let Some(mr) = &facts.mr else {
        return next(
            "no merge request yet",
            Some("pnpm wt:mr (it puts down .claude/mr-description.md to write first)"),
        );
    };
    if mr.state != "opened" {
        return next(
            format!("!{} is {}", mr.iid, mr.state),
            Some("pnpm wt:mr (a closed MR is history, it opens a new one)"),
        );
    }
    if mr.pipeline.as_deref() == Some("failed") {
        return next(
            format!("the pipeline of !{} failed", mr.iid),
            Some(&format!(
                "glab ci status --branch {}, then glab ci trace <job id>: fix what is this MR's, commit, bb submit",
                facts.branch
            )),
        );
    }
    let open = facts.open_boxes();
    if let Some(issue) = facts.issue.filter(|_| !open.is_empty()) {
        let numbers: Vec<String> = open.iter().map(|(number, _)| number.to_string()).collect();
        return next(
            format!("{} of #{issue} open: {}", plural(open.len(), "box"), numbers.join(", ")),
            Some(&format!(
                "do the work of each, then bb submit {} ticks it",
                numbers.first().cloned().unwrap_or_default()
            )),
        );
    }
    if let Some(first) = facts.tries.first() {
        return next(
            format!("try #{first} waits for the user's answer in the overview"),
            None,
        );
    }
    if !facts.waits.is_empty() {
        let mrs: Vec<String> = facts.waits.iter().map(|iid| format!("!{iid}")).collect();
        return next(format!("waits for {}", mrs.join(", ")), None);
    }
    match mr.pipeline.as_deref() {
        Some("success") => {}
        Some(status) => return next(format!("the pipeline of !{} is {status}", mr.iid), None),
        None if facts.orchestration.is_some() => {
            return next(
                "no pipeline yet: breadboard starts it in the runner's order (every box ticked, wt:check passed)",
                None,
            );
        }
        None => return next("no pipeline yet: GitLab starts one with the next push", None),
    }
    if mr.is_draft {
        return match facts.orchestration {
            Some(_) => next(format!("the bar is full: breadboard marks !{} ready", mr.iid), None),
            None => next(format!("!{} is a draft with a full bar", mr.iid), Some("bb submit")),
        };
    }
    if facts.orchestration.is_some() && !facts.is_merge_posted {
        return next(
            format!("!{} is done", mr.iid),
            Some("pnpm wt:action merge \"<what is in it>\""),
        );
    }
    next(format!("!{} waits for the user's merge", mr.iid), None)
}

/// The screen of `bb status`.
pub fn lines_of(facts: &Facts) -> Vec<String> {
    let mut lines = Vec::new();
    if facts.is_worktree {
        let issue = match facts.issue {
            Some(number) if facts.title.is_empty() => format!("#{number}"),
            Some(number) => format!("#{number} {}", facts.title),
            None => "no issue in the branch's name".to_string(),
        };
        lines.push(format!("\n{issue}"));
        let row = |name: &str, value: String| format!("  {name:<9} {value}");
        lines.push(row("branch", format!("{} → {}", facts.branch, facts.base)));
        if let Some(name) = &facts.orchestration {
            lines.push(row("started", format!("by the orchestration {name}")));
        }
        let mut changes = Vec::new();
        if !facts.changed.is_empty() {
            changes.push(format!("{} uncommitted", plural(facts.changed.len(), "file")));
        }
        changes.push(format!("{} on {}", plural(facts.ahead as usize, "commit"), facts.base));
        if facts.push.unpushed > 0 || !facts.push.has_remote_branch {
            changes.push(if facts.push.has_remote_branch {
                format!("{} unpushed", facts.push.unpushed)
            } else {
                "not on origin".to_string()
            });
        }
        lines.push(row("changes", changes.join(" · ")));
        lines.push(row("check", check_text(&facts.check)));
        lines.push(row(
            "MR",
            match &facts.mr {
                Some(mr) => format!(
                    "!{} {}{} {}",
                    mr.iid,
                    mr.state,
                    if mr.is_draft { ", draft" } else { "" },
                    mr.url
                ),
                None => "none".to_string(),
            },
        ));
        if let Some(mr) = &facts.mr {
            lines.push(row("pipeline", mr.pipeline.clone().unwrap_or_else(|| "none".into())));
        }
        if !facts.boxes.is_empty() {
            let open = facts.open_boxes();
            lines.push(row("boxes", format!("{} ticked", tally(&facts.boxes))));
            for (number, found) in open {
                lines.push(format!("{:>14} [ ] {}", number, boxes::cut(&found.text)));
            }
        }
        if !facts.tries.is_empty() {
            let tries: Vec<String> = facts.tries.iter().map(|id| format!("#{id}")).collect();
            lines.push(row("tries", format!("{} open", tries.join(", "))));
        }
        if !facts.waits.is_empty() {
            let mrs: Vec<String> = facts.waits.iter().map(|iid| format!("!{iid}")).collect();
            lines.push(row("waits", format!("for {}", mrs.join(", "))));
        }
        if let Some(why) = &facts.gitlab {
            lines.push(row("GitLab", format!("couldn't be read: {why}")));
        }
    }
    let next = next_of(facts);
    lines.push(String::new());
    lines.push(match &next.command {
        Some(command) => format!("next: {command}\n      ({})", next.why),
        None => format!("next: nothing for you: {}", next.why),
    });
    lines
}

fn check_text(check: &CheckState) -> String {
    match check {
        CheckState::Checking { .. } => "wt:check runs".into(),
        CheckState::Tested { packages, .. } if packages.is_empty() => "✓ HEAD passed wt:check".into(),
        CheckState::Tested { packages, .. } => format!("✓ HEAD passed wt:check ({})", packages.join(", ")),
        CheckState::Failed { failures, .. } => {
            format!("✗ failed: {}", breadboard_core::check::failures_text(failures))
        }
        CheckState::Untested { count } => {
            format!(
                "not run on HEAD ({} since the last pass)",
                plural(*count as usize, "commit")
            )
        }
        CheckState::None => "nothing to check yet".into(),
    }
}

pub fn run(argv: &Argv) -> Outcome {
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let here = Here::find();
    if here.top.is_none() {
        return Err("not in a git checkout: cd into a worktree of the repo, then bb status".into());
    }
    for line in lines_of(&facts_of(&here)) {
        say(&line);
    }
    say("");
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done() -> Facts {
        Facts {
            is_worktree: true,
            branch: "feature/7-x".into(),
            base: "develop".into(),
            issue: Some(7),
            push: PushState {
                pushed: Some("a".into()),
                has_remote_branch: true,
                ..PushState::default()
            },
            ahead: 2,
            check: CheckState::Tested {
                sha: "a".into(),
                packages: vec![],
            },
            mr: Some(MrFacts {
                iid: 9,
                state: "opened".into(),
                is_draft: false,
                url: "https://g/9".into(),
                pipeline: Some("success".into()),
            }),
            boxes: boxes_of("## Work\n- [x] one\n- [x] two (after deploy, try #2)"),
            orchestration: Some("breadboard".into()),
            ..Facts::default()
        }
    }

    fn command_of(facts: &Facts) -> Option<String> {
        next_of(facts).command
    }

    #[test]
    fn names_the_next_command_in_the_workflows_order() {
        let facts = done();
        assert_eq!(
            command_of(&facts).as_deref(),
            Some("pnpm wt:action merge \"<what is in it>\"")
        );
        let posted = Facts {
            is_merge_posted: true,
            ..done()
        };
        assert_eq!(next_of(&posted).why, "!9 waits for the user's merge");
        assert_eq!(
            command_of(&Facts::default()).as_deref(),
            Some("pnpm wt:new <n>-<feature>")
        );
        let dirty = Facts {
            changed: vec!["a.rs".into()],
            ..done()
        };
        assert!(command_of(&dirty).unwrap().starts_with("git add <paths> && git commit"));
        let unchecked = Facts {
            check: CheckState::Untested { count: 1 },
            ..done()
        };
        assert_eq!(command_of(&unchecked).as_deref(), Some("bb submit"));
        assert_eq!(next_of(&unchecked).why, "HEAD isn't checked yet");
        let unpushed = Facts {
            push: PushState {
                unpushed: 2,
                has_remote_branch: true,
                ..PushState::default()
            },
            ..done()
        };
        assert_eq!(next_of(&unpushed), next("2 commits unpushed", Some("bb submit")));
        let no_mr = Facts { mr: None, ..done() };
        assert!(command_of(&no_mr).unwrap().starts_with("pnpm wt:mr"));
        let failed = Facts {
            mr: Some(MrFacts {
                pipeline: Some("failed".into()),
                ..done().mr.unwrap()
            }),
            ..done()
        };
        assert!(
            command_of(&failed)
                .unwrap()
                .starts_with("glab ci status --branch feature/7-x")
        );
    }

    #[test]
    fn names_the_open_boxes_but_not_those_after_the_deploy() {
        let facts = Facts {
            boxes: boxes_of("## Work\n- [x] one\n- [ ] two\n- [ ] three (after deploy, try #2)"),
            ..done()
        };
        assert_eq!(
            next_of(&facts),
            next(
                "1 box of #7 open: 2",
                Some("do the work of each, then bb submit 2 ticks it")
            )
        );
        let lines = lines_of(&facts);
        assert!(lines.iter().any(|line| line == "             2 [ ] two"), "{lines:?}");
        assert!(lines.contains(&"  boxes     1/3 ticked".to_string()));
    }

    #[test]
    fn says_who_acts_when_it_isnt_the_agent() {
        let draft = Facts {
            mr: Some(MrFacts {
                is_draft: true,
                ..done().mr.unwrap()
            }),
            ..done()
        };
        assert_eq!(
            next_of(&draft),
            next("the bar is full: breadboard marks !9 ready", None)
        );
        let own = Facts {
            orchestration: None,
            ..draft.clone()
        };
        assert_eq!(command_of(&own).as_deref(), Some("bb submit"));
        let tried = Facts {
            tries: vec![3],
            ..done()
        };
        assert_eq!(
            lines_of(&tried).last().unwrap(),
            "next: nothing for you: try #3 waits for the user's answer in the overview"
        );
        let merged = Facts {
            mr: Some(MrFacts {
                state: "merged".into(),
                ..done().mr.unwrap()
            }),
            changed: vec!["x".into()],
            ..done()
        };
        assert!(next_of(&merged).why.starts_with("!9 is merged"));
    }

    #[test]
    fn shows_one_screen() {
        let lines = lines_of(&Facts {
            title: "the CLI explains itself".into(),
            ..done()
        });
        assert_eq!(lines[0], "\n#7 the CLI explains itself");
        assert_eq!(lines[1], "  branch    feature/7-x → develop");
        assert!(lines.contains(&"  check     ✓ HEAD passed wt:check".to_string()));
        assert!(lines.contains(&"  MR        !9 opened https://g/9".to_string()));
        assert_eq!(
            lines.last().unwrap(),
            "next: pnpm wt:action merge \"<what is in it>\"\n      (!9 is done)"
        );
    }
}
