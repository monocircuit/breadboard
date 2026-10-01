# The rewrite's decision log (#314)

Newest first, one entry per decision with the rule of
[`rewrite-rules.md`](rewrite-rules.md) it follows. The loop decides; the
user reads this instead of answering questions.

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
