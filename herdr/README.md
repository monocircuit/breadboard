# herdr for this repo

[herdr](https://herdr.dev) is a terminal workspace for running several coding
agents side by side. This directory makes it fit the repo: a plugin that puts
the parallel-work commands (skill `parallel-work`) on keys and rebuilds the
space of the repo when herdr starts, plus the config `pnpm herdr` sets up.
It is a thin layer: every command of `herdr-plugin.toml` runs
`sh breadboard.sh herdr <action>`, the binary of this checkout (built first
when it is missing or older than the checkout, see `breadboard.sh`), which
turns what herdr passes (the focused pane) into the same code a plain `pnpm`
command runs without herdr. Nobody has to use it.

## Start

```sh
pnpm herdr
```

From any checkout of the repo, also from a pane of another herdr. The first
time it

1. downloads the pinned herdr (`VERSION` in `../crates/breadboard/src/herdr/session.rs`) into
   `.herdr/bin/` of the main checkout, checked against its pinned sha256 — no
   sudo, your own herdr is not touched,
2. writes `.herdr/config.toml`: your config as the base (`.herdr/config.local.toml`,
   else `~/.config/herdr/config.toml`, which is only read), else `config.toml`
   here, `keys.toml` appended, and the sidebar rows (`../crates/breadboard/src/herdr/sidebar.rs`,
   colours from your `[theme.custom]`): the spaces of an orchestration
   (`pnpm wt:orchestrate`) marked `⇶ <name>` (or its `--title`) in its colour (`$orch1`..`$orch3`
   in `blue`, `teal`, `mauve`). A base with its own
   `[ui.sidebar.spaces]` keeps it; one with its own `[ui.sidebar.agents]`
   keeps its rows and gets them for claude with the repo added
   (`rows_by_agent`),
3. starts the herdr **session `platform`** — its own server next to your
   default session — with the pinned herdr first on `PATH`,
4. links this plugin, which builds the layout: the main space (breadboard.toml's `[herdr] space`)
   with `root` and `overview`, a space per worktree, the tabs of every running
   stack (skill `parallel-work`, "In herdr"),
5. opens it in a **new window of your terminal without padding** (kitty,
   alacritty, ghostty): the terminal's padding would otherwise sit as a
   black frame around herdr, and only a window's start can drop it. With
   another terminal, or without a display, it attaches right where you are.
   Closing the window only detaches; `pnpm herdr` again opens it again.

| | |
| - | - |
| `pnpm herdr --here` | attach in this terminal instead of a new window |
| `pnpm herdr --no-attach` | all of it, without attaching |
| `pnpm herdr --reload` | new workflow code into a running herdr, `prefix+shift+l` inside it: `develop` of the main checkout fast-forwarded (only when clean; then the rest runs as the pulled code), the config written again, the overviews restarted, the plugin linked again when its manifest changed. claude, nvim, `pnpm dev` and the logs keep running. The key reloads the herdr it is pressed in, the session `platform` or your own (`pnpm herdr:install`); from a plain terminal every one of them that runs with the space |
| `pnpm herdr --stop` | stop the session `platform`; its panes end |
| `pnpm herdr:install` | the plugin in your own herdr instead, e.g. Omarchy's: see below |

The session's server and its panes get no `CLAUDE*` variables, even when
`pnpm herdr` runs inside Claude Code: a `claude` started in a pane would
otherwise take itself for a child session and save no transcript.

**New herdr version:** bump `VERSION` and the sha256 in `../crates/breadboard/src/herdr/session.rs`
(`gh api repos/herdrdev/herdr/releases/tags/v<version>`). `pnpm herdr`
downloads it; a running session says it is older and needs `pnpm herdr --stop`
once — that ends its panes, so pick the moment.

## Your own herdr (Omarchy)

Omarchy ships herdr (`super+ctrl+enter`). To use that one, your default
session and your config, instead of the session `platform`:

```sh
pnpm herdr:install
```

It "installs" the repo into your herdr: the main space (`[herdr] space`)
with a space per worktree, and everything the session `platform` adds to
your config. It needs a herdr and a running server ≥ 0.9
(`min_herdr_version` in `herdr-plugin.toml`: an older herdr lists the
plugin but doesn't load it), then

1. links the plugin,
2. writes one block between `# >>> <plugin id>` and
   `# <<< <plugin id>` (`[herdr] plugin`) at the end of `~/.config/herdr/config.toml`:
   the plugin's keys and the sidebar rows, as for the session `platform`.
   The first time after a question, then renewed on every run and by
   `--reload`,
3. builds the spaces once. From then on the plugin's `restore` keeps them at
   every start, as your session has the space.

The spaces are built by the scripts of the main checkout, so they look like
`develop` has them. For what was merged since, `prefix+shift+l` in your herdr
(or `pnpm herdr --reload`): it pulls `develop`, renews the block and restarts
the overviews, which otherwise keep the code they were started with — a
plain `git pull` leaves the running layout as it was.

**Omarchy's herdr too old:** it is a pacman package of Omarchy's own repo,
and its `stable` channel can lag behind (0.8.2 when 0.9 was out). The script
then prints the package of the `edge` channel to install:

```sh
sudo pacman -U https://pkgs.omarchy.org/edge/x86_64/herdr-<version>-1-x86_64.pkg.tar.zst
herdr server stop     # the running server stays old until then; ends its panes
```

pacman keeps track of it, so the next `omarchy-update` carries on from there.
Not `herdr update`: it doesn't know pacman and would overwrite a file pacman
owns. `omarchy-refresh-config herdr/config.toml` resets your config, the
block included: run `pnpm herdr:install` again after it.

## What the plugin does

| Key | Action | Opens | Does |
| --- | ------ | ----- | ---- |
| `prefix+o` | `overview` | zoomed pane | `pnpm overview`, `→` on the worktree of the focused tab; `q` closes it |
| `prefix+g` | `commands` | popup | `pnpm commands` of the package the focused pane is in, paged (`q`) |
| `prefix+s` | `stack-ps` | popup | the containers of the **focused tab's** worktree stack |
| `prefix+shift+u` / `+d` | `stack-up` / `stack-down` | popup | `pnpm stack:up` / `down` for that worktree, no port to type |
| `prefix+shift+l` | `reload` | popup | `pnpm herdr --reload`: new workflow code in, conversations keep running |
| — | `restore` | — | `pnpm wt:herdr`: the main space, a space per worktree, the stack tabs |
`restore` also runs at every start of a session that is `platform` or already
has the space; plugins are linked for all sessions of a user, and a personal
session doesn't get a space it never had. At a start it never waits for a
build: the binary from before lays out, the new one builds behind it. The
keys are in `keys.toml`.

The **tab bar** shows `⬢ 3 stacks · 1 needs you` (`breadboard herdr status`,
every 10 s): the running vita stacks, and the agents in the repo's spaces
that ask something. It is also the tick that brings the overviews back
(#388): an `overview` tab of the repo that holds only a shell (after a herdr
or machine restart) gets its overview again, `breadboard herdr heal` under
the layout's lock, never typed into a pane that runs something. The session `platform` puts it in front of your own
`tab_bar_right` (`config.toml` here has it for a session without a config of
yours). Your own herdr: a block can't add to your `[ui]`, so add it to your
`tab_bar_right` yourself:
`{ type = "command", command = "sh <main checkout>/packages/breadboard/herdr/breadboard.sh --no-wait herdr status", interval_seconds = 10, timeout_seconds = 3 }`
(`pnpm herdr:install` prints it, and turns an old `node …/run.mjs status` into it).

## An agent card

A claude agent in the agents panel shows its space, then the repo it belongs
to next to the agent, then what it waits for; no name twice:

```
● 144-branch-pill-morph
  Acme/Shop · claude
  waits for you: merge · !172
● Acme/Shop
  claude
```

The repo (`$wt_repo`, `../crates/breadboard/src/tui/overview/actor/sync.rs`) is on agents in a
worktree's space, an orchestration's too: the main space's name. The
repo's own space names it already,
agents outside the repo get nothing. herdr can't group the agents panel
(herdrdev/herdr#1907); `agent_panel_sort = "spaces"` keeps a repo's agents
together. Rows of your own stay for other agents; claude gets these
(`rows_by_agent`).

## What an agent waits for

A worktree has no space of its own (#409): what its agent waits for is its
row in the overview of its product or orchestration (WORK), not a sidebar
line. The token `$wt_state` of before is cleared from the panes that had it.

The same overview **notifies** (herdr's notifications, titled with the
worktree) when its agent starts asking something, the pipeline of its MR
fails, or its MR is merged: on the change only, never for what it finds at
its start (`../crates/breadboard/src/tui/overview/actor/`).

## The browser

With the extension in [`chromium`](../chromium/README.md), what
`wt:new` and the overview open lands in a Chromium tab group named like the
space: a new worktree's app, and every link clicked in an overview.

## When something does nothing

`herdr --session <main checkout's folder> plugin log --plugin <plugin id>` shows the
output of every plugin command. A stack action on a tab outside the repo says
so in its popup instead of guessing a stack.
