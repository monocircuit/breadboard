# breadboard

A dev workflow for running several coding agents (Claude Code) in parallel
on one repository: every task gets its own git worktree with its own dev
stack, a live overview in the terminal shows every worktree with its agent,
merge request and pipeline, and the decisions an agent needs come to you as
actions you answer with one click. It plugs into [herdr](https://herdr.dev),
a terminal workspace for agents, and comes with a herdr plugin, nvim
integration and a Chromium extension for tab groups. One binary, written in
Rust with [ratatui](https://ratatui.rs).

**Status:** young. breadboard grew inside one monorepo, which is still its
main user; everything it knows about a repository comes from that
repository's `breadboard.toml` ([`docs/config.md`](docs/config.md)). It
works with GitLab (through `glab`) and Linux is where it is used every day;
macOS builds, but is less tried.

## Install

A release binary (Linux x86_64 and aarch64, macOS Intel and Apple Silicon)
from the [releases](https://github.com/monocircuit/breadboard/releases), or
from source with a Rust toolchain (https://rustup.rs):

```sh
cargo install --locked --git https://github.com/monocircuit/breadboard breadboard
```

Then, in your repository, `breadboard init` writes a `breadboard.toml`
when there is none and brings in what breadboard adds for Claude Code and
herdr: the skill `parallel-work`, the hooks, the herdr plugin, the nvim Lua
and the Chromium extension. Every other command needs the file and says so
without it; what goes in it: [`docs/config.md`](docs/config.md). Run init
again after an update. `breadboard herdr`
starts herdr with the plugin and the layout of your worktrees
([`herdr/`](herdr/README.md)).

## Where it is developed

This repository is a read-only mirror. breadboard is developed in a
monorepo, in `packages/breadboard`; every change merged there shows up
here within minutes, with the same history. Issues and pull requests are welcome here:
how a pull request gets in is in [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state
otherwise, any contribution intentionally submitted for inclusion in
breadboard by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.

## In the monorepo

The rest of this README is about breadboard as a package of the monorepo
(`@monocircuit/breadboard`), where it is the dev workflow of the repo.

The dev workflow of the repo. The name says what it does: worktrees and their
stacks side by side, plugged in, tried, rewired and pulled out again, like
circuits on a breadboard before they are soldered. It holds the worktrees with
their own dev stacks, the live overview, the herdr plugin and the Chromium
extension; the commands keep their names (`wt:*`, `stack:*`, `overview`). How
to use it: skill
[`parallel-work`](../../.claude/skills/parallel-work/SKILL.md). The package is
private to npm; its public mirror is described above.

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
| [`herdr/`](herdr/README.md) | the herdr plugin of the repo breadboard is developed in (`herdr-plugin.toml`, its id `[herdr] plugin`; `breadboard init` writes another repo's) and `breadboard.sh`, the launcher everything starts the binary through (below) |
| `hooks/` | the shims of the git pre-push hook (`wt:new` and `wt:mr` install it) and of Claude Code's Stop hook (`.claude/settings.json`) |
| [`chromium/`](chromium/README.md) | the Chromium extension for tab groups |
| `nvim/changes.lua` | a worktree's nvim (the detail pane's `e`): neo-tree and gitsigns against develop, `:Changes`, `:ChangesDiff`, `:ChangesInline`; see "The nvim tab" below |
| `nvim/colours.lua` | the colours nvim marks changes in (gitsigns' lines included), readable on any theme |
| `nvim/orchestration.lua` | `:OrchestrationChanges` and `:OrchestrationDiff` in an orchestration's `changes` tab: what its MRs changed |
| `nvim/ide.lua` | Claude Code's IDE integration in a worktree's `nvim` tab (coder/claudecode.nvim): see "claude in nvim" below |

What stays outside Rust, and why: [`docs/rewrite-plan.md`](docs/rewrite-plan.md).
What it knows about the repo (the default branch, the stack, the MR
template's boxes, the changelog, its name in herdr) comes from the repo's
`breadboard.toml`: [`docs/config.md`](docs/config.md).
The web UI of `breadboard serve` from the phone or another laptop, through
`tailscale serve`: [`docs/tailscale.md`](docs/tailscale.md).
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
- else `cargo build --release` in the checkout's own `target/`, its progress
  on stderr, kept in the store for the others (the newest 8).

`BREADBOARD_STORE` moves the store (#456): CI's `build:breadboard` keeps it
in a cache, so a pipeline whose sources an earlier one built copies the
binary instead of compiling it; a build there takes the dependencies from a
second cache, by `Cargo.lock`.

Uncommitted changes to the crates build there too, but stay out of the
store's `bin/`. **One build per tree** (#451): a build is keyed by the
sources as they are on disk, uncommitted and untracked ones too (the
checkout's index copied, the package added to it, its tree listed), so the
build of a change before its commit carries the key the commit has.
`breadboard.sh --tree` (`pnpm build:rs`, and the gauntlet before it runs)
builds the tree when it isn't built; the scripts keep running that binary
until HEAD's sources move, and once the change is committed as it was built
it is HEAD's binary: `wt:check` builds nothing, and the first launch after
the commit puts it into the store. So a `git pull` never leaves the code from before running and
`wt:new` needs no `pnpm install`. Without cargo (https://rustup.rs) and
without a build of those sources, the binary from before runs and says so.

**Every cargo run goes through `herdr/cargo.sh`** (#336, #446): the
launcher's build and `lint:rs`, `test:rs`. Each checkout compiles in its own
`target/`, so no worktree waits for another's build or check; a
`CARGO_TARGET_DIR` you set wins (one of your own for an uncommitted binary).
It runs at `nice 10` on half the cores (`CARGO_BUILD_JOBS`).

**Install [sccache](https://github.com/mozilla/sccache)** once per machine:
`pacman -S sccache`, `brew install sccache`, `apt install sccache` or
`cargo install --locked sccache`. `cargo.sh` makes it rustc's wrapper (cc-rs
uses it for the C code too, `aws-lc-sys`), so a dependency is compiled once
per version and feature set for every `target/`: a new worktree's first
build and check compile only the workspace's crates. `sccache --show-stats`
shows the hits. `cargo.sh` hands cargo the target and the jobs as `--config`,
not as `CARGO_*` variables: sccache counts those into every crate's key, so
another target would miss them all. Without sccache every `target/` compiles
all dependencies (minutes, ~2 GB), after one line that says so; a
`RUSTC_WRAPPER` you set (empty: none) wins. CI's cargo jobs don't use it.
The store's `target/` from before (`<git common dir>/breadboard/target`) is
unused once every worktree has this `cargo.sh`: remove it then.

**A stored copy is what its key says** (#339). cargo's hash of a workspace
crate leaves out where the workspace is, and it judges freshness by mtimes:
in a `target/` two checkouts share (a `CARGO_TARGET_DIR` set by hand), a
worktree's older files count as built after the other's build, and the
binary (or clippy's verdict, the test binaries) is the other one's. So
`cargo.sh` holds a `flock` on the target for the whole
run and, when another checkout used it last (`.breadboard-from`), cleans the
workspace's own crates first: switching worktrees recompiles breadboard and
core, never the dependencies. The launcher reads the sources' key (of the tree, #451) right
before the build, passes it to the build (`BREADBOARD_SOURCE_KEY`), and the
binary carries it (`breadboard --source-key`). It stores a build only when
the sources were committed and unchanged across it and the file carries the
key, and it uses a stored copy only when that copy carries its key (found with
`grep`, never run); one that doesn't is removed. A stamp from before #339
counts as none, so every checkout takes a checked binary once.

## Headless agents

A task agent is a Claude Code session, not a process (#358, rollout
#359–#362): every agent of an orchestration runs headless, and a worktree
of your own keeps its claude in a tab (#361).

- **A click opens one in the detail pane** (its node, an action that opens
  it, the worktree view's `resume` row, `a`): `claude --resume <id> --name
  <feature>` beside the overview, once no run of it is in flight; its queue
  waits until that claude ends.
- **The tab bar reads the status table**: "N need you" counts its `asks`;
  what an agent waits for is its row in the overview (a worktree has no
  space of its own, #409).

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
  `BREADBOARD_LOOKS` names a file each look at the queue appends
  `{ at, started }` to (the gauntlet's, #470).
- **Capacity**: a run starts while `MemAvailable` less 6 GB (the reserve
  `gitlab-runner-autoscale` keeps) and 0.5 GB for each run still starting
  leaves 0.5 GB. Waiting sessions go in the merge order of their
  orchestration's plan (`wt:plan`), then by age. `BREADBOARD_MEMINFO` names
  another `/proc/meminfo` (the gauntlet's low memory).
- **The daily limit** (#372), `[agents] daily_limit` in breadboard.toml
  ([`docs/config.md`](docs/config.md)): once the repo's claudes spent it
  since midnight (`breadboard_core::spend`, from their transcripts) no run
  starts, the lines wait and the overview says why. Its `continue anyway`,
  or `breadboard runs continue`, lifts it until midnight
  (`.claude/runs/limit.json`).
- **Pause** (#397): `breadboard runs pause` (or `⏸ pause agents` under the
  orchestration overview's chat) starts no run until `breadboard runs
  resume`; runs in flight finish, `--now` ends them and puts their lines
  back in the queue: each claude and every process its run started (its
  `BREADBOARD_RUN` marker, #488), SIGTERM, SIGKILL after 10 s, waited for;
  breadboard's own daemons and sccache's server stay. `--ci` switches the CI queue off too
  (`breadboard.ci = push`, undone by the resume). Lines keep queueing; the
  band says `agents paused since 13:05 · N lines waiting`. The state is
  `.claude/runs/pause.json`, so it outlives `serve`, herdr and a reboot.
- **The review run** (#374), `[review]` in breadboard.toml
  ([`docs/config.md`](docs/config.md)): an MR whose bar is full and whose
  head is green (its pipeline, or `wt:check` where it merges without one) gets one `claude -p` on a small model over its diff and
  issue before its merge is offered (`breadboard_core::review`, the
  overview's `actor/review.rs`). Not a task run: it starts beside the queue,
  within the same memory and never while the daily limit or a pause holds.
  Serious findings go to the MR's agent, all of them into an MR comment.
- **The Claude usage limit** (#393): a run whose result is the limit (an
  error with a 429 or the limit's words, `resets 11:30pm`) leaves its
  session `limit` and holds every run until a minute after the reset, or
  15 min, 30 min, then 1 h when it names no time
  (`.claude/runs/usage.json`, `breadboard_core::runs::Usage`). Lines
  meanwhile wait; at the reset each stopped session gets one run with
  `[breadboard] the usage limit reset: continue where you stopped` ahead of
  them. The band says `usage limit until 23:30`, and the orchestrator's
  chat is told once when it holds and once when the runs go on.
- **Unpushed commits** (#450, `crates/breadboard/src/daemon/push.rs`, the
  rules in `crates/core/src/push.rs`): a run cut off (herdr killed, OOM)
  leaves its commits unpushed while the agent idles, and a push needs no
  LLM. Every 15 s the daemon looks at each orchestrated worktree; one with
  commits its upstream lacks, its agent's last run ended a minute ago and
  none queued, no claude open in it, no `wt:check` or background command
  running and nothing uncommitted gets `wt:check` run by breadboard, then
  `git push` and a draft MR (`wt:mr`, `Closes #<n>`) when it has none. One
  at a time across the clone's daemons (`.claude/runs/push.pid`), only while
  memory allows a run. A red check is one line to the agent (the failing
  task, the log's end); red again after its fix, or a push git refused, a
  `look` action of the worktree. Uncommitted changes are one line to the
  agent, never checked. ACTIONS has no row for them: the node says
  `2 commits + a merge of develop not pushed · wt:check running, then push`
  (`.claude/pushing.json`). Off with `git config breadboard.push off`
  (every gauntlet world but `push.check.mjs`'s).
- **The status table**, `.claude/runs/status.json`: `running`, `idle`,
  `asks` (the run was denied a tool, its `permission_denials`), `limit`
  (the usage limit), and `queued` from the queue. The overview reads it as
  herdr's agent states (`working`, `idle`, `blocked`, `queued for a run`,
  `waits for the usage limit`). `breadboard runs`
  prints it, paused or not; each run's stream is in `.claude/runs/log/<session>.jsonl`.

## The daemon acts, the command table

- **One headless overview per orchestration** (#498,
  `crates/breadboard/src/daemon/acts.rs`): `breadboard serve` runs the
  overview's model and actor for every orchestration with a record, nothing
  drawn. It hands out issues, merges into the batch, delivers answers
  through the run queue and removes merged worktrees with no overview open.
  It holds the orchestration's pid file and writes what its actor did to
  `.claude/orchestrations/<name>.acting` (`breadboard_core::model::acting`);
  an overview in a pane draws that and acts by itself only while the daemon
  can't be reached. `BREADBOARD_NO_ACTS` switches it off (the gauntlet's
  checks of the daemon's other parts).
- **The command table** (`breadboard_core::command`, what lab web will ask
  of a node): `breadboard command '<json>' [--id <id>]` asks the repo's
  daemon one of `start_issue`, `answer_action`, `merge_into_batch`,
  `ship_batch` (done by the orchestration's actor, as its click does),
  `pause_runs`, `resume_runs`, `tell`, `stop_agent` (done by the daemon),
  e.g. `{"kind":"tell","dir":"<worktree>","line":"…"}`. It prints the
  result, exit 1 when refused. The same id again is done once and gets the
  first result (the newest 200 in `.claude/commands.json`); every command
  carries `protocol: 1`. A merge or a start is done once it started; how it
  ends is in the acting file.

## The main checkout follows develop

The main checkout keeps itself on `origin/develop` (#348). `breadboard
serve`, the daemon of the overviews, fetches every 2 minutes and right after
a merge it saw (the overview's own, or develop's head or pipeline moving on
GitLab), and fast-forwards develop only when that is safe: on `develop`, no
changes to tracked files, `--ff-only`. Another branch, local changes, own
commits are skipped with the reason, never stashed, reset or forced. After an
update `pnpm install --frozen-lockfile` runs when `pnpm-lock.yaml` changed,
and the launcher builds the binary when breadboard's sources changed; vita's
new migrations are only named (each stack runs `pnpm pg:migrate`). Off, the
interval and the install: `[main_checkout]` of breadboard.toml
([`docs/config.md`](docs/config.md)).

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

## The detail pane

A click on a worktree or an MR in the overview splits the overview's pane
to the right (herdr, half its width) and runs that worktree's nvim there, the
one of "The nvim tab" below. There is one per overview: a click on another
worktree ends what runs in it (nvim quits; with unsaved changes it refuses
and the band says so) and runs that one's nvim in the same pane; a click on
the worktree it shows, or Esc, closes it. A click on a file opens it there,
at its line, and a click on a try with a terminal (`--run`) runs its command
there: the same try again runs again once it ended, another try replaces it
(a `--url` try stays a page in the browser). Keys of the overview switch it:

| Key | What the pane shows |
| --- | ------------------- |
| `e` | the worktree's nvim |
| `t` | the terminal of its newest open try (`wt:action try … --run`) |
| `l` | its stack's logs, the services' and its background processes' (`.claude/stack/*.log`) together, or its dev server while nothing runs |
| `a` | its agent's conversation (`claude --resume`, once no headless run is in flight; its runs wait while it is open) |

Each worktree's block in WORK has the same as buttons, without a click on
the block first: `agent`, `nvim`, `try`, `logs`, and `MR` for its page in
the browser; a button is drawn only where it has something to show (a
bound conversation, an open try with a terminal, a stack). Every second
block sits on a lighter grey, so the blocks stand apart.

The pane is kept next to the overview's folds (`.claude/orchestrations/<name>.detail`),
with the try it runs, so a reload of the overview finds it again; one herdr
closed is forgotten.

## The nvim tab

A worktree's nvim (the detail pane's `e`, or one you start yourself) is your own nvim (LazyVim with neo-tree and gitsigns)
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

The first nvim clones the plugin, pinned by tag and commit (`PLUGIN_TAG` in
`crates/breadboard/src/cli/lifecycle/layout/nvim.rs`), into `.claude/nvim/` of the main checkout; git
ignores it. Your nvim and claude config stay as they are; while it runs, the
plugin writes its lock file into `~/.claude/ide/` like any IDE.

**Off** for every worktree of the repo, from any checkout:

```sh
git config breadboard.ide false   # off
git config --unset breadboard.ide # on again (the default)
```

The next nvim and the next claude breadboard starts follow it; quit the
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
  what got into the default branch, a section per product and package
  (`[changelog] area_dirs`, `repo` for CI, `.claude` and root config).
  A merged MR goes into every section whose paths it changes, a changeset
  counts for the packages it names; per section the days newest first, then
  the type (Features, Fixes, …), with `!N`, `#issue`, the orchestration it
  came from, the time, a line of its description and its changeset. Releases
  are markers, each tag of `[[releases]]` in its paths, with their
  `CHANGELOG.md` entry. The overview of the main space
  rewrites it when develop's head moves; `pnpm changelog [--since 30d|2w|<date>]`
  rewrites and prints it (the window sticks, default 30 days). What a merged
  MR changed is kept in `.claude/changelog.json`, so a rewrite reads only the
  new ones. The tab is in the main space, `root · overview · changelog`.

## The web UI

The daemon behind the overviews (`breadboard serve`) serves the overview as
a page too: `http://127.0.0.1:7435/` once an overview runs (#435). It shows
the view of everything or one orchestration of each repo, drawn as the
terminal draws it and read again every two seconds; GitLab comes from the
daemon's feed of the view. It binds 127.0.0.1 only, answers only to its own
names, loads nothing from elsewhere and only reads. Port, a proxy's names
(`tailscale serve`) and the write switch: [`[web]`](docs/config.md#web).

## The CI queue

GitLab has no job priorities: its runner takes jobs first come, first served,
so a pipeline of an MR far back could hold up the one next to merge (#357).
breadboard decides which pipelines exist at all:

- **Pushes from a worktree skip CI while a queue runs**: `wt:new` and the
  push guard set `git config --worktree push.pushOption ci.skip` (from the
  push after the guard first ran) while the queue's heartbeat
  (`.claude/ci/queue.json`, written every 10 s) is younger than a minute. When it
  is stale (no `breadboard serve` of a build with the queue runs), the guard
  drops the option and refuses that one push: pushed again, GitLab starts
  the pipeline itself, so nothing stalls. The main checkout and a local
  origin are left alone.
- **The kill switch**: `git config breadboard.ci push` (in the main
  checkout) turns the queue off within 10 s: it starts and cancels nothing,
  and pushes start their pipelines again. `git config --unset breadboard.ci`
  turns it on.
- **`breadboard serve` starts them** (`crates/breadboard/src/daemon/ci/`,
  the rules in `crates/core/src/ci.rs`): every 10 s, while an overview of the
  repo is open, it reads the open MRs and the runner's free slots
  (`concurrent` of `/etc/gitlab-runner/config.toml` minus the running job
  containers, counted as `gitlab-runner-autoscale` counts them;
  `BREADBOARD_RUNNER_CONFIG` names another config). It creates one MR pipeline
  per look, for the MR's newest commit, while a slot is free and no pipeline
  waits for the runner. An older pipeline of that MR still running is
  cancelled. While the runner is full and a click or a hotfix waits first
  (a mere rank in the merge order never does, #419), the lowest-ranked MR
  pipeline behind it is cancelled to make room, one per 30 s: only one
  still waiting for the runner or running for less than 2 minutes with no
  job past its first stage (read back over REST right before), and not
  of an MR preempted in the last 30 minutes. The preempted MR starts
  first once a slot is free, its row says `CI: preempted by !N, restarts
  next`. A pipeline into `main`, of develop or of a tag is never
  cancelled, and neither is one a click started.
- **One pipeline per commit** (#369): an MR's newest pipeline is the one
  with the highest id (in a list of MRs gitlab.com gives their pipelines in
  no order of time). Right before creating one the queue reads the MR and
  its pipelines back over REST: pushed meanwhile, or a pipeline of that
  commit there already (any status but cancelled or skipped; for a click,
  any still to finish), it creates none. A commit asked for once is never
  asked for again by itself, not even when GitLab refused it or it failed
  at its creation. It cancels only pipelines it created itself.
- **A blocked merge**: GitLab's merge waits on an MR's newest pipeline.
  When that is a cancelled one, or one GitLab refused at its creation (a
  `failureReason`, its job activity limit), while an older pipeline of the
  same commit passed, the queue asks for one new pipeline, once per
  blocking one, never over a refused one of its own. What it
  asked for and created is kept in `.claude/ci/asked.json` over restarts,
  and one daemon per checkout schedules (`.claude/ci/scheduler.pid`).
- **Only the orchestrations' MRs**: those in an `order-<name>.json`. An MR
  of no orchestration (vita's, a colleague's) is never touched: no
  pipeline, no cancel.
- **No pipeline before the bar is full** (the user's rule): an
  orchestration's MR gets one only once every box of its issue is ticked
  (after-deploy ones excepted) and `wt:check` passed at its HEAD, the
  commit GitLab has as the MR's newest; its
  live overviews write both, with the merge order, to
  `.claude/ci/order-<name>.json`. Then the agent marks it ready and posts
  the merge after the pipeline is green.
- **The order**: a click on the row (run now, also before the bar is full),
  a hotfix or anything into `main`, the MRs with a full bar in the merge
  order, fixes after a red run. A pipeline that a person cancelled, or that GitLab
  skipped, for the newest commit stays that way until a click.
- **A hard wait holds the pipeline** (#392): an MR whose issue needs an open
  one, or that `wt:wait`s for an open MR of its orchestration, gets none
  (`CI: held, after !N`) until that one is merged; a click still runs it.
  MRs that only share files rank by the merge order and start as usual:
  a shared code file is a soft reason (`goes after !N: shares X`), `.md`
  overlaps none. A green pipeline stays valid however far develop moves; an
  MR merges develop in only on a real conflict (GitLab says so, after a
  merge it is asked to recheck every open MR), and a red develop holds new
  merge offers (`develop red (job): merges held`).
- **The overview** shows a waiting MR's place instead of its pipeline mark,
  e.g. `CI: next (2nd) ▸`, `CI: after the boxes (3/7) ▸` or `CI: after wt:check ▸`; a click on it moves the MR to the front
  (`.claude/ci/run-now.json`). The band shows the runner's load:
  `CI 3/4 slots · 2 queued`. The queue's state is
  `.claude/ci/queue.json`.

## The bench

What breadboard's instructions save (#430): the same small issues, each
run by one headless agent in a probe of its own (a throwaway clone with the
gauntlet's fake GitLab, nothing reaching the real one), with the long brief
and AGENTS.md (`full`), the pointer, skill and lean brief of #426–#429
(`lean`), or no breadboard instructions at all (`none`: hooks, lint and the
pre-push check stay, they are not instructions).
`packages/breadboard-gauntlet/src/bench/` holds it.

```sh
pnpm --filter @monocircuit/breadboard-gauntlet bench           # every task × full and none
pnpm --filter @monocircuit/breadboard-gauntlet bench --fake    # the harness alone, no spend
pnpm --filter @monocircuit/breadboard-gauntlet bench --table   # this table from bench/results/
```

A real run needs Claude's login in the environment
(`CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY`): a probe has no
`~/.claude`.

<!-- BEGIN:bench -->
No results yet: `pnpm --filter @monocircuit/breadboard-gauntlet bench`.
<!-- END:bench -->

## Checks

```sh
pnpm --filter @monocircuit/breadboard lint      # ESLint of the JavaScript left
pnpm --filter @monocircuit/breadboard lint:rs   # cargo fmt, clippy -D warnings
pnpm --filter @monocircuit/breadboard test      # nvim's Lua in a headless nvim
pnpm --filter @monocircuit/breadboard test:rs   # cargo test (tests next to the code)
pnpm --filter @monocircuit/breadboard build:rs  # the binary of the tree (uncommitted changes too), through the launcher
pnpm --filter @monocircuit/breadboard-gauntlet gauntlet   # the black-box suite against this checkout's binary
```

`pnpm wt:check` runs `lint:rs` and `test:rs` only when the branch changes the
Rust sources (`crates/` and what it embeds, `breadboard.sh`'s `sources`), never
because develop was merged in; CI (`check:breadboard-rs`) runs them on every
MR that touches the package. Which packages it checks comes from turbo's
`affectedPackages`, not from `...[<base>]`: where the root depends on a
package, turbo counts a change there as one of every package.
