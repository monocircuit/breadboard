# Agent guidelines: breadboard

On top of the root [`AGENTS.md`](../../AGENTS.md). What breadboard is and
how it is built: [`README.md`](README.md).

## Verify a change: one build per tree (#451)

```sh
pnpm --filter @monocircuit/breadboard-gauntlet gauntlet [src/checks/<file>.check.mjs]
pnpm wt:check
```

- **The gauntlet** builds the binary of your tree itself (uncommitted
  changes too) when it isn't built yet, once per tree, then runs the checks
  against it. Name the check files your change touches while you iterate;
  CI runs all of them.
- **`pnpm wt:check`** runs `lint:rs` and `test:rs` (clippy, cargo test) and
  reuses the binary the gauntlet built: after your commit it compiles
  breadboard's release binary 0 more times.
- **Never by hand:** `cargo build --release` or a `CARGO_TARGET_DIR` of
  your own. A build with another key compiles the crate once more (1–2 min
  each), a target/ of your own repeats every compile. One test while you
  work on it: `sh herdr/cargo.sh test <name>` in this package (wt:check's
  target/ and profile, nothing compiled twice).

Every cargo run goes through `herdr/cargo.sh` (the checkout's own target/,
sccache, half the cores); `pnpm build:rs` is the gauntlet's build alone.
How long each step takes: [`bench/measure/`](bench/measure/README.md).
