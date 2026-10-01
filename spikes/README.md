# Spikes: the stack for breadboard (#307)

Two prototypes of the **same slice** of the orchestration overview, one per
finalist stack, so the stacks can be compared by hand and by numbers. They
are not wired into the live workflow: nothing in `pnpm overview`, `wt:*` or
the herdr plugin uses them. What they showed, measured:
[`../docs/stack-evidence.md`](../docs/stack-evidence.md).

| Dir | Stack |
| --- | ----- |
| [`ratatui/`](ratatui/) | Rust, ratatui 0.30 + crossterm 0.29, tokio, reqwest |
| [`opentui/`](opentui/) | TypeScript, OpenTUI (`@opentui/core`) on Bun |

## The slice, the same in both

The MERGE REQUESTS section of the orchestration overview, reading live
GitLab, measured against the open issues of the current overview:

- **One batched read** (#305): open MRs, each with its head pipeline and all
  of its jobs, in one GraphQL request with glab's token
  (`glab config get token --host gitlab.com`), no process per call. Read in
  the background every 30 s and on `r`; the view never waits for it and
  shows how old its data is.
- **Per-element buttons** (#303): every MR block has `open` (the MR),
  `pipeline` (its pipeline) and `jobs` (fold/unfold the job list). A click
  hits the element drawn under the mouse, not a row: the hit test asks the
  layout, not a `clicks[row]` table.
- **Block hover** (#296): the whole block of an MR lights up under the
  mouse; the button under it gets its own stronger highlight on top.
- **Wrapping** (#304): titles and the job list wrap at any width, a job is
  never split and never cut with `…`; failed and running jobs first.
- **Isolation** (#306): a second section, `BROKEN`, throws on every draw.
  It draws its error in its own box; the others draw as usual.
- **A headless test** that renders frames at several widths, finds every
  button in the frame and clicks every one of its cells, and asserts its own
  handler ran (not the block's, not nothing).

Keys: `q` quits, `r` reads again, wheel / `j` `k` scroll. Without a token or
with `--fixture`, the prototypes read `fixtures/merge-requests.json` (a
recorded answer of the same query).

## Run

```sh
# Rust
cd packages/breadboard/spikes/ratatui
cargo run --release                 # live GitLab
cargo test                          # headless frames + every button clicked

# OpenTUI (Bun comes as a local devDependency, nothing global)
cd packages/breadboard/spikes/opentui
pnpm install --ignore-workspace
pnpm start                          # live GitLab
pnpm test                           # headless frames + every button clicked
```
