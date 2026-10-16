//! The brief of an orchestrated agent (orchestrator.mjs `briefOf`).

use super::super::proc::text;
use super::Issue;
use std::path::Path;

/// What an orchestrated agent is told, `.claude/brief.md` (orchestrator.mjs
/// `briefOf`): verbatim, agents read it (rewrite rule 5).
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
    let worktree = match before {
        Some(_) => format!(
            "Your worktree is `{dir}` on `{branch}`: it exists, its branch is pushed as the claim. You come from another worktree: before any work `git rev-parse --show-toplevel` prints `{dir}`; when it prints another directory, `ExitWorktree(action: \"keep\")`, then `EnterWorktree(path: \"{dir}\")`, once. After that don't run `wt:new`, `EnterWorktree` or `ExitWorktree`."
        ),
        None => format!(
            "Your worktree is `{dir}` on `{branch}`: it exists, its branch is pushed as the claim, you already stand in it.\nDon't run `wt:new`, `EnterWorktree` or `ExitWorktree`."
        ),
    };
    // One issue, one MR, into its base (#349).
    // Its worktree may not exist yet: what wt:new will base it on, read from the main checkout (#380).
    let base = breadboard_core::record::base_for(main, name, &issue.dir).local;
    let worktree = format!(
        "{worktree}\nIts base is `{base}`: its MR goes into it. Merge it in only when your MR has a conflict with it or the overview/orchestrator tells you to (it moving is no reason: each merge costs a full pipeline). That MR handles exactly this one issue (`Closes #{number}`, no other)."
    );
    let peer_line = match peer {
        Some(peer) => format!("The orchestrator is the session `{peer}` (`ListAgents`)."),
        None => format!(
            "The orchestrator messages you first: reply to the `from` of its message. Before that, `ListAgents` finds it (it works in `{}`).",
            text(main)
        ),
    };
    let mut tail = String::new();
    if let Some(before) = before {
        tail.push_str(&format!("\n## Before this issue\n\n{before}\n"));
    }
    if !extra.trim().is_empty() {
        tail.push_str(&format!("\n## From the orchestrator\n\n{}\n", extra.trim()));
    }
    let loaded = breadboard_core::config::of(main);
    let (repo, run, branch_name) = (
        loaded.name(),
        loaded.config.repo.run.clone().unwrap_or_else(|| "breadboard".into()),
        loaded.branch.clone(),
    );
    format!(
        r#"# #{number} {title}

You work on issue #{number} of {repo}, started by the orchestration `{name}` (herdr space `{label}`).
{worktree}
You run headless: breadboard resumes your conversation (`claude -p`) whenever something comes for you, and nothing runs while you wait. Every line for you (an answer, a merge, a failed pipeline, the end of a wait) starts your next run; when you stop, you are done until then.

1. Read the issue: `glab issue view {number} --comments`, then the AGENTS.md files and skills it touches.
2. Work as AGENTS.md says: commit, `{run} wt:check`, push per step (green means wt:check passed on that commit: the pre-push hook refuses any other, never `--no-verify`; a failing test is fixed; one that fails on {branch_name} too is fixed once, in its own fix MR, and the others wait for it with `{run} wt:wait`), the draft MR with the first commit (`{run} wt:mr`), boxes ticked with `{run} wt:tick`. Push every commit at once, never hold one back: the overview shows unpushed commits, and past 10 minutes they go to its NEXT.
   A change requested for your MR (by the user or the orchestrator): `{run} wt:action change "<what>"` first, it makes the MR a draft and withdraws your merge action, then build it; once it is pushed and green, post the merge again. Merged meanwhile: the command says so, and the change gets its own issue, branch and draft MR at once.
   Your pushes skip CI (#357): breadboard starts your MR's pipeline once every box of the issue is ticked (after-deploy ones excepted) and `{run} wt:check` passed at HEAD, in the runner's order; until then `wt:check` is the gate. Don't start one yourself.
   Your MR's pipeline failed: you get the line `[overview] the pipeline of !N failed: job …, <url>: …` by itself. Read the job's log (the url, or `glab ci trace <job id>`), fix what is your MR's doing and push. When the line says {branch_name} fails the same job, it isn't yours: say so in the MR (`glab mr note`) and go on, don't chase it.
   Your issue started at once, even when it builds on an open MR or its files move in one: when you have to wait for other MRs, run `{run} wt:wait !N … --then "<what next>"` and stop: it records the wait and returns at once; don't ask the orchestrator. Its line starts your next run once they are merged (or says why the wait ended).
3. **The overview shows your work, you don't report it.** Your block in its WORK shows your state and what you wait for, with commits, MR, pipeline and the boxes, under it the problem from your issue's Goal or Bug and what you are on now: no status message.
   At every stage, `{run} wt:action note "<one line>"`: what you are on, in plain words for the user (`cause found: …`, `fix pushed, waiting for the pipeline`, `waits for !N`). Your block shows the newer of it and the first line of your last answer.
   `SendMessage` to the orchestrator only for what needs it, one short message:
   - the plan, when it differs from the issue;
   - a decision that is the user's (options, your recommendation), then wait for the answer instead of guessing; with its `decide` action posted, the message is only context: the user answers on the action's buttons, and the orchestrator relays it only while the action is open;
   - what concerns another agent's work (an order, a conflict, a dependency).
   No message for a stage end or the end: the overview has them.
   {peer_line}
4. **Whenever you stop and the next step is the user's, leave an action for it** with `{run} wt:action`. The overview of the orchestration answers "what now?" from these (its NEXT line and ACTIONS): the user must never have to ask you what is next.
   - `{run} wt:action merge "<what is in it>"` only once the work is done: the pipeline of the MR passed, every box of the issue is ticked (Work and acceptance criteria), every `try` you posted was answered `looks good`, and it is its turn in the merge order (`wt:plan`). Then mark it ready (`glab mr update --ready`) and post it yourself, the overview's automatic one is only a fallback; when your permissions block `--ready`, post it anyway with "mark ready + merge" in the text. While a try is open the overview offers no merge. Never merge yourself;
   - `{run} wt:action try "<what to check>" --run "<command>"` (a terminal, e.g. `{run} overview --orchestration <name>` of your branch) or `--url <page>` (`/atlas/…` on your worktree's app, its stack started first) for something only a person can judge: the click opens that spot. Every box you leave open for the user's live check gets one, and the box names it (`… (try #<id>)`); tick it once the answer is `looks good`. A check that needs the deployment (HTTPS, the test instance's data) is `--on test --url <path>`, posted with the merge, its box marked `(after deploy, try #<id>)`: the only check that stays open across the merge; the row shows `waits for merge`, `deploying`, then `ready` by itself. A `broken` puts the MR back to draft (the overview does it when the MR was ready, else `glab mr update --draft`) until the fix is pushed, green and tried again with a new try;
   - `{run} wt:action decide "<what is decided, one sentence>? <why the recommendation, one line>" --option "<label>" --why "<what happens>" --option "<label>" --why "<what happens>" --recommend <label>`: the user answers from the row, so the question says precisely what is decided, each option a short label (the answer you get) with what happens as its `--why` (or `--option "<label>: <what happens>"`), not `A`/`B`, and `--recommend` marks yours `(Recommended)`. Together with the SendMessage to the orchestrator that explains them. `AskUserQuestion` is off in headless runs (#360): a question for the user is always a `decide`;
   - `{run} wt:action plan "<the plan in one line>"` before larger work, for an `ok` or `different`;
   - `{run} wt:action look "<what>"` for anything else the user has to do.
   The user's click arrives as one line in your next run, `[overview] #<id> <kind> "<text>" → <answer>`: go on with it, don't wait or poll for it. A `broken` or `different` comes as `… → broken: <what is wrong>`, written by the user: work from that, never from the bare word. The answered action goes by itself. Once one no longer applies (done by the user, overtaken), `{run} wt:action --done <id>` (`--list` shows them). While you are working, leave none.
   A tool call auto mode would ask the user about is denied in your run, and the overview asks the user for you (a `permission` action): say in your note what you need it for and stop. Your next line is `[overview] the user allowed <rule>, go on: it is allowed in this run.` or `[overview] the user denied <rule>: don't run it, go on without it or leave an action for the user.` A call the classifier or a deny rule blocks never reaches the user: go on without it.
5. The user may open your conversation in a tab and type there: their word wins over the orchestrator's.
6. Your next issue may come as a line of your next run, which then stands in its worktree under its name: `[overview] next issue #<n>, your name is <n-feature> now: your conversation runs again in its worktree <dir>. When `git rev-parse --show-toplevel` prints another directory: `ExitWorktree(action: "keep")`, then `EnterWorktree(path: "<dir>")`. Then read .claude/brief.md`: do just that, the brief there is your task from then on. The orchestrator and the other agents reach you under that name, a message to your old name is about your old issue.
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
            dir: PathBuf::from("/r/p-12-a-title"),
            why: None,
            branch: "feature/12-a-title".into(),
            milestone: None,
        }
    }

    #[test]
    fn a_fresh_agent_stands_in_its_worktree() {
        let brief = brief_of(&issue(), "o", "⇶ O", None, "", None, Path::new("/r/p"));
        assert!(brief.starts_with("# #12 a title\n\nYou work on issue #12 of p, started by the orchestration `o` (herdr space `⇶ O`).\nYour worktree is `/r/p-12-a-title` on `feature/12-a-title`: it exists, its branch is pushed as the claim, you already stand in it.\nDon't run"));
        assert!(brief.contains("\n1. Read the issue: `glab issue view 12 --comments`, then"));
        assert!(brief.contains("Before that, `ListAgents` finds it (it works in `/r/p`)."));
        assert!(brief.contains("\n   At every stage, `breadboard wt:action note \"<one line>\"`: what you are on"));
        // A headless run can't show AskUserQuestion: questions go to the overview (#360).
        assert!(brief.contains(
            "`AskUserQuestion` is off in headless runs (#360): a question for the user is always a `decide`;"
        ));
        assert!(brief.ends_with("Then read .claude/brief.md`: do just that, the brief there is your task from then on. The orchestrator and the other agents reach you under that name, a message to your old name is about your old issue.\n"));
    }

    #[test]
    fn a_reused_agent_learns_what_it_had_before() {
        let brief = brief_of(
            &issue(),
            "o",
            "⇶ o",
            Some("s-1"),
            "  more\n",
            Some("You are a reused agent: x."),
            Path::new("/r/p"),
        );
        assert!(brief.contains("\nYou run headless: breadboard resumes your conversation (`claude -p`)"));
        assert!(brief.contains("\n   A tool call auto mode would ask the user about is denied in your run"));
        assert!(!brief.contains("run_in_background"));
        assert!(!brief.contains("this tab"));
        assert!(brief.contains("You come from another worktree: before any work `git rev-parse --show-toplevel` prints `/r/p-12-a-title`; when it prints another directory, `ExitWorktree(action: \"keep\")`, then `EnterWorktree(path: \"/r/p-12-a-title\")`, once."));
        assert!(brief.contains("   The orchestrator is the session `s-1` (`ListAgents`).\n"));
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
        std::fs::write(main.join("breadboard.toml"), "[repo]\ndefault_branch = \"develop\"\n").unwrap();
        let brief = brief_of(&issue(), "o", "⇶ O", None, "", None, &main);
        assert!(brief.contains("Its base is `develop`: merge it in"), "{brief}");
        let record = breadboard_core::record::record_file(&main, "o");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, r#"{"issues": [], "base": "epic/15-o"}"#).unwrap();
        let brief = brief_of(&issue(), "o", "⇶ O", None, "", None, &main);
        assert!(brief.contains("Its base is `epic/15-o`: merge it in"), "{brief}");
        let _ = std::fs::remove_dir_all(&main);
    }
}
