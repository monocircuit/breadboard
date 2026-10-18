# parallel-work in herdr

Part of the skill [`parallel-work`](SKILL.md): how the worktree scripts arrange
herdr (breadboard's `herdr/README.md`). An agent needs little of it (see
"In herdr" in the skill); this is for the human, and for work on the scripts.

## Spaces and tabs

Inside herdr (`HERDR_ENV=1`) the scripts arrange the terminal, so nobody has
to call `herdr` by hand. There is a space per purpose:

| Space | Tabs |
| ----- | ---- |
| `{{space}}` | `root` (a shell in the main checkout), `overview` (`{{run}} overview --herdr`, live) |
| `<product>`, one per product of `breadboard.toml` (`[products.<name>]`, #405) | `root`, `overview` (`{{run}} overview --product <product>`: each orchestration of the product a section), `orchestrator` (one chat for the product's orchestrations), `changelog` (the repo's) and `changes`. With products the main space keeps only `root` and `overview`. A click on a worktree or an MR in `overview` splits it to the right with a **detail pane**: the worktree's nvim, its try's terminal, its stack's logs or its agent's conversation, switched by `e` `t` `l` `a` ("The detail pane" in `packages/breadboard/README.md`) |
| `⇶ <name>`, one per orchestration of no product | `root`, `overview` (`{{run}} overview --orchestration <name>`: its issues and merge requests) and `orchestrator`, the tab of the session that ran `{{run}} wt:orchestrate` ([orchestrate.md](orchestrate.md)) |

A worktree has **no space of its own** (#409): a click on its row splits
the overview and shows its nvim, its try, its stack's logs or its agent
beside (the detail pane). The spaces worktrees had before close on the next
layout (`wt:herdr`, a reload, the main space's overview) once every pane in
them idles in its shell or runs nvim or a log viewer; one where anything
else runs (a claude, a dev server, this command) is kept, said in the
overview's band, until it ends. What an agent waits for (`at work`,
`asks you`, `waits for the pipeline`, `waits for you: merge`, `free`, …) is
its row in the overview, which also notifies when one starts asking, fails or is merged;
the tab bar counts who needs you (herdr README, "What an agent waits for").
An agent card names its repo instead of repeating its space ("An agent
card").

An orchestration sits in the same group ([orchestrate.md](orchestrate.md)):
its space `⇶ <name>` stands on a detached checkout in
`.claude/orchestrations/<name>/`, its worktrees are worktrees of the main
checkout, and the orchestration's colour mark tells them from the rest.

The order is fixed: `root`, `overview`, `<feature>` (always the third tab),
`nvim`, then the rest. herdr can't reorder tabs, so a tab out of place (an
older space, a tab closed and back, the tab an agent moved in) moves its pane
into a new tab at the end, what runs in it keeps running. A space never keeps
two `<feature>` tabs: a spare one, whose shell idles and holds no agent,
closes.

**Conversations** of every worktree are kept in the claude project of the
main checkout (`~/.claude/projects/<main checkout>`), so `/resume` in
`{{main}}/` lists them all, also those of a removed worktree. The project of a
worktree (and of a directory in one) is a link to it: `wt:new` makes it before
the first `claude` runs there, and the layout merges an older one in once none
of its transcripts was written to for 10 minutes (a file both have arrives as
`<name>.from-<feature>`). The overview of a worktree shows under `claude` the
command that resumes its conversation and the file it is stored in.

{{#stack}}
A stack has **no tabs** (#407): while it runs, `dev` (`{{app.dev}}`){{stack.with_stack}}
run in the background, each under a `breadboard supervise` that restarts it
when it crashes, their output in `.claude/stack/<name>.log`. They start with
`stack:up`, `wt:stack` and `wt:new`, only while their port is free (a server
started by hand isn't started twice), and end with `stack:down` and
`wt:remove`. `l` in the overview's detail pane follows the services' logs and
those files together. A worktree on the shared stack has none: that stack is
the main checkout's.
{{/stack}}

The `overview` of the main space keeps it all in step: a stack that comes
up or goes down gets or loses its background processes, within a
couple of seconds. `stack:*`, `wt:new`, `wt:stack` and `wt:remove` do their
part right away.

`wt:new` opens no space and moves no tab (#409): a human or an agent goes on
where they are (an agent with `EnterWorktree`), an orchestration's agent
runs headless, its worktree a row in the overview.

`wt:remove` (and so `wt:prune`) **closes** what is left of a worktree's space or tabs. Not blindly:
a tab with an agent at work, more than one pane, or the tab `wt:remove`
itself runs in stays. Closing it would cut off a turn or kill a running
server. An idle agent's tab closes: its conversation lives in the main
checkout's project, so `/resume` there brings it back. The `✕ delete worktree` of the overview (`wt:remove --close-space`)
closes the space as a whole, the conversation's tab too: it stays resumable
from the main checkout. The overview never writes the conversation binding
into a worktree being removed, and `wt:remove` finishes when git leaves only
an empty directory (or that binding) behind. However a worktree goes
(`git worktree remove` by hand, `wt:remove` outside herdr), the overview of
the main space and `wt:herdr` close its space: one whose label names no
worktree and whose panes all stand in that deleted directory. A tab an agent
still runs in stays until the agent ends.

**`{{run}} wt:herdr`** builds the whole layout from `git worktree list` and only
adds what is missing, so it is also the way back after a herdr restart or
after tabs were closed by hand. herdr restores its tabs after a restart, but
as bare shells: a tab of ours whose shell idles runs its command again
(`overview`, `nvim`). No claude is
resumed in a tab, not even one you ran yourself: resume that by hand
(`claude --resume`, the worktree's view shows the command). An
orchestration's agent is the conversation bound to its worktree
(`.claude/herdr-session`) and runs headless whatever herdr does
([orchestrate.md](orchestrate.md) "How agents run"). It moves the layout before this one over too:
`{{branch}}` becomes `root`, and a tab whose directory is gone closes.

**`{{run}} herdr`** sets all of it up for a human in one command: the pinned
herdr in `.herdr/`, its own session `{{main}}` with the repo's config, and
the plugin. An agent never runs it (it attaches a terminal). Someone with a
herdr of their own, like the one Omarchy ships, uses
`{{run}} herdr:install` instead: the same plugin and spaces in their
default session (README, "Your own herdr"). New workflow code reaches a
running layout only through `prefix+shift+l` (`{{run}} herdr --reload`), in
either herdr: the overviews keep the code they were started with.

**The plugin** `{{plugin}}` (herdr ≥ 0.9, see breadboard's `herdr/README.md`) puts
`{{run}} overview`, `{{run}} commands` and the `stack:*` of the focused tab on keys,
and runs `wt:herdr` at every herdr start. It is for the human at the keyboard:
an agent runs the commands directly.

Outside herdr, and when a `herdr` call fails, `wt:new` does its work and says
nothing: the worktree is the point, the space is comfort. The main space's
name is breadboard.toml's `[herdr] space` ({{space}}).

## Next issue

An agent that finished one issue starts the next in the same conversation:

1. `{{run}} wt:new <n>-<feature>`, from wherever it is. Its tab moves into the
   new space, as always. `wt:new` sees the worktree the agent came from (its
   tab's space, else where it ran). It clears that worktree's conversation
   binding, so a herdr restart doesn't resume the conversation in both spaces.
   When that work is done (the branch merged and gone from origin, nothing
   uncommitted), it removes the worktree and closes its space. Otherwise the
   worktree stays with its overview and nvim, and the summary says why.
2. `ExitWorktree(action: "keep")`: from one of our worktrees `EnterWorktree`
   refuses another ("cannot be a worktree managed by Claude Code").
3. `EnterWorktree(path: "<the new directory>")`.

`wt:new` prints steps 2 and 3 with the path when an agent comes from a worktree.

## The overview

`{{run}} overview`: **live**, for the human: every worktree (app up, stack, ahead/changed, its MR and pipeline), stacks without a worktree, the open MRs with their pipelines, the open issues with the branch that claims them. Local state every 2 s, GitLab every 30 s; `q` quits, `r` fetches GitLab now. A click on a row opens it (a worktree or a claimed issue: its herdr space; an MR or an issue: a browser popup, a small Chromium window of its own without tabs (`--app`, your profile, so logged in) that floats centered under Hyprland — placed by the script through `hyprctl`, no rule in your config; a URL: the browser, in the Chromium tab group named like the space it belongs to, see `packages/breadboard/chromium/README.md`), Ctrl-click on `!n`, `#n` or a pipeline opens that link as a tab. Its WHERE (the view of one worktree) lists `storybook` on 6006 + the offset; a click on it while it is down starts it in a herdr tab `storybook` of the space. A stack that is down gets a button `▶ start the stack` (the default stack in the full view, the own one in a worktree's view): a click or `u` runs `stack:up` for it, the button shows how far it got, and once up the stack gets its tabs. `--once` prints it once, `--worktree <dir>` shows only that worktree, `--orchestration <name>` only the issues and merge requests of that orchestration

The **view of one worktree** (`--worktree <dir>`, the tab `overview` of its
space), top to bottom: WORK (conversation, issue, MR, the ✕), AGENT (its last
prompt and answer, what herdr says it does), ISSUE (the boxes, the open ones
by name), PIPELINE (every job; a failed one with the end of its log), DIFF,
DEVELOP (the MRs {{branch}} got since the branch started, and whether a merge
would conflict: `git merge-tree`, nothing touched), WHERE, STACK (with the
migrations the stack's database lacks and a button that applies them).
