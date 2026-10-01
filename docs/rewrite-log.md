# The rewrite's decision log (#314)

Newest first, one entry per decision with the rule of
[`rewrite-rules.md`](rewrite-rules.md) it follows. The loop decides; the
user reads this instead of answering questions.

## 2026-10-02 01:00 · CLI / overview --once · GitLab's call budget (#305) and the disk cache
Chose: the printed orchestration reads GitLab in at most three rounds of
calls side by side: (1) the open MRs, the last 100 pipelines, the issues
not known as closed; (2) the MRs updated lately, only for a branch neither
open nor known as final, the blocked-by links of all open issues in one
GraphQL query (`glab api graphql`, `blockedByIssues { nodes { iid } }` as
real GitLab has it), the jobs of a failed open pipeline; (3) single MRs,
only for what the lists don't hold. What can't change any more comes from
`.claude/gitlab-cache.json` and goes into it: Node's file, kinds and shapes
(`mrs` + `branches`, `issues`, `states`; `jobs` only read: the print reads
no job logs, and Node caches a failed pipeline's jobs only with them).
`--once` writes that cache: it holds only facts GitLab can't change any
more, Node's `--once` writes it too, and spec §9's "changes nothing" is
about the workflow (actions, deliveries, herdr, GitLab), which it still
doesn't touch. Measured in the breadboard world at COLUMNS=120: 5 calls
cold (was 11), 4 on the second run (was the same 11), whatever the number
of open issues (was 4 + one per open issue); the print is byte-identical
to before, and Node's print on the cache ours wrote is byte-identical to
its cold one. In the world the GraphQL call is a 404: the fake glab sends
`api graphql` to its REST routes, and the fake's `blockedByIssues` is a
list where GitLab has a connection (both for the gauntlet to fix). The
links are then left out, as Node's failed `issues/<n>/links` read did;
the scenario has none. Supersedes "GitLab's call budget waits for core's
client". Against: waiting longer for core's client (the critic's top gap
two rounds running; its cache is this file and format, so the move is a
swap); a second HTTP client with token, retries and rate limit in the read
group (core's job, rule 4).
Why: rule 1 and rule 7 (#305: at most 10 calls, a second run asks nothing
final), rule 5 (the cache file Node reads), rule 4 (glab, no new crate).

## 2026-10-02 00:20 · CLI / wt:remove, wt:prune, wt:herdr, handover · the plugin after a move (#230)
Chose: once per run inside herdr, `wt:remove`, `wt:prune`, `wt:herdr` and
the handover ask `herdr plugin list`; when `monocircuit.platform` is linked
from a path that is gone, they `plugin unlink` + `plugin link` the
package's current `herdr/` (the main checkout's, else the one next to the
binary, as session.mjs `pluginDir`) and say `▸ plugin monocircuit.platform
linked again from <dir>: <old> is gone`. The repo's `.herdr/config.toml`
gets every gone `…/herdr` (the listed one, and any `<dir>/run.mjs` whose
dir is gone) replaced, `server reload-config` only when this herdr runs
with that file (the session `platform`). `~/.config/herdr/config.toml` is
only read: a gone path there gets one line naming it and what to do
(`pnpm herdr:install writes the plugin's block again; a line of your own
there needs <dir>`). wt:remove checks after the removal (the plugin may
have been linked from the tree that just went), wt:prune after all its
removals; `wt:prune --dry-run` and a handover `--dry-run` ask herdr nothing.
herdr's `plugin list` format is read loosely (JSON: the entry naming the
plugin; text: its line and the indented ones under it); a listing that
names no path changes nothing.
Against: editing `~/.config` (the user's memory ask-before-global-config:
a change under ~ gets a question first, and the loop asks none; the plugin
piece's `herdr:install` owns that block); relinking a path that exists
(a plugin linked from a worktree on purpose stays).
Why: rule 2 (#230's criteria beat parity); rule 6 (herdr writes only when
the link is broken: one read a run otherwise). Note for the suite: the fake
herdr counts `plugin list` as a write (it is not in its READS); it is a read.

## 2026-10-02 00:15 · CLI / internal names · remove, prune, orchestrate
Chose: `breadboard remove|prune <…>` and `breadboard orchestrate <name>
<issue> --to <agent>` are `wt:remove`, `wt:prune` and the handover, as
today's `worktree.mjs remove|prune|orchestrate` (spec §0.1, the overview's
✕ and the tabs spawn them). `breadboard herdr` is not taken for wt:herdr:
it is the name `pnpm herdr` (setup/plugin piece) wants, and the plugin's
`restore` calls `breadboard wt:herdr --no-focus` at the cut-over.
Why: rule 5 (today's names keep working), rule 4 (one name per command
where two pieces want the same word).

## 2026-10-02 00:10 · CLI / wt:herdr · whose conversations are merged
Chose: the layout merges only the claude projects of worktree roots
(`<main>-<feature>`, there or gone) into the main checkout's project, told
by the `cwd` its transcripts started in; a project of a session started in
a subdirectory (`platform/packages/breadboard`, `platform-314-x/.claude/…`)
stays its own. Against: today's prefix match on the project's name, which
merged and symlinked those too (seen on the live machine).
Why: rule 3 says decide it; the name can't tell `/` from `-`, the
transcript can, and a merge is not undone (rule 5's spirit: no lost work).

## 2026-10-02 00:05 · CLI / wt:remove, wt:herdr, handover · what they say
Chose: `wt:remove` refused for uncommitted changes lists them first
(`▸ uncommitted in <feature>:`, the porcelain lines, at most 10), the ✗
line stays today's and last; refused for commits develop lacks it says
how many and its MR (`…has 2 commits that are not in develop, its MR !308
is open. Merge its MR first, or keep the branch with --keep-branch`;
merged without them → `An MR of their own (pnpm wt:mr)`; none → `pnpm
wt:mr`; GitLab silent → today's sentence), one glab read on that path.
`wt:herdr` says `▸ herdr: the layout of Monocircuit/Platform and its <n>
worktrees` before its seconds of work. A handover whose `/rename` doesn't
show at once says `▸ /rename <feature> typed, waiting for its session to
show the name` (and again when typed a second time); the record's
`renamed` is set only once the rename took (#313). A handover whose
wt:new failed after the worktree existed ends `✓ … told in its prompt (wt:new
failed after the setup, see above)`, as fresh starts do.
Why: rule 3 + the bar (lazygit names the files; a silent 20 s wait reads as
hung); the ✗ line other processes parse is unchanged.

## 2026-10-01 23:55 · CLI / overview --once · GitLab's call budget waits for core's client
Chose: the printed orchestration keeps its 11 GitLab calls (4 lists, then
the links of each open issue) and reads no disk cache, until the core
snapshot carries core's GitLab client (`crates/core/src/gitlab` of the
branch: one GraphQL query for the links, final things in
`.claude/gitlab-cache.json`); the print then moves onto it. `wt:issues`
keeps its notes per issue the same way; its error names GitLab once
(`✗ GitLab could not be read: Error: bad port (projects/…)`).
Against: a second HTTP and GraphQL client in the read group.
Why: rule 4, one GitLab client; `glab api graphql` is no way around it
in the world (the fake glab sends it to the REST routes), and the fake's
`blockedByIssues` is a list where real GitLab has a connection
(`{ nodes { iid } }`), which the suite should match.

## 2026-10-01 23:50 · CLI / overview --once · headings, refusals, what a print says
Chose: a box heading too long for its line drops its ` · ` parts from
the end, its counts first stay (boxes.mjs `fitParts`, no `…`): at 80
columns `WORKTREES 7 wait for GitLab · 1 will be removed · 41 removed`,
`AGENTS 3 free`, `MERGE REQUESTS 7 open · 42 merged`; only a first part
too wide for the line wraps. Round one wrapped the tail into the box,
where it read as the box's first line. `overview --worktree <dir>` of a
directory in no worktree is `✗ <dir> is not a worktree of the repo: pnpm
wt:list shows them` on stderr, exit 1 (Node printed `null` or the name
and went on, exit 0); `--orchestration` with `--worktree` is `✗
--orchestration and --worktree exclude each other: one view at a time`
and the usage, exit 1 (Node silently took the orchestration, round one
the worktree). GitLab's error line ends after its reason: a print asks
once, `tried again` was not true.
Why: rule 3 with rule 2 (#304: no `…`); rule 5 keeps the flags, the
input that can't be shown is refused instead of drawn.

## 2026-10-01 23:45 · CLI / overview --once · agents and tries as the live view knows them
Chose: the printed orchestration reads herdr once (`herdr pane list`,
inside herdr only, read only) and the conversation bound to each worktree
(`.claude/herdr-session`, its transcript's tail): WORKTREES says `at work`,
`asks you`, `→ next #N after !M`, `being set up for … (wt:new) …, leave
it`, `waits: its agent is working` / `<claude> still stands in it` for a
merged one, and per node `agent <name>`, `was <name>`, its context (`30%
of 200k · 5 min ago`); ACTIONS gets the `waits` of an agent's question or
permission prompt, `clean up` of a merged issue's open boxes, a failed
pipeline's `click to tell its agent`; a try shows its state in its head
(`ready`, `not pushed`, `waits for merge !N`, `deploying (publish, 3
min)`, `deploy failed (job)`, the deploy read only for an open try on the
test instance) and one not ready is grey, without buttons, skipped by
NEXT; UP NEXT the handovers (announced, or to the free agent that knows
the issue's files, else the one with most room) and fresh starts
(memory permitting); AGENTS each free agent's state and context; NEXT
`· 2 agents at work`; the resolve button only when a click reaches the
agent; `--worktree` the AGENT section. In a world with sessions and
transcripts the output equals Node's but for the heading (above) and
the uncut titles (#304); 1 herdr call where Node makes 4 (it reads the
orchestrator's tabs and panes for a chat button `--once` doesn't draw).
A print starts nothing: a due handover or fresh start is named, not run.
Not yet: STACK's migrations line (psql).
Against: waiting for core: its `conversations.rs` is in the snapshot but
not compiled; `cli/read/overview/agents.rs` is a trimmed copy of it and
goes once core exports it.
Why: rule 3, the critic's biggest gap (the 22:50 entry's lost lines);
rule 6, herdr is only read.

## 2026-10-01 23:40 · CLI / wt:orchestrate --to · one handover
Chose: `wt:orchestrate <name> <issue> --to <agent> [--after !<mr>…]`, the
call people and the overview make, runs the lifecycle group's handover
(`cli/lifecycle/handover.rs`): `cli/lifecycle.rs` takes `wt:orchestrate`
when `--to` is given and none of `--end`, `--changelog`, `--changes`;
every other `wt:orchestrate` goes on to the setup group. Its command line
is checked first: `--dryrun` is `✗ unknown flag --dryrun` and the usage,
exit 1, nothing read or written (it handed over for real before: worktree
made, branch pushed, herdr `pane move` and both prompts); `--title`,
`--queue`, `--now`, `--cap`, `--watch` with `--to` say they don't go with
it; `--help` prints what a handover does. The names `wt:handover` and
`wt:change` are gone: they were in no package.json. A change for an MR is
`wt:action change` alone (the write group's, which tells "no MR" from
"glab failed" and takes the actions lock); lifecycle's copy is deleted.
Against: two implementations each, the weaker one the one that ran.
Why: rule 5 (the names stay exactly, no new ones), rule 4 (one way to do
a thing), rule 6 (a typo is no herdr write). The setup group's
`hand_over`/`announce` in `cli/setup/orchestrate/mod.rs` are unreachable
now and go in its next commit.

## 2026-10-01 23:35 · CLI / wt:remove, wt:prune, wt:herdr, handover · the command line
Chose: the read and write groups' check and wording: `--help` / `-h`
prints what the command does and its usage on stdout, exit 0; an unknown
flag (`✗ unknown flag --keep-brnach`), a valued flag without its value, a
switch given one, a word too many (`✗ x is one argument too many`) is
refused with the usage, exit 1, before anything is read. `wt:remove`
takes the worktree's folder too (`../platform-303-x`, `platform-303-x`),
and a name that is none lists the worktrees after today's
`✗ <dir> is not a worktree (pnpm wt:list)`. Against: Node's way of
running on (`wt:remove 303-… --keep-brnach` removed the worktree and
deleted its branch, exit 0).
Why: rule 6 over rule 3: a typo of a flag that keeps something must not
destroy it; rule 4: one wording across the CLI (the three checkers belong
in one `cli/args.rs` once the groups meet).

## 2026-10-01 23:30 · CLI / wt:prune · a failed remove, the columns
Chose: a worktree whose removal fails gets `kept, wt:remove failed:
<reason>` on its line and the prune goes on with the others; at the end
`✗ <n> worktree(s) could not be removed: the reason is on each line
above`, exit 1. The names are padded to 32 as today, or to the longest
name when one is longer, so the second column stays one column.
Against: Node, whose in-process `fail` exited in the middle of the prune
(the branches and folders after it untouched, no report); the frozen
32-column padding that pushed long names' lines out of line.
Why: rule 3 says decide it; the entries are independent, and a prune
that stops halfway without its report is the worse tool (rule 4: the
same loop, no early exit). The padding changes nothing for names under
32 characters.

## 2026-10-01 23:25 · CLI / wt:remove · GitLab reads, the changelog, what it says
Chose: one `merge_requests?state=all&source_branch=` read answers both
questions of a removal (an open MR on its claim, when it was merged):
1 GitLab request where Node made 2. The orchestration's changelog rewrite
after a merged removal runs as a detached `wt:orchestrate <name>
--changelog` process (its own process group, output to nowhere), so the
removal neither waits for it nor cuts it off. `--keep-branch` ends
`✓ <feature> removed, <branch> kept` merged or not; when GitLab doesn't
answer whether an open MR builds on the claim, the claim stays and a step
says so with the command to delete it.
Against: Node's `writeChangelog` promise kept alive in the same process;
its silent claim kept on a GitLab error.
Why: rule 7 / #305: the rewrite itself still asks GitLab about 96 times
(one MR list per record branch, one issue per record issue) in the
world; it is the setup group's `changelog.rs` and moves to core's
batched/GraphQL client. Rule 3: the background rewrite is today's, only
the mechanism differs.

## 2026-10-01 23:20 · CLI / overview --worktree --once · the worktree's view
Chose: the printed view of one worktree ports render-worktree.mjs: WORK
(origin, issue, MR with its URL, `tests untested: 1 commit pnpm wt:check
before the push`, the delete button or why not yet), WAITING FOR, ISSUE
with its boxes per heading, PIPELINE with every job and a failed one's
log, DIFF against develop by area with its overlaps, DEVELOP since the
branch started with `✓ merges cleanly` (`git merge-tree`), WHERE and STACK
with containers. In the scenario world it equals Node's but for the
PIPELINE line, which wraps instead of cutting the last job (#304). 9
GitLab calls (Node 11 requests, 17 glab processes), 230 ms (Node 600 ms).
Not yet: AGENT (the conversation, core's transcript reader), the try
state of a merged MR's deploy, and STACK's migrations line (psql).
`--worktree` takes the worktree the directory lies in, not only its root;
one that isn't prints the directory (Node printed `null`).
Why: rule 3, the critic's "a developer would rather use Node's here";
rule 2 (#304) for the wrap.

## 2026-10-01 23:15 · CLI / overview --once · the view of everything
Chose: render.mjs ported: `platform  develop ◐ running` in the band,
WORKTREES with app, stack (`no .env.local`), work and `untested: 1
commit`, the MR and its pipeline; STACKS WITHOUT A WORKTREE; MERGE
REQUESTS with branch and assignee; ISSUES claimed first, claimed meaning
a branch on origin named for it (Node's rule; round one also counted an
open MR's branch, which said claimed where Node said unclaimed). Every
cell whole: an issue's title comes before its claim and labels and wraps
under itself (one line for most of the 64 issues, Node cut them with
`…`); a cell with too little room left goes on its own line under the
name (`!303 draft ✓ passed` under its worktree). 83 ms, 3 GitLab calls
and one `git ls-remote` (Node 365 ms).
Why: rule 3; rule 2 (#304) over Node's cuts; rule 4: one table helper
that wraps for all three views.

## 2026-10-01 23:10 · CLI / wt:plan, wt:list, wt:issues · small deviations of round one
Chose: outside a git checkout every read command says `✗ not in a git
checkout of the platform repo: run it in the main checkout or one of its
worktrees` (Node ran in its own source's repo); `wt:plan` refuses a name
outside kebab-case (`../x` read a file outside the records) and writes
the plan under `<record>.lock`, the lock `wt:action --orchestration`
takes, so the two never lose each other's write (Node wrote unlocked).
Why: rule 3 with the spec's bug list (§11): the same files and formats
(rule 5), the lock directory is Node's own, so Node and Rust keep working
side by side.

## 2026-10-01 23:05 · CLI / wt:list, wt:plan, wt:issues, overview --once · the command line
Chose: the read commands check their command line as the write ones do:
`--help` / `-h` prints what the command does and its usage (exit 0, no
GitLab read); an unknown flag, a valued flag without its value
(`overview --orchestration --once`), a word too many (`wt:list x`) is
`✗ <problem>` and the usage, exit 1. `wt:plan x --work=3` is refused with
`✗ --work takes its issues as separate arguments: --work 3`, `--work abc`
with `✗ abc is no issue number`. `overview` checks only when it prints
(the live view's flags are the TUI's); `--worktree` alone stays the cwd.
Against: Node's silence (`wt:plan x --work=3` wrote nothing, exit 0;
`wt:issues --help` read GitLab).
Why: rule 3 over the letter of rule 5: the names and flags stay, a typo
no longer passes as success. Hand parsing, one 60-line checker shared by
the group (rule 4); the write group has the same one, they belong in one
`cli/args.rs` once the groups meet.

## 2026-10-01 23:00 · CLI / overview --orchestration --once · the frame the live view draws
Chose: the printed view ports render-orchestration.mjs and what it draws
from (merge-order, mr-blockers, tree, numbering, history, next,
action-block, the auto actions, boxes and the title band) into
`cli/read/overview/orchestration/`, painted as Node paints (`FORCE_COLOR`:
islands, truecolor band, herdr's tones and pink, OSC 8 links). Every open
MR has its second line (`checking for conflicts… · pipeline pending →
GitLab · draft: mark ready → agent`), merged rows their pipeline, worktree
nodes their issue state, merge time, `untested: 1 commit`, `agent: 2nd
issue (#297 → #305 at 13%)` and wait, the state in the colour of whom it
waits for; WORKTREES counts as Node (`7 wait for GitLab · 1 will be
removed · 41 removed · 9.3 GB free …`, `agents 45 · issues 49 · reused 4,
started at 13% · handovers failed 1`; #242, no worktree and no removedAt,
counts as removed again). In the scenario world the output equals Node's
but for the free memory and the two differences below. Against: the
TUI's snapshot and section renderers, which aren't in this clone yet:
when core's model lands the printing moves onto it, the layout code
here is the reference for it.
Why: rule 3 (spec §9: `--once` prints the frame the TUI draws). Two
deliberate differences: nothing is cut with `…` (#304, rule 2): a long MR
title wraps under itself (`!318 … (#305)`, the merged titles); and a
wrapped line uses the box's full width (Node lost two columns to its
indent strip: AGENTS' `dismiss ]`, the reasons of !316 and !318). Rule 7:
171 ms and 11 GitLab calls against Node's 1.6 s and 157.

## 2026-10-01 22:55 · CLI / overview --once · GitLab, read only, no cache
Chose: an orchestration's GitLab read in lists side by side: open MRs,
the MRs updated lately, the last 100 pipelines (head pipeline by commit),
the issues by iid; then the links of the open issues, the jobs of failed
pipelines only, single reads only for what the lists don't hold (an MR's
pipeline, an awaited MR's state). No `.claude/gitlab-cache.json` is read
or written. Against: Node's per-branch reads (157 calls cold) and its
cache writes from `--once` (#305).
Why: spec §9 says `--once` changes nothing; a print that writes a cache
other overviews merge into is a write the user didn't ask for (rule 6 in
spirit), and the cache belongs to core's GitLab client (plan step 1),
which every view will share. Rule 7: 11 calls in the world, the jobs of a
running pipeline aren't needed for any line it prints.

## 2026-10-01 22:50 · CLI / overview --once · what the print can't know yet
Chose: until core reads herdr and the agents' transcripts (its
`conversations`, `check` and `wait` modules are in the snapshot but not
compiled), the printed orchestration knows no agent's herdr state: a
worktree whose agent has nothing to wait for says `no agent`, a free agent
shows `0 %` of its context, there is no `at work` / `asks you`, no try
state in an ACTIONS head, no `waits` action from an agent's question, no
handover or start in UP NEXT (workers.mjs), no `being set up` (setups) and
no clean-up held by an agent; GitLab's minute shows without the request
rate. Each comes with core's reader; none is dropped. With GitLab down the
error line under the band names the reason (`GitLab didn't answer 22:55
(Error: connect ECONNREFUSED …), tried again`, the first meaningful line
of glab's stderr, not its last `Node.js v25` line) and NEXT says `what is
for you can't be told: GitLab didn't answer` instead of Node's progress
counter.
Why: rule 3: logged, not lost; rule 4: no second herdr and transcript
reader next to core's.

## 2026-10-01 20:10 · CLI / wt:action, wt:tick, wt:note, wt:mr, wt:wait · the command line
Chose: each command checks its whole command line before it reads or
writes anything: an unknown flag (`✗ unknown flag --dryrun`), a valued flag
without its value, a switch given one (`--update=yes`) or a word too many
is refused with the usage, exit 1. `--help` / `-h` print what the command
does and its usage on stdout, exit 0. `wt:mr <x>` refuses an `<x>` that is
no issue number at once. Parsed by hand, no clap. Against: Node's way of
reading only the flags it knows and running on.
Why: rule 6 over rule 3: in Node `wt:mr --dryrun` (or `--help`) installed
the push guard and ran `glab mr create --draft`, `wt:tick 307 3 --help`
ticked box 3 on GitLab, `wt:action look x --bogus` posted and dropped the
flag; a typo must never be a GitLab write nobody asked for. `wt:wait`
already refused unknown flags in Node and keeps its wording. Hand parsing:
rule 5 keeps `flagValue`'s exact reading (first occurrence wins, a next
`--x` is no value, `--flag=value`), which clap would change; rule 4: one
50-line checker, no dependency, start stays ~2 ms.

## 2026-10-01 20:05 · CLI / wt:mr · closed, merged, missing template, URL
Chose: a merged MR of the branch is refused (`!N of <branch> is merged
already: new work goes on a new branch …`); a closed one is history, the
branch gets a new draft MR (`--dry-run` previews it), and the output says
`!N … is closed: a new merge request was opened, that one stays closed`.
A missing `.gitlab/merge_request_templates/Default.md` is a `✗` line that
says what to write. When glab prints no URL after `mr create`, it is read
back with `glab mr view <branch>`; failing that the line says how to find
it. Against: Node's "open already" for any MR glab returns, its ENOENT
stack trace, its empty URL line.
Why: rule 3, the documented intent (parity spec §7, §11): "open already"
for a merged or closed MR is wrong, and a reopen had no way around but a
hand-written `glab mr create`; rule 6: the create is the same write Node
makes, only no longer blocked.

## 2026-10-01 20:00 · CLI / wt:wait · ends when it should
Chose: an MR GitLab answers 404 for ends the wait at once (`✗ GitLab has
no MR !99999 and !99998`, the wait cleared). Between asks it sleeps at most
until `--hours` runs out (`--hours 0.0001` ends after ~0.4 s, Node 60 s)
and looks every second whether `.claude/wait.json` is still its own: after
`--cancel` or a newer wt:wait it exits within a second (Node polled GitLab
up to another minute).
Why: rule 3, Node's intent: its 404 test read a field its errors don't
have, so a typo waited 8 h; rule 7: measured 92 ms for the 404, 433 ms for
the limit, 564 ms after a cancel. One ask a minute stays (rule 6).

## 2026-10-01 19:55 · CLI / wt:tick, wt:mr --update · the description written as text (#157)
Chose: `glab api --method PUT projects/:fullpath/issues/<n>` (and
`merge_requests/<iid>`) `--raw-field description=<text>`. Against:
`glab issue|mr update --description-file`, which glab 1.114 doesn't have.
Why: rule 2, #157's acceptance; the route GitLab sees is the same PUT
(rule 6). `--raw-field`, not `--field`: a body starting with `@` would be
read as a file.

## 2026-10-01 19:50 · CLI / wt:note, wt:action · small deviations
Chose: wt:note's read-back that is no JSON is `⚠ … GitLab does not show
it back` (Node crashed). `wt:action --orchestration` refuses a name outside
`NAME` (`../orchestrations/breadboard` was a path out of the records).
`wt:action --done=5` works like `--done 5` (Node printed the usage).
`--list` leaves out the time of an answered action without `answeredAt`
(Node printed `(undefined)`). A write that finds `<file>.lock` held says
on stderr what it waits for (`waits for …/actions.json.lock, held by
another writer (taken over after 5 s)`; Node was silent for 5 s).
Why: rule 3 says no deviation is silent; each is a crash, a path traversal
or a wrong line in Node, and the formats read by others stay (rule 5).

## 2026-10-01 19:45 · CLI / wt:mr · the push guard shim stays Node's for now
Chose: the pre-push shim wt:mr installs stays byte for byte today's
(`exec node "$top/packages/{breadboard,workflow}/src/git-hooks/pre-push.mjs"`).
Open: at the cut-over (plan step 6) the shim must call the binary, and
wt:new installs the same shim; both change in that one commit.
Why: rule 5: every worktree shares the hooks folder, and a shim calling a
binary that Node worktrees don't have would block their pushes.

## 2026-10-01 17:40 · CI · the suite against the binary
Chose: `check:breadboard-rs` (fmt, clippy `-D warnings`, `cargo test`, the
release binary) and `check:breadboard-gauntlet-rs` (the suite with
`BREADBOARD_SUBJECT` = that binary), the latter `allow_failure` until the
loop has built every piece. `check:breadboard` keeps the Node half (ESLint,
`node --test`) without cargo.
Why: rule 1, the suite is the spec, and it must run on every push; a job
that is red by design while the pieces are missing would only hide a new
failure, so it reports without blocking until the cut-over, then blocks.

## 2026-10-01 17:35 · workspace · two crates, one binary
Chose: a cargo workspace in `packages/breadboard/` with `crates/core`
(`breadboard-core`: records, actions, origin, git, herdr, GitLab, the
snapshot the views draw) and `crates/breadboard` (the binary: `tui/`,
`cli/`, the herdr plugin, the daemon). Against: the plan's five crates.
Why: rule 4: the TUI, CLI, daemon and plugin are one binary and share all
their data code; splitting them into crates adds manifests and no boundary
anyone reads. `core` stays apart so the readers are tested without a
terminal and compile once. The package's `lint`/`test` run cargo too, so
`pnpm wt:check` gates the Rust code like the Node code.

## 2026-10-01 17:30 · tools · `snap`, a screenshot for builders and critics
Chose: `pnpm --filter @monocircuit/breadboard-gauntlet snap`: any TUI
(Node, Rust, lazygit) in the gauntlet's pty and world, a script of keys and
mouse played in, the screen as text and as a PNG drawn by headless
Chromium, `--blind` masks the names.
Why: the critic compares screenshots and interaction blind (the loop's
prompt); one tool for both subjects keeps the comparison fair (same
terminal, size, font, palette).
