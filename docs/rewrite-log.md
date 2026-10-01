# The rewrite's decision log (#314)

Newest first, one entry per decision with the rule of
[`rewrite-rules.md`](rewrite-rules.md) it follows. The loop decides; the
user reads this instead of answering questions.

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
