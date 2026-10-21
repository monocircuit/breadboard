# Orchestrating several issues

Part of the skill [`parallel-work`](SKILL.md): one Claude session (the
orchestrator) gets several issues done at once, each by an agent of its own:
a Claude Code session that breadboard runs headless in the issue's worktree
whenever something happens for it (see How agents run). The overview shows
what they do; the orchestrator only carries decisions between them and the
user.

## When

The user asks for several independent issues in one go ("do #127–#129", "work
through Epic 5"). Not for one issue (just `wt:new`), and not for issues that
build on each other's code: they would wait for each other's merge.
Background subagents are not the tool for it: their chats are only files
under the orchestrator's transcript, nobody can type into them, and their
spaces stay empty.

## The command

```sh
{{run}} wt:orchestrate <name> <issue>… --dry-run   # the plan: branches, skipped issues, the brief
{{run}} wt:orchestrate <name> <issue>…             # do it
```

| Option | Does |
| ------ | ---- |
| `<name>` | kebab-case, names the orchestration: the space `⇶ <name>`, the origin of its worktrees |
| `--title "<text>"` | what the overview's title band, the space and the sidebar show instead: `⇶ <title>` (e.g. `@acme/web`). Recorded, so a later run keeps it; `wt:orchestrate <name> --title "…"` alone changes it. Commands, files and `SendMessage` keep the name |
| `--product <name>` | the product of `breadboard.toml` it belongs to (#405): it works in the product's space with the product's other orchestrations. Without it the issues' milestone names it (`vita · …` → `vita`); recorded |
| `<issue>` | a number (the worktree is named after the issue's title, `fix/` with the label `bug`), or `<n>-<feature>` to name it yourself |
| `--shared` | every worktree on the shared stack: for issues that never start the app (~1 GB RAM less each) |
| `--queue` | start none: the overview hands each to a free agent (the one that knows its files first), else starts a fresh one by itself, memory permitting, in the work order of `wt:plan` |
| `--queue --after !<mr>…` | the same, but none starts before those MRs are merged (their node: `waits for !282 to be merged`) |
| `--peer <session>` | the name `ListAgents` shows for the orchestrator, written into each brief |
| `--brief <file>` | text added to every brief, e.g. an order to keep between the MRs |

What it does, in this order:

1. Reads every issue. Closed ones, ones another branch claims and ones with a
   worktree already are skipped and listed; nothing to start stops here. A
   worktree of the orchestration whose agent is gone (none bound, its MR not
   merged) gets a fresh agent in it instead (#455), its brief saying it takes
   over; the overview does the same for such a worktree in the work order.
2. Opens the space `⇶ <name>`, or its product's (`<product>`), (`root`, `overview`, `orchestrator`,
   `changelog` and `changes`, see The changelog) and
   moves the tab it runs in there: the orchestrator's own tab. That happens only
   when the space has no orchestrator yet and the calling tab is nobody's:
   run from another orchestration's orchestrator or from an agent's worktree,
   it just starts the issues, and every tab stays where it is (#287).
   There is one `orchestrator` tab per space. Records the
   issues, started or skipped and why, in `.claude/orchestrations/<name>.json`
   of the main checkout, with the orchestrator's conversation
   (`orchestrator: { sessionId, cwd }`, see The orchestrator's chat). A
   second run with the same name adds to it. The
   space `⇶ <name>` stands on its product's batch
   `batch/<product>-<n>` (opened from `origin/{{branch}}` when the product has none, the base
   of every task{{#batches}}, [batches.md](batches.md){{/batches}}) in `.claude/orchestrations/<name>/`,
   so herdr hangs it under `{{space}}` like a worktree's.
3. Per issue, **at once** whatever the number at work: to a free or idle
   agent whose worktrees share the most files with the paths the issue
   names, else the one with the least context used (its MR merged, see
   Agents are workers), else to a fresh agent. Agents are
   started for speed; an idle one is reused when it fits. Free memory is the
   only limit: below 1.5 GB (own stack ~1 GB + agent ~0.5 GB) a fresh agent
   starts on the shared stack anyway, and the command says so.
4. For a fresh agent `{{run}} wt:new <feature> --origin <name>`, **one after the other**
   (two at once would pick the same free ports). The worktree is one of the
   main checkout like any other, its space in the same group; `wt:list`, the
   overview, `wt:remove` and `wt:prune` know it. The origin is recorded in `.claude/origin` of the
   worktree; its space opens unfocused, with the usual tabs.
5. Per worktree a new session id, bound to it (`.claude/herdr-session`), and
   its first run queued: `.claude/brief.md` of the worktree as the prompt.
   No tab: the worktree's space has `root`, `overview`, `nvim` and its stack
   tabs, no claude.

A worktree counts as made when it exists: `wt:new` may still fail in its
summary (#130), and the next issue still starts.

## How agents run

**An agent is a Claude Code session id, not a process** (#358). Nothing runs
while it waits for a pipeline, the merge order or an answer. When something
happens for it (its brief, an answer, a merge, a failed pipeline, the end of
a `wt:wait`, a handover) the line goes into its queue, and breadboard's
daemon runs it:

```sh
claude -p --resume <id> --name <feature> --permission-mode auto \
  --permission-prompts none --disallowedTools AskUserQuestion \
  --output-format stream-json --verbose '<the lines>'
```

in the agent's current worktree (`--session-id <id>` instead of `--resume`
for its first run, while it has no transcript; `core/src/runs.rs`). One run
per agent at a time: lines that arrive during a run are batched into the
next, a line repeated in a row once. Runs start while the free memory
(`MemAvailable`) minus a 6 GB reserve leaves room for 0.5 GB per run (one
that started in the last 15 s counts as not yet in `MemAvailable`), in merge
order (`wt:plan`), then by age; the rest wait `queued`. The transcript is
the one Claude Code always writes (`~/.claude/projects/…/<id>.jsonl`), so
the context share and the node's `now` line read it as before. The Stop
hook (`stop-answers`, see Answers) stays the safety net for a line that came
while a run ended.

The daemon (`breadboard serve`) keeps it all in `<main checkout>/.claude/runs/`:
`queue.json` (the lines waiting, written by every process that talks to an
agent, under a lock), `status.json` (the status table, by session) and
`log/<id>.jsonl` (the stream of each agent's last run). Each agent has a
state the overview and the sidebar show (the table replaces herdr's state of
a pane): `running` (a run in flight), `idle`, `asks` (its last run was
denied a tool and waits for the user, see Permissions) and `queued` (lines
wait for memory, or for its tab, below).

**Pause all agents** (#397): when the user says "pause", run

```sh
sh packages/breadboard/herdr/breadboard.sh runs pause [--now] [--ci]
sh packages/breadboard/herdr/breadboard.sh runs resume
```

or click `⏸ pause agents` / `▶ resume agents` under the overview's chat.
Paused, runs in flight finish (`--now` ends them, their lines come back to
the queue), no run starts and lines keep queueing; the band says `agents
paused since 13:05 · N lines waiting`. `--ci` switches the CI queue off too.
The state is a file (`.claude/runs/pause.json`): it outlives `serve`, herdr
and a reboot. `resume` runs every waiting line, one run per agent, and
switches the CI queue on again if the pause switched it off. Never end runs
or daemons by hand to pause.

**A headless agent waits only through a line** (#417). A background command
(a backgrounded Bash call, a Monitor) is stopped when its run ends, and its
notification never comes: an agent runs `wt:check`, the gauntlet and builds
in the foreground, and waits for something external only with `wt:wait`,
never with a background command and "I'll wait". The engine wakes one that
does anyway: a run that ends with a background task still running (from its
stream) leaves `.claude/wake.json` in its worktree, and once the processes it
left behind (they carry `BREADBOARD_RUN=<id>`) are gone, at the latest after
15 min, the line `[breadboard] your background command finished: <cmd>,
continue. …` is queued. A run whose last answer says it waits, with no
`wt:wait` recorded and no open action, gets its line after 15 min (not a
second time in a row); until then its node says `waits on itself since
00:25`. A run or a queued line before drops the wake.

**Open an agent in a tab** (#361): a click on an agent (its block in
WORK, an action that opens it, the `resume` row of its worktree's view)
runs `cd <worktree> && claude --resume <id> --name <feature>` in the
`<feature>` tab of its space, once no run is in flight (it waits for one up
to 15 min); a claude that already runs the conversation is put in front
instead. While that claude runs, the agent's queue waits; when it ends, the
queue goes on headless. Type there for anything the overview has no button
for: your word wins over the orchestrator's.

## Permissions

Agents keep auto mode (#360): every run passes `--permission-mode auto`, so
the classifier, the repo's allow and deny lists and the hooks decide tool
calls exactly as in an interactive session. Only where a question lands
changes. What auto mode would ask the user is denied in the run
(`--permission-prompts none`); the daemon reads the denials from the stream
(`permission_denials` of its result, `core/src/permissions.rs`). A denial
that the classifier or a deny rule stands behind stays one: the agent reads
it and works without. Only one that would have been a question becomes a
`permission` action, `may <agent> run <Tool>: <first line of the command,
path or url>?` (`may <agent> use <Tool>?` without one), with `allow once`,
`allow always` and `deny`. `<rule>` is the rule that allows exactly that
call (`Bash(docker system prune)`, `Edit(//abs/path)`,
`WebFetch(domain:host)`). Allowed, the agent's next run gets
`--allowedTools <rule>` and the line `[overview] the user allowed <rule>, go
on: it is allowed in this run.`; `allow always` also writes the rule into
the worktree's `.claude/settings.local.json`. Denied: `[overview] the user
denied <rule>: don't run it, go on without it or leave an action for the
user.` (`core/src/permissions.rs`, `answer_run`). `AskUserQuestion` is disallowed in runs: an
agent asks with `{{run}} wt:action decide` (Answers).

## Talking to the agents

The agents are Claude Code sessions named after their feature, but
`ListAgents` shows one only during a run: between runs there is no process
to message, and a `SendMessage` to an idle agent goes nowhere. An agent
reaches the orchestrator with `SendMessage` (the orchestrator's chat runs
all the time); the orchestrator reaches an agent through its queue:

```sh
sh packages/breadboard/herdr/breadboard.sh runs tell <worktree dir> "<line>"
```

The line starts the agent's next run, or goes into the next one when a run
is in flight. A line it needn't act on now (`!N is merged`, a hotfix's
pipeline runs, a rule for later) goes with `runs tell --notice <dir>
"<line>"`: it starts no run but rides along with the next line that does
(#445). Every run resumes the agent's whole history, so a notice of its own
costs a full read, and past the cache's hour a full write too. The
overview's own `!N merged` is such a notice.

**After a crash, an OOM kill or a reboot** the run engine picks up every
run that was cut off by itself, once and a few at a time (`[agents]
max_runs`): don't wake the agents by hand. A small session is resumed with
one line; one past the fresh-start rule is taken over by a fresh session in
its worktree with a handoff from the disk (`.claude/handoff-cut.md`), never
resumed with its whole history (#445).

**Lean: the overview reports, the chat decides.** Status never goes through
the orchestrator's conversation: every message there stays in its context.
An agent reports nothing to the chat: its block in WORK shows its state
(How agents run) and what it waits for, with commits, MR, pipeline and
boxes, the problem it works on and what it is on now (its `{{run}}
wt:action note "<one line>"`, asked for at every stage, or its last answer's
first line). The brief asks for `SendMessage`
only for:

- its plan, when it differs from the issue;
- a decision that is the user's, with options and a recommendation — and to
  wait for the answer;
- what concerns another agent's work (an order, a conflict, a dependency).

The orchestrator relays nothing the overview shows, and doesn't ask an agent
where it stands: WORK says it. **A decision with a `decide` (or `plan`)
action is the overview's**: the agent's message is only context. Messages
arrive late, often after the user clicked. So before it asks the user
anything an agent brought, the orchestrator checks the action:
`{{run}} wt:action --list` in the worktree shows it open, or answered under
`answered, delivered` (then it is taken: nothing to ask). It relays only
while the action is open, and never asks in its own chat what the overview
has buttons for. It answers with the user's word, not its own
guess. Never merge, approve or tag for an agent. The user can also open an
agent in a tab (How agents run) and type there: that wins.

**Waiting for other MRs needs no orchestrator, and no process** (#359). An
agent that has to wait runs `{{run}} wt:wait !N … --then "<what next>"` and
stops: the command records the wait in `.claude/wait.json` and returns at
once (MRs already merged or closed: it says so then). The daemon asks
GitLab every minute and queues the line to go on with (`!213 is merged:
develop is 3 behind: merge it in only if your work needs it or it
conflicts, then …`), which starts the agent's next run;
early when one is closed, stays in conflict for 30 min, or after 8 h.
Meanwhile the MR row says `waits for !213`.

**The overview answers "what now?"**, so the user never has to ask. The
orchestrator sets the order right after the start and again whenever it
changes: `{{run}} wt:plan <name> --work <issue>… --merge <issue>… --why "…"`
(`wt:plan <name>` alone prints it). The live overview keeps the plan in step
with the merge order it computes, by itself (#376): when an issue in it is
merged, closed or gone, its why names one done, or the merge order left it,
it writes the computed order (done and gone issues out, new ones at their
place, the why kept unless it names a done issue, a line `updated from the
merge order <time>` under it). A hand-set order of independent MRs holds:
the computed order follows the plan where nothing decides otherwise. A step that is the user's and no agent's
it leaves as `{{run}} wt:action <kind> "<text>" --orchestration <name>`; a
merge names its MR (`--mr !N`), so its click merges that one. Each
agent leaves an action whenever it waits for the user (its brief says so).

## The orchestrator's chat

The conversation in the tab `orchestrator` is recorded like an agent's
binding: when the space opens, and by the overview of the main space
whenever a fresh chat starts in that tab. A lone claude in a space without
the tab gets the tab's name. On top of the orchestration's overview,
**`open orchestrator chat`** does one of three things:

- a running orchestrator: the click only focuses it;
- its claude died: the click resumes the recorded conversation
  (`claude --resume <id>`) in its tab;
- its tab is gone: the click makes the tab, with the conversation in it.

It never makes a second tab, and never resumes a conversation that runs
elsewhere. After a herdr restart, the layout resumes the orchestrator in its
idle tab; agents have no tab to resume (How agents run). No script closes or replaces a tab
`orchestrator` that holds more than an idle shell.

## Staying lean

The orchestrator needs to know little: the state lives in GitLab (issues,
MRs, pipelines), the record (`plan`, `workers`), the agents' worktrees and
the overview. What its chat holds it pays for in every turn after (#382):

- **Investigate through a subagent**, not in its own chat: logs, transcripts,
  pipeline traces, code searches go to an `Agent`, only the conclusion comes
  back.
- **Never paste large command output**: `head`, `grep`, counts. No
  `overview --once` printed into the chat: the overview is on screen.
- **Agents' routine reports go to the overview**, not the chat (status as
  well as decisions, above). A screenshot only when nothing else shows it.
- **The journal is its memory**: `{{run}} wt:journal "<line>" --orchestration
  <name>` for every decision it took for the user, every open thread and
  who waits for what, one line each, when it happens
  (`.claude/orchestrations/<name>.journal.md`, `wt:journal` alone prints it).
  The overview shows its last lines under JOURNAL.

**The fresh start.** The title band shows the orchestrator's context
(`orchestrator 62 % · restart at a quiet point`). Above `[orchestrator]
fresh_above` of `breadboard.toml` (50 %) and at a quiet point (its chat idle,
no `decide` or `plan` open, no merge in flight) the chat row offers
**`↻ fresh start`** (with `auto_fresh = true` the overview makes it by
itself): `{{run}} wt:orchestrate <name> --fresh` tells the old chat to write
its journal entries, waits for its answer, ends its claude and starts a new
one in the tab `orchestrator`, its first prompt the journal and the record.
The old conversation stays in `/resume`, the journal says its id. A fresh
orchestrator starts from that prompt: every open thread in the journal is
its own, and it asks GitLab, the overview and the agents rather than the
user.

## The overview

The tab `overview` of `⇶ <name>` runs `{{run}} overview --orchestration <name>`
(`--once` prints it, for the orchestrator itself): NEXT, only what the user
can do now with one click there (`core/src/model/next.rs`) — the first action
not answered yet, else the first clickable row of UP NEXT (not one under
way), else an MR ready to merge (pipeline passed, Work boxes ticked, its
turn); else one dim line, `nothing for you · 2 agents at work · next merge:
!N once its pipeline passes`. The plan's why stands under NEXT only beside
a merge and never once the plan is stale. ACTIONS holds only what needs the user: what was asked with
`{{run}} wt:action <kind> <text>` — `merge` (the human merges on two clicks,
see Merging; gone once merged; each agent posts its own), `try`
(what only the human can check, and the click that opens its spot:
`--run <command>` the overview's detail pane running it in the
worktree, `--url <page>` the page on the worktree's app in its tab group, a
path `/atlas/…` on the worktree's port, its stack and app started
first when the app is down; the row shows the command or page before;
`browser-check --url` is its old name; `--on test --url <path>` the path on
the test instance, origin from `{{test_origin_env}}`. A try shows its state
before the text (`core/src/model/try_state.rs`): `not pushed` / `ready` on the
worktree, `waits for merge !N` → `deploying (publish, 3 min)` → `ready` or
`deploy failed (<job>)`, a click opens the job, on the test instance. One
that isn't ready is grey, its buttons don't answer, NEXT skips it, and the
overview notifies once it turns ready. A check that needs the deployment is
a `try --on test` posted with the merge, not a box in prose; its worktree
isn't cleaned up while it is open), `decide`, `plan` and
`look` (go to the agent's space) — and the automatic `permission` (what auto
mode would have asked in a run, see Permissions) and `merge` (a ready, green MR without its agent's, none while a
change is requested). Commits an agent left unpushed get no row:
breadboard checks and pushes them itself (README, "Headless agents").
Sorted by what blocks most: `permission`, `decide`, `plan`, `merge`,
`try`, `look`; within a kind by merge order, then age. UP NEXT,
the automatic follow-ups in merge order, one click each: `clean up` (remove
the worktree, a small run of its own ticks the done boxes: Sonnet at low effort with the issue, the MR and the agent's handoff, its agent not woken, #445), `pipeline` (a failed
MR pipeline whose line didn't reach its agent; queued, it arrives on the
first try, so the row is rare: a worktree without an agent), `hand over` and `start` (see Agents are workers), `end` (see The end). `wt:action --list` / `--done <id>` from the
worktree. WORK (#347: what WORKTREES and MERGE REQUESTS showed apart),
under the orchestration's record and how reuse went, one block per piece
of work: the open MRs in the merge order, the MR graph's lanes in front
("side by side where independent"), then the worktrees without one in the
work order, the queue folded, and one `done (N)` fold for the merged,
closed and removed (`wt:remove` notes `removedAt` and `mergedAt`; shown
until folded). Its title counts the agents by whom they wait for and the MRs
(open, merged, without one yet). A block, line by line:

- **head**: its number in the work order, the feature, `!M` and its
  pipeline mark, the app (`● :3002`) and stack (`▶ stack down` starts it),
  and at the right where it stands in its life: `merged: will be removed`,
  `merged: kept` with the reason, else what its agent waits for (#288,
  never a bare `idle`; the same words in herdr's sidebar), the first that
  holds: `at work` (a run in flight), `queued` (an event waits for memory
  or for its open tab), `asks you` (an open `permission` action), `waits
  for the pipeline` (running, or none yet for its push), `pipeline failed`,
  `waits for you: merge, try #4, decide` (its open actions, a merge once its
  MR's pipeline passed and no try or acceptance box is open, else `checks
  open: …`), `waits for !N` (wt:wait), `→ next #N after !M` (an announced
  handover), `change requested`, `not pushed: N commits`, else `free` (or
  `no agent`); a done MR that still lacks something names its first
  blocker (`conflicts with develop`, `waits for !N`, #416). Coloured by
  whom it waits for, in herdr's theme colours: at work yellow, GitLab
  blue, you red, free teal;
- **title**: the MR's, with `ready to merge` (`merging …`, `merged`) or a
  merged one's age at the right;
- **bar** of its issue's boxes (#294): `9/12 boxes · work 9/9 · acceptance 0/3`;
- **what it lacks**, each `→ who` acts on it (agent, you, GitLab, `!N`;
  `you` only for your steps: merge clicks, tries, decides): the button
  `resolve merge conflicts` with GitLab's `conflicts with develop (files…)
  → agent` in red while GitLab reports some, its last answer kept while it
  rechecks the same head (#416) (its click
  queues the order to merge its base in, resolve, check and push for the
  MR's agent; grey with the reason when it can't help; after a
  click the block says where it stands: `queued for its agent (at work)`,
  `its agent merges its base in`, `pushed, GitLab checks`, `conflicts
  resolved`, #290), `checking for conflicts …` (GitLab lacks the pushed
  head), `GitLab rechecks mergeability …` (GitLab hasn't computed it: the
  overview asks it to, #391), the reasons (`push blocked: develop red
  (<test>) → a hotfix on develop` when wt:check fails on a test develop's
  pipeline fails too, #416); nothing when nothing lacks;
- **facts**: its issue, work, whether its HEAD passed `{{run}} wt:check`
  (`✓ tested <sha>`, `untested: N commits`, `checking…`, `✗ tests failed:
  <package> (<tests>)`, #298), its agent (`agent <name>` when it isn't the
  worktree's, after a handover) and its context, `change requested: <what>`
  and `N commits not pushed`; at their right its agent (#292): `● Mira
  Albrecht · 3 issues · 2 MRs merged`, the dot in the colour of its state;
- where it collides with develop and the other worktrees, whether a merge
  of develop conflicts, the migrations its database lacks, why its stack
  didn't start;
- its issue's title and, in plain words (#341), `problem`: the first
  sentences of its issue's `## Goal` (or `## Bug`), and `now`: its agent's
  latest note (`.claude/note.json`), or `said`: the first line of its last
  answer when that is newer, with how long ago; a block without either
  shows only what it has.

An issue that got no worktree says why; an MR without a worktree gets a
block of the same shape. A click on a block goes to its worktree's space
(without one to its MR, else its issue), on `!M` to the MR, on the mark to
its pipeline; hover lights the whole block. Look here before asking an
agent where it stands.

## Merging

A merge is offered only when the work is done (#280): the pipeline passed,
every box of the issue is ticked (Work and acceptance criteria), every `try`
of the worktree was answered `looks good` (a `broken` holds until a later
try looks good), and it is its turn in the merge order. The one exception
are the checks that need the deployment: a `try --on test` and its box,
marked `(after deploy, try #N)`, stay open across the merge, and only they
count as `· N to check after merge`. Until then there is no automatic
`merge`, none in NEXT and ACTIONS, and the MR row names what is missing
(`try #1 open → you`, `2 acceptance boxes open → agent`) instead of `ready
to merge`. A `broken` on a try of a ready MR puts it back to draft and
withdraws its agent's merge; the agent posts it again once the fix is
pushed, green and tried again.

**Temporary** (#443, until the batch branches carry the testing): with
`[merge] needs_pipeline = false` in breadboard.toml an MR into develop needs
no pipeline of its own. `{{run}} wt:check` passed at its pushed head stands
for it, its row says `merges on wt:check (no MR pipeline)`, and the CI queue
starts none for it (a click on its row still runs one). develop's pipeline
after each merge is the safety net: while it is red, no merge into develop
is offered. Into an epic nothing changes.

No agent marks an MR ready (#399): `breadboard serve` does it once its bar
is full, every box of its issue ticked (after-deploy ones excepted) and no
try or change holding it, once per head, and notes `!N marked ready
automatically` in the orchestration's journal. Its row says `ready (bar
full) → breadboard` until then. A box unticked again puts an MR it marked
ready back to draft; a person who sets it back to draft is respected until
the next push (`draft by hand: ready after the next push → you`). Into
develop and into a batch alike, for a repo GitLab serves.

A `merge` action, posted or found by the overview (ready, pipeline passed),
merges on two clicks: the first arms it (`click again to merge !N`, for 5 s),
the second merges the MR as the user who runs the overview, with the MR's
squash setting, the source branch deleted, at the commit whose pipeline
passed. A merge the agent posted may be a draft: the click marks it ready
first. While anything blocks (open boxes or tries, a pipeline that didn't
pass, conflicts, a draft nobody posted the merge for), the row is grey and
names it in one line, and a click opens the MR instead. After the merge the
agent gets `[overview] !N merged`, NEXT moves on to the next ready MR, and that
MR's agent gets `merge <base> in` only when GitLab says the merge conflicts with it (#392);
an MR that merges cleanly keeps its green pipeline and stays offered;
`clean up` offers the worktree. Agents still never run `glab mr merge`: the
click is the user's (`core/src/model/merge.rs`).

**Into a batch the merge needs no click.** A task MR whose target is its
product's batch (or an epic branch of before) merges by itself under the same rule (bar full,
fast checks green, no conflicts, its turn), without a posted `merge`: the
overview marks a draft ready and merges it with a merge commit, never squashed; the row says `in batch` until the
batch reaches `{{branch}}`. The two clicks stay for shipping the batch to
`{{branch}}` and for anything into `{{branch}}` or `main`{{#batches}} ([batches.md](batches.md)){{/batches}}.

## Change requests

Every piece of work is pushed at once, a draft MR from the first commit;
waiting for another MR (`wt:wait`) comes after the push, never instead of
it. A change the user asks for on an MR first blocks it: `{{run}} wt:action
change "<what>" --orchestration <name> --mr !N` (or the agent's own, in its
worktree) makes the MR a draft before any code is written, withdraws its
merge actions and shows `change requested: <what>` on the worktree and the
MR row until a commit after it is pushed and green; then the agent posts
its merge again. Send the change to the agent after the command. An MR
merged meanwhile takes no change: the command refuses, and the change gets
its own issue, branch and draft MR at once (`cli/write/change.rs`).

## Answers

Every action in ACTIONS is a block of its own, a thin line between them
(#273): a head (its kind as a chip, `merge · auto` for an automatic one,
the agent, the age, a try's state), its text at the full width, then how it
is answered. A `decide` reads like Claude Code's question dialog: the
question bold, then its options one under the other, numbered, each with
what happens dim below it and the recommended one marked `(Recommended)`; a
click on an option answers it. The other kinds have small buttons on a line
after the text: `try` `looks good` / `broken`, `look` `done`, `plan` `ok` /
`different`, a merge only `✓`, which ticks it off. The rest of a block does
the action. NEXT shows its first action as the same block.

A decide is answered from its row, so `wt:action decide` asks for it
precisely and refuses one that lacks a part:

- the question: what is decided, in one sentence, with its `?`; a second
  sentence may give the recommendation's why, in one line;
- two or more options, each a short label (the answer the agent gets) and
  what happens, its consequence: `--option "Go" --why "phases 2-4 build
  the Skia painter"`, or `--option "Go: phases 2-4 …"` split at the first
  colon; a label alone must say it in a few words (`keep both lines`, not `A`);
- `--recommend <label>` (or its number), marked `(Recommended)`.

```sh
{{run}} wt:action decide "Which line of the merge hint stays? The new one says what the two clicks do." \
  --option "Both" --why "the row keeps the old and the new line" \
  --option "New only" --why "the row says what the two clicks do, WORK the rest" \
  --recommend 2
``` The answer is stored with the action
(`answer`, `answeredAt`) and goes to the agent as one line, `[overview] #3
decide "A or B" → B`: into its queue, so it starts the agent's next run, or
goes into the next one when a run is in flight (#359). The orchestrator's
own actions are typed into its tab `orchestrator` (`herdr agent prompt`),
its chat being the one interactive claude. A delivered answer leaves ACTIONS
and stays readable in the file's `answered` (the newest 50, with
`deliveredAt`; `wt:action --list` shows them), so a late message about it is
known as decided.

`broken` and `different` need the user's words: the click opens a one-line
prompt under the action, and Enter queues the whole line `[overview] #3 try
"…" → broken: <what is wrong>` for the agent. The agent works from that
line, never from the bare word. A `broken` on a try of a ready MR also puts
the MR back to draft and withdraws its merge (see Merging).

The Stop hook (`breadboard hook stop-answers`, `breadboard init` puts it
into `.claude/settings.json`) is the safety net for a line that came while a
run was ending: it only reads the worktree's `.claude/actions.json` (in the
main checkout, the orchestrator's actions whose `pane` is its
`$HERDR_PANE_ID`), blocks the stop with the lines and takes them out.
Without an answer it exits silently. `{{run}} wt:action --answers` does the
same by hand. Each answer arrives once, and no agent turn is spent waiting
for one.

A click on a button shows at once (#303): the buttons give way to `→
looks good` and where it is (`queued for its agent`, `in its run`), before
the reread. The button a click hits is read
off the line on screen, every cell of it and the column after; each line
with buttons gets its click from the hit regions of its layout (`tui/scene.rs`), and a test
clicks every cell of every drawn button of every rendered frame. Every
click is logged with its row, column and what ran in
`$TMPDIR/overview-clicks.log`, a release without its press too: a click
that did nothing can be traced there.

## Agents are workers

An issue keeps its worktree, branch and MR; an agent moves from one worktree
to the next, so a follow-up doesn't pay the whole start-up again.
`{{run}} wt:orchestrate <name> <issue> --to <agent>` (its `claude --name`, or
the feature of the worktree it is in) makes the worktree, writes a brief
that names what the agent had before and moves the binding
(`.claude/herdr-session`) along. The handover is the agent's next run
(#359): it runs in the new worktree, under the new name, with the line
`[overview] next issue #N, your name is <feature> now: your conversation runs
again in its worktree <dir>, read .claude/brief.md`. Nothing ends, no tab
moves. So its Claude Code name is the feature's (`ListAgents` and
`SendMessage` go by it during a run), the record's too (its node shows `was
<old>` for a day). The old worktree keeps its space until its clean-up.
`--to` takes the old name too. Refused while a run of the agent is in
flight or it waits for an answer, or when it starts fresh (below). An issue no free
agent shares a file with goes to a fresh agent: another issue's context
buys it nothing.

**A merged agent stays free** (#289): it is a record entry, not a process.
Before the clean-up of its worktree its last line `[overview] !N merged,
your worktree goes: you are free, …` is queued; once that run ended, the
record's `workers` keeps it by its conversation's id (`free: { at, from, mr
}`, `mrs`, the files it knows, its name and issues), its binding goes with
the worktree's, and the worktree goes. An agent that never ran is freed
too. Its next issue is its next run, in the new worktree, the path of every
handover. While the record keeps a free agent, the space `⇶ <name>` stays.

**A full agent retires** (#381, #445): every resume reads its whole history
again, and once it idled past the prompt cache's hour a resume writes all of
it to the cache anew. So above `[agents] fresh_above` (150 k tokens of
context), or above `idle_fresh_above` (100 k) and idle longer than
`idle_minutes` (55), it isn't freed but retires at its merge, never mid-issue:
`{{run}} wt:orchestrate <name> --retire <feature> --mr <iid>` takes the
handoff it wrote at its last real run (`.claude/handoff-next.md`, a page at
most: what it learned about the files it changed; `wt:action merge` asks for
it once its context passes `idle_fresh_above`), else gives its conversation
one last run that writes `.claude/handoff.md`, but only while its cache is
warm: a cold one is never woken for it. It keeps the handoff next to the record
(`<name>.handoffs/<id>.md`) and marks the agent `retired` there; then its
worktree goes. A fresh agent on those files gets the handoff of the retired
agent sharing the most of them as its own `.claude/handoff.md`, its brief's
`## Handoff` says whose it is; a freed agent's handoff is kept too, for
when it starts fresh later. The overview says it in advance: the agent's
node `fresh start at its next issue`, UP NEXT `#380 → fresh agent (Mira
retires, 162 k)`, and the changelog notes the retirement under its merge.

A handover that has to wait for other MRs is announced, not kept in your
head: `{{run}} wt:orchestrate <name> <issue> --to <agent> --after !232 [!233…]`
records it (the record's `handovers`, the issue queued), the agent's node
in WORK says `→ next #226 after !232` and UP NEXT lists it. The live
overview hands the issue over by itself once the MRs are merged and no run
of the agent is in flight, free or still in its worktree; nothing holds the
clean-up for it but the move itself: `wt:remove` and `wt:prune` refuse a
worktree a handed-over agent is still bound to.
A try that fails (the agent took up something while wt:new ran) is tried
again a minute later once it is idle; its row says why it waits.

No count holds an issue back (#229): `wt:orchestrate` starts each at once,
in a free or idle agent that fits (the one sharing the most files, else the
one with the least context used) or a fresh one. A queue only where it buys
something: an issue that builds on another MR (or whose files move, like a
rename) starts all the same, and its agent waits with `{{run}} wt:wait !N`,
pushed and visible. `--queue` queues on purpose (WORK shows them
`queued`): for each queued issue UP NEXT suggests the idle agent whose
worktrees share the most files with the paths its description names, else
(#265) the merged one with the most room in its context (`#N → <agent>
(free, shares render-orchestration.mjs, 18 %)`), a free agent too; the
old worktree goes once the agent entered the new one (#252). The live
overview hands it over by itself once that agent's MR is merged. One no agent is free for starts a fresh agent by itself while the
free memory is above the 1.5 GB threshold (`start` row: `starts a fresh
agent`), else it waits for an agent to get free and says so; a click starts
one anyway. An agent nothing needs now goes free with its clean-up and
waits for the next issue.

A start runs for minutes (wt:new), and until then its worktree has no brief
and no agent (#272). Every handover and fresh start claims its issue first
(`.claude/orchestrations/<name>.setups/<n>.json`, gone with its process):
the node says `being set up for <agent> (wt:new) …, leave it`, the agent
gets no second issue meanwhile, a second start of the issue is refused, and
`wt:remove` refuses both the new worktree and the one the agent leaves. The
issue stays queued until its agent moved in. When the start fails after
wt:new (the agent went or took up something), its worktree is removed again
and the issue goes back to the queue (`back in the queue: <why>`): no
worktree stays without its agent. `wt:remove` of a worktree whose branch
carries nothing its base lacks also deletes the empty claim on origin, so the
issue can start again.

WORK's title counts the agents by whom they wait for and the rest
(`1 at work · 2 wait for GitLab · 3 wait for you · 1 free · 1 queued · 14
removed`) and names the
free memory against the 1.5 GB threshold (`12.9 GB free (shared stack below
1.5 GB)`, red below it: fresh agents get the shared stack). A queued issue's
node says who takes it (`→ <agent> after !232`, `→ <agent> (free, 18 %)`,
`→ <agent> once its MR is merged`, else `starts a fresh agent` or what it
waits for).

AGENTS, below WORK while there is one, lists the free agents (#289,
`core/src/model/free_agents.rs`): their state in #288's words and colours
(`free`, `at work` while it takes its line, `asks you`), their context, the
worktree and MR they came from and how many issues they did, and their
conversation's id, short (#345: `free  Mira Albrecht  agent-a  0f3c2a91
✕ dismiss`, `18 % · from 7-thing (!31) · 2 issues` under it). A click on
the name opens it in a tab (How agents run); a click on the id copies
`claude --resume <the whole id>` (wl-copy, xclip or xsel; the log shows the
command either way). Two clicks on a row dismiss one: the record forgets it
as free, the conversation stays in `/resume`.

Every agent has a person's name (#292), the one the overview shows it by
while it moves from issue to issue: picked from its conversation id on its
first start, no one else's in the orchestration, kept in the record as its
`displayName` across handovers and herdr restarts. Its node's name line,
ACTIONS (`Mira Albrecht · #292`), AGENTS, UP NEXT and the queue (`#226 → Mira
Albrecht`) name it so; `claude --name`, `--to` and SendMessage keep the
feature. How reuse works (#264): the name line counts its issues, and the line under the root
counts for the whole orchestration (`agents 14 · issues 22 · reused 5,
started at 34% · handovers failed 2`). The record keeps it per agent, by its
conversation id: `history` (each issue, how it came: `fresh`, `handover`,
`announced`, and its context then) and `failed` (handovers that failed, why).

## In the sidebar

The space `⇶ <name>` and the spaces of its worktrees, all under
`{{space}}`, show `⇶ <name>` (`⇶ <title>` with `--title`) in
front of the branch, in the orchestration's colour (a slot of three: blue,
teal, mauve of the theme; a fourth orchestration shares one). The rows come
from the config `{{run}} herdr` writes (breadboard's `herdr/README.md`); herdr forgets
the marks on a restart and the layout sets them again. The overview shows the
origin next to the worktree and in its view. They stay together (#255): the
worktrees of no orchestration first, then per orchestration its space with
its worktrees right below in the work order of `wt:plan`; the overview of
the main space sorts them on every read (`core/src/model/order.rs`). A
worktree has no space of its own (#409): its agent's state is its row in
the overview, from the status table (How agents run).
The tab bar's "N need you" counts the agents that ask (`asks`: a denied
tool waiting for the user).

## The changelog

The tab `changelog` of the space shows what the orchestration got done:
`.claude/orchestrations/<name>.changelog.md` of the main checkout, every
merged MR newest first, grouped by day and by its Conventional Commit type,
with `!N`, `#issue`, the merge time and a line or two of its description's
summary; issues closed without an MR below. Written from the record and
GitLab, no model asked: by the overview when its GitLab read sees a merge
it lacks (at once after its merge click), and by `wt:remove` / `wt:prune`.
The tab redraws when the file changes and scrolls like the overview.
`{{run}} wt:orchestrate <name> --changelog` rewrites and prints it.

The tab `changes` (with nvim on the PATH) shows what it changed in the code,
read only: breadboard's `nvim/orchestration.lua` lists the files of its
MRs, merged and open, per MR (`!N #issue title`) or as one list (<Tab>), new /
changed / deleted with +/- counts, read again every minute and on `R`. <CR>
opens a file at its first change, its new lines marked; `:OrchestrationDiff`
shows it against its state before the MR (combined: before the
orchestration). Merged MRs count as their merge commit, open ones as their
branch against where it left its base: not what the base got meanwhile.
`{{run}} wt:orchestrate <name> --changes` prints the set as JSON.

## The end

Each worktree goes with `wt:remove` / `wt:prune` after its merge, as always,
its origin with it, its agent free. Once none is left and all is merged, UP NEXT (and so NEXT)
has the row `end`, and `✕ end orchestration` at the overview's end ends
it with two clicks (greyed out while a worktree is left, naming it; free
agents don't hold it):
the space `⇶ <name>` closes, the orchestrator's claude and the overview with
it (its agents' queues go, a run in flight ends first), every conversation
stays in `/resume` of the main checkout, and its record
and checkout go (a checkout with a change stays, the log says where). Its
changelog stays; the log names where it is.
`{{run}} wt:orchestrate <name> --end` does the same from the command line, with
the same check; from inside the space it runs in the background
(`$TMPDIR/orchestration-end-<name>.log`). Without either, the space closes by
itself once its last worktree is gone and nothing runs in it but its
overview, and no free agent is in its record: a free agent keeps it open until the end.
