# The rewrite's decision log (#314)

Newest first, one entry per decision with the rule of
[`rewrite-rules.md`](rewrite-rules.md) it follows. The loop decides; the
user reads this instead of answering questions.

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
