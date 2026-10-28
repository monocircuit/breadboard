# breadboard.toml

Everything breadboard knows about the repository it runs in comes from
`breadboard.toml` at the root of a checkout (#342). A worktree reads its own
file, so a branch that changes it uses its version; a worktree made before the
file existed reads the main checkout's. Without the file no command runs but
`breadboard init`, which writes one (#320): each names the missing file and
prints the smallest one with the repo's names. A key left out takes its default
below; a section left out turns its feature off (no `[stack]`: no stacks).

The file is read again when it changes. One that doesn't match this schema is
refused with its line, the key and why (`breadboard.toml line 12: stack (db):
port 3000 is app's already`), never read in half: a command fails with it, and
a running overview keeps the defaults until it is fixed. Unknown keys are
refused too, so a typo doesn't pass for a default.

## The smallest file

```toml
[repo]
name = "me/shop"
```

Even `[repo]` may stay empty: then the default branch is origin's `HEAD`, the
commands show as `breadboard <command>`, and there is no stack.

## `[repo]`

| Key | Default | What it does |
| --- | ------- | ------------ |
| `default_branch` | origin's `HEAD`, else `main` or `master` where origin has one, else `main` | the branch worktrees start from and MRs target; every ahead/behind, merge base and diff is against `origin/<it>` |
| `run` | `breadboard` | how a command is shown to type: `<run> wt:new`. `pnpm` where the root `package.json` has the commands as scripts |
| `name` | the GitLab project of `origin` | how the repo is named in briefs and errors |

The GitLab project is always `origin`'s (its URL's host and path), as glab
finds it too.

## `[herdr]`

How the repo shows in herdr, one plugin per repo.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `plugin` | `breadboard.<main checkout's folder>` | the plugin's id: its manifest's, the sidebar tokens' source, the marker of its block in herdr's configs |
| `space` | `[repo] name`, else the main checkout's folder | the label of the main checkout's space; a product's is its name, an orchestration of no product `⇶ <title>`; a worktree has none (#409) |

## `[[worktrees.setup]]`

What `wt:new` runs in a new worktree after `git worktree add` and the claim's
push, in order. A step that fails names the steps left to finish it by hand.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `run` | (required) | a shell command, in the worktree |
| `says` | the command | the step line |
| `is_quiet` | `false` | its output shows only when it fails (always quiet for an agent) |

```toml
[[worktrees.setup]]
run = "npm ci"
says = "npm ci"
```

## `[stack]`

A checkout's dev stack: containers in docker compose, the app on the host. A
worktree shares the main checkout's stack, or gets its own (`wt:stack`, or
`wt:new` without `--shared`) with every port moved by an offset and a compose
project `<project>-<feature>`, written into its env file. Without `[stack]`
there is no env file, no offset, no stack processes, no `stack:*`.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `dir` | `.` | the folder of a checkout the stack's files are in and its commands run in |
| `env_file` | `.env.local` | the env file in `dir`: an own stack's is the main checkout's with the ports moved |
| `compose_file` | `docker-compose.yml` | docker compose's file in `dir` |
| `project` | (required) | the compose project of the main checkout |
| `setup` | `docker compose … up -d` | what sets an own stack up (containers, migrations, seed): a step as in `[[worktrees.setup]]` |
| `up`, `down`, `reset` | compose's `up -d --wait`, `down`, `down -v` and `up -d --wait` | what `stack:up`, `stack:down`, `stack:reset` run in `dir` |

A worktree's offset is read back from its env file: the variable of the lowest
port with an `env`, moved from that port, when the file names a compose project
of its own (`COMPOSE_PROJECT_NAME`). So at least one port needs an `env`.

### `[stack.app]`

| Key | Default | What it does |
| --- | ------- | ------------ |
| `port` | (required) | the app's port; an own stack's is moved by its offset |
| `dev` | `pnpm dev` | what starts it in `dir` (`--port <p>` follows for an own stack); it runs in the background while the stack is up, its output in `.claude/stack/dev.log` |
| `env` | none | the variable it is moved with |

### `[[stack.services]]`

The ports besides the app's, each moved by the offset.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `name` | (required) | the compose service: its row in WHERE |
| `label` | none | how lines name it (`Postgres 5434`); a service without one is moved but not named |
| `port` | (required) | its port |
| `env` | none | the variable it is moved with |
| `url` | none | where it is opened, `{port}` its moved port (`http://localhost:{port}/admin`): a link in WHERE |
| `address` | none | or where to connect (`localhost:{port}`); with neither it has no row |

### `[stack.migrations]`

The migrations a worktree's database may lack: drizzle's journal against its
`__drizzle_migrations` table, asked with `psql`.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `journal` | (required) | drizzle's `meta/_journal.json`, in `dir` |
| `run` | (required) | what applies them, in `dir`: STACK's button |
| `database_env` | `DATABASE_URL` | the env file's variable with the database's URL |

### `[[stack.tools]]`

What a worktree may run beside its stack on a port moved with it: a row in
WHERE and a herdr tab of its name.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `name` | (required) | its tab and row |
| `dir` | the stack's `dir` | the folder it runs in |
| `port` | (required) | its port |
| `run` | (required) | what starts it, `{port}` its moved port |
| `url` | `http://localhost:{port}` | where it is looked at |
| `is_with_stack` | `false` | its tab opens while the stack runs, as `dev` does; else WHERE has a button that starts it |

### `[stack.batch]`

The batch stack (#411, the epic stack of #351) and the task stacks on it.
The orchestration's checkout, on its product's batch, runs a full stack. Its database service keeps a template database,
migrated and seeded, that nothing connects to. A worktree of the orchestration
gets a task stack: its own stack without that service, on the batch stack's compose
network, with its databases made on the batch stack's service. A task stack starts on
demand (`stack:up`, `wt:stack`), never with `wt:new`. Without `[stack.batch]`, an
orchestration's worktrees get own stacks like any other. It needs
`[stack.migrations]`, whose `run` migrates the template.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `service` | `postgres` | the `[[stack.services]]` the task stacks share (it needs an `env`); `psql` runs in its container |
| `superuser` | `postgres` | `psql`'s user there |
| `template` | (required) | the template database |
| `stamp` | (required) | a folder in `dir` whose git tree the template is made from: when it changes, the template is made again, and a task's clone is made again once its branch has the same tree |
| `template_sql` | none | statements run in a new template before its migrations (schema grants) |
| `seed` | none | what seeds the template after its migrations, in `dir` |
| `task_setup` | none | what a task stack runs, in order, after its containers are up and its migrations applied |
| `env` | none | a table of what a task stack's env file sets besides; `{network}` is the epic's compose network |

`[[stack.batch.databases]]` are the databases a task gets, `<name>_<feature>`:

| Key | Default | What it does |
| --- | ------- | ------------ |
| `name` | (required) | the name before `_<feature>` |
| `is_clone` | `false` | cloned from the template; exactly one database is, the others are made empty |
| `owner` | (required) | the role that owns it (and the template, for the clone) |
| `connect` | none | the roles that may connect besides its owner |
| `urls` | none | the env file's variables whose URL points at it (on the epic's port); the template's migrations and seed get them pointed at the template |
| `name_env` | none | the env file's variable set to its name |

```toml
[stack.batch]
template = "app_tpl"
stamp = "db"
seed = "npm run seed"

[stack.batch.env]
NETWORK = "{network}"

[[stack.batch.databases]]
name = "app"
is_clone = true
owner = "app"
urls = ["DATABASE_URL"]
```

## `[mr]`

| Key | Default | What it does |
| --- | ------- | ------------ |
| `template` | `.gitlab/merge_request_templates/Default.md` | what `wt:mr` puts down for a branch's first description |
| `[[mr.tests]]` | none | boxes of the template's Tests section the pre-fill decides on: `line` (the box, `- [ ] …`) goes in when the branch changes a file one of `paths` (regular expressions) matches, and out otherwise |

## `[merge]`

| Key | Default | What it does |
| --- | ------- | ------------ |
| `needs_pipeline` | `true` | an MR into the default branch is offered for its merge only once its own pipeline passed. `false` is temporary (#443, until the batch branches carry the testing): `wt:check` passed at its pushed head is enough, its row says `merges on wt:check (no MR pipeline)`, and the CI queue starts no pipeline for it by itself (a click still does). The default branch's pipeline after each merge still runs, and a red one holds the next merge offers. GitLab must let it merge without one: Settings → Merge requests → Merge checks, "Skipped pipelines are considered successful" on, or "Pipelines must succeed" off |
| `batch_needs_pipeline` | `true` | a task MR into its batch (`batch/<product>-<n>`) is offered, and merged by itself (`auto_merge_into_batch`), only once its own pipeline passed. `false` (#461): every box, every try `looks good`, `wt:check` passed at its pushed head, no conflicts and its turn are enough; its row says `merges on wt:check (no MR pipeline)` and the CI queue starts no pipeline for it by itself (a click still asks for one; this repo's `.gitlab-ci.yml` skips MR pipelines into `batch/*`). The batch's own MR into the default branch always runs the full pipeline, whatever `needs_pipeline` says, and `ship batch` waits for it green. `needs_pipeline` is for the other MRs into the default branch (a hotfix) |

## `[products.<name>]`

The products of a monorepo (#405). An orchestration belongs to one: its
issues' milestone names it (`vita · Epic 12: …` → `vita`), or
`wt:orchestrate --product <name>`; the record keeps it (`product`). Every
orchestration of a product works in the product's one herdr space, its
overview showing each as a section. Without any, an orchestration has a
space of its own (`⇶ <title>`).

| Key | Default | What it does |
| --- | ------- | ------------ |
| `paths` | none | the folders that are the product's; at least one. The name labels the herdr space: letters, digits, `-` and `_` |

## `[settings]`

The team's defaults of the overview's settings panel (#415; `,` or `⚙
settings` in an orchestration's overview). A change in the panel is kept
per product in `.claude/products/<name>.json` (for an orchestration of no
product in its record), local and never committed; it wins over the
default until `reset`, holds from the next tick, and the orchestration's
journal says it (`auto-merge into batch: off (by you)`).

| Key | Default | What it does |
| --- | ------- | ------------ |
| `batching` | `true` | a task of a product branches off the product's rolling batch `batch/<product>-<n>` and its MR goes into it (#411): opened from the default branch when the product has none, the next the moment one shipped, kept in the product's file (`batch`). Off: their MRs go straight into the default branch. A hotfix (`wt:new --fix` of an issue labelled `hotfix`) always does |
| `auto_merge_into_batch` | `true` | a task MR into the batch merges by itself once its bar is full and its checks are green, with a merge commit, never squashed; off: it waits for the user's click |
| `ship_after_tasks` | `5` | the overview suggests shipping the batch once this many tasks are in it (#414), 1 to 99 |
| `ship_after_days` | `2` | or once its oldest task waited this long, 1 to 30 |
| `try_checker` | `false` | a small model checks and answers the tries (auto mode, #384); a record's `auto` of before holds until the panel changes it |
| `auto_fresh` | `[orchestrator] auto_fresh` | the overview starts the orchestrator's chat fresh at a quiet point by itself |
| (panel only) `agent fresh start`, `agent fresh start when idle`, `agent idle time` | `[agents] fresh_above`, `idle_fresh_above`, `idle_minutes` | when an agent retires at its merge (#381, #445), in thousands of tokens and minutes; their team defaults stay `[agents]`'s |

## `[notifications]`

The desktop notifications of the repo's events (#449): `notify-send` with
the app name `breadboard`, the issue or MR as its title (`!449 feat: …`),
what happened as its body. The daemon behind the overviews (`breadboard
serve`) reads GitLab's newest issues and MRs and the repo's open `decide`
actions every 10 s, and one daemon per checkout sends each event once: what
it saw is kept in `.claude/ci/notified.json`, so a restart repeats nothing,
and its first read only learns what is there. Only for a repo GitLab serves.
Without `notify-send` on the machine the overview says so in one line and
none are sent. herdr's own notifications (an agent asks, a pipeline failed
in a worktree's space) stay as they are.

The settings panel has a switch for all (`desktop notifications`) and one
per event; they are the repo's, not an orchestration's: a change is kept in
`.claude/notifications.json` and holds in every orchestration's panel.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `desktop` | `true` | any desktop notification at all; off: none, and what was seen is forgotten (switched on again, it starts from what is there) |
| `events` | every one | the events that send one: `issue_created` (by anyone, an agent or the orchestrator too), `mr_opened`, `mr_merged` (into develop, an epic or a batch), `mr_ready`, `pipeline_failed` (an MR's newest pipeline), `mr_conflict`, `decide` (an agent or an orchestrator asks the user) |

## `[architecture]`

The LikeC4 model of the repo (#439). `arch:deltas` says which of its
elements a change adds, changes or removes, as JSON for the architecture
view: `--worktree [<dir>]` (the default, the worktree you stand in: its work
against its base, uncommitted changes included), `--mr <iid>` (GitLab's
diff of it, fetched where it isn't here yet) or `--epic [<branch>]` (an epic
or batch branch against the default branch; alone, your worktree's base);
`--out <file>` writes it there instead of stdout. Without the section there
is no `arch:deltas`.

An element owns the paths of its `metadata { paths '<path>, <path>' }`
(from the repo's root, a folder or a file); a changed file belongs to the
element with the longest path that holds it, and one that none holds is
`unmapped`. An element only the change's model has is `added`, one only its
base's has is `removed` (with the deleted files it owned), one in both is
`changed` when it owns a changed file or its kind, title or paths differ.
A head without the model (a branch from before it) is mapped with the
model of the checkout the command runs in: then nothing is added or removed.
Of LikeC4's language only the kinds of `specification`, the elements of
`model` (nested, `extend`), their `title` and `metadata` are read.

```json
{
  "scope": "worktree",
  "name": "feature/439-…",
  "base": "<sha>",
  "head": "<sha>",
  "model": "architecture",
  "elements": [{ "id": "breadboard.core", "status": "changed", "files": ["…"] }],
  "unmapped": []
}
```

| Key | Default | What it does |
| --- | ------- | ------------ |
| `model` | `architecture` | the folder of the model's `.c4` / `.likec4` files, from the repo's root |

## `[orchestrator]`

The orchestrator's chat (#382): its context shows in the overview's title band.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `fresh_above` | `50` | the share of its context (percent) from which the overview offers `↻ fresh start` at a quiet point: its chat idle, no `decide` or `plan` open, no merge in flight. The old chat writes its journal, a new one starts from the journal and the record (`wt:orchestrate <name> --fresh`). The task agents' own is `[agents] fresh_above` |
| `auto_fresh` | `false` | the overview makes that fresh start by itself, without a click |

## `[agents]`

What the agents may spend (#372). The overview reads it from claude's
transcripts: every answer's tokens, priced per model. Tokens are what an
answer read and wrote anew (input, cache writes, output); cache reads are in
the dollars, not in the tokens. An issue's cost is its agents' answers
while they worked on it and its worktree's try checkers' (#387). A limit is tokens (`"20 M"`, `"500k"`,
`20000000`) or dollars (`"$40"`).

| Key | Default | What it does |
| --- | ------- | ------------ |
| `daily_limit` | none | what the repo's claudes may spend from local midnight: once it is reached the run engine of headless agents starts no new run, their lines wait in the queue and the overview says why; its `continue anyway` lifts it until midnight (`breadboard runs continue`) |
| `orchestration_budget` | none | what an orchestration's agents may spend in all: shown against what they spent in its band |
| `fresh_above` | `150_000` | the context, in tokens, above which an orchestration's agent gets no next issue: it retires at its merge, writing a handoff of what it learned, and its next issue goes to a fresh agent that reads it (#381, #445). Every resume reads the whole history again, so a large one costs on every event |
| `idle_fresh_above` | `100_000` | the context above which it retires too once idle longer than `idle_minutes`: its prompt cache (an hour) is gone, and a resume writes all of it to the cache again before it does anything. Below both, an agent that shares files with the next issue takes it |
| `idle_minutes` | `55` | how long idle that is: a little under the cache's hour |
| `mcp_configs` | `[]` | the MCP configs (files relative to the main checkout) a headless run loads (#445). A run starts lean: `--strict-mcp-config` with only these, `--setting-sources project,local` (the repo's skills and settings, no user-level ones), and of `~/.claude/settings.json` only the model, effort, auto mode and permissions. Measured: the first call of a run 31.9 k → 23.4 k tokens |
| `max_runs` | `4` | how many headless runs may be in flight at once (#445). After a crash, an OOM kill or a reboot the run engine picks every cut-off run up by itself, a few at a time: a small session is resumed with one line, one past the fresh-start rule is not resumed but taken over by a fresh session in its worktree, with a handoff breadboard writes from the disk (`.claude/handoff-cut.md`: its last prompt and answer, uncommitted and unpushed work) |
| `docs_labels` | `["documentation"]` | the issue labels that make its agent a `docs` one (`[agents.models]`) |

```toml
[agents]
daily_limit = "$40"
orchestration_budget = "200 M"
fresh_above = 150_000
idle_fresh_above = 100_000
idle_minutes = 55
```

### `[agents.deny]`

What the repo's agents must never touch (#375), on top of Claude Code's own
deny rules. Run `breadboard init` after a change: CI's `init --check` fails
while `.claude/settings.json` lacks a rule.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `paths` | `[]` | gitignore-style globs from the repo's top. `breadboard init` makes each an `Edit(<path>)` deny rule (every tool that edits), and the pre-push gate refuses a push whose diff against its base changes or deletes a file one holds, naming the rule and the files. A new file passes: a tool made it (a generated migration), since agents can't edit it. So a file that tools rewrite (a lockfile) belongs in Claude Code's own deny rules, not here |
| `commands` | `[]` | Claude Code Bash rule patterns (`git tag:*`): `breadboard init` makes each a `Bash(<command>)` deny rule |

```toml
[agents.deny]
paths = ["**/migrations/*.sql"]
commands = ["git tag:*"]
```

### `[agents.models]`

A model per kind of work (#373): every claude breadboard starts gets
`--model` of its stage. A task agent's stage comes from its issue's labels
when it starts (`docs` with a `docs_labels` one, else `build`) and is kept in
its worktree (`.claude/stage`); a run whose every line is its MR's failed
pipeline is `ci_fix`. The try checker is `review`, the orchestrator's chat
`orchestrator` (its tab's `claude`, a resume, a fresh start). The overview
shows the model each agent last answered with next to its name.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `default` | none | the model of every stage without its own; none at all: claude's own default, no `--model` |
| `build` | `default` | a feature or fix agent |
| `docs` | `default` | an agent of an issue with a `docs_labels` label |
| `ci_fix` | `default` | a run that only gets a failed pipeline of its MR |
| `review` | `[tries] checker` | the try checker (not `default`: a small model unless set) |
| `orchestrator` | `default` | the orchestrator's chat |

```toml
[agents.models]
default = "opus"
docs = "sonnet"
ci_fix = "sonnet"
```

## `[tries]`

A `try --on test` waits for the merge's deployment to the test instance, then
opens its path there.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `test_instance` | `the test instance` | what the skill calls the test instance (`dev.example`) |
| `test_origin_env` | `TEST_ORIGIN` | the variable with the test instance's origin: the environment's, else the main checkout's env file |
| `deploy_jobs` | none | the jobs of the default branch's pipeline that deploy; a failure of another doesn't hold the try |
| `deploy_job` | none | the last of them; without one the pipeline passing is the deploy |
| `publish_job` | none | the last step of a pipeline that has no `deploy_job` |
| `checker` | `haiku` | the model that checks and answers a try while its orchestration's auto mode is on (#384): `claude -p --model <it>` in the try's worktree; `[agents.models] review` wins over it |

A deploy counts as done 90 s after its last job passed (the host pulls and
starts the image).

## `[review]`

A cheap review of an MR before its merge is offered (#374): once its bar is
full and its head green (its pipeline passed, or `wt:check` at its pushed
head where `[merge] needs_pipeline = false` merges without one), the live overview of its orchestration starts
one `claude -p --model <model>` in its worktree. It is given only the diff
against the MR's target (not the repo), the issue and the AGENTS.md of the
folders it changes, with read-only tools (`Read,Grep,Glob`), no MCP servers
and `--max-budget-usd`. Its findings go into an MR comment; `serious` ones
go back to the MR's agent and hold the merge (the MR back to draft) until a
new head is reviewed, `small` ones are the user's. One review per head; the
MR row says `reviewed ✓` or `review: 2 findings`, with its tokens. None
starts while the daily limit (`[agents] daily_limit`) or a pause holds the
runs, and its transcript counts toward the daily cost. Every review done or
skipped is a line of `.claude/reviews.jsonl` in the main checkout (model,
lines, tokens, dollars, findings): the cost per review is measured there.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `enabled` | `false` | reviews before the merge; off until its cost per review is measured |
| `model` | `haiku` | the model of the review run |
| `min_lines` | `20` | a diff of fewer changed lines is skipped |
| `max_lines` | `3000` | a diff of more is skipped: no cheap review, the user reads it |
| `max_dollars` | `0.5` | what one review may cost at most (`--max-budget-usd`) |
| `exclude` | `pnpm-lock.yaml`, `**/Cargo.lock`, `**/migrations/meta/**` | pathspecs left out of the diff and its count |

A docs-only diff (`.md`, `.mdx`, `.txt`, `.rst`, `.changelog/`, `.changeset/`)
is skipped too.

## `[check]`

What `wt:check` runs and the pre-push gate waits for.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `turbo` | `false` | lint, typecheck and test of every changed package and its dependents through turbo (a pnpm monorepo) |
| `run` | none | else one command over the branch (`cargo test`, `npm test`); with neither there is nothing to check and every push passes |
| `[[check.stack_tests]]` | none | `package` and `says`: what `wt:check` says of a package whose tests need the stack instead of running them |

## `[main_checkout]`

The main checkout keeps itself on the default branch (#348): the daemon of
the overviews fetches and fast-forwards it only when that is safe (on the
branch, no changes to tracked files, `--ff-only`), and the overviews' band
says when it last did or why it didn't.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `follow` | `true` | off: the main checkout stays where it is until someone pulls |
| `every_minutes` | `2` | how often it looks when no merge woke it (the overview's own merges and the branch moving on GitLab wake it at once) |
| `install` | none | what runs in it after an update that changed a file of `install_when` (`pnpm install --frozen-lockfile`) |
| `install_when` | none | the files whose change runs `install` (`pnpm-lock.yaml`) |

New migrations (`[stack.migrations]`'s journal changed) are only named in
the band with their command: each stack runs them itself.

## `[ci]`

A failed MR pipeline (#371): `breadboard serve` retries a job the runner
failed (its system, a timeout) and a job that failed its first try (a flake
passes the second); one that failed twice its agent fixes, a headless run
told the job and its log's end. Each head gets one such run; past
`max_fix_attempts` of them the MR gets a `look` for the user.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `max_fix_attempts` | `3` | the runs of its agent that may fix an MR's failed pipeline (the row shows `CI fix 2/3`); `0`: the user looks at once |

## `[ci.watch]`

The CI job watcher (#379): `breadboard serve` reads every finished job of the
project's pipelines into `.claude/ci/jobs.json` (14 days), samples the CPU and
memory of the jobs the local runner runs (`docker stats`), and flags a job
that holds pipelines up. The overview shows it under its band; a slow job
becomes a `plan` to file an issue, a hung one a `decide` (retry, cancel,
wait), both in every orchestration's ACTIONS.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `is_on` | `true` | off: nothing read, nothing flagged |
| `window` | `10` | the newest pipelines of a job the critical path and idle count over |
| `min_runs` | `5` | runs a job needs before it is flagged |
| `min_seconds` | `120` | a job whose median is shorter holds nothing up |
| `critical_margin_seconds` | `60` | the last job of a pipeline is its critical path when the one before ended this much earlier |
| `critical_share` | `0.5` | flagged when it is the critical path of more than this share of its pipelines |
| `slowdown` | `0.3` | flagged when its median of the last `slowdown_days` grew this much over the days before |
| `slowdown_days` | `7` | |
| `hang_factor` | `2.0` | a running job past this many times its median is hung (and marked on its MR's row) |
| `hang_seconds` | `120` | … and past this many seconds |
| `hang_quiet_seconds` | `60` | … and its log hasn't grown for this many seconds: a job that still writes its log is slow (a busy runner), not hung |
| `idle_seconds` | `300` | a job whose median run is this long … |
| `idle_cores` | `0.5` | … on fewer cores than this mostly waits |
| `history_days` | `14` | how long a run is kept |

## `[changelog]`

The fragments `changelog:*` folds into `CHANGELOG.md` at a release (Keep a
Changelog). Without it `changelog:*` refuses.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `title` | `this project` | "All notable changes to <title> …" in a new file's head |
| `versions` | ``its releases (`vX.Y.Z`)`` | what its versions are, in the same sentence |
| `area_dirs` | none | folders whose subfolders are areas (`packages`, `apps`) |
| `strip` | none | taken off a subfolder's name for its area |
| `areas` | `ci`, `repo` | areas besides those |
| `version_files` | none | the `package.json` files a release sets to its version |
| `[[changelog.libraries]]` | none | released on their own and named in the release after theirs: `name` (its area), `path`, `tag` (before the version), `with` (released along) |

## `[[releases]]`

The release tags the repo's live changelog (`changelog`) names.

| Key | What it does |
| --- | ------------ |
| `tag` | what comes before the version: `v`, `lib-v` |
| `name` | the release's name |
| `paths` | the folders it releases; the first holds its `CHANGELOG.md` |

## A full example

```toml
[repo]
default_branch = "main"
run = "pnpm"
name = "me/shop"

[[worktrees.setup]]
run = "pnpm install --frozen-lockfile"
says = "pnpm install"

[stack]
dir = "apps/web"
project = "shop"

[stack.app]
port = 3000

[[stack.services]]
name = "db"
label = "Postgres"
port = 5432
env = "DB_PORT"
address = "localhost:{port}"

[stack.migrations]
journal = "drizzle/meta/_journal.json"
run = "pnpm db:migrate"

[[stack.tools]]
name = "storybook"
dir = "packages/ui"
port = 6006
run = "pnpm storybook -p {port}"

[[mr.tests]]
paths = ["^apps/web/src/"]
line = "- [ ] clicked through in the browser"

[check]
turbo = true

[changelog]
title = "Shop"
area_dirs = ["apps", "packages"]
```

The monorepo breadboard is developed in has its own at its root.
