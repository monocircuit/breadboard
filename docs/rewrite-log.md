# The rewrite's decision log (#314)

Newest first, one entry per decision with the rule of
[`rewrite-rules.md`](rewrite-rules.md) it follows. The loop decides; the
user reads this instead of answering questions.

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
