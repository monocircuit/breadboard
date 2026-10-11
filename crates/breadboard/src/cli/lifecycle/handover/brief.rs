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
            "Your worktree is `{dir}` on `{branch}`: it exists, its branch is pushed as the claim. You come from another worktree: `EnterWorktree(path: \"{dir}\")` once, if you don't stand in it yet; after that don't run `wt:new`, `EnterWorktree` or `ExitWorktree`."
        ),
        None => format!(
            "Your worktree is `{dir}` on `{branch}`: it exists, its branch is pushed as the claim, you already stand in it.\nDon't run `wt:new`, `EnterWorktree` or `ExitWorktree`."
        ),
    };
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
    format!(
        r#"# #{number} {title}

You work on issue #{number} of monocircuit/platform, started by the orchestration `{name}` (herdr space `{label}`).
{worktree}

1. Read the issue: `glab issue view {number} --comments`, then the AGENTS.md files and skills it touches.
2. Work as AGENTS.md says: commit, `pnpm wt:check`, push per step (green means wt:check passed on that commit: the pre-push hook refuses any other, never `--no-verify`; a failing test is fixed; one that fails on develop too is fixed once, in its own fix MR, and the others wait for it with `pnpm wt:wait`), the draft MR with the first commit (`pnpm wt:mr`), boxes ticked with `pnpm wt:tick`. Push every commit at once, never hold one back: the overview shows unpushed commits, and past 10 minutes they go to its NEXT.
   A change requested for your MR (by the user or the orchestrator): `pnpm wt:action change "<what>"` first, it makes the MR a draft and withdraws your merge action, then build it; once it is pushed and green, post the merge again. Merged meanwhile: the command says so, and the change gets its own issue, branch and draft MR at once.
   Your MR's pipeline failed: the overview types `[overview] the pipeline of !N failed: job …, <url>: …` into this tab by itself. Read the job's log (the url, or `glab ci trace <job id>`), fix what is your MR's doing and push. When the line says develop fails the same job, it isn't yours: say so in the MR (`glab mr note`) and go on, don't chase it.
   Your issue started at once, even when it builds on an open MR or its files move in one: when you have to wait for other MRs, run `pnpm wt:wait !N … --then "<what next>"` with `run_in_background` and stop; don't ask the orchestrator. Its line wakes you once they are merged (or why the wait ended).
3. **The overview shows your work, you don't report it.** Your node in its WORKTREES shows herdr's state of your agent and what you wait for, with commits, MR and pipeline, under its title the problem from your issue's Goal or Bug and what you are on now; MERGE REQUESTS shows the boxes: no status message.
   At every stage, `pnpm wt:action note "<one line>"`: what you are on, in plain words for the user (`cause found: …`, `fix pushed, waiting for the pipeline`, `waits for !N`). Your node shows the newer of it and the first line of your last answer.
   `SendMessage` to the orchestrator only for what needs it, one short message:
   - the plan, when it differs from the issue;
   - a decision that is the user's (options, your recommendation), then wait for the answer instead of guessing; with its `decide` action posted, the message is only context: the user answers on the action's buttons, and the orchestrator relays it only while the action is open;
   - what concerns another agent's work (an order, a conflict, a dependency).
   No message for a stage end or the end: the overview has them.
   {peer_line}
4. **Whenever you stop and the next step is the user's, leave an action for it** with `pnpm wt:action`. The overview of the orchestration answers "what now?" from these (its NEXT line and ACTIONS): the user must never have to ask you what is next.
   - `pnpm wt:action merge "<what is in it>"` only once the work is done: the pipeline of the MR passed, every box of the issue is ticked (Work and acceptance criteria), every `try` you posted was answered `looks good`, and it is its turn in the merge order (`wt:plan`). Then mark it ready (`glab mr update --ready`) and post it yourself, the overview's automatic one is only a fallback; when your permissions block `--ready`, post it anyway with "mark ready + merge" in the text. While a try is open the overview offers no merge. Never merge yourself;
   - `pnpm wt:action try "<what to check>" --run "<command>"` (a terminal, e.g. `pnpm overview --orchestration <name>` of your branch) or `--url <page>` (`/atlas/…` on your worktree's app, its stack started first) for something only a person can judge: the click opens that spot. Every box you leave open for the user's live check gets one, and the box names it (`… (try #<id>)`); tick it once the answer is `looks good`. A check that needs the deployment (HTTPS, the test instance's data) is `--on test --url <path>`, posted with the merge, its box marked `(after deploy, try #<id>)`: the only check that stays open across the merge; the row shows `waits for merge`, `deploying`, then `ready` by itself. A `broken` puts the MR back to draft (the overview does it when the MR was ready, else `glab mr update --draft`) until the fix is pushed, green and tried again with a new try;
   - `pnpm wt:action decide "<what is decided, one sentence>? <why the recommendation, one line>" --option "<label>" --why "<what happens>" --option "<label>" --why "<what happens>" --recommend <label>`: the user answers from the row, so the question says precisely what is decided, each option a short label (the answer you get) with what happens as its `--why` (or `--option "<label>: <what happens>"`), not `A`/`B`, and `--recommend` marks yours `(Recommended)`. Together with the SendMessage to the orchestrator that explains them;
   - `pnpm wt:action plan "<the plan in one line>"` before larger work, for an `ok` or `different`;
   - `pnpm wt:action look "<what>"` for anything else the user has to do.
   The user's click arrives typed into this tab as one line, `[overview] #<id> <kind> "<text>" → <answer>`: go on with it, don't wait or poll for it. A `broken` or `different` comes as `… → broken: <what is wrong>`, written by the user, maybe with images: until it arrives, wait for it instead of guessing from the bare word. The answered action goes by itself. Once one no longer applies (done by the user, overtaken), `pnpm wt:action --done <id>` (`--list` shows them). While you are working, leave none.
5. The user may type into this tab directly: their word wins over the orchestrator's.
6. Your next issue may come as the first prompt of your conversation, resumed in its worktree under its name: `[overview] next issue #<n>, your name is <n-feature> now: your conversation runs again in its worktree <dir>, read .claude/brief.md`: do just that, the brief there is your task from then on. The orchestrator and the other agents reach you under that name, a message to your old name is about your old issue.
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
        assert!(brief.starts_with("# #12 a title\n\nYou work on issue #12 of monocircuit/platform, started by the orchestration `o` (herdr space `⇶ O`).\nYour worktree is `/r/p-12-a-title` on `feature/12-a-title`: it exists, its branch is pushed as the claim, you already stand in it.\nDon't run"));
        assert!(brief.contains("\n1. Read the issue: `glab issue view 12 --comments`, then"));
        assert!(brief.contains("Before that, `ListAgents` finds it (it works in `/r/p`)."));
        assert!(brief.contains("\n   At every stage, `pnpm wt:action note \"<one line>\"`: what you are on"));
        assert!(brief.ends_with("read .claude/brief.md`: do just that, the brief there is your task from then on. The orchestrator and the other agents reach you under that name, a message to your old name is about your old issue.\n"));
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
        assert!(brief.contains("You come from another worktree: `EnterWorktree(path: \"/r/p-12-a-title\")` once"));
        assert!(brief.contains("   The orchestrator is the session `s-1` (`ListAgents`).\n"));
        assert!(brief.ends_with(
            ".\n\n## Before this issue\n\nYou are a reused agent: x.\n\n## From the orchestrator\n\nmore\n"
        ));
    }
}
