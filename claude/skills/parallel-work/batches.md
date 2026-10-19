# Batches: one rolling integration branch per product

Every product of the repo (`[products.<name>]` in breadboard.toml) has one
open **batch**, `batch/<product>-<n>`: a branch off `{{branch}}` that every task
of the product branches off and merges into, with a stack of its own. The
user ships the batch to `{{branch}}` (which deploys {{test_instance}}) when they want
to; the next batch opens at once.

```
main            ← production
 └─ {{branch}}     ← {{test_instance}} (deploy), the full pipeline
     └─ batch/<product>-<n>    ← one open per product, its batch stack
         ├─ feature/<n>-…      ← task worktrees, fast checks, on demand a task stack
         └─ fix/<n>-…
hotfix (fix/…, label hotfix) ← off {{branch}}, back into {{branch}}, past the batch
```

## The base

Every worktree has a **base**, `"base": "batch/<product>-<n>"` in its
`.claude/origin`: the product's batch for a task of a product, else
`{{branch}}` (no `base` recorded means `{{branch}}`). `{{run}} wt:new <n>-<feature>
--base <branch>` sets it; without `--base` it is `{{branch}}` for a hotfix, the
batch of the `--origin` orchestration's product or of the issue's
milestone's product (`vita · …` → `vita`), else `{{branch}}`.
Everything that compared with `{{branch}}` compares with the base:

- `wt:new` branches off `origin/<base>`; the claim push of the empty branch
  passes the pre-push gate against the base;
- `wt:mr` targets the base; `wt:check` checks what the branch changes against it;
- the worktree view's section is `BASE` on a batch (`DEVELOP` on `{{branch}}`),
  its DIFF reads `against batch/<product>-<n>`, "N behind" is about the base;
- **merge your base in** only when your MR conflicts with it or the overview/orchestrator
  tells you to (the base moving is no reason: each merge costs a pipeline),
  never rebase, and only with `{{run}} wt:merge-base` (#396), never a
  plain `git merge`: a conflict is aborted with its files named, `--resolve`
  keeps it open to finish in the same run;
- `wt:remove` / `wt:prune` count a branch merged into its base as done.

**One issue, one branch, one MR.** `wt:mr` refuses a description that closes
a second issue (any of GitLab's closing keywords) or an issue other than the
branch's, and `wt:orchestrate` never puts two issues into one worktree:
review, revert, the progress bar and the auto-merge stay per issue, and the
batch keeps one merge commit per task.

## The batch branch

The first batch of a product, `batch/<product>-1`, opens by itself: made from
`origin/{{branch}}` and pushed by the first `wt:orchestrate` or `wt:new` of the
product that needs it. The product's file `.claude/products/<product>.json`
keeps it (`batch`: branch, number, the `{{branch}}` commit it was opened from).
Once a batch is in `{{branch}}` (shipped), the next one, `batch/<product>-<n+1>`,
opens the same way. A worktree still on the shipped batch gets the next one
as its base at its next push, its MR retargeted, with no rebase: the shipped
batch is in `{{branch}}` and the next one starts from there.

An orchestration's checkout `.claude/orchestrations/<name>/` stands detached
on its product's batch and has its setup (`[[worktrees.setup]]`). The setting `batching` (#415, the overview's settings panel; its default
`[settings] batching` in breadboard.toml) off switches batches off: tasks go into `{{branch}}` as before.

## Merges

| MR | Merged by |
| -- | --------- |
| task → its batch | **itself**, once done: every box ticked, every try `looks good`, its fast checks green, no conflicts, its turn in the merge order; a merge commit, no squash |
| batch → `{{branch}}` (ship) | the user's two clicks |
| anything else into `{{branch}}` or `main` (a hotfix) | the user's two clicks |

Into a batch no merge needs to be posted: once the merge turn offers a task
MR (every box, tries `looks good`, pipeline green on its current head, no
conflicts, its turn), the overview marks a draft ready and merges it, one at
a time, never twice for the same head. A posted `wt:action merge` does no
harm (it is marked done with the merge). The overview marks a merged task
`in batch` until its batch reaches `{{branch}}`. The setting
`auto-merge into batch` off leaves that merge to the user's click too.

A task MR into a batch runs only the fast checks (lint and unit tests of the
packages it changes); the full pipeline (gauntlet, integration tests, image,
deploy) runs on the batch's MR into `{{branch}}` and on `{{branch}}`.

## Hotfixes

`{{run}} wt:new <n>-<feature> --fix` of an issue labelled `hotfix`, or `--base main`,
bypasses the batch: it branches off `{{branch}}` (or `main`), its MR goes there with
the full pipeline. The batch takes `{{branch}}` in only when the two conflict:
the overview says so, and the batch's checkout merges it with `{{run}} wt:merge-base`.

## Epic branches of before

An orchestration made before the batches (#350) keeps its `epic/<N>-<name>`
until it ships: its tasks still branch off and merge into it, and the
overview offers it as `ship as batch` (`{{run}} wt:epic <name> --mr`).

{{#epic_stack}}
## The batch stack and task stacks

- **Batch stack** (compose project `{{stack.project}}-epic-<orchestration>`, its own
  offset): made on first need, by a task's `stack:up` or by `{{run}} wt:stack` in
  the orchestration's checkout; `wt:orchestrate` starts none. It then runs for
  the whole batch and serves the batch's app: what the user looks at
  between milestones.
- **Task databases live on the batch's {{epic.service}}.** The batch stack keeps a template
  database `{{epic.template}}` (never connected to), refreshed when needed: migrated
  and seeded again once the tree of the db folder differs from its stamp. The
  batch's app runs on `{{epic.database}}`. A task gets `{{epic.database}}_<feature>`, created with
  `CREATE DATABASE … TEMPLATE {{epic.template}}`, and {{epic.others}} next to
  it (the feature lower-cased, every other character `_`: `351-x-y` → `{{epic.database}}_351_x_y`, at most
  63 bytes). {{epic.service}} clones only within one server, so tasks have
  no {{epic.service}} of their own.
- **A task stack is {{stack.others}} + `{{app.dev}}`**, about half of an own
  stack, and **on demand**: `wt:new` in an orchestration only writes the
  task's `{{stack.env}}` (ports reserved, the database on the batch stack's port, the
  line `BREADBOARD_EPIC=<orchestration>`) and starts nothing. `{{run}} stack:up`
  (or `{{run}} wt:stack`) starts it: the batch stack up and its template current,
  the clone, {{stack.others}} up, the migrations and the stack's setup.
  `stack:reset` on a task: `down -v`, drop, clone again.
- **After merging the batch in**, the next `stack:up` drops the task's database
  and clones it again from `{{epic.template}}`, then its own migrations run. A task
  migration older than the base's newest would be skipped by drizzle:
  `wt:check` refuses it, regenerate it ([db-migration](../db-migration/SKILL.md)).
- **`wt:remove`** of a task drops its databases with its containers.

A worktree outside an orchestration keeps an own full stack, or `--shared`.
{{/epic_stack}}
