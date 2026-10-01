# Rules for the rewrite loop (#307)

The rewrite of breadboard runs as a gauntlet loop (user, 2026-10-01): a
builder and a separate harsh critic per piece, a blind comparison against
**lazygit**, looping until ours wins and
[the black-box suite](../../breadboard-gauntlet/README.md) is green. It is
not steered by questions. When the loop has to choose, it chooses by these
rules, in this order, and writes the choice to its decision log.

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
   The stack is chosen by these numbers, the suite and the critic, and the
   reasons go into the log ([evidence so far](stack-evidence.md)).

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
- **What stays in Node / Lua / shell**: the Claude Code hooks
  (`.claude/hooks/*.mjs`, run by Claude Code), the nvim Lua (`nvim/*.lua`,
  run by nvim), the Chromium extension (JavaScript, run by the browser).
  They talk to breadboard through files and its CLI; a rewrite keeps those.

## The decision log

`packages/breadboard/docs/rewrite-log.md`, newest first, one entry per
decision, for example:

```md
## 2026-10-02 14:10 · overview / MERGE REQUESTS · stack
Chose: Rust + ratatui. Against: OpenTUI on Bun.
Why: rule 7: idle 0.1 % vs 0.5 %, 9 vs 81 MiB, tests 0.5 s vs 92 s; the
critic picked it blind 3 of 3 against lazygit's status panel.
Suite: 212/240 green (was 180).
```

The user reads it instead of answering questions. A `try` for the user
comes only at the end, when every check is green and the critic picked
ours blind for every piece.
