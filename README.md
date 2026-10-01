# @monocircuit/breadboard

The dev workflow of the repo. The name says what it does: worktrees and their
stacks side by side, plugged in, tried, rewired and pulled out again, like
circuits on a breadboard before they are soldered. It holds the worktrees with
their own dev stacks, the live overview, the herdr plugin and the Chromium
extension; the commands keep their names (`wt:*`, `stack:*`, `overview`). How
to use it: skill
[`parallel-work`](../../.claude/skills/parallel-work/SKILL.md). Nothing here
ships; the package is private.

| Path | What it is |
| ---- | ---------- |
| `src/worktree.mjs` | `pnpm wt:*` and `pnpm stack:*`: the entry, its exports are what the rest imports |
| `src/worktree/` | its parts: `commands/` (one file per command), `layout`, `spaces`, `tabs`, `lock`, `conversations`, `origins` and `orchestrator` (orchestrations), `changelog` and `overall-changelog` with their view `changelog-view`, `prune`, `stack`, `worktrees`, `issues`, `constants` |
| `src/overview.mjs` | `pnpm overview`: the loop of the live view; `--herdr` keeps herdr in step |
| `src/overview/` | its parts: reading (`read`, `claude`, `gitlab`, `diff`, `develop`), drawing (`render*`, `boxes`, `agents`), `actions` (clicks and keys), `screen`, `view`, `args` |
| `src/lib/` | what both use: `proc` (`run`, `git`, `sh`, `lines`, `step`, `fail`), `glab`, `herdr`, `tty` (colours, widths, tables) |
| `src/herdr.mjs`, `src/session/` | `pnpm herdr`, `pnpm herdr:install`, `--reload`: the session `platform` (`session`, `setup`), your own herdr (`own`), `reload` |
| `src/git-hooks/pre-push.mjs` | the push guard (`worktree/push-guard`): refuses a push to the branch of a merged or closed MR. `wt:new` and `wt:mr` install it as a shim into the hooks folder all worktrees share |
| `src/browser.mjs` | `pnpm browser`: a URL in the Chromium tab group of its herdr space |
| `src/commands.mjs` | `pnpm commands`: every script with what it is for |
| [`herdr/`](herdr/README.md) | the herdr plugin `monocircuit.platform` |
| [`chromium/`](chromium/README.md) | the Chromium extension for tab groups |
| `nvim/changes.lua` | `:Changes` and `:ChangesDiff` in a worktree's `nvim` tab |
| `src/worktree/nvim.mjs` | the socket of an nvim tab (`.claude/nvim.sock`, relative to its checkout: a socket's path holds 107 bytes at most) and `nvim --server` to open `file:line` in it; the overview's clicks on files use it |

The commands run from the root `package.json` (`node packages/breadboard/src/…`),
so they work from every checkout without a `--filter`.

## Changelogs

Two, both written in node from GitLab (no model asked) into the ignored
`.claude/` of the main checkout, both shown live in a tab `changelog`
(scroll like the overview; `]` `[` and `1`–`9` jump between sections, a click
on a heading or `z` folds it, `Z` folds all):

- **An orchestration's** (`src/worktree/changelog.mjs`):
  `.claude/orchestrations/<name>.changelog.md`, every merged MR of it, in the
  tab of its space `⇶ <title>`. `pnpm wt:orchestrate <name> --changelog`.
- **The repo's** (`src/worktree/overall-changelog.mjs`): `.claude/changelog.md`,
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

## No dependencies

`src/` and `herdr/` import `node:` modules and each other, nothing else.
*Why:* `wt:new` runs before `pnpm install` of the worktree it creates, and the
herdr plugin runs from its directory in any session. `devDependencies` (lint)
are fine.

## Checks

```sh
pnpm --filter @monocircuit/breadboard lint
pnpm --filter @monocircuit/breadboard test   # node --test, *.test.mjs next to the code
```
