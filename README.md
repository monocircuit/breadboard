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
| `src/worktree/` | its parts: `commands/` (one file per command), `layout`, `spaces`, `tabs`, `lock`, `conversations`, `origins` and `orchestrator` (orchestrations), `prune`, `stack`, `worktrees`, `issues`, `constants` |
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
