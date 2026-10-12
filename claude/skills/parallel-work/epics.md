# Epics: an integration level per orchestration

Every orchestration (`{{run}} wt:orchestrate <name>`) is an **epic**: a branch
of its own between `{{branch}}` and its tasks, with a stack of its own. The user
sees the epic's result on one app, and only a finished epic reaches `{{branch}}`
(which deploys {{test_instance}}).

```
main            ← production
 └─ {{branch}}     ← {{test_instance}} (deploy)
     └─ epic/<N>-<name>        ← one per orchestration, its own epic stack
         ├─ feature/<n>-…      ← task worktrees, on demand a task stack
         └─ fix/<n>-…
hotfix/*        ← from main, back into {{branch}}, then into every open epic
```

Several epics run side by side, each with its branch and its stack.

## The base

Every worktree has a **base**, `"base": "epic/<N>-<name>"` in its
`.claude/origin` (and in the orchestration's record): the epic branch for a
task of an orchestration, else `{{branch}}` (no `base` recorded means
`{{branch}}`, as before). `{{run}} wt:new <n>-<feature> --base <branch>` sets it;
without `--base` it is the `--origin` orchestration's base, else `{{branch}}`.
Everything that compared with `{{branch}}` compares with the base:

- `wt:new` branches off `origin/<base>`; the claim push of the empty branch
  passes the pre-push gate against the base;
- `wt:mr` targets the base; `wt:check` checks what the branch changes against it;
- the worktree view's section is `BASE` on an epic (`DEVELOP` on `{{branch}}`),
  its DIFF reads `against epic/<N>-<name>`, "N behind" is about the base;
- **merge your base in**, never rebase: `git fetch && git merge origin/<base>`;
- `wt:remove` / `wt:prune` count a branch merged into its base as done.

**One issue, one branch, one MR.** `wt:mr` refuses a description that closes
a second issue (any of GitLab's closing keywords) or an issue other than the
branch's, and `wt:orchestrate` never puts two issues into one worktree:
review, revert, the progress bar and the auto-merge stay per issue.

## The epic branch

`wt:orchestrate <name>` creates `epic/<N>-<name>` from `origin/{{branch}}`, `<N>`
the "Epic N" of its issues' milestone (`breadboard · Epic 15` →
`epic/15-breadboard`; epics are milestones, there is no epic issue), or
`--epic <N>` when they have none. It pushes the branch and records it.
The orchestration's checkout `.claude/orchestrations/<name>/` stands on that
branch (not on a detached `{{branch}}`) and has its setup (`[[worktrees.setup]]`). CI runs the
fast checks of what changed on every push to `epic/*`; deploys stay
`{{branch}}`'s.

## Merges

| MR | Merged by |
| -- | --------- |
| task → its epic | **itself**, once done: every box ticked, every try `looks good`, pipeline green, no conflicts, its turn in the merge order |
| epic → `{{branch}}` (the milestone MR) | the user's two clicks in the overview |
| anything else into `{{branch}}` or `main` | the user's two clicks |

Into an epic no merge needs to be posted: once the merge turn offers a task
MR (every box, tries `looks good`, pipeline green on its current head, no
conflicts, its turn), the overview marks a draft ready and merges it, one at
a time, never twice for the same head. A posted `wt:action merge` does no
harm (it is marked done with the merge). The overview marks a merged task
`in epic` until its epic reaches `{{branch}}`.

## The epic keeps up with {{branch}} (coming with #352)

After every change of `{{branch}}` (a merge, a hotfix) the daemon merges
`origin/{{branch}}` into the epic's checkout, and pushes it when the merge is
clean and `wt:check` passes. A conflict becomes an action for the user
(`decide` / `look`), never a force push. Running tasks then get "merge the
epic in": `git fetch && git merge origin/epic/<N>-<name>`.

## The milestone MR (coming with #352)

At a milestone (the plan marks one, or on demand) `{{run}} wt:epic <name> --mr`,
or its row in UP NEXT, opens the MR epic → `{{branch}}` with the full pipeline.
Its description lists `Closes #a, #b, …` of every task merged into the epic
since the last milestone: an MR into an epic closes nothing (GitLab only
closes from the default branch), this one closes them all when it lands, and
breadboard closes any GitLab left open. The epic branch lives on until the
orchestration ends.

{{#stack}}
## The epic stack and task stacks

- **Epic stack** (compose project `{{stack.project}}-epic-<orchestration>`, its own
  offset): made on first need, by a task's `stack:up` or by `{{run}} wt:stack` in
  the orchestration's checkout; `wt:orchestrate` starts none. It then runs for
  the whole epic and serves the epic branch's app: what the user looks at
  between milestones.
- **Task databases live on the epic's {{stack.database}}.** The epic keeps a template
  database `{{stack.project}}_tpl` (never connected to), refreshed when needed: migrated
  and seeded again once the tree of the db folder differs from its stamp. The
  epic's app runs on `{{stack.project}}`. A task gets `{{stack.project}}_<feature>`, created with
  `CREATE DATABASE … TEMPLATE {{stack.project}}_tpl`, and the databases of the
  other services next to it (`<service>_<feature>`; the feature lower-cased,
  every other character `_`: `351-x-y` → `{{stack.project}}_351_x_y`, at most
  63 bytes). {{stack.database}} clones only within one server, so tasks have
  no {{stack.database}} of their own.
- **A task stack is {{stack.others}} + `{{app.dev}}`**, about half of an own
  stack, and **on demand**: `wt:new` in an orchestration only writes the
  task's `{{stack.env}}` (ports reserved, the database on the epic's port, the
  line `BREADBOARD_EPIC=<orchestration>`) and starts nothing. `{{run}} stack:up`
  (or `{{run}} wt:stack`) starts it: the epic stack up and its template current,
  the clone, {{stack.others}} up, the migrations and the stack's setup.
  `stack:reset` on a task: `down -v`, drop, clone again.
- **After merging the epic in**, the next `stack:up` drops the task's database
  and clones it again from `{{stack.project}}_tpl`, then its own migrations run. A task
  migration older than the base's newest would be skipped by drizzle:
  `wt:check` refuses it, regenerate it ([db-migration](../db-migration/SKILL.md)).
- **`wt:remove`** of a task drops both of its databases with its containers.

A worktree outside an orchestration keeps an own full stack, or `--shared`.
{{/stack}}
