# Spike: breadboard on Rust and ratatui (#307)

A prototype of one slice of the orchestration overview on the stack the
rewrite uses, Rust + ratatui (user, 2026-10-01: "drop OpenTUI and move
completely to Rust"). It is not wired into the live workflow: nothing in
`pnpm overview`, `wt:*` or the herdr plugin uses it. The OpenTUI prototype
it was measured against is gone from the branch; its numbers are in
[`../docs/stack-evidence.md`](../docs/stack-evidence.md).

## The slice

The MERGE REQUESTS section of the orchestration overview, reading live
GitLab, measured against the open issues of the current overview:

- **One batched read** (#305): open MRs, each with its head pipeline and all
  of its jobs, in one GraphQL request with glab's token
  (`glab config get token --host gitlab.com`), no process per call. Read in
  the background every 30 s and on `r`; the view never waits for it and
  shows how old its data is.
- **Per-element buttons** (#303): every MR block has `open` (the MR),
  `pipeline` (its pipeline) and `jobs` (fold/unfold the job list). A click
  hits the element drawn under the mouse: the hit regions come out of the
  same code that places the text (`src/scene.rs`), they can't drift.
- **Block hover** (#296): the whole block of an MR lights up under the
  mouse; the button under it gets its own stronger highlight on top.
- **Wrapping** (#304): titles and the job list wrap at any width, a job is
  never split and never cut with `…`; failed and running jobs first.
- **Isolation** (#306): a second section, `BROKEN`, panics on every draw.
  It draws its error in its own place; the others draw as usual.
- **A headless test** (`cargo test`, ratatui's `TestBackend`) that renders
  frames at several widths, scrolls through them, clicks every cell of every
  button and checks its own action ran; hover, wrapping and the broken
  section too.

Keys: `q` quits, `r` reads again, wheel / `j` `k` scroll. Without a token or
with `--fixture`, it reads `fixtures/merge-requests.json` (a recorded answer
of the same query).

## Run

```sh
cd packages/breadboard/spikes/ratatui
cargo run --release                 # live GitLab
cargo run --release -- --fixture    # the recorded answer
cargo test                          # headless frames + every button clicked
```
