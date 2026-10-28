---
name: parallel-work
description: Start every task (feature, issue, fix, docs) in its own git worktree of {{repo}} with {{run}} wt:new, and clean it up with {{run}} wt:remove after the merge. Use at the start of any work that will change files, when another session works in the same checkout, when two branches must run in the browser or migrate the database at the same time, when asked to work on several issues at once ({{run}} wt:orchestrate), and for every question or step of breadboard's workflow: the wt:*, stack:* and overview commands, herdr spaces, batches, and an orchestration's agents (their brief, waiting for another MR with wt:wait, wt:check before a push). Load it before answering how or when to post a wt:action (note, merge, try, decide, plan, look).
---

# Every task in its own worktree

**Every task starts with `{{run}} wt:new <feature>`**, whether or not anything
else runs at the same time. There is no decision to make, and no agent ever
works in another's checkout: two agents in one directory switch the branch
under each other and mix their uncommitted changes.

- The **main checkout** stays on `{{branch}}`. It is where `wt:new` is run from
  and where the default dev stack lives, not where work happens.{{#batches}} A worktree's
  **base** is `{{branch}}`, or for a task of a product its batch branch:
  branch, batch stack, merges and hotfixes in [batches.md](batches.md).{{/batches}}
- Start the worktree before the first change to a file, right after reading
  the issue. **On an issue, the name starts with its number**
  (`{{run}} wt:new 34-full-test-pipeline` → `feature/34-full-test-pipeline`),
  otherwise it is named after the task (`chronicle-tags`,
  `login-theme`).
- `wt:new` claims the issue (pushes the empty branch); it stops when origin
  has a branch with the number: say so instead of starting; `--also <m>` bundles
  another issue into the task (#321), refused when a branch or another open
  MR claims it.
- Reading, answering questions and reviewing need no worktree.
- **An agent that starts a worktree moves into it**: right after `wt:new`,
  `EnterWorktree(path: "<the directory wt:new printed>")`. That changes the
  working directory of the session itself, so every command, every edit tool
  and the label of the session in the sidebar point at the worktree. A `cd`
  inside a Bash call does not: the session stays in the main checkout and
  every parallel session looks like `{{main}}` / `{{branch}}`, so nobody sees
  who works where. Never `EnterWorktree` without `path` — it would create its
  own worktree under `.claude/worktrees/`, without what `wt:new` sets up
  (its stack, its setup steps). Next issue, same session: `wt:new`, `ExitWorktree(keep)`,
  then `EnterWorktree(path)` ([herdr.md](herdr.md), "Next issue").
- At the end of the session, Claude Code asks whether to keep or remove the
  worktree: **keep** — `{{run}} wt:remove <feature>` closes it after the merge,
  not here. `ExitWorktree` never deletes a worktree entered with `path`.
- The hook `pre-edit` blocks Edit/Write on tracked files of the main
  checkout (`CLAUDE_ALLOW_MAIN_CHECKOUT=1` only when the user asks); never
  get around it with `sed -i` or `>`.

If `git status` shows changes you didn't make, or the branch changed without
you, another session is working here: don't touch them.

Rare details sit next to this file, read when needed:
[orchestrate.md](orchestrate.md) (orchestrations and their agents), [herdr.md](herdr.md),
[cleanup.md](cleanup.md){{#batches}}, [batches.md](batches.md){{/batches}}.

## Commands

Run them from any checkout of the repo (the `breadboard` binary; what the
repo is to it comes from its `breadboard.toml`):

| Command | Does |
| ------- | ---- |
| `{{run}} wt:new <feature>` | `git fetch`, worktree `../{{main}}-<feature>` on `feature/<feature>` from `origin/<base>` (`{{branch}}`; a task of a product: its batch, a hotfix: `{{branch}}`; `--base <branch>` sets it), pushes the branch, {{setup}}{{#stack}}, and gives it an **own dev stack**: free ports, own `{{stack.env}}`, `{{stack.setup}}`{{#batches}}{{#epic_stack}}. An orchestration's task gets a task stack on demand instead ([batches.md](batches.md)){{/epic_stack}}{{/batches}}{{/stack}} |
{{#stack}}
| `{{run}} wt:new <feature> --shared` | the same but on the **shared** stack of the main checkout (copies its `{{stack.env}}`), no containers — for work that never starts the app |
{{/stack}}
| `{{run}} wt:new <feature> --fix` | branch `fix/<feature>` instead |
| `{{run}} wt:orchestrate <name> <issue>…` | **several issues at once**: a space `⇶ <name>` for this session, per issue a worktree (`wt:new --origin <name>`) and an agent briefed on it, run headless (`claude -p --resume`) only when something happens for it, no tab, nothing running while it waits; a permission it would ask for becomes an overview action, a click on its name opens it in a tab: WORK of its overview shows where each is, `SendMessage` carries only decisions. `--dry-run` first. When and how: [orchestrate.md](orchestrate.md); an agent started by it: [agent.md](agent.md) |
{{#stack}}
| `{{run}} wt:stack` | inside a worktree: move it from the shared stack to an own one |
{{/stack}}
| `{{run}} wt:mr [<issue>]` | inside a worktree: opens the **draft MR** into the worktree's base (`{{branch}}`, or its product's batch; `wt:new --base <branch>`) once, for **exactly its task** (the branch's issue and those `wt:new --also` bundled, #321; a description closing another is refused) — title from the last commit, description from the template with `Closes #<issue>` for each, the **labels of all its issues and the first milestone**, assigned to you, source branch deleted on merge. The issue number comes from the worktree name (`34-…`) or from the argument; without one it refuses. Run again and it only prints the MR that is there. `--dry-run` shows what it would send |
| `{{run}} wt:check [--no-test "<reason>"]` | inside a worktree, **before every push**: {{check}}. The pre-push hook refuses a commit whose tree it didn't pass (`.claude/checked.json`; docs-only passes): check, commit, push. A `fix/*` branch that changes no test is refused (#316): write the test, or `--no-test "<reason>"` when none can (it goes into the MR description for CI's `check:fix-test`). The overview shows `✓ tested <sha>` / `untested: N commits` / `checking…` / `✗ tests failed: …` / `✗ fix without test` per worktree |
| `{{run}} wt:herdr` | inside herdr: the main space (`root`, `overview`) and the space of each product or orchestration; no space per worktree and no stack tabs (see below) |
| `{{run}} wt:prune [--dry-run]` | **after merges**: every worktree whose branch is gone from origin and fully in its base is closed with `wt:remove`, its leftover local branches deleted. Everything with own commits or uncommitted changes is only listed |
| `{{run}} wt:merge-base [--resolve]` | inside a worktree, **the only way to merge its base in** (and only on a conflict or when told): fetches `origin/<base>`, `git merge --no-commit`; clean, it commits; on a conflict it names the files and runs `git merge --abort`, so the worktree is never left half-merged. `--resolve` keeps the conflicted merge open: resolve, `git add`, `git commit --no-edit`, `wt:check`, push in the same run. The overview flags a merge left open for 10 min with no run in flight (`half-merged since 14:02`) |
| `{{run}} wt:tick [<issue>] [<box>…]` | the boxes of the issue (default: the worktree's), numbered under their headings; with box numbers, ticks them in the description and reads them back. `wt:mr` ends with how many are ticked |
| `{{run}} wt:issues` | **after merges**: the issues of merged MRs that are open or have a box left open and got no comment since the merge, with ticked/total — catch them up with `wt:tick` and `wt:note` |
| `{{run}} wt:list` | **the overview**: per worktree the branch, its url with `●` when something answers there, {{#stack}}the stack with its {{stack.first}} port and how many containers run, {{/stack}}and `→` for the one you are standing in |
| `{{run}} overview` | **live**, for the human: every worktree with its app, stack, MR and pipeline, the open MRs and issues; a click on a worktree or an MR opens the detail pane beside it (nvim, try, logs, agent), on a link a browser tab ([herdr.md](herdr.md)). `--once` prints it once, `--worktree <dir>` shows one, with how far it is from {{branch}} and which files collide with {{branch}} or another worktree |
| `{{run}} wt:remove <feature>` | own stack (`down -v`), worktree and branch away |
{{#stack}}
| `{{run}} stack:up\|down\|reset\|ps --port <port>` | from **any** checkout: the stack of the worktree that serves `<port>` — the app port `wt:list` and the worktree's overview show, or a port of its services ({{stack.first}} `{{stack.first_port}}+n`). `<feature>` instead of `--port` works too, nothing at all means the checkout you are in. A worktree on the shared stack acts on the main checkout's, and says so |

`stack:*` with `--port` is for the human who watches several agents. An
agent only controls the stack of its own worktree; the others belong to
other sessions (`stack:ps` to look is fine).
{{/stack}}

`<feature>` is kebab-case, e.g. `chronicle-tags` or `34-full-test-pipeline`
for an issue. At the end, `wt:new` prints the directory, the branch, the
ports and the commands to continue:

```sh
cd ../{{main}}-<feature> && claude
{{#stack}}
{{stack.cd}}{{app.dev}}    # takes the port of this worktree by itself
{{/stack}}
```


`AGENTS.md`, the skills, the hooks and `.claude/settings.json` are tracked, so
they apply in the new worktree. Work in that directory for the whole task.

## In herdr

Inside herdr (`HERDR_ENV=1`) the scripts arrange spaces and tabs themselves:
an agent never calls `herdr` by hand and never runs `{{run}} herdr`. When an
agent runs `wt:new`, its tab stays where it is (a worktree has no space of
its own, #409); it follows with `EnterWorktree`. `wt:remove` closes the
worktree's leftover tabs, idle agents too, but never an agent at work or a running server. The spaces, the stack's processes, the
restore after a restart and the plugin: [herdr.md](herdr.md).

{{#stack}}
## Shared or own stack

Every worktree gets its **own** stack, because that is the only way two
versions run at once. `--shared` is the exception for work that never starts
the app.

| The task needs | Stack |
| -------------- | ----- |
| `{{app.dev}}`, its own migration, a second version in the browser | **own** (the default) |
| only code, lint, typecheck, tests without the app | `--shared` saves the setup and the containers' memory |

- **Browser:** on the shared stack every worktree's app wants the same port,
  `{{app.port}}`; only one runs at a time, and an app that pins its origin (an
  OAuth redirect) works on that one alone. An own stack's app serves on
  `{{app.port}}+n`. In herdr the tab of such a worktree carries the port:
  `40-worktree-ports :{{app.port_1}}`.
- **Migrations:** two branches that each add one would change the same
  shared database.

An own stack moves every port by an offset (`n` = 1, 2, …; the main checkout
is 0): {{stack.ports}}, compose project `{{stack.project}}-<feature>`.

{{/stack}}
## Clean up after the merge

```sh
{{run}} wt:remove <feature>                 # from outside that worktree
{{run}} wt:prune --dry-run                  # every worktree GitLab is done with; without --dry-run it closes them
```

`wt:remove` refuses uncommitted changes and commits that are not in its
base; ask before removing a worktree you did not create. `wt:new` names
the sweep when there is something to close (`{{run}} wt:prune`) or an issue of
merged work nobody came back to (`{{run}} wt:issues`): run it then, nobody else
is around after a merge. What `prune` closes and what it only lists:
[cleanup.md](cleanup.md).

## If a step fails

`wt:new` says which step failed and leaves the half set up worktree. Fix the
cause and run the step by hand in that directory, or start over with
`{{run}} wt:remove <feature>` (a branch without commits counts as merged).

Worktrees created with `claude -w` live elsewhere and are not found by
`wt:remove`; remove them with `git worktree remove <dir>`.
