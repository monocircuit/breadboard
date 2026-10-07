# The rewrite's decision log (#314)

Newest first, one entry per decision with the rule of
[`rewrite-rules.md`](rewrite-rules.md) it follows. The loop decides; the
user reads this instead of answering questions.

## 2026-10-07 20:50 · overview / MERGE REQUESTS · round 1 of the rebuild loop: the merged rows' age alone, closed apart, `→ you` bold
Checked against the spec (§11) first: merge order with lanes and joins
(core's `graph_of`, Node's glyphs, probed with `needs`/`after` edges:
`├─┐`, `├─┘`, lanes through bars and reasons), pipeline marks that open
their pipeline, whole titles (#304), the bar (#294), reasons with `→ who`,
the conflict cell and its button, the two-click merge (probed in a world:
`click again to merge` in red in the button's own cells, the second click
a single `PUT merge_requests/308/merge`, then `[overview] !308 merged` to
its agent), the merged fold kept over redraws, all of it in `--once`.
Nothing of Node's is missing; three things change:
Chose: a merged row's status is its age alone (`6 days ago`), dim, under
the fold's `▾ 42 merged`. Against: Node's `merged 6 days ago` on each of
42 rows. Why: rule 4 (the fold and the title say `merged` already; git
log and lazygit show a date column, not a word), and #304: the 7 columns
go to the title, so at 120 columns 13 of the fixture's 42 merged titles
fit on one line that wrapped a single orphan word (`back`, `agent`,
`(#293)`) onto a second.
Chose: an MR closed without a merge says `closed` (core's
`merge_requests_of`, a 3-line change there) and the fold counts it apart:
`▾ 41 merged · 1 closed`. Against: Node, whose rows and fold said
`merged` for a closed MR too. Why: rule 3 deviation, logged: a closed MR
read as merged is a wrong fact, not a wording; the title's `N merged`
already counted only the merged.
Chose: in what an MR lacks, `→ you` is bold, every other who stays dim.
Against: Node's dim for all. Why: the bar (lazygit's status colours say
at a glance which rows are yours); the words are Node's.
Kept: the bar under every open MR with boxes, a full one too (#294's
acceptance, rule 2); `checking for conflicts …` on each unsettled row
(rule 1, the `…` after a space).
Suite: content 24/24, hover 4/4, buttons 23/23 (before and after).

## 2026-10-07 20:40 · suite · the CLI clone's checks reviewed
Kept: `lifecycle.check.mjs`, `setup.check.mjs`, the CLI checks (blocked-by
links with and without GraphQL, GitLab down, a note on a stale issue, the
handover's refusal), the fake GitLab's `blockedByIssues` as a connection,
`isGraphqlDown`, `POST merge_requests`, a note counted on its issue, glab's
`api graphql` and typed `--field`, herdr's `plugin list` as a read,
`GIT_CEILING_DIRECTORIES` in a world (a world under a checkout reaches no
repo around it). Against today's Node every new or changed check passes or
is a known bug: cli 17 + 2 todo, lifecycle 4 + 2 todo + 2 rewrite-only,
setup 18 + 5 rewrite-only. Fixed: the three known bugs without an issue
number say `to file:` (the numbers come when the lead files them), lint
(prettier, an unused constant). Kept as they are: the `rewriteOnly` skips
(an unknown flag Node ignores would act on the shared world; Node's
internal names and a binary copied out of the repo the harness can't run).
Why: the suite's rules (README, "Adding to it"); rule 1.

## 2026-10-07 20:30 · CLI / setup, lifecycle · core's setups and local time, one copy
Chose: the setup claims of wt:orchestrate and the handover are core's
(`breadboard_core::setups`; its `Claim` got `none`, `file` and `release`
from the lifecycle copy), the setup group keeps only the hold for its
`fail` path; the changelog's local day and time are core's `time::day` and
`time::hm`. Gone: `cli/lifecycle/setups.rs`, the setup group's copy,
`cli/setup/localtime.rs`. Not yet: the groups' copies of core's
conversations (`lifecycle/claude.rs`, `setup/conversations.rs`), wait
(`write/wait.rs`), check (`setup/check.rs`), herdr (`lifecycle/herdr.rs`,
`setup/herdr.rs`), the repo (`*/repo.rs`) and the glab readers of each
group: each has its own tests and critic rounds; the next CLI round moves
them onto core one group at a time, the suite judging each move.
Why: rule 4 (one way to do a thing); rule 5 (the files stay as written).

## 2026-10-07 20:20 · CLI + overview · wt:herdr and wt:remove once
Chose: the lifecycle group's `wt:herdr` and `wt:remove` (critic rounds
against Node, `lifecycle.check.mjs`) are the only ones; the main space's
overview (`--herdr`) calls its layout in-process: `layout`,
`sync_stack_tabs`, `prune_spaces`, `sort_spaces`, under its `exclusive`
lock (the lock Node's layout takes too). Against: the overview piece's own
`cli/layout.rs`, `cli/layout/order.rs` and `cli/remove.rs` (2.1k lines,
built while the CLI was in its clone), now deleted. Why: rule 4; rule 6
(the herdr writes are the lifecycle group's, measured against Node there).

## 2026-10-07 20:10 · overview --once · the TUI's print, what the CLI's did better carried over
Chose: `overview` is the TUI's, live and printed once (`--once`, or stdout
no terminal); `main.rs` asks the read group only whether the command line
is refused. The read group's printer (`cli/read/overview/**`, 9k lines)
and its copy of core's GitLab cache are deleted. Carried over from it:
(1) the command line: `--help`, an unknown flag, `--orchestration` with
`--worktree`, a `--worktree` directory in no worktree are refused with
the usage, exit 1; (2) while GitLab hasn't answered a worktree says
`MR …`, as its issue's `#303 …`, never `no MR yet` (the suite's GitLab-down
check); (3) NEXT says `what is for you can't be told: GitLab didn't answer`
where a failed first read left `GitLab …`; (4) a print's error line ends
after its reason (`tried again` only where it is); (5) a GitLab that
answers REST and not GraphQL: core's orchestration read takes Node's REST
route (`gitlab/rest.rs`: the open MRs, the last 100 pipelines, the issues,
the lately updated MRs only for an unsettled branch, each open issue's
links, a failed pipeline's jobs, the awaited singles), remembered until
the next full read; a merged MR's deployment stays GraphQL's. Not carried:
`agent: 2nd issue (#297 → #305 at 13%)` (the TUI's #292 line names the
agent, its issues and merged MRs); colours with `FORCE_COLOR` in a print
(nothing pipes the overview: the plugin's overview pane runs live); the
merged worktree's own row before its removal (the TUI folds a due removal
into "done and removed", rule 1). Measured in the breadboard world at 120
columns: GitLab calls of a print 2 (TUI) against 5 (the read group's)
and 5 (Node); with GraphQL down 5 s of GraphQL's retries, then REST.
Why: rule 4 (one way to draw the overview), rule 1 (the cli checks), rule
3 (Node read REST), rule 7 (#305: the fewer calls).
Suite: cli 18/18 (+1 todo: wt:orchestrate needs claude).

## 2026-10-07 20:00 · merge · the CLI from its clone onto the branch's core
Chose: the branch's `breadboard-core` as it is: the CLI clone's changes to
actions, git, json and lock were on the branch already, record and time
are the branch's (#292's `display_name`, `hm`, `clock`), the rest the
clone's formatting at another width; the CLI built against it unchanged.
`cli/mod.rs` the clone's (a group per module, typo hints), `main.rs` and
the Cargo files the union. Both halves' log entries below, by time.
Why: rule 4 (one core), rule 5 (the formats both read and write).

## 2026-10-02 09:20 · overview / everything · herdr's layout in Rust, the work cell, the MRs at 80
Chose: herdr's layout in Rust (`cli/layout.rs`, `cli/layout/order.rs`):
`wt:herdr` and the main overview's `--herdr` build the main space, a space
per worktree (`worktree open`, else `workspace create`), the
orchestrations' spaces, stack tabs, the marks (`markOrigins`), prune the
spaces of removed worktrees (only when every pane stands in the deleted
directory, a tab an agent works in stays: wt:remove's `releaseSpace`) and
sort the group over herdr's socket (`workspace.move_block`, std's
UnixStream). The overview calls it in-process (Node's `keepInStep`): prune
and sort every tick, the layout when the set of worktrees changes, a
stack's tabs when it goes up or down, without resuming conversations.
Against: spawning `wt:herdr` (it resumes conversations and focuses the
main space, Node's loop does neither; it answered `not built yet`, so the
band showed a red `herdr not in step` all the time). Why: rules 3, 6, 8.
Measured in the breadboard world: `wt:herdr` writes exactly Node's list
(8 `tab close` of 299's space, kept the tab its agent works in; `worktree
open` and `workspace create` for 303; `report-metadata w4P --clear-token
orch1`; `workspace focus wD`), with the same 237 reads; the `--herdr` loop
over 30 s the same writes as Node's apart from how often it ticks (entry
below) and the order of `pane report-metadata`'s flags.
Chose: a new tab runs this binary under the package.json names: `'<exe>'
overview --herdr|--worktree <dir>|--orchestration <name>`, `'<exe>'
changelog --watch`, `'<exe>' wt:orchestrate <name> --changelog --watch`.
Against: Node's `node …/overview.mjs`. Why: rules 5 and 8; the changelog
commands are the CLI piece's to build.
Chose: the nvim tab's command (and the claudecode.nvim clone of ide.mjs
with it) is made only when the tab is made or its shell idles. Against:
Node, which clones on every layout while the plugin is missing. Why: rule
7; the herdr writes are the same.
Chose: a worktree's work names its check first (`untested: 1 commit · 1
changed · 1 ahead`) and is never shortened: where each fact fits the work
column it wraps in the column; where the column is narrower than a fact
(80 columns), the column holds the check in a word and the whole work
stands on the row's next line, under the name. Against: round 1, which
dropped `N ahead` and the commit count unmarked; Node, which cuts with `…`
and names the check last. Why: #304 and the critic; check first, so one
column of checks runs down the rows.
Chose: where an MR's cells go under its title (80 columns), its state and
pipeline always start the next line, padded to columns, branch and owner
after them where they fit whole. Against: after the title's last word.
Why: the critic: the pipeline landed somewhere else on every row.
Chose: the orchestration's heading `⇶ <name>` is a row: hover, and a click
brings its space `⇶ <title>` to the front (`FocusSpace` of its checkout;
`actor/herdr::space_label` names an orchestration checkout's space).
Against: Node's `⇶ <name>` after every worktree's name. Why: one line
instead of the same words on each row; the critic: it had no click.
Logged now, from round 1 (rule 3): without any .env.local the app and stack
columns go and the title says `no .env.local in any` once (Node: `—  no
.env.local` on every row); a worktree's MR and pipeline stand in a column
before the work, the work wraps last; ISSUES has the claim (`← !316`) as a
column and the claiming branch after the title (Node: right after the
`←`), so the titles line up; MR pipelines come from GraphQL's head
pipeline of each MR, so !102 and !184–!188 show `✓ passed` where Node says
`no pipeline` (its REST read pages only 100 pipelines and misses
`refs/merge-requests/102/head`, rule 1 over 3); the `wt_state` of a pane
names its open tries (`checks open: try #1 broken, … · !303`) where Node's
view of everything reads no actions (`waits for the pipeline · !303`), so
the main and the orchestration's overview no longer write two lines for
one pane (rule 4).
Suite: views 9/9, content 24/24, buttons 23/23, hover 4/4, isolation 4/4,
budgets 9/9; cli: wt:herdr green (the other wt:* are the CLI piece's).

## 2026-10-02 09:20 · CLI / setup group · round 2: wt:stack idempotent, exact claims, the Rust pre-push hook
Chose: (1) `wt:stack` in a worktree that has its own stack already
(COMPOSE_PROJECT_NAME and POSTGRES_PORT in its .env.local) sets it up
again instead of refusing: `▸ own stack on offset N already: app …,
Postgres …, Keycloak …, SeaweedFS …, set up again`, `pnpm dev:setup`, the
stack-tab sync, the summary, exit 0. Only an .env.local that differs from
the main one and is no stack's is still refused, now saying so. Node
wrote .env.local before dev:setup, so a failed dev:setup could never be
finished; the failure's hint now says `run pnpm wt:stack in <dir>, it
sets the stack up again`, and every other failing step of wt:new names
the commands that finish the worktree from there (`pnpm install
--frozen-lockfile && … && pnpm wt:stack`, `cp <main .env.local> …` with
--shared) instead of "run the step again". An agent's wt:stack summary is
`✓ own stack +N ready: app …, Postgres …, herdr: stack tabs …`, not wt:new's
"pushed, EnterWorktree" (nothing was pushed, the agent stands there).
(2) `wt:orchestrate <name> --end` of an orchestration nothing is left of
is ✓ and exit 0 as in Node (an end run twice), the line says what was
looked for: `✓ ⇶ tiny ended already: no herdr space, record, setups or
checkout of tiny is left`. Round one's `✗ no orchestration …
(pnpm wt:list …)` named a command that lists no orchestrations.
(3) The claim check of wt:new and wt:orchestrate matches only
`feature/<n>-…` and `fix/<n>-…`: Node's `*<n>-*` refused issue 5 for
`fix/305-…` and 23 for `feature/123-…`. (4) The plugin check (#230) runs
once nothing refused: a usage error asks herdr nothing (2–5 ms instead of
30–40), `--changes` (nvim polls it) never, and a child of this binary
(`wt:new --origin`, `wt:remove`, `--end` in the background, `--to`) gets
`BREADBOARD_PLUGIN_CHECKED=1` and leaves it to its parent. (5) Errors say
what is wrong: `the feature Bad_Name is not kebab-case (…): bad-name?`,
`the orchestration is missing`, `Foo is no issue: its number (12) or
<n>-<feature>`, `which issues? …`, `--cap is gone` before anything else,
`v1.5.0 is the tag, the version is 1.5.0: pnpm changelog:release 1.5.0`,
`the version is missing: …`, `the sentence is missing: …`. (6) wt:new's
stale-issue sweep reads the notes only of issues people commented on
(`user_notes_count`), as wt:issues does: one request per commented issue
instead of one per open one. The sweep stays its own reader until the
read group's `stale_issues` is reachable from here (its module is
private) or moves to core. (7) The brief of a fresh start is the
lifecycle group's `handover::brief_of`, one text for both (round one's
entry 00:55 (1) waited for core; the copies had drifted). (8) `breadboard
hook pre-push <remote> <url>`: Node's hook in Rust (the #184 MR guard
and the #298 gate, same texts, the docs-only stamp), passing the suite's
three #298 checks with a shim that calls it. The installed shim stays
Node's for now: wt:mr's installer (write group) writes the same file,
and plan step 5 switches both in one commit; the shim to switch to tries
`<top>/packages/breadboard/target/release/breadboard hook pre-push` when
that binary answers `hook --help`, else falls back to the Node hooks of
older branches, the marker kept.
Why: rule 1 (setup.check.mjs:236, the #298 checks), rule 3 ((2) Node's
exit code), rule 2 and the issue's "clear errors that say what to do"
((1), (5)), rule 7 and #305 ((4), (6)), rule 4 ((7), one brief), rule 8
((8)), rule 5 ((8): the shim's marker and fallback for older branches).
Suite: setup.check.mjs 23/23, cli.check.mjs 18/18 (+1 todo).

## 2026-10-02 09:10 · overview / actor · retried answers, the band's log, the --herdr look
Chose: the local read's 2 s look also asks herdr (one `pane list`, hashed)
and reads again when an agent's state or a pane changed, so an answer held
back by a blocked agent goes in within one look of it being free. Against:
reading only on a file change or after 15 s (14 s late). Why: rule 1 (the
buttons check "reaches it within 3 s"), rule 3 (Node reads every 2 s).
Idle CPU stays 0.2 % (rule 7): herdr is asked, nothing is read again unless
it changed.
Chose: an answer that gets in on a retry logs a green `decide #2 of #307 →
answer · typed into its agent's pane`, which replaces the yellow waiting
entry in the band and DONE. Why: the critic: the UI never said it arrived.
Chose: the band's short entry glues `of #307` and `→ answer` with no-break
spaces, so a narrow band shows the answer whole or leaves it out (DONE has
it). Against: `→ Rust +`. Why: #304 (no silent cut).
Chose: `· N more` counts every other thing done in the band's minute, not
at most 2. Why: it said `2 more` beside a DONE of 8.
Chose: with `--herdr` the view of everything watches herdr's states too, so
the sidebar's line of an agent follows within a look and pruning/sorting
runs on every herdr change. Against: Node's sync after every 2 s read,
which in the world closes the same tabs every tick (40 in 20 s, ours 8).
Why: rule 7 (idle near 0), rule 6 (no repeated writes herdr took already).
Chose: a try whose terminal didn't open names the step (`no space …`,
`herdr shows no tab try …`, `herdr didn't bring its tab try to the front`).
Against: Node's `no space of its worktree` for each. Why: the critic: it
blamed a space that exists.
Suite: buttons 23/23, isolation 4/4, budgets 9/9; cli: overview --once,
wt:remove ×2, wt:herdr green, the other wt:* are the CLI piece's.

## 2026-10-02 09:10 · overview / NEXT, ACTIONS · keys only for answers its agent gets, ✓ says what it does
Chose: in NEXT the digit keys give only answers that go to the agent (a
decide's options, a try's or plan's buttons, `done`); the ✓ of a kind
without answers (merge, change, a decide without options) has no key and
reads `✓ tick off  takes it off the list · its agent is not told`. Against:
round 1's `1 ✓` with `press 1 to answer`. Why: rule 3: Node has no keys
here at all, and its ✓ only ticks the item off (answers.mjs `answerWith`:
`doneAction`, nothing typed); the critic: one keystroke removed an agent's
merge ask under a hint that said it answered. A key in a lazygit menu says
what its row does; a ✓ that tells no one gets no single-key shortcut.
Chose: Enter is not an answer key: it runs only the block picked with Tab
(the frame), which is lit with its bar, as lazygit's menu runs its
highlighted row. Against: round 1's Enter = the recommended option with
nothing on screen selected. Why: rule 4, one way to pick a row; the
critic. The hint under NEXT names each key and where its answer goes:
`press 1-2 or click an option: typed to its agent`, `press 1 looks good:
typed to its agent · 2 broken: written into its agent's input, for you to
finish · or click`.
Chose: ACTIONS' note says what a click on the rest of a block does, from
the blocks shown: `a click on the rest opens its space, merges or runs its
try`. Against: Node's and round 1's `the rest of a block does it`. Why:
the critic (lazygit names what each row does); the note drops it whole
where it doesn't fit (80 columns), where each block's own text still says
`opens a terminal: …` / `two clicks merge`.
Chose: ACTIONS draws the action NEXT shows only by its head and `↑ in NEXT:
answer it there` (a click on it does the action). Against: Node, which
draws the full block in both places. Why: rule 3 deviation, logged: NEXT
is sticky, so the block is always on screen above; the same options twice
made two places to answer one question and cost ACTIONS 6-8 rows at 80x24.
Chose: an answer not yet delivered whose agent's pane was `blocked` at the
last try says `its agent is at a question: it goes in once that is
answered`. Against: Node's `at its next stop` for it. Why: the critic: the
agent is not going to stop; the words say what holds it up (a draft's own
`waits` is kept, as Node).
Chose: the answer in the log is quoted (`decide #2 of 307-x → “OpenTUI on
Bun” · typed into its agent's pane`). Why: the critic: a cut answer must
never read as a shorter one; the band (actor's `short_of`) shows it whole
or not, DONE in full.
Deferred: #308's auto-merge line under NEXT and its toggle. Node does not
have it yet; it comes with #308 (core's record, the band's toggle, the
actor's merge), not as a NEXT line alone. Rule 3 then makes it parity.
Suite: buttons 23/23, content 24/24.

## 2026-10-02 09:10 · CLI / wt:remove, wt:prune, wt:orchestrate --to, wt:herdr · lifecycle, round 2
Chose: (1) A removal whose MR read GitLab didn't answer starts no
changelog rewrite: it says `▸ the changelog of <name> is not rewritten:
GitLab did not answer; pnpm wt:orchestrate <name> --changelog tries
again`, still ✓ and exit 0; wt:prune leaves an orchestration's changelog
as it is when one of its removals got no answer. A rewrite that loses a
merge the changelog listed (a merge never becomes unmerged) is put back
the same way. Node blanked it to "Nothing merged yet."; the suite checks
it byte-identical now. (2) The /rename failure of a handover prints the
two lines to type on stdout above, the ✗ reason stays one line: the
overview shows a background command's last line. (3) An announced
handover keeps its --peer next to its --brief (`peers` of the record,
read by the later handover before the orchestration's peer); its dry run
and ✓ say which brief and peer apply. (4) ✓ green, ✗ red, ▸ cyan, and
wt:prune's verdict (kept yellow, would be cyan, done green), on a
terminal without NO_COLOR only. (5) Wording: a release names the feature
once; the leftIn refusal says "it enters its next worktree …" only for
an agent bound to another worktree; an issue that can't be read says
whether GitLab didn't answer or it doesn't exist, and what to do.
(6) Kept: current_pane is None without HERDR_PANE_ID, also for wt:herdr
from the plugin's restore: herdr answers with the focused pane then,
which doesn't run the command (Node's releaseSpace guards the same way).
Why: rule 2 (#306: a GitLab failure never blanks what is the point;
#247 brief and peer; #313 the overview's error line), rule 3 for the
rest of the wording (same facts, said once), rule 5 (`peers` is a new
field beside `briefs`, the old records read as before). Suite: the
lifecycle checks 8/8 and the CLI checks of these commands 6/6.

## 2026-10-02 09:04 · overview / WORKTREES · a node in 3 rows, a long state at the right
Chose: who works in a worktree (#292's `● name · N issues · N MRs merged`)
at the right of its facts line, under the state, when both fit; else on a
line of its own before the facts, as Node. Against: Node's agent line
always of its own. Why: the critic (lazygit wins on density, 4 rows a
node): at 120 columns a node is 3 rows (name/state, facts/agent, title),
7 nodes in 21 rows instead of 28; nothing is dropped (rule 3), the state
and the agent's dot share a column in the colour of whom it waits for.
Chose: a state wider than the line goes on lines of its own, each at the
right edge (tree.mjs). Against: round 1's words at the facts indent. Why:
rule 3, an undocumented deviation the critic found on queued nodes at 80.
Not here: OSC 8 links on `#N`, `!iid` and the pipeline mark need the
frame to carry a span's url to the terminal (core's `Span::url` has
them); the queued nodes' `handover failed: wt:orchestrate: not built yet`
is the CLI's stub. The settled-then-shift of WORKTREES is gone with the
frame's wake of the local reader (08:55): `removed #303` is in the first
settled frame.
Suite: content 24/24, hover 4/4, buttons 23/23, views 9/9, isolation 4/4.

## 2026-10-02 08:55 · TUI frame · the overview's own acts show at once, pages land on a head, the band
Chose: what the overview did by itself (a `wt:remove`, a delivery) wakes
the local reader when its effect comes back, so the result shows at once;
the wake only looks at the reader's stamp, the 2 s poll stays. Against:
waiting for the next poll. Why: rule 1 (#296): the reflow came up to 2 s
late, after the first screen that held still, so hover.check measured a
layout that then moved 3 rows; hover 2/4 at 00:50 was this, not the frame.
Chose: PgDn, space and PgUp land where the top row starts a block or a
section shows its title or pin, up to half a page less far, never further;
the end and the top as they are. Against: Node's fixed `rows − sticky − 1`.
Why: the critic: a page landed on an AGENTS line without its agent or
title; lazygit's pages start at an item; rule 3's reason (the last row
seen stays on screen) still holds.
Chose: a rebuild places the scroll by the anchor it was last placed by
while the reader hasn't scrolled since. Why: the critic: 120x40 → 80x24 →
120x40 moved the view a row (a block's rows in proportion rounded twice);
now the round trip gives back the same frame.
Chose: the scroll thumb runs beside the rows under a pinned title, not on
it. Why: the critic: `WORKTREES … removed ┃` read as part of the title.
Chose: below 30 rows (`band::TIGHT_BELOW`) the band leaves out its blank
first and last line and the sticky part the blank after the chat row and
after NEXT. Against: Node's 5-row band and blanks. Why: rule 7, the bar:
at 80x24 the list gets 18 rows instead of 14 (lazygit's commits: 21); the
band's ground and the islands keep the parts apart.
Chose: no wall clock in the band. Against: Node's `5:40:23 PM` on line 2.
Why: Node's was fixed at render time, so it showed the last redraw, not
now; a true one costs a frame a second (rule 7, idle CPU), the pane's
host shows the time, lazygit has none; `GitLab 14:57` says how old the
data is, the thing the clock stood in for.
Chose: the keys line is Node's, then ` · Tab Enter pick a row`; Tab,
Shift-Tab, Enter, Ctrl-D, Ctrl-U added. Why: rule 3 keeps every Node key
and its words in front (a narrow band drops the new ones first); the bar
is lazygit, which works without a mouse.
Suite: hover 4/4 (twice), budgets 9/9 (first frame 160 ms, idle 0.10 %,
24 MiB, 40 wheel reports/s at 1.6-1.9 % CPU settled 7 ms), views 9/9,
content 24/24, isolation 4/4; buttons 22/23 (the new answers-retry test of
the actor piece).

## 2026-10-02 08:40 · CLI / overview --once, wt:plan · what can't be read is said, not guessed
Chose: (1) the "blocked by" links of the open issues come from one
GraphQL query; when GitLab doesn't answer it (an older GitLab, a proxy
that passes REST only) each issue's REST `issues/<n>/links` is read side by
side, as Node did, and what neither answers is a ⚠ line under the title
band ("the merge order may miss one"); the suite checks the order with
GraphQL down. (2) An orchestration record that doesn't parse or lists no
`issues` is named with what to do (overview and wt:plan), where both said
"no orchestration recorded". (3) A worktree's `.claude/actions.json` that
can't be read is a ⚠ line in ACTIONS and its count in the heading, where
the view said "nothing waits for you". (4) An MR row's reason that names
the MR acting already (`waits for !311`) has no `→ !311` after it. (5)
wt:plan says when another change holds the record's lock and how long it
waits, instead of hanging silently. (6) While GitLab doesn't answer, a
worktree's facts say `MR …` (unknown), as its issue's `#303 …`, not
`no MR yet`: Node's guess is a known bug of the suite now.
Why: rule 3 (no silent loss: Node's REST links, every row stays), rule 7
(1 call where GraphQL answers, 5 for the whole view), rule 4 (4: the
arrow only repeated the MR), and errors that say what to do. Suite: the
CLI checks 21/21 against the binary (1 todo: wt:orchestrate needs claude);
Node fails the new GitLab-down check as its known bug.

## 2026-10-02 01:50 · CLI / wt:tick, wt:note, wt:mr --update, wt:action change · what the suite can't reach
Chose: the verdicts the world can't provoke are functions with tests of
their own: `is_ticked_back` (wt:tick: as many boxes as before, the chosen
ones ticked; a body that became a path has none), `is_note_back` (wt:note:
one of the newest notes is the text; an error object or no JSON is `⚠`),
`is_description_back` (wt:mr --update), and change's `report`: a draft
that failed keeps the change written and said, drops `is a draft now`, and
ends with `✗ glab could not make !N a draft (…): do it by hand …`, exit 1.
The 30-minute conflicts verdict was tested already (`verdict_of`). The
pre-push shim (19:45) is now step 5 of rewrite-plan.md, so the cut-over
can't miss it. Against: failure switches in the gauntlet's fake GitLab,
which three builders change at once this round.
Why: rule 1 (each `⚠` and `✗` path held by a check), rule 4 (pure
functions, no fake-GitLab knobs for four lines). Re-checked in a world:
`wt:mr --dryrun`, `wt:mr --help`, `wt:tick 307 3 --help`, `wt:action look
x --bogus` make 0 glab calls; a closed !316 gets a new draft (!319);
`wt:wait !308 --hours 0.0001` ends after 71 ms, `!99999 !99998` at once.

## 2026-10-02 01:40 · CLI / wt:remove, wt:prune · a changelog that lists the merge is not rewritten (#305)
Chose: a removal of merged work reads its MRs once anyway (the claim on
origin, `mergedAt`); when its orchestration's changelog lists the last
merged one (`[!<iid>](`) already, as the overview or an earlier removal
wrote it, nothing is rewritten: 22 calls, 1 of them glab, where it made
218 (99 glab), and in a world with a 200 ms GitLab it exits at 0.9 s
(Node 2.4 s, round four 3.7 s). Otherwise `▸ rewrite the changelog of
<name> (!<iid> is not in it yet)` says it, the rewrite runs beside
herdr's calls and the ✓ comes before the wait, as remove.mjs did; a
failure is said after the ✓ (exit 0: the removal is done). wt:prune
starts its rewrites at once, beside the plugin's check, and waits after
its report. When GitLab doesn't answer or there is no merged MR, the
changelog is rewritten as today's always did. Open: the rewrite itself
(setup group, `wt:orchestrate --changelog`) still asks GitLab per issue,
12 workers with two calls one after the other each: 2.8 s at 200 ms
where Node's takes ~1 s; a removal of a merge the changelog misses exits
at 3.1 s against Node's 2.4 s until it runs both reads of an issue at
once or adds the one merge (#305).
Why: rule 7 (measured above), rule 3 (the ✓ before the wait, as Node),
rule 4 (the check reads the file it would write, no new state).

## 2026-10-02 01:40 · CLI / lifecycle group · flags, the plugin check, wt:herdr's silent space, prune's columns
Chose: an unknown flag of wt:remove, wt:prune, wt:herdr and the
handover says the one meant (`unknown flag --dryrun: did you mean
--dry-run?`, cli/typo.rs, the same as the setup and write groups), on
the same two lines as theirs (`✗ …`, the usage). The plugin check
(#230, one `herdr plugin list`) runs once nothing refused: after a
removal's own steps, after the handover's agent, context and issue
checks; a refusal costs no extra process (round four checked on every
refusal). wt:herdr says `space <feature> not set up: herdr gave no id
back for it (worktree open, workspace create)` where both tools said
nothing. wt:prune pads the names to 32 as today's, or to the longest
when one is longer, so the column stays one: nothing reads these lines
(the overview runs wt:remove, not wt:prune; checked in src/).
Kept: wt:remove and wt:prune refuse outside a checkout (01:15).
Why: rule 3 (exact errors, no silent outcome), rule 7 (no process a
refusal doesn't need), rule 2 for the columns (the wording is today's).
Suite: `lifecycle.check.mjs` keeps --keep-branch, the uncommitted
refusal, #305's calls, wt:prune of a gone branch and leftover folders,
wt:herdr outside herdr and the flag hints green (Node: 4 pass, #305 a
known bug, the hints rewrite-only).

## 2026-10-02 01:20 · CLI / every command · an unknown command, `--help`, a bare `breadboard`
Chose: a name no group takes says `✗ unknown command <name>: did you
mean <nearest>?` and the usage with every command, a group per line, on
stderr, exit 1 (`wt:actoin` → wt:action, `action` → wt:action, `up` →
stack:up, `overvieww` → overview). `breadboard --help` (`-h`, `help`)
prints the same usage on stdout, exit 0; a bare `breadboard` on stderr,
exit 1. Against: round four's leftover `<name>: not built yet`, exit 2,
which named nothing; and Node's one line
`usage: node …/worktree.mjs new|orchestrate|…|remove …`, which lists its
internal names, not the ones people type (`pnpm wt:*`, `stack:*`).
Why: rule 3 (stderr, exit 1, `✗` as `fail`), rule 5 (the names typed
stay, the usage lists exactly those), rule 4 (one list in cli/mod.rs,
`cli/typo.rs` the one place that guesses, for commands and the write
group's flags). Suite: cli.check.mjs 14/14, 1 todo (wt:orchestrate).

## 2026-10-02 01:20 · CLI / wt:action, wt:tick, wt:note, wt:mr, wt:wait · a typo says what was meant
Chose: an unknown flag says the one it likely is, as setup's wt:new does:
`✗ unknown flag --dryrun: did you mean --dry-run?` (a flag two edits
away, or the only one it begins: `--message` → `--message-file`), then
the usage while it has at most 3 lines, else
`  pnpm wt:action --help shows every form` (one wrong flag was 11 lines
of usage). wt:wait says it the same way (`--hour`, `--cancle`; was
`--hour is no MR (!213) and no flag`), a word that is no MR
`x2 is no MR (!213 or 213)`. `wt:action lok x` says
`lok is no kind of action: did you mean look? (…)`. `wt:tick x` says
`x is no issue number (34 or #34)` as wt:mr does (Node's "This branch
carries no issue number" was untrue: the word was wrong, the branch had
one); a box that is no number `a is no box number: pnpm wt:tick 307 lists
them, numbered` (was the usage). `wt:mr --description-file <f>` with no
such file is `✗ <f> does not exist — write the description there first`
before GitLab is asked; it put the template down at
.claude/mr-description.md and asked to fill that instead.
`wt:action --orchestration ../../x` says wt:plan's
`../../x is no orchestration's name: names are kebab-case (…)`, not
`no orchestration … is recorded`. `wt:action --list` right-aligns the ids
to the widest one, so `  3  plan` and ` 155  look` start their kinds in
one column (spec §11 names Node's shift a bug).
Kept: `wt:mr --dry-run` prints `body <TMPDIR>/wt-mr-…/description.md`
as Node does, and that file stays (checked in a world: it exists after
the run). Still open: the pre-push shim runs `node …/pre-push.mjs` until
the hook is a breadboard command (wt:new installs the same shim; the
cut-over changes both at once).
Why: rule 3 + the bar (exact errors that say what to do), rule 4 (one
`nearest`, the same answer from every group of one binary), rule 6 (a
missing description file sends nothing and writes nothing).
Suite: wt:action, wt:tick, wt:note, wt:mr, wt:wait 5/5.

## 2026-10-02 01:10 · CLI / wt:issues · 2 calls, and an error that says what to do
Chose: an issue GitLab counts no comment of (`user_notes_count`, the
notes people wrote, in the issues list already read) is stale without
reading its notes; only an issue with comments has its notes read, to
see whether one came after the merge. In the breadboard world: 2 calls
where it made 13 (the 11 candidates have none), ~230 ms against Node's
~450 ms, the output byte-identical. Against: one GraphQL query for MRs,
issues and notes (spec §3's "simpler form"): the fake GitLab's schema has
no notes and the fake glab no `api graphql`, so it could not be checked,
and the count already leaves nothing to batch here. A GitLab error now
says what went wrong in words, glab's own reason after it, and what to
try: `✗ GitLab could not be read: GitLab can't be reached (Error: bad
port)` and `  glab api <call> failed: check the login (glab auth status)
and the network, then pnpm wt:issues again.` (a 401 reads `glab is not
logged in, or its login expired`, a missing glab `install glab: …`).
Node printed `GitLab: Node.js v25.8.0 (…)` there. For the gauntlet: the
fake GitLab doesn't raise `user_notes_count` when a note is posted, as
GitLab does; a world that notes an issue nobody had commented on and
then runs wt:issues would see it stale.
Why: rule 7 (measured: 2 calls, nothing lost), rule 3 with the bar for
the error (an exact error that says what to do), rule 4 (one REST path).

## 2026-10-02 01:05 · CLI / overview --once · a button never splits, narrow widths read as rows
Chose: wrapping keeps a button or chip whole (`[ ✕ dismiss ]` moves to
the next line, with colour too, where it is the coloured ground that
holds it); a word longer than its line breaks after its last `-` or `/`
that fits (`306-overview-merge-requests-` / `crash-row`, a URL after a
`/`), a hard cut only without either. A table row whose columns are wider
than the line is stacked: its first two cells on the first line, the rest
below a step further in (WORKTREES at 60 columns no longer wraps back to
the marker column); a last cell with too little room beside a wide first
column goes a step in from the edge instead of under it (AGENTS at 60: 11
columns of one-word lines before). An MR's status (`merged 14 h ago`)
goes on the line below its title where both don't fit, instead of past
the edge. The title band wraps a name too wide for it over its lines
(`--worktree` at 60 cut the branch to `feature/307-bre`, #304); the
worktree's path wraps under itself. STACK's migrations line is built
(develop.mjs `migrationsOf`, actions.mjs `pushMigrations`: `psql` with
`MIGRATION_DATABASE_URL`, the journal's `when` against drizzle's newest,
`✓ all N applied` / `⚠ N migrations not applied: …` and `[ ▶ pnpm
pg:migrate ]` / `the database doesn't answer: …`), with
`PGCONNECT_TIMEOUT=3` so a gone database costs seconds, not minutes.
Kept: ACTIONS' empty line ` none: …` and the `GitLab HH:MM` line have no
extra indent with FORCE_COLOR. Node's coloured print shows them two
columns further in only because its `dim()` puts an escape before the
box's two-space prefix, which its plain print strips; every other line
of the box agrees, so ours aligns them in both modes. Measured against
Node with FORCE_COLOR at 120, escapes stripped: the remaining
differences are these two lines, the dismiss buttons, and the titles
Node cuts with `…` (#304). At 120 and 200 columns plain, every view is
byte-identical to before but for the dismiss buttons.
Why: rule 3 + the bar (lazygit never splits a button), rule 2 (#304,
nothing cut), rule 3 for the migrations line (a parity loss until now).

## 2026-10-02 01:15 · CLI / lifecycle group · herdr's reads remembered only inside a layout
Chose: `cli/lifecycle/herdr.rs` remembers a read only inside
`herdr::remembering` (wt:herdr under its layout lock, wt:remove's release
of its tab and space), there until the next write, per thread; everywhere
else every read asks herdr. Against: round three's process-wide cache,
cleared only by this process's own writes: the handover's #272 re-check
after wt:new got the agent's state from before the setup, typed `/rename`
and the next issue into a working agent and said ✓ (Node refuses and
removes the worktree again). Now as Node: refused, worktree and claim gone.
Why: rule 3 (#272's refusal), rule 6 (no prompt into a working agent),
rule 7 (wt:herdr keeps its ~70 reads against Node's ~240: the repeats are
all inside the layout). Suite: `the handover` check (a fake pnpm sets the
agent working during wt:new) fails on round three's binary, passes on
Node and ours.

## 2026-10-02 01:15 · CLI / wt:remove, wt:prune · the changelog is waited for, once per prune
Chose: a removal of merged work rewrites its orchestration's changelog
(`wt:orchestrate <name> --changelog`) before its ✓ and waits for it, quiet
unless it fails (`▸ the changelog of <name> is not rewritten: <reason>;
pnpm wt:orchestrate <name> --changelog tries again`); wt:prune rewrites
each orchestration's once after all its removals. Against: round three's
detached process per removal, which outlived the command, dropped its
errors, and in a prune of N merged worktrees ran N rewrites of the same
file at once (~200 GitLab calls each).
Why: rule 3 (Node's wt:remove awaited the rewrite before exiting), rule 4
(one rewrite, no orphan process), rule 7 (a prune asks GitLab once per
orchestration, not per worktree). The same changelog as Node's in a world.

## 2026-10-02 01:15 · CLI / wt:remove, handover · small deviations of round four
Chose: wt:remove takes the feature in any place (`--keep-branch 303-x`,
its flags all stand alone; both tools refused it with the usage). The
word right behind `--after` is its value even without `!`: `--after x308`
says `✗ x308 is no MR (!232)` (today's took it for a second issue).
A handover whose session can't be read (no running claude shows it)
types the next-issue line at once, as Node did, instead of waiting a
second (handover in a world: 0.8 s, Node 1.3 s). Kept: the commands run
from a checkout of the repo (`✗ <dir> is in no checkout …: run it from
the main checkout or one of its worktrees`); Node found the repo from its
script's path, the binary's place (a target dir, later CI's prebuilt) says
nothing about which checkout is meant, and pnpm, the overview and the
plugin all start it in one.
Why: rule 3 + the bar (flags in any order, the exact error), rule 4.

## 2026-10-02 01:05 · CLI / wt:tick, wt:mr, wt:note · every text to GitLab through a file
Chose: descriptions and comments reach glab as `--field <key>=@<file>`
of `glab api` (a temp file, removed after): `PUT issues/<n>` (wt:tick),
`PUT merge_requests/<iid>` (wt:mr --update), `POST merge_requests`
(wt:mr: the branches, `Draft: <title>`, the labels, `assignee_id` from
`glab api user`, the issue's `milestone_id`, `remove_source_branch`) and
`POST issues/<n>/notes` (wt:note, after a `glab issue view` that gives the
comment's URL and the same `✗ GitLab has no issue #N` as wt:tick). The
value is always a path, so a text starting with `@` is never read as one.
Against: the text as one argument (`--raw-field description=<text>`,
`mr create --description`, `issue note --message`), which Linux caps at
128 KiB: a 180 KB description failed with E2BIG before it reached GitLab,
where Node ticked it; `--description-file`, missing in glab 1.114 (#157).
Supersedes 19:55.
Why: rule 3 (Node ticks a long description), rule 2 (#157), rule 6 (the
routes `glab issue|mr update`, `mr create` and `issue note` take; one read
more each, `user` or the issue). In a world: a 190 KB issue description
ticked, a 186 KB MR description sent, a 200 KB comment posted. Gauntlet:
the fake glab reads `--field k=@file` and typed values as glab does, the
fake GitLab answers `GET user` and `POST merge_requests`.

## 2026-10-02 01:05 · CLI / write group · glab's failures said exactly
Chose: a refusal is said with its status, `(glab auth status)` only where
the login can be the cause: `GitLab answered 404 Not found`, `GitLab
refused it (403 Forbidden): the account glab uses may not do this here
(glab auth status)`, `GitLab does not take glab's login (401 …): glab auth
login`, `GitLab failed (503 …): try again in a minute`; GitLab's JSON
message as words (`title is too long`); glab not starting (`glab could
not be started: …`) without the suffix. A call that got no answer keeps
`<glab's line> (glab auth status)`. A create GitLab refuses as invalid
adds `Is everything pushed? git push`. Against: the suffix after every
failure, raw JSON included (`404 {"message":…} (glab auth status)`).
Why: rule 3, an error says what to do; nothing another program reads
changes (rule 5).

## 2026-10-02 01:05 · CLI / wt:action · a broken actions.json is kept
Chose: an `actions.json` that is there but no JSON object (cut off
mid-write) is refused by every form of wt:action (`change` before it
drafts the MR): `✗ .claude/actions.json is no JSON object (cut off
mid-write?): its actions can't be read, so it is kept as it is.` and the
`mv … .broken` that starts empty. Against: Node and #306's parity, which
read it as empty, so the next post overwrote it and every open action
was gone without a word. The overview still reads it as empty (#306): a
view must draw, a write must not destroy.
Why: rule 3, #306 names refusing as better; rule 5, the file stays as
the other sessions wrote it.

## 2026-10-02 01:05 · CLI / wt:action · a lock left behind
Chose: a `<file>.lock` older than 5 s is taken over at once and said so:
`  takes over <lock>, left behind by a writer that ended`; only a younger
one prints `waits for …, held by another writer (taken over after 5 s)`.
Supersedes that line of 19:50 for a stale lock.
Why: rule 3: the old line promised a wait that did not happen.

## 2026-10-02 01:05 · CLI / wt:mr --update · refused without an open MR
Chose: `wt:mr --update` without an open MR of the branch (none, or a
closed one) is `✗ no open MR of <branch> to update: pnpm wt:mr opens it`,
before the push guard and any write. Against: Node, which opened a new
draft MR.
Why: rule 6 over rule 3: `--update` asks to send a description again;
an MR opened instead is a GitLab write nobody asked for (as 20:10 says
of typos).

## 2026-10-02 01:05 · gauntlet · a world reaches no repo around it
Chose: a world sets `GIT_CEILING_DIRECTORIES` to the folder that holds
it. Built under a checkout's `.claude/gauntlet/tmp`, a cwd outside the
world's repos (its home) found that checkout, and wt:action wrote into
its real `.claude/actions.json`.
Why: the loop never acts on the live workflow (rewrite-rules.md, Working).

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

## 2026-10-02 00:55 · CLI / setup group · what waits for the halves to meet
Chose: three things of round two's critic stay open in the setup group,
on purpose. (1) The brief text exists twice, `cli/setup/orchestrate/issue.rs`
(a fresh start) and `cli/lifecycle/handover/brief.rs` (a handover): agents
read it (rule 5), so one copy goes into breadboard-core once the halves
meet, not into one group's module the other imports. (2) #230: the
lifecycle group builds `plugin::heal` (the plugin relinked, the repo's
config repaired, once per run inside herdr); `wt:new`, `wt:orchestrate`
and `wt:stack` call it from there instead of a second copy. (3) An unknown
command, `--help` and `help` belong to `cli/mod.rs`, which no group owns:
today's `usage: … new|orchestrate|…` with exit 1 goes there.
Why: rule 4, one way to do a thing; a copy now would be the thing the
critic flagged next round.

## 2026-10-02 00:50 · overview / MERGE REQUESTS · merged rows, reasons, the `…` mark
Chose: a merged row is Node's again: `!iid`, its pipeline mark (a click
opens the pipeline) and the dim `merged <ago>` at the right of its first
line, the title between. Against: round 2's age in the pipeline's column.
Why: rule 3: GitLab merges on a red pipeline too, and #308's `merged
automatically HH:MM` needs the status's own column.
Chose: a title's words that don't fit beside the status wrap under the
title column across the whole row, not only the title column. Why: #304
takes the `…`, so a long merged title at 80 columns needs two lines; this
keeps it at two instead of three (lazygit's subjects use the whole width).
Chose: a reason no wider than half the line is glued into one word, so
`try #2 broken → you` wraps whole; a longer one wraps at its words, its
`→ who` always glued. Against: Node, which breaks inside a reason. Why: the
critic (round 2); half the line, so a long reason never leaves the line
before it half empty.
Chose: `checking for conflicts …` with a space, as every `…` mark after a
word (`look::uncut`). Against: Node's `conflicts…`. Why: rule 1: the
suite's CUT check (`/[\w)(#]…/`, #304) reads `s…` as a cut.
Suite: content 24/24, buttons 22/22; hover 2/4 (one row too many lit in
both MERGE REQUESTS and WORKTREES, from the frame's change at 00:44).

## 2026-10-02 00:50 · CLI / pnpm commands · its lines from commands.mjs
Chose: `pnpm commands` reads the `DESCRIPTIONS` object of
`packages/breadboard/src/commands.mjs` as data (a small reader of its
`key: "…" + '…'` literals, no Node) instead of a Rust copy of it.
Against: the copy of round one, which drifts the first time someone adds
a script: AGENTS.md sends the new line to commands.mjs. Byte-identical
to Node's output for `commands` and `commands --all` in this repo; a
world without commands.mjs shows the scripts' commands, as Node does for
a script without a line. When commands.mjs goes with the cut-over, its
object moves to a data file both read, and AGENTS.md says so.
Why: rule 4 (one source of truth), rule 8 (no Node at run time).

## 2026-10-02 00:45 · CLI / stack:up · a herdr tab without a pane
Chose: a tab herdr creates without a `root_pane` (the command could not
be started in it) no longer counts as running: `▸ herdr: postgres
keycloak seaweedfs studio opened empty: herdr gave no pane, nothing runs
there` instead of `+postgres +keycloak …`. Node dies there with a
TypeError. Why: rule 3 says decide it; a tool that reports logs running
in an empty tab is wrong, and the line says what to look at.

## 2026-10-02 00:40 · CLI / setup group · the command line
Chose: the command line read by hand (`proc::Args`, ~60 lines) instead
of clap; each command names the flags it takes, and any other is refused
before anything happens: `✗ unknown flag --shraed: did you mean
--shared?` and the usage, exit 1 (a near flag within two edits is
named). Today's internal names run the same commands: `control
up|down|reset|ps`, `platform-changelog add|…`, `new`, `orchestrate`,
`stack`, `check` (the herdr plugin, tabs and the Node overview spawn
them). Against: clap, which would need a custom setting for each of
today's quirks (`--x=y` counted only so spelled, a value starting with
`--` counted as missing, `--after !1 !2` taking several, a flag before
the feature being a usage error); Node's silence (`wt:new 12-x --shraed`
made the worktree on its own stack and pushed the claim).
Why: rule 4 (fewer dependencies, the quirks in one place), rule 5 (the
names and flags stay), rule 6 (a typo is no push and no herdr write).

## 2026-10-02 00:35 · CLI / pnpm changelog · a failed GitLab read is no "changed nothing"
Chose: every GitLab read of the setup group is tried again on a 429, a
5xx, a timeout or the network, after 1 s and 4 s as lib/glab.mjs does,
and a `glab api` is killed after 15 s. A merged MR whose diffs still
fail stays out of `.claude/changelog.json`, so the next run reads it
again, and the run says so last: `! GitLab: HTTP 502 Bad Gateway (!310's
diffs): read again next time` (exit 0, the changelog is written). The
release tags keep the pages read and say what is missing. The kept MRs
are written by iid ascending, as Node's `Object.fromEntries` does.
Against: round one's `unwrap_or_default`, which stored `"310":
{"paths":[],"changesets":[]}` after one 502 and never read it again (the
MR fell into no group, gone from the changelog until changelog.json was
deleted); Node's way of dying with a stack trace and writing nothing.
Why: rule 5 (changelog.json is read by the overview: same shape, same
order); rule 3 says decide it, and a tool that heals itself on the next
run beats one that refuses the whole rewrite.

## 2026-10-02 00:32 · CLI / wt:orchestrate · the brief of an issue (#247)
Chose: an issue gets the `--brief` of the very run that starts it, else
the one an earlier run kept for that issue (`record.briefs[<n>]`), else
none; the record's single `brief` is still written for the Node
overview's queue starts while both run, but no longer read for issues of
another run. The dry run says per issue which it gets: `#285 gets the
brief of --brief <file>`, `… the brief an earlier run gave it` or `… no
brief of the orchestrator`.
Against: Node's `--brief ?? record.brief`, which handed one issue's
brief to every later issue of the orchestration. The wait for `/rename`
(#313) went with the handover to the lifecycle group (its log entry).
Why: rule 2, #247's criteria over today's behaviour; rule 5 for the
`brief` field other readers still use.

## 2026-10-02 00:30 · overview / worktree view · DIFF's files, its columns, DEVELOP's rows
Chose: DIFF lists the files of an area with more than one under it (up to
5, then `… N more:` by name), each with git's letter in lazygit's colours
(A/? green, M yellow, D red, R cyan), its path below the area's folder, its
`+a −r`, and a dim `uncommitted` when it is not committed yet; a click
opens nvim at that file's first change. An area of one file keeps Node's
row, `area +a −r bar 1 file · <name>`. Against: Node's one row per area
(spec §2.8). Why: the bar is lazygit's files panel (loop prompt): the file
a developer wants is one click away instead of behind its area's top file;
rule 3 keeps every Node row and click, this only adds rows.
Chose: the name column fits the area names, and a file's path only while
the bars keep their full 24 cells beside it; a longer path runs into the
counts (they follow it, or go on the next line in their columns) and wraps
after a `/`. The last cell gets what its widest wants up to 24 columns,
at least 16 (`MIN_WRAP`); the bars narrow for it (to 6) before it wraps,
and it wraps between `N files ·` and the name. Why: #304 and the previous
critic: one long path made the column 68 wide and split `uncommitted`
two letters a line at 80 columns.
Chose: DIFF's title parts its hint with ` · `, not Node's 2 spaces, so at
80 columns the hint drops alone. Against: Node, whose `fitParts` drops
`N uncommitted` together with the hint there. Why: rule 1 (#304: nothing
lost by a cut that a reader needs), the count is the fact, the hint is not.
Chose: a DEVELOP merge's title and branch wrap under the title column
(tableRows' hanging indent), not at column 1; the ISSUE title is Node's
`#N  d/t done` again, without the extra ` ✓`. Why: rule 3.
Suite: views 9/9, content 24/24, buttons 22/22, hover 4/4.

## 2026-10-02 · TUI frame · pinned titles, the sticky cut, the page, the band's cuts
Chose: a section's title stays pinned on top of the scrolling rows while a
line of that section with text on it shows under the pin, and only in a
pane of at least 4 title heights. Once only the section's trailing blank is
left, or it is gone, there is no pin and the row goes back to the content.
Against: Node, which has no pin. Why: lazygit's panels never lose their
title, and in a long WORKTREES or MERGE REQUESTS list the reader still
knows which section the rows belong to. Rule 1 for the fix: a pin over
the section's last line (`built yet` at 80x24, offset 6) hid a row that
the `↑ N` mark did not count.
Chose: a page (PgDn, space, PgUp) is `room − 1` rows, as Node's
`max(1, rows − sticky − 1)`, less the pin's rows when a pin covers the top
of where it lands. So 28 rows at 120x40 under a pin, 29 without one.
Why: the last row seen stays on screen as the first, and is not hidden
under the pin (Node's own reason for the `− 1`).
Chose: the sticky part (chat row, NEXT) is cut to half the pane, and its
rows past the cut scroll with the rest. Against: the spec's cut to the
whole pane. Why: rule 1: when the sticky part is cut to the pane, a NEXT
taller than the pane hides every other section and they cannot be
reached. The cut depends only on the pane size, never on the scroll.
Chose: the band's log of the last thing done is cut at a word, with no
`…`, as Node cuts the band's right side. The keys at 80 columns drop
`g G scroll` the same way, as Node does. Why: #304 (no `…`), rule 3.
Chose: an atom wider than a line (a branch, a long button) breaks after
its last `-`, `/`, `_`, `:`, `.` or space that fits, and only otherwise
in the middle of a word (`303-overview-every-drawn-` / `button-works`).
Why: #304, it still reads as the name.
Suite: budgets, hover, content, views, buttons, isolation green. Release, 120x40: first frame 115 ms
(the suite's clock, with six builds running on the machine), idle CPU
0.10 %, 23 MiB peak, 40 wheel reports/s at 1.3 % CPU, settled 7 ms after
the last report.

## 2026-10-02 00:30 · CLI / setup group · GitLab through glab until core's client
Chose: the setup group keeps one `glab api` per call (up to 12 side by
side, a pool instead of round one's chunks) until breadboard-core's
kept-alive HTTP client (`gitlab/client.rs` of wave 1, with the token,
Retry-After and the rate limit) is in this crate's core; then every read
of the group moves to it. Against: a second HTTP client in
`cli/setup/gitlab.rs` now. GitLab's rate-limit headers stay unread until
then (glab hides them). Also kept, from round one: the pre-push hook
stays Node's behind the shim wt:new and wt:mr install; wt:check writes
the stamp in Node's format, which that hook reads (#298, checked by the
suite's `setup.check.mjs`). `wt:orchestrate --end` says `✓ ⇶ <title>
ended` where Node says the name: the title is what the space shows.
Why: rule 4 (one client, not one per group); rule 5 (the stamp's
format, the hook's shim).

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

## 2026-10-02 00:10 · overview / WORKTREES, AGENTS · clock form, armed dismiss
Chose: times in WORKTREES (`merged 16:01`, `removed 1 Oct 15:30`) in 24 h
`HH:MM`, the form of the band. Against: Node's locale form (`04:01 PM`
under en-US, 24 h under de-DE). Why: rule 4: one form on the whole screen,
no locale tables (`localtime_r` only).
Chose: the armed dismiss names the agent by its display name
(free-agents.mjs `dismissOf`, #292); the AGENTS layout is chosen once for
all rows against the widest label the button can show, armed included, so
a first click never reflows a row; where it does not fit beside the name,
the name goes on its own line under the display name instead of being cut.
Why: rule 2 (#292), rule 3, #304 (nothing cut with `…`).

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

## 2026-10-01 23:00 · TUI frame · redraw on change, atoms that never split
Chose: a generic frame (`tui/`): sections render into a Scene of atoms that
wrap whole, hit regions from the code that places the text, blocks and
buttons, each section in `catch_unwind`; redraw only on a change, at most
once per 16 ms; the clock ticks only for a section that asks
(`redraw_in`); the terminal size polled every 250 ms (a hidden herdr pane
may get no SIGWINCH); a lone Escape held until the next input so a split
mouse report still clicks; a scroll thumb in the last column; no crate but
`libc` (local time).
Why: rules 1 and 7 (#296, #303, #304, #306; idle CPU, wheel bursts);
rule 4 for the crates. Demo, release: first frame 16 ms, idle 0 %, 0.8 %
CPU at 40 wheel reports/s, 6.6 MiB (Node: 1.7 s, 10 %, 12 %, 146 MiB).

## 2026-10-01 23:00 · core · one GraphQL request a refresh, final things from disk
Chose: the orchestration's GitLab read is one GraphQL request (open MRs,
pipelines, jobs, issues) plus the shared disk cache `.claude/gitlab-cache.json`
in Node's shapes for merged and closed things; blocked-by links in a second
request in real GitLab's shape on full reads only; failed-job logs over
REST once per job; deployments only for branches with an open try on the
test instance; pipelines filtered on the client so the query is valid for
real GitLab and the fake.
Why: rule 7 (#305): the breadboard world at 250 ms a request, first
refresh 2 calls in 0.6 s, the next 1 call (Node: 154 calls, 7.2 s);
rule 5: the cache file is Node's, so both run side by side.

## 2026-10-01 23:00 · core · models are data, never actions
Chose: `orchestration_of` / `worktree_of` / `everything_of` return plain
structs, each section derived in its own `catch_unwind`; what a click or the
overview itself would do comes back as values (`Go`, `orders`, `tells`),
executed by the TUI; change detection by stat stamps plus a 15 s window,
no watcher thread; time via `localtime_r`, no chrono.
Why: rule 6 (writes only where the Node overview writes, in one place),
#306, rules 4 and 7.

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

## 2026-10-01 22:25 · CLI · built in a private clone, judged against Node
Chose: the four command groups (read, write, lifecycle, setup) built in
parallel in a local clone from a snapshot of core, each looped builder →
fresh critic. lazygit has no CLI, so the CLI critic compares ours blind
with today's Node output, with lazygit's polish as the bar, plus parity
and the suite.
Why: the loop's prompt (a critic per piece) where the bar has no
counterpart; a clone instead of a registered worktree so the live overview
never sees it ("never against the live workflow").

## 2026-10-01 22:15 · loop · durable scratch after a reboot
Chose: specs, results and scratch under `.claude/gauntlet/` of this
worktree (git-ignored), not `/tmp`.
Why: a reboot wiped `/tmp` with the six parity specs; they were recovered
from the agents' transcripts.

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
