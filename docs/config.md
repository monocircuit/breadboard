# breadboard.toml

Everything breadboard knows about the repository it runs in comes from
`breadboard.toml` at the root of a checkout (#342). A worktree reads its own
file, so a branch that changes it uses its version; a worktree made before the
file existed reads the main checkout's. A repo without the file gets the
defaults below: worktrees, the overview and orchestrations work, with no stack.

The file is read again when it changes. One that doesn't match this schema is
refused with the key and why (`stack (db): port 3000 is app's already`), never
read in half: the defaults hold until it is fixed. Unknown keys are refused too,
so a typo doesn't pass for a default.

## The smallest file

```toml
[repo]
name = "me/shop"
```

Or none at all: then the default branch is origin's `HEAD`, the commands show
as `breadboard <command>`, and there is no stack.

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
| `space` | `[repo] name`, else the main checkout's folder | the label of the main checkout's space; a worktree's space is its feature, an orchestration's `⇶ <title>` |

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
there is no env file, no offset, no stack tabs, no `stack:*`.

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
| `dev` | `pnpm dev` | what starts it in `dir` (`--port <p>` follows for an own stack); the `dev` tab runs it while the stack is up |
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

## `[mr]`

| Key | Default | What it does |
| --- | ------- | ------------ |
| `template` | `.gitlab/merge_request_templates/Default.md` | what `wt:mr` puts down for a branch's first description |
| `[[mr.tests]]` | none | boxes of the template's Tests section the pre-fill decides on: `line` (the box, `- [ ] …`) goes in when the branch changes a file one of `paths` (regular expressions) matches, and out otherwise |

## `[orchestration]`

| Key | Default | What it does |
| --- | ------- | ------------ |
| `epic_branches` | `false` | a new orchestration makes an epic branch `epic/<n>-<name>` from the default branch: its tasks branch off it and merge into it by themselves (#350). `<n>` is the epic of its issues' milestone (`… Epic N: …`) or `--epic <n>`; without it an orchestration works on the default branch, unless `--epic` names one |

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

A deploy counts as done 90 s after its last job passed (the host pulls and
starts the image).

## `[check]`

What `wt:check` runs and the pre-push gate waits for.

| Key | Default | What it does |
| --- | ------- | ------------ |
| `turbo` | `false` | lint, typecheck and test of every changed package and its dependents through turbo (a pnpm monorepo) |
| `run` | none | else one command over the branch (`cargo test`, `npm test`); with neither there is nothing to check and every push passes |
| `[[check.stack_tests]]` | none | `package` and `says`: what `wt:check` says of a package whose tests need the stack instead of running them |

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
