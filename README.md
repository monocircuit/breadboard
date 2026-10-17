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
and the Chromium extension. Run it again after an update. `breadboard herdr`
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
| `nvim/changes.lua` | a worktree's `nvim` tab: neo-tree and gitsigns against develop, `:Changes`, `:ChangesDiff`, `:ChangesInline`; see "The nvim tab" below |
| `nvim/colours.lua` | the colours the nvim tabs mark changes in (gitsigns' lines included), readable on any theme |
| `nvim/orchestration.lua` | `:OrchestrationChanges` and `:OrchestrationDiff` in an orchestration's `changes` tab: what its MRs changed |
| `nvim/ide.lua` | Claude Code's IDE integration in a worktree's `nvim` tab (coder/claudecode.nvim): see "claude in nvim" below |

What stays outside Rust, and why: [`docs/rewrite-plan.md`](docs/rewrite-plan.md).
What it knows about the repo (the default branch, the stack, the MR
template's boxes, the changelog, its name in herdr) comes from the repo's
`breadboard.toml`: [`docs/config.md`](docs/config.md).
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

A task agent is a Claude Code session, not a process (#358, rollout
#359–#362): every agent of an orchestration runs headless, and a worktree
of your own keeps its claude in a tab (#361).

- **A click opens one in a tab** (its node, an action that opens it, the
  worktree view's `resume` row): `claude --resume <id> --name <feature>` in
  the tab `<feature>` of its worktree's space, once no run of it is in
  flight; its queue waits until that claude ends.
- **The sidebar and the tab bar read the status table**: the space of a
  worktree shows what its agent waits for (`$wt_state`, the space's third
  row), and "N need you" counts its `asks`.

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
- **The daily limit** (#372), `[agents] daily_limit` in breadboard.toml
  ([`docs/config.md`](docs/config.md)): once the repo's claudes spent it
  since midnight (`breadboard_core::spend`, from their transcripts) no run
  starts, the lines wait and the overview says why. Its `continue anyway`,
  or `breadboard runs continue`, lifts it until midnight
  (`.claude/runs/limit.json`).
- **Pause** (#397): `breadboard runs pause` (or `⏸ pause agents` under the
  orchestration overview's chat) starts no run until `breadboard runs
  resume`; runs in flight finish, `--now` ends them and puts their lines
  back in the queue, `--ci` switches the CI queue off too
  (`breadboard.ci = push`, undone by the resume). Lines keep queueing; the
  band says `agents paused since 13:05 · N lines waiting`. The state is
  `.claude/runs/pause.json`, so it outlives `serve`, herdr and a reboot.
- **The Claude usage limit** (#393): a run whose result is the limit (an
  error with a 429 or the limit's words, `resets 11:30pm`) leaves its
  session `limit` and holds every run until a minute after the reset, or
  15 min, 30 min, then 1 h when it names no time
  (`.claude/runs/usage.json`, `breadboard_core::runs::Usage`). Lines
  meanwhile wait; at the reset each stopped session gets one run with
  `[breadboard] the usage limit reset: continue where you stopped` ahead of
  them. The band says `usage limit until 23:30`, and the orchestrator's
  chat is told once when it holds and once when the runs go on.
- **The status table**, `.claude/runs/status.json`: `running`, `idle`,
  `asks` (the run was denied a tool, its `permission_denials`), `limit`
  (the usage limit), and `queued` from the queue. The overview reads it as
  herdr's agent states (`working`, `idle`, `blocked`, `queued for a run`,
  `waits for the usage limit`). `breadboard runs`
  prints it, paused or not; each run's stream is in `.claude/runs/log/<session>.jsonl`.

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

The pane is kept next to the overview's folds (`.claude/orchestrations/<name>.detail`),
with the try it runs, so a reload of the overview finds it again; one herdr
closed is forgotten.

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
- **`breadboard serve` starts them** (`crates/breadboard/src/daemon/ci.rs`,
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
Rust sources (`crates/` and what it embeds, `breadboard.sh`'s `sources`), never
because develop was merged in; CI (`check:breadboard-rs`) runs them on every
MR that touches the package. Which packages it checks comes from turbo's
`affectedPackages`, not from `...[<base>]`: where the root depends on a
package, turbo counts a change there as one of every package.
