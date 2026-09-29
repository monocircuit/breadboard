# @monocircuit/workflow

The dev workflow of the repo: worktrees with their own dev stacks, the live
overview, the herdr plugin and the Chromium extension. How to use it: skill
[`parallel-work`](../../.claude/skills/parallel-work/SKILL.md). Nothing here
ships; the package is private.

| Path | What it is |
| ---- | ---------- |
| `src/worktree.mjs` | `pnpm wt:*` and `pnpm stack:*`: the entry, its exports are what the rest imports |
| `src/worktree/` | its parts: `commands/` (one file per command), `layout`, `spaces`, `tabs`, `lock`, `conversations`, `origins` and `orchestrator` (orchestrations), `prune`, `stack`, `worktrees`, `issues`, `constants` |
| `src/overview.mjs` | `pnpm overview`: the loop of the live view; `--herdr` keeps herdr in step |
| `src/overview/` | its parts: reading (`read`, `claude`, `gitlab`, `diff`, `develop`), drawing (`render*`, `boxes`, `feed`), `actions` (clicks and keys), `screen`, `view`, `args` |
| `src/lib/` | what both use: `proc` (`run`, `git`, `sh`, `lines`, `step`, `fail`), `glab`, `herdr`, `tty` (colours, widths, tables) |
| `src/herdr.mjs`, `src/session/` | `pnpm herdr`, `pnpm herdr:install`, `--reload`: the session `platform` (`session`, `setup`), your own herdr (`own`), `reload` |
| `src/git-hooks/pre-push.mjs` | the push guard (`worktree/push-guard`): refuses a push to the branch of a merged or closed MR. `wt:new` and `wt:mr` install it as a shim into the hooks folder all worktrees share |
| `src/browser.mjs` | `pnpm browser`: a URL in the Chromium tab group of its herdr space |
| `src/commands.mjs` | `pnpm commands`: every script with what it is for |
| [`herdr/`](herdr/README.md) | the herdr plugin `monocircuit.platform` |
| [`chromium/`](chromium/README.md) | the Chromium extension for tab groups |
| `nvim/changes.lua` | `:Changes` and `:ChangesDiff` in a worktree's `nvim` tab |

The commands run from the root `package.json` (`node packages/workflow/src/…`),
so they work from every checkout without a `--filter`.

## No dependencies

`src/` and `herdr/` import `node:` modules and each other, nothing else.
*Why:* `wt:new` runs before `pnpm install` of the worktree it creates, and the
herdr plugin runs from its directory in any session. `devDependencies` (lint)
are fine.

## Checks

```sh
pnpm --filter @monocircuit/workflow lint
pnpm --filter @monocircuit/workflow test   # node --test, *.test.mjs next to the code
```
