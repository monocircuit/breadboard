# @monocircuit/breadboard

The dev workflow of the repo. The name says what it does: worktrees and their
stacks side by side, plugged in, tried, rewired and pulled out again, like
circuits on a breadboard before they are soldered. It holds the worktrees with
their own dev stacks, the live overview, the herdr plugin and the Chromium
extension; the commands keep their names (`wt:*`, `stack:*`, `overview`). How
to use it: skill
[`parallel-work`](../../.claude/skills/parallel-work/SKILL.md). Nothing here
ships; the package is private.

One binary, `breadboard`, in Rust with ratatui (#314; why:
[`docs/stack-evidence.md`](docs/stack-evidence.md), how it was rebuilt:
[`docs/rewrite-plan.md`](docs/rewrite-plan.md) and
[`docs/rewrite-log.md`](docs/rewrite-log.md)).

| Path | What it is |
| ---- | ---------- |
| `crates/core/` | `breadboard-core`: the data formats (records, actions, origin, waits), git, the herdr client, GitLab (GraphQL, cached on disk), the view models (`model/`) |
| `crates/breadboard/src/main.rs` | the commands, named as in the root `package.json`: `breadboard overview`, `wt:*`, `stack:*`, `changelog*`, `commands`, `browser`, `herdr`, `herdr:install`, `hook` |
| `crates/breadboard/src/tui/` | `pnpm overview`: the frame (`run`, `scene`, `view`, `band`, clicks and hover) and `overview/`, one file per section, `actor/` (what the overview does on its own: answers, merges, `--herdr`'s sidebar) |
| `crates/breadboard/src/daemon/` | `breadboard serve`: one process per user reads herdr, GitLab and git's status once for every overview; started by the first, gone after the last |
| `crates/breadboard/src/cli/` | `wt:*`, `stack:*`, `changelog*` in four groups: `read` (`wt:list`, `wt:plan`, `wt:issues`, `overview --once`), `write` (`wt:action`, `wt:tick`, `wt:note`, `wt:mr`, `wt:wait`), `lifecycle` (`wt:remove`, `wt:prune`, `wt:herdr`, the layout, handovers), `setup` (`wt:new`, `wt:stack`, `stack:*`, `wt:check`, `wt:orchestrate`, `commands`, `browser`, the changelogs, `hook pre-push`) |
| `crates/breadboard/src/herdr/` | `pnpm herdr`, `pnpm herdr:install`, `--reload`, the plugin's actions, the Stop hook's `hook stop-answers` |
| [`herdr/`](herdr/README.md) | the herdr plugin `monocircuit.platform` (`herdr-plugin.toml`) and `breadboard.sh`, the launcher everything starts the binary through (below) |
| `hooks/` | the shims of the git pre-push hook (`wt:new` and `wt:mr` install it) and of Claude Code's Stop hook (`.claude/settings.json`) |
| `commands.json` | the line of every script `pnpm commands` shows: a new script gets its line here |
| [`chromium/`](chromium/README.md) | the Chromium extension for tab groups |
| `nvim/changes.lua` | a worktree's `nvim` tab: neo-tree and gitsigns against develop, `:Changes`, `:ChangesDiff`, `:ChangesInline`; see "The nvim tab" below |
| `nvim/colours.lua` | the colours the nvim tabs mark changes in (gitsigns' lines included), readable on any theme |
| `nvim/orchestration.lua` | `:OrchestrationChanges` and `:OrchestrationDiff` in an orchestration's `changes` tab: what its MRs changed |
| `nvim/ide.lua` | Claude Code's IDE integration in a worktree's `nvim` tab (coder/claudecode.nvim): see "claude in nvim" below |

What stays outside Rust, and why: [`docs/rewrite-plan.md`](docs/rewrite-plan.md).
The commands run from the root `package.json` (`sh packages/breadboard/herdr/breadboard.sh <command>`),
so they work from every checkout without a `--filter`.

## How a checkout gets its binary

Everything starts the binary through `herdr/breadboard.sh`: the root
scripts, the herdr plugin, the tab bar, the hooks, nvim's `changes` tab. It
runs `target/release/breadboard` of its checkout when that is the build of
the checkout's HEAD (`breadboard.source` next to it says what it was built
from: a few ms), else makes it first:

- from the repo's store, `<git common dir>/breadboard/bin/<key>/`, when any
  checkout of the clone built these sources before (a copy, ~15 MB): a new
  worktree of `wt:new`, a pull another checkout made first;
- else `cargo build --release` into the store's `target/`, one for every
  worktree (the dependencies are built once; one build at a time), its
  progress on stderr, kept in the store for the others (the newest 8).

Uncommitted changes to the crates build in the store's `target/` too, but
stay out of the store's `bin/`. So a `git pull` never leaves the code from
before running, `wt:new` needs no `pnpm install`, and a worktree costs no
cargo target of its own. Without cargo (https://rustup.rs) and without a build
of those sources, the binary from before runs and says so.

**Every cargo run goes through `herdr/cargo.sh`** (#336): the launcher's
build and `lint:rs`, `test:rs`. It sets `CARGO_TARGET_DIR` to the store's
`target/` (cargo locks it: a second run waits instead of compiling beside
it), runs at `nice 10` on half the cores (`CARGO_BUILD_JOBS`). A plain
`cargo` in the package would compile everything again into a `target/` of
its own (~2 GB): use the scripts. The launcher removes such a `target/` once
(keeping the binary), unless a cargo holds it.

**A stored copy is what its key says** (#339). cargo's hash of a workspace
crate leaves out where the workspace is, and it judges freshness by mtimes:
in a shared `target/`, a worktree's older files count as built after another
worktree's build, and the binary (or clippy's verdict, the test binaries) is
the other one's. So `cargo.sh` holds a `flock` on the target for the whole
run and, when another checkout used it last (`.breadboard-from`), cleans the
workspace's own crates first: switching worktrees recompiles breadboard and
core, never the dependencies. The launcher reads the sources' key under the
store's lock, passes it to the build (`BREADBOARD_SOURCE_KEY`), and the
binary carries it (`breadboard --source-key`). It stores a build only when
the sources were committed and unchanged across it and the file carries the
key, and it uses a stored copy only when that copy carries its key (found with
`grep`, never run); one that doesn't is removed. A stamp from before #339
counts as none, so every checkout takes a checked binary once.

## Headless agents

A task agent can be a Claude Code session instead of a process (#358,
rollout #359–#362): `git config breadboard.agents headless` in the main
checkout turns it on (tabs stay the default until #361).

- **A line for an agent is queued**, not typed into a pane: the overview's
  deliveries (`deliver_to`) and `breadboard runs tell <dir> <line…>` append
  it to `.claude/runs/queue.json` (`breadboard_core::runs::enqueue_line`,
  the session bound in `<dir>/.claude/herdr-session`) and wake the daemon.
- **`breadboard serve` runs it** (`crates/breadboard/src/daemon/runs.rs`):
  `claude -p --resume <id> --name <feature> --permission-mode auto
  --permission-prompts none --disallowedTools AskUserQuestion
  --output-format stream-json --verbose '<lines>'` in the worktree
  (`--session-id` for a conversation without a transcript). One run per
  session at a time; lines queued meanwhile go into the next run together.
  A session open in an interactive claude (`~/.claude/sessions`) holds its
  queue. Between runs no claude process exists. The daemon stays while a
  line is queued or a run in flight; one engine per repo (`.claude/runs/engine.pid`).
- **Capacity**: a run starts while `MemAvailable` less 6 GB (the reserve
  `gitlab-runner-autoscale` keeps) and 0.5 GB for each run still starting
  leaves 0.5 GB. Waiting sessions go in the merge order of their
  orchestration's plan (`wt:plan`), then by age. `BREADBOARD_MEMINFO` names
  another `/proc/meminfo` (the gauntlet's low memory).
- **The status table**, `.claude/runs/status.json`: `running`, `idle`,
  `asks` (the run was denied a tool, its `permission_denials`), and
  `queued` from the queue. The overview reads it as herdr's agent states
  (`working`, `idle`, `blocked`, `queued for a run`). `breadboard runs`
  prints it; each run's stream is in `.claude/runs/log/<session>.jsonl`.

## The main checkout follows develop

The main checkout keeps itself on `origin/develop` (#348). `breadboard
serve`, the daemon of the overviews, fetches every 2 minutes and right after
a merge it saw (the overview's own, or develop's head or pipeline moving on
GitLab), and fast-forwards develop only when that is safe: on `develop`, no
changes to tracked files, `--ff-only`. Another branch, local changes, own
commits are skipped with the reason, never stashed, reset or forced. After an
update `pnpm install --frozen-lockfile` runs when `pnpm-lock.yaml` changed,
and the launcher builds the binary when breadboard's sources changed; vita's
new migrations are only named (each stack runs `pnpm pg:migrate`).

The band of the view of everything and of every orchestration says it:
`develop updated 17:42 · +4 commits (!331, !332…)`, or `develop: not updated
(↓3), local changes in …` in yellow. What happened is in `<git common
dir>/breadboard/develop.json`, the steps' output in `develop-*.log` beside it.

A build installed over the binary a live overview or the daemon runs (the
launcher renames it over the old file) makes it go on in the new one: the
overview ends once no merge of its own runs and starts the new binary with
the same arguments and its folds (the scroll starts at the top); the daemon,
once no headless run is queued or in flight, gives up its socket and starts
the new binary in its place.

## The nvim tab

A worktree's `nvim` tab is your own nvim (LazyVim with neo-tree and gitsigns)
looking at the branch: everything it changed since it left develop
(`git merge-base HEAD origin/develop`), committed, uncommitted or untracked,
what the MR will hold. `nvim/changes.lua` sets the base and the layout,
nothing more; it installs no plugin and names a missing one in the tab.

- **left** neo-tree, every changed file and the folders holding them marked
  (its git status against the base);
- **middle** the first changed file, then whatever you open: gitsigns'
  signs in the gutter, added lines green, changed ones blue with the changed
  words stronger, removed lines in red where they were; it follows your typing
  and an agent's writes. A file new since develop has no old version, so it is
  marked in the tree and the list only;
- **below** `:Changes`, the changed files in the quickfix list with `+added
  -removed`, kept up to date.

| Key | What it does |
| --- | ------------ |
| `]q` / `[q`, `<CR>` in the list | next / previous changed file, the one under the cursor |
| `]h` / `[h` | next / previous change in the file |
| `<leader>ghp` | one change inline, as gitsigns previews it |
| `:ChangesDiff` | the file side by side with its version in the base |
| `:ChangesInline` | the marked lines and the removed ones off and on again (the signs stay) |
| `<leader>e` | neo-tree closed and open again |

A tab reads `nvim/changes.lua` of the main checkout once, when it starts:
one started before an update of it keeps the old view (#297) and says so
(`changes.lua was updated since this tab started`). Quit nvim (`:qa`); a
click on a file in the overview or the next layout starts it again. A tree
that doesn't open is asked for once more, then named in the tab.

The colours come from `nvim/colours.lua`, not the theme's `DiffAdd`: some
themes draw it barely apart from the background. The orchestration's
`changes` tab keeps its own list (`nvim/orchestration.lua`): it shows what
each merged MR changed, which no single base of a tree can.

## claude in nvim

A worktree's `nvim` tab runs [claudecode.nvim](https://github.com/coder/claudecode.nvim)
(`nvim/ide.lua`), and the claude breadboard starts or resumes in the worktree
runs with `--ide`. claude connects to the nvim whose directory holds its
own, so every worktree's claude connects to its own nvim. With that

- an edit claude asks about (default mode, not auto) opens in nvim as a
  diff in a new tab: `:w` or `<leader>aa` accepts it (your changes to it
  included), `:q` or `<leader>ad` rejects it. The prompt in claude's tab
  works as before. In auto mode claude asks about no edit, so no diff opens;
- claude's accepted edits show up in the tab's marks against develop like
  any other change (see "The nvim tab");
- claude sees the open file and the selection: select lines, then ask about
  "this" in claude's tab; `<leader>as` sends the selection as a reference.

The first nvim tab clones the plugin, pinned by tag and commit (`PLUGIN_TAG` in
`crates/breadboard/src/cli/lifecycle/layout/nvim.rs`), into `.claude/nvim/` of the main checkout; git
ignores it. Your nvim and claude config stay as they are; while it runs, the
plugin writes its lock file into `~/.claude/ide/` like any IDE.

**Off** for every worktree of the repo, from any checkout:

```sh
git config breadboard.ide false   # off
git config --unset breadboard.ide # on again (the default)
```

The next nvim tab and the next claude breadboard starts follow it; quit the
nvim of a running tab (`:qa`), the next layout starts it again. claude looks
for its nvim for 30 s after it starts: one started earlier, or a claude that
came into the worktree with EnterWorktree, connects with `/ide`.

## Changelogs

The platform's `CHANGELOG.md` (Keep a Changelog, a section per vita release)
is written by people: fragments in `.changelog/`, folded in at the release by
`crates/breadboard/src/cli/setup/fragments.rs` (`pnpm changelog:*`,
[`.changelog/README.md`](../../.changelog/README.md)). The two below are live
views for working, not a record:

Two, both written by breadboard from GitLab (no model asked) into the ignored
`.claude/` of the main checkout, both shown live in a tab `changelog`
(scroll like the overview; `]` `[` and `1`–`9` jump between sections, a click
on a heading or `z` folds it, `Z` folds all):

- **An orchestration's** (`crates/breadboard/src/cli/setup/changelog.rs`):
  `.claude/orchestrations/<name>.changelog.md`, every merged MR of it, in the
  tab of its space `⇶ <title>`. `pnpm wt:orchestrate <name> --changelog`.
- **The repo's** (the same file): `.claude/changelog.md`,
  what got into develop, a section per product and package (`vita`,
  `monolithium`, `breadboard`, …, `repo` for CI, `.claude` and root config).
  A merged MR goes into every section whose paths it changes, a changeset
  counts for the packages it names; per section the days newest first, then
  the type (Features, Fixes, …), with `!N`, `#issue`, the orchestration it
  came from, the time, a line of its description and its changeset. Releases
  are markers: `vX.Y.Z` in vita, `monolithium-vX.Y.Z` in monolithium and
  utilities, with their `CHANGELOG.md` entry. The overview of the main space
  rewrites it when develop's head moves; `pnpm changelog [--since 30d|2w|<date>]`
  rewrites and prints it (the window sticks, default 30 days). What a merged
  MR changed is kept in `.claude/changelog.json`, so a rewrite reads only the
  new ones. The tab is in the main space, `root · overview · changelog`.

## Checks

```sh
pnpm --filter @monocircuit/breadboard lint      # ESLint of the JavaScript left
pnpm --filter @monocircuit/breadboard lint:rs   # cargo fmt, clippy -D warnings
pnpm --filter @monocircuit/breadboard test      # nvim's Lua in a headless nvim
pnpm --filter @monocircuit/breadboard test:rs   # cargo test (tests next to the code)
pnpm --filter @monocircuit/breadboard build:rs  # the binary of HEAD, through the launcher
pnpm --filter @monocircuit/breadboard-gauntlet gauntlet   # the black-box suite against this checkout's binary
```

`pnpm wt:check` runs `lint:rs` and `test:rs` only when the branch changes the
Rust sources (`crates/`, `hooks/`, the Cargo files, `rustfmt.toml`), never
because develop was merged in; CI (`check:breadboard-rs`) runs them on every
MR that touches the package. Which packages it checks comes from turbo's
`affectedPackages`, not from `...[<base>]`: the root depends on
`@monocircuit/vita`, and turbo counts a change there as one of every package.
