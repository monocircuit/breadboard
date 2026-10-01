# The stack for breadboard: why Rust and ratatui (#307)

**Decided by the user (2026-10-01): "drop OpenTUI and move completely to
Rust."** breadboard is rewritten in Rust with ratatui (crossterm, tokio),
the stack of herdr itself: the overview, the CLI and the herdr plugin. This
file is the evidence the decision rests on: the candidates, what comparable
tools use, and two prototypes of the same slice, measured. The OpenTUI
prototype is gone from the branch; its numbers stay here. How the rewrite
goes: [`rewrite-plan.md`](rewrite-plan.md); the rules its loop decides the
rest by: [`rewrite-rules.md`](rewrite-rules.md); the suite it must pass:
[`packages/breadboard-gauntlet`](../../breadboard-gauntlet/README.md).

## What today's breadboard costs (2026-10-01)

Plain Node, no runtime dependencies, ~31k lines in `src/`. The overview is
hand-rolled (`lib/tty.mjs`, `overview/frame.mjs`): line arrays per section,
clicks by row (`screen.clicks[row]`), one render guard for the whole view.
The bugs open today come from that design, not from Node: #303 (clicks by
row drift), #296 (hover has no blocks), #304 (each section wraps by hand),
#306 (one exception blanks the view), #305 (312 `glab` processes, 29 s per
refresh).

## Candidates

| Stack | Mouse per element | Layout / wrapping | Headless tests | Runtime / distribution | Seen in |
| ----- | ----------------- | ----------------- | -------------- | ---------------------- | ------- |
| **Rust + ratatui 0.30** (crossterm, tokio) | none built in; regions recorded by the code that lays out (prototype: `scene.rs`, 110 lines) | constraint layout; wrapping by hand in atoms | `TestBackend`: a buffer of cells, styles included | one static binary, 6 MB | **herdr itself** (ratatui 0.30, crossterm 0.29, tokio: read from `/usr/bin/herdr`), gitui, yazi |
| **TypeScript + OpenTUI 0.5** (`@opentui/core`, Zig core) | built in: `onMouseDown/Over/Out` on every renderable, a native hit grid | flexbox (Yoga), `flexWrap`, word wrap | `createTestRenderer`, mock mouse and keys, char and span frames | **Bun ≥ 1.3 or Node ≥ 26.4** with `--experimental-ffi` (we have Node 25.8); pre-1.0 | opencode |
| TypeScript + Ink 7 | none: keyboard only (`useInput`); mouse would be parsed by hand, positions from `measureElement` | flexbox (Yoga) | `ink-testing-library`: last frame as text | Node ≥ 22, React 19 | Claude Code, Gemini CLI |
| Go + Bubble Tea (+ Lip Gloss) | mouse messages with cell positions; per-element zones via the third-party bubblezone | string joining; no flexbox | `teatest` golden output | one static binary; **Go not installed** | k9s is tview, lazygit is gocui (both Go), glow, soft-serve |

Ink and Bubble Tea were not prototyped: Ink lacks the mouse the overview is
built around (#303, #296) and would rebuild it on top of React; Bubble Tea
needs a Go toolchain and gives the same single-binary shape as Rust while
the tool we integrate with most (herdr) is Rust + ratatui already.

**The bar the user named, lazygit**, is Go + gocui: a hand-rolled view
layer like today's breadboard, kept fast and tidy by discipline, not by a
framework. The stack is not what makes it good.

## The prototypes

The same slice in both (the ratatui one: `packages/breadboard/spikes/`; the OpenTUI one ran on Bun as a local devDependency and was removed after the decision): MERGE
REQUESTS from live GitLab in one GraphQL request with glab's token,
per-element buttons, block hover, wrapping, a section that throws, and a
headless test that clicks every cell of every button.

Measured on the user's machine (2026-10-01, 120×40 pty, `breadboard-gauntlet/src/measure.mjs`,
recorded answer of 17 MRs; live: 15 open MRs):

| | ratatui | OpenTUI on Bun |
| --- | ---: | ---: |
| first full screen (pty, recorded data) | **94 ms** | 185 ms |
| first screen with live GitLab data (3 runs) | 580 / 1103 / 898 ms | 856 / 685 / 3950 ms |
| the GitLab read itself (one GraphQL call) | 505–750 ms | 467–725 ms |
| CPU idle (10 s) | **0.1 %** | 0.5 % |
| CPU scrolling (40 wheel reports/s, 5 s) | **0.2 %** | 7.2 % |
| peak memory (RSS) | **9 MiB** | 81 MiB |
| what ships | one 6.1 MB binary | 76 MB Bun + 6 MB native core + JS |
| lines, app / test | 620 / 150 (6 tests) | 327 / 121 (4 tests) |
| test run | **0.5 s** (1.4 s after a change) | 92 s |
| type check / build after a change | 1.4 s test build, 11 s release | 1.3 s `tsc`, no build |
| cold build | 37 s (test), 65 s (release) | 10 s `pnpm install` |
| tries until every test was green (the agent writing it) | **1**: compiled and passed on the first run | 8 runs: four framework surprises, below |

What the OpenTUI tries found, each a thing the rewrite would live with:

1. After a scroll, a renderable's `screenX/screenY` still name the last
   frame's place: a test that reads positions from the tree clicks the
   wrong cell. Tests have to find buttons in the drawn frame (black box).
2. A `scrollTop` set right after rebuilding a scroll box's content is
   reported back but not drawn; it needs a frame in between.
3. The scroll bar takes the last column; hover there lights nothing.
4. Rebuilding the tree on each update (the plain core API) destroys the
   renderables a caller still holds; keeping them means a reconciler
   (`@opentui/solid` or `/react`) on top.

What ratatui needed instead: the hit regions written by hand
(`scene.rs`), and wrapping by hand (atoms). Both are small and, because the
same code places text and regions, they cannot drift apart (the cause of
#303 today).

## Findings that hold for any stack

- **#305 is a design fix, not a stack fix**: one GraphQL request for the
  open MRs with their pipelines and all jobs takes 0.5–0.75 s, against 312
  `glab` processes and 29 s today. Both prototypes do it in ~40 lines.
- **#303 / #296** go away when the hit regions come out of the layout pass
  (ratatui prototype) or the framework owns them (OpenTUI), instead of a
  row table filled beside the text.
- **#306**: a section drawn inside its own `catch_unwind` / `try` with its
  own error box; both prototypes show it.
- **Many overviews open** (37 processes seen): either stack idles near
  zero; the cost today is the polling, which a single daemon that reads
  GitLab / git / herdr once and serves every view removes.

## Integration notes

- herdr: Rust (ratatui, tokio, serde). Its socket API is JSON; a Rust
  client is serde structs, a TS one hand-written types.
- GitLab: glab's token from `glab config get token --host gitlab.com`
  (this glab, 1.116, has no `glab auth token`); GraphQL at
  `https://gitlab.com/api/graphql`.
- Hooks (`.claude/hooks/*.mjs`) and nvim Lua run inside Claude Code and
  nvim: they stay what they are whatever the overview is written in.
