# The rewrite of breadboard in Rust (#307)

All of breadboard moves to Rust with ratatui (user, 2026-10-01): the
overview TUI, the `wt:*` / `stack:*` CLI and the herdr plugin. Why this
stack: [`stack-evidence.md`](stack-evidence.md). How the loop decides the
rest: [`rewrite-rules.md`](rewrite-rules.md). What it must pass:
[`breadboard-gauntlet`](../../breadboard-gauntlet/README.md).

## Architecture

One binary, `breadboard`, a cargo workspace in `packages/breadboard/`
next to today's Node code until that is gone:

| Crate | What |
| ----- | ---- |
| `core` | the data formats (records, actions, origin, waits: serde, read and written exactly as today, rule 5), git, the herdr CLI/socket client, GitLab (GraphQL first, one request per refresh, final things cached on disk) |
| `daemon` | `breadboard serve`: one process per user that reads GitLab, git and herdr **once** and serves every view over a Unix socket (`$XDG_RUNTIME_DIR/breadboard.sock`), started by the first client, gone after the last one plus a grace period. 37 overviews today poll 37 times; then one does |
| `tui` | `breadboard overview …`: ratatui + crossterm, a client of the daemon; the layout pass emits the hit regions (the spike's `scene.rs`), each section drawn in its own `catch_unwind` |
| `cli` | `breadboard wt:<cmd>` / `stack:<cmd>`: clap, the same names and flags; the root `package.json` scripts call the binary |
| `plugin` | the herdr plugin: `herdr-plugin.toml` runs `breadboard herdr <action>` instead of `node run.mjs` |

Tests: unit tests per crate (`cargo test`, `TestBackend` frames), and
the gauntlet against the binary (`BREADBOARD_SUBJECT`). CI: a
`check:breadboard-rs` job (`cargo fmt --check`, `clippy -D warnings`,
`cargo test`) and `check:breadboard-gauntlet` against the built binary.
Distribution: `cargo build --release` once per checkout (`pnpm install`
runs it, ~1 min cold, seconds after); a prebuilt binary per platform from
CI later, if building gets in the way.

## What stays outside Rust, and why

| Part | Language | Why |
| ---- | -------- | --- |
| nvim plugins (`nvim/*.lua`) | Lua | nvim runs Lua; they call the binary for data |
| Chromium extension (`chromium/`) | JavaScript | the browser runs it; it only opens URLs in tab groups |
| Claude Code hooks (`.claude/hooks/*.mjs`) | a few lines of Node each | Claude Code starts hooks as commands; the repo's hooks are Node scripts run with the repo's node. Their breadboard part (`stop-answers.mjs` takes answers via `worktree/actions.mjs`) becomes `breadboard hook stop-answers`, and the `.mjs` shrinks to calling it, or the hook entry in `settings.json` calls the binary directly |

Everything else (the 31k lines of `src/`) is Rust at the end.

## Order

Each step side by side with the Node code, switched in one commit when its
part of the suite is green and the critic picked it blind (rules,
"Working"). Rough size in today's lines.

1. **`core` + `daemon` reading** (~4k): records, actions, git, herdr, GitLab
   in one GraphQL query with the disk cache. Proof: the gauntlet's budget
   checks (#305) against `breadboard overview --once`.
2. **Orchestration overview** (~8k): every section, buttons, hover, scroll,
   answers and merges; the Node overview stays the live one until the
   switch. The biggest piece; one section at a time.
3. **Worktree overview and the overview of everything** (~4k).
4. **CLI** (~10k): read-only commands first (`wt:list`, `wt:plan`,
   `wt:issues`, `wt:action --list`), then the writing ones (`wt:action`,
   `wt:tick`, `wt:note`, `wt:mr`, `wt:wait`, `wt:remove`, `wt:prune`), then
   the ones that set up (`wt:new`, `wt:stack`, `stack:*`, `wt:orchestrate`,
   `wt:herdr`). The gauntlet's todo commands get their checks before their
   rewrite.
5. **herdr plugin and hooks** (~1k): the manifest calls the binary; the
   hooks call it. That includes the git pre-push guard (#184): the shim
   `wt:new` and `wt:mr` install still runs
   `node …/git-hooks/pre-push.mjs`; it becomes `breadboard hook pre-push`,
   and both installers change in the same commit (log, 2026-10-01 19:45).
6. **Node goes**: `packages/breadboard/src` deleted when no script, hook or
   plugin calls it; the gauntlet's Node subject stays in its history.

## Existing state

Nothing is migrated: the Rust code reads and writes the same files in the
same places (`.claude/orchestrations/<name>.json`, `.claude/actions.json`,
`.claude/origin`, waits, setups, the herdr plugin id). Node and Rust may
run at the same time on the same files during the side-by-side phase, so
`core` keeps today's locks (`<file>.lock` directories, the pid files in
`$TMPDIR`). A format change, if one is ever wanted, comes with a reader for
the old form (rule 5).
