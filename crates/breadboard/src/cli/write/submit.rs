//! `bb submit [<box>…]` (#427): the step after a commit, whole: wt:check
//! (unless HEAD passed it), the push, the boxes given ticked, and the MR
//! ready once its bar is full, or exactly what it still lacks. It ends with
//! the next command, as `bb status` names it. Uncommitted changes are
//! refused: what is pushed is what was checked, and a commit is staged by
//! path by whoever made the change.

use super::common::{Argv, Here, Outcome, Takes, help, number_of, plural, say};
use super::glab;
use super::status::{Facts, facts_of, next_of};
use breadboard_core::check::CheckState;
use breadboard_core::git::git;
use std::path::Path;
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: bb submit [<box>…]   (the boxes of the branch's issue whose work is done)";
const ABOUT: &str = "bb submit runs wt:check on HEAD (unless it passed already), pushes, ticks the boxes given,
and marks the MR ready once every box is ticked, or says exactly what is missing.
Commit first: uncommitted changes are refused.";
const TAKES: Takes = Takes {
    valued: &[],
    switches: &[],
    words: None,
    usage: USAGE,
    command: "submit",
};

/// How many uncommitted files the refusal names.
const NAMED_FILES: usize = 10;

pub fn run(argv: &Argv) -> Outcome {
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let wanted: Vec<&str> = argv.all().iter().map(String::as_str).collect();
    if let Some(given) = wanted.iter().find(|given| number_of(given) != Some(**given)) {
        return Err(format!(
            "{given} is no box number: bb status lists the open ones, numbered\n{USAGE}"
        ));
    }
    let here = Here::find();
    let Some(top) = here.worktree().map(Path::to_path_buf) else {
        return Err(match here.top {
            Some(_) => "bb submit runs in the worktree of a task, not in the main checkout: pnpm wt:new <n>-<feature>, then work there".into(),
            None => "not in a git checkout: cd into a worktree of the repo, then bb submit".into(),
        });
    };
    let facts = facts_of(&here);
    refusal_of(&facts, &wanted)?;

    if facts.ahead > 0 {
        check(&top, &facts)?;
        push(&top, &facts)?;
    }
    if let (Some(issue), false) = (facts.issue, wanted.is_empty()) {
        let mut args = vec!["wt:tick".to_string(), issue.to_string()];
        args.extend(wanted.iter().map(|box_| box_.to_string()));
        if !breadboard(&top, &args) {
            return Err(format!(
                "wt:tick couldn't tick {} (above says why): fix that, then bb submit {}",
                wanted.join(", "),
                wanted.join(" ")
            ));
        }
    }

    // Read again: the push and the ticks changed what GitLab and git say.
    let facts = facts_of(&here);
    for line in bar_of(&top, &facts)? {
        say(&line);
    }
    let next = next_of(&facts);
    say(&match next.command {
        Some(command) => format!("\nnext: {command}\n      ({})\n", next.why),
        None => format!("\nnext: nothing for you: {}\n", next.why),
    });
    Ok(ExitCode::SUCCESS)
}

/// Why `bb submit` doesn't start, before anything runs.
fn refusal_of(facts: &Facts, wanted: &[&str]) -> Result<(), String> {
    if facts.branch.is_empty() {
        return Err("HEAD is detached: git switch <branch>, then bb submit".into());
    }
    if facts.branch == facts.base {
        return Err(format!(
            "{} is the base, nothing is submitted to it: pnpm wt:new <n>-<feature>",
            facts.branch
        ));
    }
    if let Some(mr) = facts.mr.as_ref().filter(|mr| mr.state == "merged") {
        return Err(format!(
            "!{} of {} is merged already: new work goes on a new branch, its own issue first (glab issue create), then pnpm wt:new <n>-<feature>\n  {}",
            mr.iid, facts.branch, mr.url
        ));
    }
    if !facts.changed.is_empty() {
        let mut named: Vec<String> = facts
            .changed
            .iter()
            .take(NAMED_FILES)
            .map(|path| format!("    {path}"))
            .collect();
        if facts.changed.len() > NAMED_FILES {
            named.push(format!("    … {} more (git status)", facts.changed.len() - NAMED_FILES));
        }
        return Err(format!(
            "{} uncommitted: what is pushed is what wt:check checked.\n{}\n  Commit them by path (git add <paths> && git commit -m \"<type>(<scope>): <what>\"), then bb submit {}",
            plural(facts.changed.len(), "file"),
            named.join("\n"),
            wanted.join(" ")
        )
        .trim_end()
        .to_string());
    }
    if !wanted.is_empty() && facts.issue.is_none() {
        return Err(format!(
            "{} carries no issue number, so it has no boxes: pnpm wt:tick <issue> {}",
            facts.branch,
            wanted.join(" ")
        ));
    }
    Ok(())
}

/// wt:check of HEAD, unless it passed already.
fn check(top: &Path, facts: &Facts) -> Result<(), String> {
    if matches!(facts.check, CheckState::Tested { .. }) {
        say("\n✓ HEAD passed wt:check already");
        return Ok(());
    }
    if breadboard(top, &["wt:check".to_string()]) {
        return Ok(());
    }
    Err("wt:check failed (above): fix it, commit, then bb submit again. Red on develop too: it is fixed once, in its own fix MR, and you wait for that with pnpm wt:wait !<mr>".into())
}

/// The push of the branch, its upstream set when it has none.
fn push(top: &Path, facts: &Facts) -> Result<(), String> {
    if facts.push.has_remote_branch && facts.push.unpushed == 0 {
        say("\n✓ nothing to push: origin has HEAD");
        return Ok(());
    }
    let has_upstream = git(top, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_ok;
    let mut args = vec!["push"];
    if !has_upstream {
        args.extend(["-u", "origin", &facts.branch]);
    }
    say(&format!("\n▸ git {}", args.join(" ")));
    let is_pushed = Command::new("git")
        .args(&args)
        .current_dir(top)
        .status()
        .is_ok_and(|status| status.success());
    if is_pushed {
        return Ok(());
    }
    Err("git push was refused (above says why): do what it names, then bb submit again".into())
}

/// This binary's `args` in `top`, its output shown; whether it succeeded.
fn breadboard(top: &Path, args: &[String]) -> bool {
    let entry = std::env::current_exe().unwrap_or_else(|_| "breadboard".into());
    Command::new(entry)
        .args(args)
        .current_dir(top)
        .env(super::super::setup::PLUGIN_CHECKED, "1")
        .status()
        .is_ok_and(|status| status.success())
}

/// What the bar of the MR says: full and ready, or each thing it lacks.
fn bar_of(top: &Path, facts: &Facts) -> Result<Vec<String>, String> {
    let Some(mr) = facts.mr.as_ref().filter(|_| facts.is_open()) else {
        return Ok(vec!["\n! no open merge request yet: pnpm wt:mr opens the draft".into()]);
    };
    let open = facts.open_boxes();
    let mut lines = Vec::new();
    if !open.is_empty() {
        let issue = facts.issue.map(|issue| format!(" of #{issue}")).unwrap_or_default();
        lines.push(format!(
            "\n! the bar of !{} isn't full: {}{issue} open",
            mr.iid,
            plural(open.len(), "box")
        ));
        for (number, found) in &open {
            lines.push(format!("    {number} [ ] {}", super::boxes::cut(&found.text)));
        }
        lines.push("  Tick each once its work is done: bb submit <box>…".into());
    }
    for id in &facts.tries {
        lines.push(format!("\n! try #{id} is open: the user answers it in the overview"));
    }
    if !lines.is_empty() {
        return Ok(lines);
    }
    if !mr.is_draft {
        return Ok(vec![format!("\n✓ !{} is ready, every box ticked", mr.iid)]);
    }
    if facts.orchestration.is_some() {
        // breadboard serve marks it, and puts it back when a box is unticked (#399).
        return Ok(vec![format!(
            "\n✓ every box ticked: breadboard marks !{} ready by itself",
            mr.iid
        )]);
    }
    let ran = glab::glab(top, &["mr", "update", &mr.iid.to_string(), "--ready"]);
    if !ran.is_ok {
        return Err(format!(
            "glab couldn't mark !{} ready: {}. Mark it in GitLab, or glab mr update {} --ready",
            mr.iid,
            ran.why(),
            mr.iid
        ));
    }
    Ok(vec![format!("\n✓ every box ticked: !{} is marked ready", mr.iid)])
}

#[cfg(test)]
mod tests {
    use super::super::status::MrFacts;
    use super::*;

    fn facts() -> Facts {
        Facts {
            is_worktree: true,
            branch: "feature/7-x".into(),
            base: "develop".into(),
            issue: Some(7),
            ..Facts::default()
        }
    }

    #[test]
    fn refuses_what_it_cant_submit_with_the_command_that_fixes_it() {
        assert_eq!(refusal_of(&facts(), &["2"]), Ok(()));
        let dirty = Facts {
            changed: (0..12).map(|n| format!("f{n}.rs")).collect(),
            ..facts()
        };
        let refused = refusal_of(&dirty, &["2"]).unwrap_err();
        assert!(refused.starts_with("12 files uncommitted: what is pushed is what wt:check checked.\n    f0.rs\n"));
        assert!(refused.contains("    … 2 more (git status)\n"));
        assert!(refused.ends_with("git commit -m \"<type>(<scope>): <what>\"), then bb submit 2"));
        let merged = Facts {
            mr: Some(MrFacts {
                iid: 9,
                state: "merged".into(),
                url: "https://g/9".into(),
                ..MrFacts::default()
            }),
            ..facts()
        };
        assert!(
            refusal_of(&merged, &[])
                .unwrap_err()
                .contains("then pnpm wt:new <n>-<feature>\n  https://g/9")
        );
        let detached = Facts {
            branch: String::new(),
            ..facts()
        };
        assert!(
            refusal_of(&detached, &[])
                .unwrap_err()
                .starts_with("HEAD is detached: git switch")
        );
        let nameless = Facts { issue: None, ..facts() };
        assert!(
            refusal_of(&nameless, &["3"])
                .unwrap_err()
                .ends_with("pnpm wt:tick <issue> 3")
        );
    }

    #[test]
    fn says_what_the_bar_lacks() {
        let open = Facts {
            mr: Some(MrFacts {
                iid: 9,
                state: "opened".into(),
                is_draft: true,
                ..MrFacts::default()
            }),
            boxes: super::super::boxes::boxes_of("## Work\n- [x] one\n- [ ] two\n- [ ] three (after deploy)"),
            tries: vec![4],
            orchestration: Some("breadboard".into()),
            ..facts()
        };
        let top = Path::new("/");
        assert_eq!(
            bar_of(top, &open).unwrap(),
            [
                "\n! the bar of !9 isn't full: 1 box of #7 open",
                "    2 [ ] two",
                "  Tick each once its work is done: bb submit <box>…",
                "\n! try #4 is open: the user answers it in the overview",
            ]
        );
        let full = Facts {
            boxes: super::super::boxes::boxes_of("- [x] one"),
            tries: vec![],
            ..open.clone()
        };
        assert_eq!(
            bar_of(top, &full).unwrap(),
            ["\n✓ every box ticked: breadboard marks !9 ready by itself"]
        );
        assert_eq!(
            bar_of(top, &facts()).unwrap(),
            ["\n! no open merge request yet: pnpm wt:mr opens the draft"]
        );
    }
}
