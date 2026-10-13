# Orchestrating several issues

Part of the skill [`parallel-work`](SKILL.md): one Claude session (the
orchestrator) gets several issues done at once, each by a real `claude` in a
worktree of its own. The overview shows what they do; the orchestrator only
carries decisions between them and the user.

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
| `<issue>` | a number (the worktree is named after the issue's title, `fix/` with the label `bug`), or `<n>-<feature>` to name it yourself |
| `--shared` | every worktree on the shared stack: for issues that never start the app (~1 GB RAM less each) |
| `--queue` | start none: the overview hands each to a free agent (the one that knows its files first), else starts a fresh one by itself, memory permitting, in the work order of `wt:plan` |
| `--queue --after !<mr>…` | the same, but none starts before those MRs are merged (their node: `waits for !282 to be merged`) |
| `--peer <session>` | the name `ListAgents` shows for the orchestrator, written into each brief |
| `--brief <file>` | text added to every brief, e.g. an order to keep between the MRs |

What it does, in this order:

1. Reads every issue. Closed ones, ones another branch claims and ones with a
   worktree already are skipped and listed; nothing to start stops here.
2. Opens the space `⇶ <name>` (`root`, `overview`, `orchestrator`,
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
   space `⇶ <name>` stands on the orchestration's epic branch
   `epic/<N>-<name>` (made from `origin/{{branch}}` at the first run, the base
   of every task{{#epics}}, [epics.md](epics.md){{/epics}}) in `.claude/orchestrations/<name>/`,
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
5. In each worktree's `<feature>` tab: `claude --session-id <id> --name
   <feature>` with `.claude/brief.md` of the worktree as the first prompt.
   The id is bound to the space, so a herdr restart resumes it.

A worktree counts as made when it exists: `wt:new` may still fail in its
summary (#130), and the next issue still starts. Outside herdr (`HERDR_ENV`
unset) the origin is recorded all the same and the `claude` commands are
printed to run by hand. A new folder asks claude's trust question first: it
waits in the agent's tab.

## Talking to the agents

The agents are Claude Code sessions like any other: `ListAgents` shows them
by their name, the feature. The orchestrator says hello to each right after
the start (`SendMessage(to: "<feature>")`, the command prints the list), so
every agent has its address.

**Lean: the overview reports, the chat decides.** Status never goes through
the orchestrator's conversation: every message there stays in its context.
An agent reports nothing to the chat: its block in WORK shows herdr's
state of it and what it waits for, with commits, MR, pipeline and boxes,
the problem it works on and what it is on now (its `{{run}} wt:action note
"<one line>"`, asked for at every stage, or its last answer's first line).
The brief asks for `SendMessage`
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
guess. Never merge, approve or tag for an agent. The user can also type into
an agent's tab directly: that wins.

**Waiting for other MRs needs no orchestrator.** An agent that has to wait
runs `{{run}} wt:wait !N … --then "<what next>"` in the background and stops:
the command asks GitLab every minute and exits with the line to go
on with (`!213 is merged: merge your base in (3 behind), then …`), which wakes
the agent; early when one is closed, stays in conflict for 30 min, or after
8 h. The wait is in `.claude/wait.json`: the MR row says `waits for !213`, and
when the command is gone (the agent restarted) the overview types the line.

**The overview answers "what now?"**, so the user never has to ask. The
orchestrator sets the order right after the start and again whenever it
changes: `{{run}} wt:plan <name> --work <issue>… --merge <issue>… --why "…"`
(`wt:plan <name>` alone prints it). A step that is the user's and no agent's
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
idle tab, as it does for an agent. No script closes or replaces a tab
`orchestrator` that holds more than an idle shell.

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
`--run <command>` a tab `try` in the worktree's space running it in the
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
`look` (go to the agent's space) — and the automatic `waits` (an agent at a
question or permission), `push` (commits a worktree holds unpushed for over
10 minutes; the click tells its agent) and `merge` (a ready, green MR without
its agent's, none while a change is requested).
Sorted by what blocks most: `waits`, `decide`, `plan`, `push`, `merge`,
`try`, `look`; within a kind by merge order, then age. UP NEXT,
the automatic follow-ups in merge order, one click each: `clean up` (remove
the worktree, have its agent tick the done boxes), `pipeline` (a failed
MR pipeline whose line the live overview couldn't send its agent yet, with
why: no agent, blocked at a question, herdr refused; the click tries again), `order` (the plan
is stale: an issue in it closed or gone, its why names one done, or the
merge order left it; the click writes the computed order, the why dropped
when it names a done issue), `hand over` and `start` (see Agents are workers), `end` (see The end). `wt:action --list` / `--done <id>` from the
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
  holds: `at work`, `asks you` (blocked, or stopped on a question), `waits
  for the pipeline` (running, or none yet for its push), `pipeline failed`,
  `waits for you: merge, try #4, decide` (its open actions, a merge once its
  MR's pipeline passed and no try or acceptance box is open, else `checks
  open: …`), `waits for !N` (wt:wait), `→ next #N after !M` (an announced
  handover), `change requested`, `not pushed: N commits`, else `free` (or
  `no agent`). Coloured by whom it waits for, in herdr's theme colours: at
  work yellow, GitLab blue, you red, free teal;
- **title**: the MR's, with `ready to merge` (`merging …`, `merged`) or a
  merged one's age at the right;
- **bar** of its issue's boxes (#294): `9/12 boxes · work 9/9 · acceptance 0/3`;
- **what it lacks**, each `→ who` acts on it (agent, you, GitLab, `!N`):
  the button `resolve merge conflicts` while GitLab reports some (its click
  types the order to merge its base in, resolve, check and push into the
  MR's agent; grey with the reason when the click can't reach it; after a
  click the block says where it stands: `queued for its agent (at work)`,
  `its agent merges its base in`, `pushed, GitLab checks`, `conflicts
  resolved`, #290), `checking for conflicts …`, the reasons; nothing when
  nothing lacks;
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

A `merge` action, posted or found by the overview (ready, pipeline passed),
merges on two clicks: the first arms it (`click again to merge !N`, for 5 s),
the second merges the MR as the user who runs the overview, with the MR's
squash setting, the source branch deleted, at the commit whose pipeline
passed. A merge the agent posted may be a draft: the click marks it ready
first. While anything blocks (open boxes or tries, a pipeline that didn't
pass, conflicts, a draft nobody posted the merge for), the row is grey and
names it in one line, and a click opens the MR instead. After the merge the
agent gets `[overview] !N merged`, NEXT moves on to the next ready MR, and that
MR's agent gets `merge your base in` when the merged MR touched its files;
`clean up` offers the worktree. Agents still never run `glab mr merge`: the
click is the user's (`core/src/model/merge.rs`).

**Into an epic the merge needs no click.** A task MR whose target is its
orchestration's epic branch merges by itself under the same rule (bar full,
pipeline green, no conflicts, its turn), without a posted `merge`: the
overview marks a draft ready and merges it; the row says `in epic` until the
epic reaches `{{branch}}`. The two clicks stay for the milestone MR epic →
`{{branch}}` and for anything into `{{branch}}` or `main`{{#epics}} ([epics.md](epics.md)){{/epics}}.

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
(`answer`, `answeredAt`), and the overview types it into the agent's pane
as one line, `[overview] #3 decide "A or B" → B`: the `<feature>` tab of the
worktree's space, or the `orchestrator` tab for the orchestrator's own
(`herdr agent prompt`). An idle agent goes on; a working one queues the line.
A delivered answer leaves ACTIONS and stays readable in the file's
`answered` (the newest 50, with `deliveredAt`; `wt:action --list` shows
them), so a late message about it is known as decided.

`broken` and `different` are not sent: the click focuses the agent's tab
(its space first) and writes the draft `[overview] #3 try "…" → broken: `
into its input (`herdr pane send-text`, no Enter). The user adds what is
wrong, pastes screenshots and sends it; the agent waits for that line
instead of guessing from the bare word. The draft counts as delivered once
written: no retry and no Stop hook sends it again. A working agent gets it
all the same (Claude Code queues it). An agent blocked at a question gets
nothing typed into the dialog: the row says why, and the draft is written
once the pane changes state, unless the user typed into the input
meanwhile. A `broken` on a try of a ready MR also puts the MR back to draft
and withdraws its merge (see Merging).

herdr refuses to type into an agent blocked at a question. Any other line that
didn't get in waits in the actions file and is tried again once the pane
changes state. The Stop hook (`breadboard hook stop-answers`, `breadboard init` puts it into `.claude/settings.json`) is the safety
net: it only reads the worktree's `.claude/actions.json` (in the main
checkout, the orchestrator's actions whose `pane` is its `$HERDR_PANE_ID`),
blocks the stop with the lines and takes them out. Without an answer it
exits silently. `{{run}} wt:action --answers` does the same by hand. Each
answer arrives once, and no agent turn is spent waiting for one.

A click on a button shows at once (#303): the buttons give way to `→
looks good` and where it is (`typed into its agent's pane`, `at its next
stop`, why it waits), before the reread. The button a click hits is read
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
that names what the agent had before, moves the binding
(`.claude/herdr-session`) along and **restarts** the agent in the new
worktree (#335): its claude ends (nothing is typed into a running one), then
`claude --resume <id> --name <feature> '<line>'` runs in the tab `<feature>`
of the new space, standing in the worktree, its old tab closed. So its Claude
Code name is the feature's (`ListAgents` and `SendMessage` go by it), the
record's too (its node shows `was <old>` for a day), and its first prompt is
`[overview] next issue #N, your name is <feature> now: your conversation runs
again in its worktree <dir>, read .claude/brief.md`. A claude resumed in one
directory can't `EnterWorktree` out of it, hence the restart; the resume
starts only once the old process is gone. Without a space or an idle tab
nothing ends and the handover fails, its worktree removed again. The old
worktree keeps its space until its clean-up. `--to` takes the old name too.
Refused while the agent works or waits for an answer, or above `--limit`
(50 %) of its context.

**A merged agent stays free** (#289), without a tab (#345): before the
clean-up of its worktree the live overview types its last line, `[overview]
!N merged, your worktree goes: you are free. Your claude ends now, …`, waits
for its answer, ends its claude (waited for) and closes its tab: nothing of
it runs. The record's `workers` keeps it by its conversation's id (`free: {
at, from, mr }`, `mrs`, the files it knows, its name and issues), its
binding goes with the worktree's, and the worktree goes once its claude
ended. A herdr restart changes nothing, there is no tab to restore. Its next
issue resumes it (`claude --resume <id> --name <feature> '<line>'`) in the
new worktree's tab, the path of every handover; never two claudes on one
id. Above `--limit` (50 %) of its context it isn't freed and goes with the
worktree. While the record keeps a free agent, the space `⇶ <name>` stays.

A handover that has to wait for other MRs is announced, not kept in your
head: `{{run}} wt:orchestrate <name> <issue> --to <agent> --after !232 [!233…]`
records it (the record's `handovers`, the issue queued), the agent's node
in WORK says `→ next #226 after !232` and UP NEXT lists it. The live
overview hands the issue over by itself once the MRs are merged and the
agent is idle, free or still in its worktree; nothing holds the clean-up
for it but the move itself: `wt:remove` and `wt:prune` refuse a worktree a
handed-over agent is still in.
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
the id copies `claude --resume <the whole id>` (wl-copy, xclip or xsel; the
log shows the command either way), to resume or look into it by hand; a
handover ends such a claude before it resumes the conversation. Two clicks
on a row dismiss one: the record forgets it as free (a tab resumed by hand
closes), the conversation stays in `/resume`.

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
the main space sorts them on every read (`core/src/model/order.rs`).

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
the space `⇶ <name>` closes, the orchestrator's claude, its free agents
(also one moved elsewhere) and the overview with
it, its conversation stays in `/resume` of the main checkout, and its record
and checkout go (a checkout with a change stays, the log says where). Its
changelog stays; the log names where it is.
`{{run}} wt:orchestrate <name> --end` does the same from the command line, with
the same check; from inside the space it runs in the background
(`$TMPDIR/orchestration-end-<name>.log`). Without either, the space closes by
itself once its last worktree is gone and nothing runs in it but its
overview, and no free agent is in its record: a free agent keeps it open until the end.
