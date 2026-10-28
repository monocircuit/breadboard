//! The brief of an orchestrated agent (orchestrator.mjs `briefOf`).

use super::super::proc::text;
use super::Issue;
use std::path::Path;

/// What an orchestrated agent is told, `.claude/brief.md`: what it can't look
/// up, nothing else (#429). The rules it used to repeat are in the skill's
/// `agent.md` or enforced; the brief's tokens are paid by every agent.
#[allow(clippy::too_many_arguments)]
pub fn brief_of(
    issue: &Issue,
    name: &str,
    label: &str,
    peer: Option<&str>,
    extra: &str,
    before: Option<&str>,
    main: &Path,
) -> String {
    let (number, title, dir, branch) = (&issue.number, &issue.title, text(&issue.dir), &issue.branch);
    // Its worktree may not exist yet: what wt:new will base it on, read from the main checkout (#380).
    let base = breadboard_core::record::base_for(main, name, &issue.dir).local;
    let loaded = breadboard_core::config::of(main);
    let (repo, bb) = (loaded.name(), loaded.bb());
    // A reused agent stands in its old worktree until it moves (#359).
    let stand = match before {
        Some(_) => format!(
            "You come from another worktree: when `git rev-parse --show-toplevel` prints another directory than `{dir}`, `ExitWorktree(action: \"keep\")`, then `EnterWorktree(path: \"{dir}\")`, once."
        ),
        None => "You stand in it.".into(),
    };
    let peer = match peer {
        Some(peer) => format!("the session `{peer}`"),
        None => format!("the one in `{}` (`ListAgents`; it messages you first)", text(main)),
    };
    // A bundle (#321): one task, one MR, every issue of it closed.
    let (work, closes) = if issue.also.is_empty() {
        (
            format!("Issue #{number} of {repo} (`glab issue view {number} --comments`)"),
            format!("#{number}"),
        )
    } else {
        let numbers = issue.numbers();
        let views: Vec<String> = numbers
            .iter()
            .map(|each| format!("`glab issue view {each} --comments`"))
            .collect();
        let others: Vec<String> = issue
            .also
            .iter()
            .map(|each| format!("`{bb} wt:tick {each} <box>…`"))
            .collect();
        (
            format!(
                "Issues {} of {repo} as one task ({})",
                breadboard_core::gitlab::issues_text(&numbers),
                views.join(", ")
            ),
            format!(
                "{}, a `Closes` line each; `submit` ticks #{number}'s boxes, the others' {}",
                breadboard_core::gitlab::issues_text(&numbers),
                others.join(", ")
            ),
        )
    };
    let mut tail = String::new();
    if let Some(before) = before {
        tail.push_str(&format!("\n## Before this issue\n\n{before}\n"));
    }
    if !extra.trim().is_empty() {
        tail.push_str(&format!("\n## From the orchestrator\n\n{}\n", extra.trim()));
    }
    // Only what this agent can't look up (#429): the rules are in the skill
    // (agent.md) or enforced, the state in `bb status` (#427).
    format!(
        r#"# #{number} {title}

{work}, orchestration `{name}` (herdr space `{label}`).
Branch `{branch}` in `{dir}`, pushed as the claim. {stand}
Base `{base}`: your one MR goes into it and closes only {closes}.

- Each step: commit it (staged by path), then `{bb} submit [<box>…]`: checks, pushes, ticks.
- Where you are and what is next: `{bb} status`.
- How an orchestrated agent works (headless runs, waits, actions for the user): `.claude/skills/parallel-work/agent.md`. Read it before the first commit.
- The orchestrator: {peer}.
{tail}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn issue() -> Issue {
        Issue {
            number: 12,
            title: "a title".into(),
            description: String::new(),
            feature: "12-a-title".into(),
            is_fix: false,
            is_hotfix: false,
            dir: PathBuf::from("/r/p-12-a-title"),
            why: None,
            branch: "feature/12-a-title".into(),
            milestone: None,
            stage: breadboard_core::stage::Stage::Build,
            also: vec![],
        }
    }

    /// The whole brief (#429): issue, branch, base, the two commands and
    /// where the rest is. A line added here is paid by every agent.
    #[test]
    fn a_fresh_agent_gets_issue_branch_base_and_the_commands() {
        let brief = brief_of(&issue(), "o", "⇶ O", None, "", None, Path::new("/r/p"));
        assert_eq!(
            brief,
            "# #12 a title

Issue #12 of p (`glab issue view 12 --comments`), orchestration `o` (herdr space `⇶ O`).
Branch `feature/12-a-title` in `/r/p-12-a-title`, pushed as the claim. You stand in it.
Base `main`: your one MR goes into it and closes only #12.

- Each step: commit it (staged by path), then `breadboard submit [<box>…]`: checks, pushes, ticks.
- Where you are and what is next: `breadboard status`.
- How an orchestrated agent works (headless runs, waits, actions for the user): `.claude/skills/parallel-work/agent.md`. Read it before the first commit.
- The orchestrator: the one in `/r/p` (`ListAgents`; it messages you first).
"
        );
    }

    #[test]
    fn a_bundle_closes_every_issue_of_its_task() {
        let bundle = Issue {
            also: vec![13],
            ..issue()
        };
        let brief = brief_of(&bundle, "o", "⇶ O", None, "", None, Path::new("/r/p"));
        assert!(brief.contains("\nIssues #12 and #13 of p as one task (`glab issue view 12 --comments`, `glab issue view 13 --comments`), orchestration `o`"), "{brief}");
        assert!(brief.contains("closes only #12 and #13, a `Closes` line each; `submit` ticks #12's boxes, the others' `breadboard wt:tick 13 <box>…`.\n"), "{brief}");
    }

    #[test]
    fn a_reused_agent_moves_and_learns_what_it_had_before() {
        let brief = brief_of(
            &issue(),
            "o",
            "⇶ o",
            Some("s-1"),
            "  more\n",
            Some("You are a reused agent: x."),
            Path::new("/r/p"),
        );
        assert!(brief.contains("pushed as the claim. You come from another worktree: when `git rev-parse --show-toplevel` prints another directory than `/r/p-12-a-title`, `ExitWorktree(action: \"keep\")`, then `EnterWorktree(path: \"/r/p-12-a-title\")`, once.\n"), "{brief}");
        assert!(brief.contains("\n- The orchestrator: the session `s-1`.\n"), "{brief}");
        assert!(brief.ends_with(
            ".\n\n## Before this issue\n\nYou are a reused agent: x.\n\n## From the orchestrator\n\nmore\n"
        ));
    }

    /// #380: the brief is written before its worktree exists, from whatever
    /// directory wt:orchestrate runs in; its base is the main checkout's
    /// default branch (or the record's epic), never a fallback of a path that isn't there.
    #[test]
    fn names_the_base_of_the_main_checkout_before_the_worktree_exists() {
        let main = std::env::temp_dir().join(format!("brief-base-{}", std::process::id()));
        std::fs::create_dir_all(main.join(".git")).unwrap();
        std::fs::write(
            main.join("breadboard.toml"),
            "[repo]\ndefault_branch = \"develop\"\nrun = \"pnpm\"\n",
        )
        .unwrap();
        let brief = brief_of(&issue(), "o", "⇶ O", None, "", None, &main);
        assert!(brief.contains("Base `develop`: your one MR goes into it"), "{brief}");
        // Run through pnpm: `bb`, the monorepo's shim on every agent's PATH (#427).
        assert!(brief.contains("then `bb submit [<box>…]`"), "{brief}");
        assert!(brief.contains("`bb status`"), "{brief}");
        let record = breadboard_core::record::record_file(&main, "o");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, r#"{"issues": [], "base": "epic/15-o"}"#).unwrap();
        let brief = brief_of(&issue(), "o", "⇶ O", None, "", None, &main);
        assert!(brief.contains("Base `epic/15-o`: your one MR goes into it"), "{brief}");
        let _ = std::fs::remove_dir_all(&main);
    }
}
