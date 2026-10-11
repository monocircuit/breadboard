# Rules for the rewrite loop (#307)

The rewrite of breadboard runs as a gauntlet loop (user, 2026-10-01): a
builder and a separate harsh critic per piece, a blind comparison against
**lazygit**, looping until ours wins and
[the black-box suite](../../breadboard-gauntlet/README.md) is green. It is
not steered by questions. **The stack is set: Rust with ratatui**
(crossterm, tokio), for all of breadboard (user, 2026-10-01; the evidence:
[`stack-evidence.md`](stack-evidence.md), the plan:
[`rewrite-plan.md`](rewrite-plan.md)). Everything else the loop chooses by
these rules, in this order, and writes the choice to its decision log.

## Deciding

1. **The suite is the spec.** A check of `breadboard-gauntlet` that fails is
   the next thing to fix. A check marked `known bug #N` must pass in the
   rewrite: it describes how it should be, not how it is.
2. **An open issue's acceptance criteria win** over today's behaviour where
   they differ (`glab issue list --milestone "breadboard · Epic 15: The dev workflow"`).
   Read the issue with its comments; the latest comment of the user wins.
3. **Otherwise parity with today.** Every section, line, button, key and
   command of today's breadboard exists in the rewrite and does the same
   (the suite's screens and click checks are the list). Dropping one is a
   decision for the log with the reason, never a silent loss.
4. **Ties go to the simpler**: fewer lines, fewer dependencies, fewer
   processes, one way to do a thing. Between two equally simple: the one a
   person reading the code understands without the log.
5. **Irreversible things stay compatible**: the data formats other code
   and other sessions read (`.claude/orchestrations/*.json`, `.claude/actions.json`,
   `.claude/origin`, `.claude/brief.md`, the records and handovers in
   `.claude/`), the herdr plugin manifest and config, the git branches
   and worktree layout, the CLI names (`pnpm wt:*`, `stack:*`, `overview`,
   their flags). Change one only together with a reader for the old form,
   and log it.
6. **No GitLab or herdr writes the user didn't ask for**: the rewrite
   writes to GitLab and herdr exactly where today's breadboard does
   (merge on two clicks, notes, ticks, tab layout), never more.
7. **Measured beats argued.** Budgets of the suite: start < 300 ms, a
   GitLab refresh of the breadboard record < 5 s and within its call
   budget (#305), idle CPU near 0, scrolling smooth at 40 wheel reports/s.
   A crate, a design, a layout is chosen by these numbers, the suite and
   the critic, and the reasons go into the log.
8. **Rust only, no way around it**: no Node, Bun or Python at run time in
   what breadboard ships; a crate before hand-rolling, when it is
   maintained and the simpler (rule 4). What stays outside Rust is named in
   the plan, nothing else.

## Working

- **Never against the live workflow**: a worktree of its own; the suite
  runs every implementation in a pty against recorded fixtures with fake
  `glab`, `herdr`, `docker`, `xdg-open` on its PATH. No `wt:new`, herdr
  reload, `--herdr` sync or live overview from the loop (they act on the
  user's real herdr).
- **Side by side until it wins**: the Node breadboard keeps running the
  live workflow; the rewrite replaces a command only once its part of the
  suite is green and the critic picked it blind. Then the switch is one
  commit (the `package.json` script), and the Node code of that part goes
  in the next.
- **Commit per green step**, push at once, Conventional Commits with the
  scope `breadboard`; the decision log is committed with the code it
  explains.
- **What stays outside Rust** and why: [`rewrite-plan.md`](rewrite-plan.md)
  (the nvim Lua, the Chromium extension, the hooks' thin entry). They talk
  to breadboard through files and its CLI; the rewrite keeps those.

## The decision log

`packages/breadboard/docs/rewrite-log.md`, newest first, one entry per
decision, for example:

```md
## 2026-10-02 14:10 · overview / MERGE REQUESTS · GitLab read
Chose: one GraphQL query for open MRs, pipelines and jobs. Against: REST
with iids[] batches.
Why: rule 7: 1 call vs 4 a refresh, 0.6 s vs 1.4 s; rule 4: one parser.
Suite: 212/240 green (was 180). Critic: ours blind 3 of 3 against
lazygit's status panel.
```

The user reads it instead of answering questions. A `try` for the user
comes only at the end, when every check is green and the critic picked
ours blind for every piece.

## The loop

Started once the suite is merged, in a worktree of its own, with this
prompt (the user's, 2026-10-01, the stack set since):

```
Rebuild breadboard (packages/breadboard: the overview TUI, the wt: and stack: CLI, the herdr plugin) as the best developer tool it can be, in Rust with ratatui.

The bar is lazygit. Install it, run it in a herdr pane the same size as the overview and compare against it directly, not against a description of it. The measurable half is the black-box suite of #307: every check green, a GitLab refresh under 5 s, start under 300 ms, idle CPU near zero.

Break this into the smallest pieces that can be improved and judged on their own: each overview section, buttons and hover, scrolling, the data daemon, each CLI command. For each piece, fan out a builder and a separate critic with fresh context. The critic runs ours and lazygit side by side, screenshots and interaction blind with the labels stripped, says which one a developer would rather work in, and names the single biggest remaining gap. Then it goes back to the builder.

The critic should be a harsh critic. Praise is not useful. If ours does not win, it keeps going.

/loop on each piece until the critic picks ours blind and the suite is green. Do not stop before that. Work in a worktree, never against the live herdr or overview.

Keep a live progress page updating as the work evolves so I can watch it.

Fan out subagents and ultracode.
```

How it runs the suite against the rewrite: `BREADBOARD_SUBJECT=<its binary>
pnpm --filter @monocircuit/breadboard-gauntlet gauntlet`; known bugs count
there. The next piece is the first failing check, in the order of
[`rewrite-plan.md`](rewrite-plan.md).
