//! `breadboard.toml` (#342): everything breadboard knows about the repo it
//! runs in, read from the root of the checkout it runs in, else of the main
//! checkout (a worktree made before the file came). Without the file the
//! defaults hold here, but no command runs (`refusal`, #320). The schema is
//! `docs/config.md`; a file that doesn't match it is refused with its line,
//! the key and why, never half read.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// The file, at the root of the main checkout.
pub const FILE: &str = "breadboard.toml";

/// The whole file.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub repo: Repo,
    pub herdr: Herdr,
    pub worktrees: Worktrees,
    /// The dev stack of a checkout; None: worktrees have none.
    pub stack: Option<Stack>,
    pub mr: Mr,
    pub orchestrator: Orchestrator,
    pub agents: Agents,
    pub tries: Tries,
    /// The cheap review of an MR before its merge is offered (#374).
    pub review: Review,
    pub check: Check,
    pub main_checkout: MainCheckout,
    /// The fragments folded into a changelog at a release; None: no `changelog:*`.
    pub changelog: Option<Changelog>,
    /// Release tags and the paths each one releases.
    pub releases: Vec<Release>,
    /// What breadboard watches of the project's CI (#379).
    pub ci: Ci,
    /// The products of a monorepo (#405): each one herdr space holding its
    /// orchestrations; by name.
    pub products: std::collections::BTreeMap<String, Product>,
    /// The team's defaults of the overview's settings panel (#415).
    pub settings: Settings,
    /// What a merge into the default branch needs (#443).
    pub merge: Merge,
    /// The desktop notifications of the repo's events (#449).
    pub notifications: Notifications,
    /// The daemon's web UI (#435).
    pub web: Web,
}

/// The web UI `breadboard serve` serves (`[web]`, #435): on 127.0.0.1
/// only, read-only unless `writes` says otherwise.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Web {
    /// Its port on 127.0.0.1; 0: no web UI. `BREADBOARD_WEB_PORT` wins (a second daemon, the gauntlet).
    pub port: u16,
    /// Whether the page may do what changes something (read GitLab again
    /// for every overview); off, it only reads.
    pub writes: bool,
    /// Names the page answers to besides 127.0.0.1 and localhost: a proxy's
    /// in front of it (`tailscale serve`: `<machine>.<tailnet>.ts.net`), with
    /// or without a port.
    pub hosts: Vec<String>,
}

/// The web UI's port by default: the issue that brought it.
pub const WEB_PORT: u16 = 7435;

impl Default for Web {
    fn default() -> Self {
        Web {
            port: WEB_PORT,
            writes: false,
            hosts: Vec::new(),
        }
    }
}

/// The team's default of the desktop notifications (`[notifications]`,
/// #449, core's `notify`): on or off, and which events; the settings
/// panel's local choice wins.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Notifications {
    /// Any desktop notification at all.
    pub desktop: bool,
    /// The events that send one, by key (`issue_created`, `mr_merged`, …): every one by default.
    pub events: Option<Vec<String>>,
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications {
            desktop: true,
            events: None,
        }
    }
}

/// What a merge into the default branch needs (`[merge]`, #443).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Merge {
    /// Its MR's own pipeline must pass. Off (temporary, until the batch
    /// branches carry the testing): `wt:check` passed at its pushed head is
    /// enough, the CI queue starts none, and the default branch's pipeline
    /// after the merge is the safety net.
    pub needs_pipeline: bool,
    /// A task MR into a batch (#461) must pass its own pipeline before the
    /// overview merges it. Off: `wt:check` passed at its pushed head is
    /// enough, the CI queue starts none, and the batch MR's full pipeline
    /// into the default branch is the test.
    pub batch_needs_pipeline: bool,
}

impl Default for Merge {
    fn default() -> Self {
        Merge {
            needs_pipeline: true,
            batch_needs_pipeline: true,
        }
    }
}

/// The team's defaults of the settings panel (#415, `settings.rs`): what
/// isn't given holds as built in. The panel's changes are local and win.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub batching: Option<bool>,
    pub auto_merge_into_batch: Option<bool>,
    pub ship_after_tasks: Option<i64>,
    pub ship_after_days: Option<i64>,
    pub try_checker: Option<bool>,
    pub auto_fresh: Option<bool>,
}

/// A product of the repo (`[products.vita]`, #405): its folders. An
/// orchestration of it works in one herdr space with the product's others.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Product {
    /// The folders that are the product's.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Ci {
    /// The CI job watcher's thresholds (#379, `[ci.watch]`).
    pub watch: CiWatch,
    /// How many runs of its agent may fix an MR's failed pipeline, one per
    /// head (#371): past them the user looks. 0: no fix run, the user looks at once.
    pub max_fix_attempts: u32,
}

impl Default for Ci {
    fn default() -> Self {
        Ci {
            watch: CiWatch::default(),
            max_fix_attempts: 3,
        }
    }
}

/// When the CI job watcher (#379) flags a job: the critical path of most
/// pipelines, a median that grows, a run far past its median (hung), a run
/// that mostly waits (idle).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CiWatch {
    /// Off: nothing read, nothing flagged.
    pub is_on: bool,
    /// How many of a job's newest pipelines the critical path and idle count over.
    pub window: usize,
    /// Runs a job needs before any flag of it.
    pub min_runs: usize,
    /// A job whose median is shorter holds nothing up: no critical path, no slowdown.
    pub min_seconds: f64,
    /// The last job of a pipeline is its critical path when the one before ended this much earlier.
    pub critical_margin_seconds: f64,
    /// The share of its pipelines a job must be the critical path of.
    pub critical_share: f64,
    /// How much its median of the last `slowdown_days` grows over the days before (0.3: +30 %).
    pub slowdown: f64,
    pub slowdown_days: i64,
    /// A running job past this many times its median is hung.
    pub hang_factor: f64,
    /// … and past this many seconds.
    pub hang_seconds: f64,
    /// … and its log hasn't grown for this many seconds: one that still writes is slow, not hung (#398).
    pub hang_quiet_seconds: f64,
    /// A job that runs this long on fewer cores than `idle_cores` mostly waits.
    pub idle_seconds: f64,
    pub idle_cores: f64,
    /// How long the history keeps a run.
    pub history_days: i64,
}

impl Default for CiWatch {
    fn default() -> Self {
        CiWatch {
            is_on: true,
            window: 10,
            min_runs: 5,
            min_seconds: 120.0,
            critical_margin_seconds: 60.0,
            critical_share: 0.5,
            slowdown: 0.3,
            slowdown_days: 7,
            hang_factor: 2.0,
            hang_seconds: 120.0,
            hang_quiet_seconds: 60.0,
            idle_seconds: 300.0,
            idle_cores: 0.5,
            history_days: 14,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Repo {
    /// The branch work starts from and merges into; None: origin's HEAD, else `main`.
    pub default_branch: Option<String>,
    /// How a command is shown to type: `<run> wt:new` (`pnpm` where the
    /// root package.json has the scripts); None: `breadboard`.
    pub run: Option<String>,
    /// Shown where the repo is named (briefs, errors); None: the GitLab project from origin.
    pub name: Option<String>,
}

/// How the repo shows in herdr (#342: per repo, the answer to decide #1).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Herdr {
    /// The plugin's id: its manifest's, every sidebar token's source, the
    /// block marker in herdr's configs; None: `breadboard.<repo>`.
    pub plugin: Option<String>,
    /// The label of the main checkout's space; None: the repo's name.
    pub space: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Worktrees {
    /// What a new worktree runs after `git worktree add`, in order (`pnpm install`).
    pub setup: Vec<Step>,
}

/// A command with what it does, as the step line says it.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Step {
    pub run: String,
    /// The step line; None: the command itself.
    pub says: Option<String>,
    /// Its output stays hidden unless it fails.
    pub is_quiet: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stack {
    /// The folder of the checkout the stack's files and commands are in.
    #[serde(default = "dot")]
    pub dir: String,
    /// The env file of the stack in `dir`: copied from the main checkout, ports moved.
    #[serde(default = "env_local")]
    pub env_file: String,
    /// docker compose's file in `dir`.
    #[serde(default = "compose_yml")]
    pub compose_file: String,
    /// The compose project of the main checkout; a worktree's own is `<project>-<feature>`.
    pub project: String,
    /// What sets an own stack up (containers, migrations, seed); None: `docker compose up -d`.
    pub setup: Option<Step>,
    /// `stack:up|down|reset`; None: compose's `up -d`, `down`, `down -v` then `up -d`.
    pub up: Option<String>,
    pub down: Option<String>,
    pub reset: Option<String>,
    pub app: App,
    /// The ports besides the app's, each moved by a worktree's offset.
    #[serde(default)]
    pub services: Vec<Service>,
    pub migrations: Option<Migrations>,
    /// What a worktree may run beside its stack, on a port moved with it (Storybook).
    #[serde(default)]
    pub tools: Vec<Tool>,
    /// The batch stack (`[stack.batch]`, #411; the epic stack of #351, #365) and the task stacks on it;
    /// None: an orchestration's worktrees get own stacks.
    #[serde(rename = "batch")]
    pub epic: Option<Epic>,
}

/// The epic stack of an orchestration (#351): its checkout runs a full
/// stack whose database service keeps a template database, migrated and
/// seeded; a task worktree's stack runs without that service, on the epic's
/// network, its databases made on the epic's.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Epic {
    /// The `[[stack.services]]` the task stacks share: its container answers psql.
    #[serde(default = "postgres")]
    pub service: String,
    /// psql's user in it.
    #[serde(default = "postgres")]
    pub superuser: String,
    /// The template database the cloned database of a task is made from.
    pub template: String,
    /// The folder in the stack's `dir` whose git tree a template is made from:
    /// the template is made again when it changed.
    pub stamp: String,
    /// Run in a new template before its migrations (`[stack.migrations] run`).
    #[serde(default)]
    pub template_sql: Vec<String>,
    /// What seeds the template after its migrations, in the stack's `dir`.
    pub seed: Option<String>,
    /// What a task stack runs once its containers are up and its migrations applied, in order.
    #[serde(default)]
    pub task_setup: Vec<String>,
    /// The databases a task gets on the epic's service, `<name>_<feature>`.
    #[serde(default)]
    pub databases: Vec<Database>,
    /// What a task stack's env file sets besides, `{network}` the epic's compose network.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// A database a task gets on the epic's service.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Database {
    /// Its name before `_<feature>`.
    pub name: String,
    /// Cloned from the template (one database is); else made empty.
    #[serde(default)]
    pub is_clone: bool,
    /// The role that owns it (and the template, for the cloned one).
    pub owner: String,
    /// The roles that may connect to it besides its owner.
    #[serde(default)]
    pub connect: Vec<String>,
    /// The variables of the env file whose URL points at it.
    #[serde(default)]
    pub urls: Vec<String>,
    /// The variable of the env file set to its name.
    pub name_env: Option<String>,
}

impl Epic {
    /// The database cloned from the template.
    pub fn clone_database(&self) -> &Database {
        self.databases
            .iter()
            .find(|database| database.is_clone)
            .expect("validated: one database is cloned")
    }
}

/// The app a checkout serves: its port, how it starts.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub port: u16,
    /// Run in the stack's `dir`; `--port <p>` follows for an own stack.
    #[serde(default = "pnpm_dev")]
    pub dev: String,
    /// The variable of the env file it is moved with, None: none.
    pub env: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Service {
    /// The container's service name (`postgres`): its herdr tab, its row.
    pub name: String,
    /// As a line names it (`Postgres`); None: the name.
    pub label: Option<String>,
    pub port: u16,
    /// The variable of the env file the port is moved with.
    pub env: Option<String>,
    /// Where it answers, `{port}` its moved port (`http://localhost:{port}/admin`): a link in WHERE.
    pub url: Option<String>,
    /// Or where to connect to it (`localhost:{port}`). With neither it has no row.
    pub address: Option<String>,
}

/// The migrations a worktree's database may lack: drizzle's journal against
/// its table, asked with psql.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Migrations {
    /// drizzle's `meta/_journal.json` in the stack's `dir`.
    pub journal: String,
    /// What applies them, in the stack's `dir`.
    pub run: String,
    /// The variable of the env file with the database's URL.
    #[serde(default = "database_url")]
    pub database_env: String,
}

/// Something a worktree runs beside its stack on a port moved with it
/// (a database studio, Storybook): a row in its WHERE, a herdr tab.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    /// Its tab and row.
    pub name: String,
    /// The folder it runs in; None: the stack's.
    pub dir: Option<String>,
    pub port: u16,
    /// What starts it, `{port}` its moved port.
    pub run: String,
    /// Where it is looked at, `{port}`; None: `http://localhost:{port}`.
    pub url: Option<String>,
    /// Its tab opens while the stack runs, as the app's `dev` does; else a click starts it.
    #[serde(default)]
    pub is_with_stack: bool,
}

impl Tool {
    /// `run` with the port of `offset`.
    pub fn command(&self, offset: u16) -> String {
        self.run.replace("{port}", &(self.port + offset).to_string())
    }
    pub fn url(&self, offset: u16) -> String {
        self.url
            .as_deref()
            .unwrap_or("http://localhost:{port}")
            .replace("{port}", &(self.port + offset).to_string())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Mr {
    /// The template a description starts from.
    pub template: String,
    /// Boxes the Tests section gets when the branch changes one of their paths.
    pub tests: Vec<TestBox>,
}

impl Default for Mr {
    fn default() -> Self {
        Mr {
            template: ".gitlab/merge_request_templates/Default.md".into(),
            tests: Vec::new(),
        }
    }
}

/// The orchestrator's chat (#382): when it gets a fresh start.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Orchestrator {
    /// From which share of its context (percent) the overview offers a fresh start at a quiet point.
    pub fresh_above: u32,
    /// Whether the overview makes the fresh start by itself, without a click.
    pub auto_fresh: bool,
}

impl Default for Orchestrator {
    fn default() -> Self {
        Orchestrator {
            fresh_above: 50,
            auto_fresh: false,
        }
    }
}

/// An orchestration's agents: what they may spend (#372), in tokens (`"20 M"`)
/// or dollars (`"$40"`), and when one starts fresh (#381).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Agents {
    /// What the repo's agents may spend a day: past it the run engine starts
    /// no run until midnight or a `continue anyway`; None: no limit.
    pub daily_limit: Option<crate::spend::Limit>,
    /// What an orchestration's agents may spend in all, shown against its
    /// spend in its band; None: none shown.
    pub orchestration_budget: Option<crate::spend::Limit>,
    /// The context, in tokens, above which an agent retires at its merge
    /// instead of taking a next issue: its next one goes to a fresh agent
    /// with its written handoff (#381, #445).
    pub fresh_above: u64,
    /// The context above which it retires once idle longer than
    /// `idle_minutes`: its prompt cache is gone, a resume writes it all again.
    pub idle_fresh_above: u64,
    /// How long idle that is: a little under the cache's hour.
    pub idle_minutes: u32,
    /// The MCP configs (files, relative to the main checkout) a headless
    /// run loads (#445); none by default: every server's tools are in
    /// each call's context.
    pub mcp_configs: Vec<String>,
    /// The model of each kind of work (#373, `stage.rs`).
    pub models: Models,
    /// The labels that make an issue's agent a `docs` one (#373).
    pub docs_labels: Vec<String>,
    /// How many runs may be in flight at once (#445): after a crash every
    /// cut-off agent is picked up, a few at a time, never all together.
    pub max_runs: usize,
    /// What the repo's agents must never touch (#375), on top of Claude
    /// Code's own deny rules.
    pub deny: Deny,
}

/// `[agents.deny]` (#375): `breadboard init` renders each path as an
/// `Edit(<path>)` and each command as a `Bash(<command>)` deny rule of
/// `.claude/settings.json`, and the pre-push gate refuses a push that
/// changes or deletes a denied file its base has (a new one passes: made by
/// a tool, since agents can't edit it).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Deny {
    /// Gitignore-style globs from the repo's top (`**/migrations/*.sql`).
    pub paths: Vec<String>,
    /// Claude Code Bash rule patterns (`git tag:*`).
    pub commands: Vec<String>,
}

/// `[agents.models]` (#373): a model per stage, `default` for the ones
/// without; none at all leaves claude's own default.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Models {
    pub default: Option<String>,
    pub build: Option<String>,
    pub docs: Option<String>,
    /// The try checker's; None: `[tries] checker`.
    pub review: Option<String>,
    pub ci_fix: Option<String>,
    pub orchestrator: Option<String>,
}

impl Default for Agents {
    fn default() -> Self {
        Agents {
            daily_limit: None,
            orchestration_budget: None,
            fresh_above: 150_000,
            idle_fresh_above: 100_000,
            idle_minutes: 55,
            mcp_configs: Vec::new(),
            models: Models::default(),
            docs_labels: vec!["documentation".into()],
            max_runs: 4,
            deny: Deny::default(),
        }
    }
}

/// A `try --on test`: the test instance a merge deploys to, and the jobs of
/// the default branch's pipeline that deploy it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Tries {
    /// What the test instance is called where the skill names it (`dev.example`); None: `the test instance`.
    pub test_instance: Option<String>,
    /// The variable (the environment's, else the main checkout's env file) with the test instance's origin.
    pub test_origin_env: String,
    /// The jobs that deploy; the pipeline's others don't hold a try.
    pub deploy_jobs: Vec<String>,
    /// The last of them; None: the pipeline passing is the deploy.
    pub deploy_job: Option<String>,
    /// The last step where a pipeline has no `deploy_job`.
    pub publish_job: Option<String>,
    /// The model that checks and answers a try while its orchestration's
    /// auto mode is on (#384): `claude -p --model <it>`.
    pub checker: String,
}

/// The review run of an MR (#374): once its bar is full and its pipeline
/// green, one `claude -p` on a small model reads its diff against its issue
/// and AGENTS.md, once per head. Off until its cost per review is measured.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Review {
    pub enabled: bool,
    /// The model it runs on: `claude -p --model <it>`.
    pub model: String,
    /// A diff of fewer changed lines isn't worth a review.
    pub min_lines: u64,
    /// One of more isn't cheap to review: the user reads it.
    pub max_lines: u64,
    /// What one review may cost at most (`--max-budget-usd`).
    pub max_dollars: f64,
    /// Pathspecs left out of the diff and its count (lockfiles, generated files).
    pub exclude: Vec<String>,
}

impl Default for Review {
    fn default() -> Self {
        Review {
            enabled: false,
            model: "haiku".into(),
            min_lines: 20,
            max_lines: 3000,
            max_dollars: 0.5,
            exclude: ["pnpm-lock.yaml", "**/Cargo.lock", "**/migrations/meta/**"]
                .map(String::from)
                .to_vec(),
        }
    }
}

/// The main checkout keeps itself on the default branch (#348): the daemon
/// of the overviews fetches and fast-forwards it when that is safe.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MainCheckout {
    /// Off: it stays where it is until someone pulls.
    pub follow: bool,
    /// How often it looks when no merge woke it, in minutes.
    pub every_minutes: u64,
    /// What runs in it after an update that changed a file of `install_when`.
    pub install: Option<String>,
    pub install_when: Vec<String>,
}

impl Default for MainCheckout {
    fn default() -> Self {
        MainCheckout {
            follow: true,
            every_minutes: 2,
            install: None,
            install_when: Vec::new(),
        }
    }
}

impl Default for Tries {
    fn default() -> Self {
        Tries {
            test_instance: None,
            test_origin_env: "TEST_ORIGIN".into(),
            deploy_jobs: Vec::new(),
            deploy_job: None,
            publish_job: None,
            checker: "haiku".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TestBox {
    /// Regular expressions on the changed paths.
    pub paths: Vec<String>,
    /// The box's line, `- [ ] …`.
    pub line: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Check {
    /// wt:check runs lint, typecheck and test of the changed packages and
    /// their dependents through turbo (a pnpm monorepo).
    pub turbo: bool,
    /// Else one command checks the branch (`cargo test`, `npm test`); with
    /// neither there is nothing to check and every push passes the gate.
    pub run: Option<String>,
    /// Packages whose tests need the stack: what the check says instead of running them.
    pub stack_tests: Vec<StackTests>,
}

impl Check {
    /// What the pre-push gate waits for without turbo: one check of the
    /// whole branch, or nothing.
    pub fn plain(&self) -> Option<&str> {
        self.run.as_deref().filter(|run| !self.turbo && !run.trim().is_empty())
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct StackTests {
    pub package: String,
    pub says: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Changelog {
    /// Who the changelog is of: "All notable changes to <title> …".
    pub title: String,
    /// What its versions are, in its head's sentence.
    pub versions: String,
    /// Folders whose subfolders are areas, `strip` taken off their names.
    pub area_dirs: Vec<String>,
    pub strip: Option<String>,
    /// Areas besides those.
    pub areas: Vec<String>,
    /// The package.json files a release sets to its version.
    pub version_files: Vec<String>,
    /// Libraries released on their own, named in the release that follows theirs.
    pub libraries: Vec<Library>,
}

impl Default for Changelog {
    fn default() -> Self {
        Changelog {
            title: "this project".into(),
            versions: "its releases (`vX.Y.Z`)".into(),
            area_dirs: Vec::new(),
            strip: None,
            areas: vec!["ci".into(), "repo".into()],
            version_files: Vec::new(),
            libraries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Library {
    /// Its area.
    pub name: String,
    /// Its folder, with its package.json and CHANGELOG.md.
    pub path: String,
    /// Its tag before the version: `lib-v`.
    pub tag: String,
    /// Released together with it, as the entry says: `with utilities`.
    pub with: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Release {
    /// What the tag is before the version: `v`, `lib-v`.
    pub tag: String,
    /// The name the overview shows it under.
    pub name: String,
    /// The folders it releases.
    pub paths: Vec<String>,
}

fn dot() -> String {
    ".".into()
}
fn env_local() -> String {
    ".env.local".into()
}
fn compose_yml() -> String {
    "docker-compose.yml".into()
}
fn pnpm_dev() -> String {
    "pnpm dev".into()
}
fn postgres() -> String {
    "postgres".into()
}
fn database_url() -> String {
    "DATABASE_URL".into()
}

/// One port of a stack: the app's or a service's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub port: u16,
    pub env: Option<String>,
}

impl Stack {
    /// Every port, the app's, the services' and the tools', by number.
    pub fn ports(&self) -> Vec<Port> {
        let mut all = vec![Port {
            name: "app".into(),
            port: self.app.port,
            env: self.app.env.clone(),
        }];
        all.extend(self.services.iter().map(|service| Port {
            name: service.name.clone(),
            port: service.port,
            env: service.env.clone(),
        }));
        all.extend(self.tools.iter().map(|tool| Port {
            name: tool.name.clone(),
            port: tool.port,
            env: None,
        }));
        all.sort_by_key(|port| port.port);
        all
    }

    /// The port a worktree's offset is read back from: the lowest with a variable.
    pub fn offset_port(&self) -> Option<Port> {
        self.ports().into_iter().find(|port| port.env.is_some())
    }

    /// A service by its name.
    pub fn service(&self, name: &str) -> Option<&Service> {
        self.services.iter().find(|service| service.name == name)
    }

    /// A tool by its name.
    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}

impl Service {
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
    /// `url` with the port of `offset`.
    pub fn url(&self, offset: u16) -> Option<String> {
        let port = (self.port + offset).to_string();
        self.url.as_ref().map(|url| url.replace("{port}", &port))
    }
    pub fn address(&self, offset: u16) -> Option<String> {
        let port = (self.port + offset).to_string();
        self.address.as_ref().map(|address| address.replace("{port}", &port))
    }
}

impl Config {
    /// Parses and checks a file's text.
    pub fn parse(text: &str) -> Result<Config, String> {
        Self::located(text).map_err(|(_, error)| error)
    }

    /// `parse`, its error with the line it is on (#320): `key: why`, the key
    /// as the file spells it (`agents.max_runs`, `stack.services[1].name`).
    fn located(text: &str) -> Result<Config, (Option<usize>, String)> {
        let config: Config = toml::from_str(text).map_err(|error| {
            let line = error
                .span()
                .map(|span| text[..span.start.min(text.len())].matches('\n').count() + 1);
            let why = error.message().trim_end();
            match line.and_then(|line| key_at(text, line)) {
                Some(key) => (line, format!("{key}: {why}")),
                None => (line, why.to_string()),
            }
        })?;
        config.validate().map_err(|error| {
            let key = error.split([':', ' ']).next().unwrap_or_default();
            (line_of(text, key), error)
        })?;
        Ok(config)
    }

    /// What serde can't say: names and ports that must be there and apart.
    fn validate(&self) -> Result<(), String> {
        let bad = |key: &str, why: &str| Err(format!("{key}: {why}"));
        let agents = &self.agents;
        if agents.fresh_above < 10_000 || agents.idle_fresh_above < 10_000 {
            return bad(
                "agents.fresh_above",
                "fresh_above and idle_fresh_above are tokens of context now (#445), at least 10_000, e.g. 150_000",
            );
        }
        if agents.max_runs == 0 {
            return bad("agents.max_runs", "must be at least 1");
        }
        let deny = &agents.deny;
        if deny
            .paths
            .iter()
            .chain(&deny.commands)
            .any(|rule| rule.trim().is_empty() || rule.contains(')'))
        {
            return bad("agents.deny", "a path or command is empty or holds a `)`");
        }
        if agents.idle_minutes == 0 {
            return bad("agents.idle_minutes", "must be a number of minutes, e.g. 55");
        }
        if let Some(branch) = &self.repo.default_branch
            && (branch.trim().is_empty() || branch.contains(char::is_whitespace))
        {
            return bad("repo.default_branch", "must be a branch name");
        }
        if let Some(stack) = &self.stack {
            if stack.project.trim().is_empty()
                || !stack
                    .project
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return bad(
                    "stack.project",
                    "must be a compose project name (letters, digits, - and _)",
                );
            }
            if stack.offset_port().is_none() {
                return bad(
                    "stack",
                    "no port has an `env`: a worktree's own stack is told apart by the moved port of one (`env = \"POSTGRES_PORT\"` on a service, or on the app)",
                );
            }
            let mut seen: Vec<(u16, String)> = Vec::new();
            for port in stack.ports() {
                if let Some((_, other)) = seen.iter().find(|(taken, _)| *taken == port.port) {
                    return bad(
                        &format!("stack ({})", port.name),
                        &format!("port {} is {other}'s already", port.port),
                    );
                }
                seen.push((port.port, port.name.clone()));
            }
            for (at, service) in stack.services.iter().enumerate() {
                if service.name.trim().is_empty() {
                    return bad(&format!("stack.services[{at}].name"), "must not be empty");
                }
            }
            if let Some(epic) = &stack.epic {
                epic.validate(stack)?;
            }
        }
        if self.main_checkout.every_minutes == 0 {
            return bad("main_checkout.every_minutes", "must be 1 or more");
        }
        if !self.main_checkout.install_when.is_empty() && self.main_checkout.install.is_none() {
            return bad("main_checkout.install_when", "needs an `install` to run");
        }
        for (key, value, max) in [
            ("ship_after_tasks", self.settings.ship_after_tasks, 99),
            ("ship_after_days", self.settings.ship_after_days, 30),
        ] {
            if value.is_some_and(|value| !(1..=max).contains(&value)) {
                return bad(&format!("settings.{key}"), &format!("must be 1 to {max}"));
            }
        }
        for event in self.notifications.events.iter().flatten() {
            if crate::notify::event_of(event).is_none() {
                let keys: Vec<&str> = crate::notify::EVENTS.iter().map(|event| event.key).collect();
                return bad(
                    "notifications.events",
                    &format!("no event {event}: one of {}", keys.join(", ")),
                );
            }
        }
        for (name, product) in &self.products {
            if !is_product_name(name) {
                return bad(
                    &format!("products.{name}"),
                    "must be a name of letters, digits, - and _ (it labels a herdr space)",
                );
            }
            if product.paths.is_empty() {
                return bad(&format!("products.{name}.paths"), "must name at least one folder");
            }
        }
        for (at, test) in self.mr.tests.iter().enumerate() {
            for path in &test.paths {
                // Parsed, not compiled: a compile of each was a sixth of every start's work (#342).
                if let Err(error) = regex_syntax::Parser::new().parse(path) {
                    return bad(
                        &format!("mr.tests[{at}].paths"),
                        &format!("{path:?} is no regular expression ({error})"),
                    );
                }
            }
        }
        Ok(())
    }

    /// The product an issue's milestone names (`vita · Epic 12: …` → vita),
    /// when the file has it (#405).
    pub fn product_of_milestone(&self, milestone: &str) -> Option<String> {
        let (name, _) = milestone.split_once(" · ")?;
        let name = name.trim();
        self.products.contains_key(name).then(|| name.to_string())
    }

    /// The branch named, else None (the caller asks origin).
    pub fn branch(&self) -> Option<&str> {
        self.repo.default_branch.as_deref()
    }

    /// How a command is typed: `pnpm wt:new`, `breadboard wt:new`.
    pub fn command(&self, command: &str) -> String {
        format!("{} {command}", self.repo.run.as_deref().unwrap_or("breadboard"))
    }
}

/// A product's name labels its herdr space: letters, digits, - and _.
fn is_product_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Epic {
    fn validate(&self, stack: &Stack) -> Result<(), String> {
        let bad = |key: &str, why: &str| Err(format!("stack.batch.{key}: {why}"));
        if stack.service(&self.service).is_none_or(|service| service.env.is_none()) {
            return bad(
                "service",
                &format!(
                    "{:?} is no [[stack.services]] with an `env`: the task stacks reach it on the epic's moved port",
                    self.service
                ),
            );
        }
        if stack.migrations.is_none() {
            return Err("stack.batch: needs [stack.migrations], its `run` migrates the template".into());
        }
        if self.template.trim().is_empty() {
            return bad("template", "must not be empty");
        }
        if self.stamp.trim().is_empty() {
            return bad("stamp", "must name a folder");
        }
        if self.databases.iter().filter(|database| database.is_clone).count() != 1 {
            return bad(
                "databases",
                "exactly one has `is_clone = true`: the one made from the template",
            );
        }
        for (at, database) in self.databases.iter().enumerate() {
            if database.name.trim().is_empty() || database.owner.trim().is_empty() {
                return bad(&format!("databases[{at}]"), "needs a `name` and an `owner`");
            }
        }
        Ok(())
    }
}

/// How a command is typed in the repo of the working directory: `pnpm`, `breadboard`.
pub fn run_word() -> String {
    here().config.repo.run.clone().unwrap_or_else(|| "breadboard".into())
}

/// What a checkout without a stack lacks, as a cell says it: `no .env.local`
/// (`[stack] env_file`), or `no stack` where breadboard.toml has no `[stack]`.
pub fn env_missing() -> String {
    match &here().config.stack {
        Some(stack) => format!("no {}", stack.env_file),
        None => "no stack".into(),
    }
}

/// Whether the repo of the working directory has a stack (`[stack]`).
pub fn has_stack() -> bool {
    here().config.stack.is_some()
}

/// `text` in the words of the repo of the working directory: its commands
/// as it types them (`pnpm wt:new` → `breadboard wt:new` where breadboard.toml
/// names no `[repo] run`; `bb submit` → `breadboard submit` there too, `bb`
/// is the monorepo's shim, #427) and its default branch for `develop`. For what is
/// written once in the words of the repo breadboard is developed in: the
/// usage and help texts, the lines said.
pub fn in_repo(text: &str) -> String {
    static COMMAND: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"\bpnpm ((?:wt|stack|changelog|herdr):|overview\b|herdr\b|commands\b|changelog\b|browser\b|init\b)",
        )
        .unwrap()
    });
    static BB: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\bbb (status|submit)\b").unwrap());
    static DEVELOP: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\bdevelop\b").unwrap());
    let loaded = here();
    let run = loaded.config.repo.run.as_deref().unwrap_or("breadboard");
    let mut text = std::borrow::Cow::Borrowed(text);
    if run != "pnpm" {
        text = std::borrow::Cow::Owned(COMMAND.replace_all(&text, format!("{run} $1").as_str()).into_owned());
        text = std::borrow::Cow::Owned(BB.replace_all(&text, "breadboard $1").into_owned());
    }
    if loaded.branch != "develop" {
        text = std::borrow::Cow::Owned(DEVELOP.replace_all(&text, loaded.branch.as_str()).into_owned());
    }
    text.into_owned()
}

/// A repo's config with the branch resolved.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    /// The main checkout it was read from.
    pub main: PathBuf,
    /// `develop`, `main`.
    pub branch: String,
    /// Why the file was refused; the defaults hold meanwhile.
    pub error: Option<String>,
}

impl Loaded {
    /// How an agent of this repo types `bb status` / `bb submit`: `bb`, the
    /// monorepo's shim (#427), where breadboard runs through pnpm, else
    /// `breadboard` (as `in_repo` says it).
    pub fn bb(&self) -> &'static str {
        if self.config.repo.run.as_deref() == Some("pnpm") {
            "bb"
        } else {
            "breadboard"
        }
    }

    /// `origin/<branch>`.
    pub fn base(&self) -> String {
        format!("origin/{}", self.branch)
    }

    /// The stack's folder in a checkout.
    pub fn stack_dir(&self, checkout: &Path) -> Option<PathBuf> {
        let stack = self.config.stack.as_ref()?;
        Some(checkout.join(&stack.dir))
    }

    /// The stack's env file in a checkout, None without a stack.
    pub fn env_file(&self, checkout: &Path) -> Option<PathBuf> {
        let stack = self.config.stack.as_ref()?;
        Some(checkout.join(&stack.dir).join(&stack.env_file))
    }

    pub fn command(&self, command: &str) -> String {
        self.config.command(command)
    }

    /// The folder name of the main checkout: `platform`.
    pub fn folder(&self) -> String {
        self.main
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".into())
    }

    /// The herdr plugin's id: `[herdr] plugin`, else `breadboard.<main checkout's folder>`.
    pub fn plugin(&self) -> String {
        self.config.herdr.plugin.clone().unwrap_or_else(|| {
            let slug: String = self
                .folder()
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect();
            format!("breadboard.{slug}")
        })
    }

    /// The label of the main checkout's herdr space: `[herdr] space`, else
    /// `[repo] name`, else the main checkout's folder.
    pub fn space(&self) -> String {
        self.config
            .herdr
            .space
            .clone()
            .or_else(|| self.config.repo.name.clone())
            .unwrap_or_else(|| self.folder())
    }

    /// The name the repo goes by.
    pub fn name(&self) -> String {
        self.config.repo.name.clone().unwrap_or_else(|| {
            let origin = crate::git::git(&self.main, &["remote", "get-url", "origin"]);
            crate::gitlab::client::remote_of(origin.out())
                .filter(|_| origin.is_ok)
                .map(|(_, project)| project)
                .unwrap_or_else(|| {
                    self.main
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "the repo".into())
                })
        })
    }
}

/// Each line's key as `validate` names it: the table of a header line
/// (`stack.services[1]` for the second `[[stack.services]]`), its key on a
/// `key = …` line; None for a blank or comment line.
fn keys_of(text: &str) -> Vec<Option<String>> {
    let mut table = String::new();
    let mut arrays: HashMap<String, usize> = HashMap::new();
    let clean = |name: &str| {
        name.split('.')
            .map(|part| part.trim().trim_matches('"'))
            .collect::<Vec<_>>()
            .join(".")
    };
    text.lines()
        .map(|line| {
            let line = line.trim();
            if let Some(name) = line
                .strip_prefix("[[")
                .and_then(|rest| rest.split_once("]]"))
                .map(|(name, _)| clean(name))
            {
                let at = arrays.entry(name.clone()).or_insert(0);
                table = format!("{name}[{at}]");
                *at += 1;
                return Some(table.clone());
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|rest| rest.split_once(']'))
                .map(|(name, _)| clean(name))
            {
                // A table under an array of tables belongs to its last entry.
                table = arrays
                    .iter()
                    .filter(|(array, _)| name.starts_with(&format!("{array}.")))
                    .max_by_key(|(array, _)| array.len())
                    .map(|(array, at)| format!("{array}[{}]{}", at - 1, &name[array.len()..]))
                    .unwrap_or(name);
                return Some(table.clone());
            }
            let (key, _) = line.split_once('=').filter(|_| !line.starts_with('#'))?;
            let key = clean(key);
            Some(if table.is_empty() {
                key
            } else {
                format!("{table}.{key}")
            })
        })
        .collect()
}

/// The line (from 1) of `key`: its own line, else its table's, else that of
/// the table it is in (`stack (db)` → `[stack]`).
fn line_of(text: &str, key: &str) -> Option<usize> {
    let keys = keys_of(text);
    let mut key = key.to_string();
    loop {
        if let Some(at) = keys.iter().position(|line| line.as_deref() == Some(&key)) {
            return Some(at + 1);
        }
        let cut = key.rfind(['.', '['])?;
        key.truncate(cut);
    }
}

/// The key of line `line` (from 1).
fn key_at(text: &str, line: usize) -> Option<String> {
    keys_of(text).into_iter().nth(line.checked_sub(1)?).flatten()
}

/// `Config::parse` of a text, once per text: every worktree has its copy of
/// the file, mostly the same, and a parse of each was a share of every
/// overview's start (#342).
fn parsed(text: &str) -> Result<Config, (Option<usize>, String)> {
    type Parsed = Result<Config, (Option<usize>, String)>;
    static PARSED: LazyLock<Mutex<HashMap<String, Parsed>>> = LazyLock::new(Mutex::default);
    let mut known = PARSED.lock().unwrap_or_else(|e| e.into_inner());
    known
        .entry(text.to_string())
        .or_insert_with(|| Config::located(text))
        .clone()
}

/// Reads `<main>/breadboard.toml`, the defaults without one.
pub fn load(main: &Path) -> Loaded {
    let file = main.join(FILE);
    let (config, error) = match std::fs::read_to_string(&file) {
        Ok(text) => match parsed(&text) {
            Ok(config) => (config, None),
            Err((Some(line), error)) => (
                Config::default(),
                Some(format!("{} line {line}: {error}", file.display())),
            ),
            Err((None, error)) => (Config::default(), Some(format!("{}: {error}", file.display()))),
        },
        Err(_) => (Config::default(), None),
    };
    let branch = config
        .branch()
        .map(String::from)
        .unwrap_or_else(|| origin_head(main).unwrap_or_else(|| "main".into()));
    Loaded {
        config,
        main: main.to_path_buf(),
        branch,
        error,
    }
}

/// The branch origin's HEAD points at, from the refs git has.
fn origin_head(main: &Path) -> Option<String> {
    let out = crate::git::git(
        main,
        &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"],
    );
    out.out()
        .strip_prefix("origin/")
        .filter(|branch| out.is_ok && !branch.is_empty())
        .map(String::from)
}

/// How long a file's answer holds before it is looked at again: the readers
/// ask for the config of every worktree many times a frame, a stat each was
/// a sixth of the overview's first frame (#342). A changed file shows within it.
const LOOK_AGAIN: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(1)
};

/// The configs read, by the checkout they are in: the file's stamp then, when it was looked at.
type Cache = HashMap<PathBuf, (Option<Stamp>, Instant, Arc<Loaded>)>;
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// A directory asked before: its checkout, the main checkout, whether the
/// checkout has a file of its own and when that was looked at.
type Mains = HashMap<PathBuf, (PathBuf, PathBuf, bool, Instant)>;
static MAINS: LazyLock<Mutex<Mains>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// A file's time and size: a write that truncates and fills it within one
/// tick of the file system's clock keeps its time, and a read in between
/// (git rewriting it on a pull or merge) would hold an empty config (#447).
type Stamp = (SystemTime, u64);

fn stamp(main: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(main.join(FILE)).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The config of the main checkout `main`, read again once its file changed.
pub fn of_main(main: &Path) -> Arc<Loaded> {
    of_root(main, main)
}

/// The config in the checkout `root` of the repo whose main checkout is `main`.
fn of_root(root: &Path, main: &Path) -> Arc<Loaded> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, at, loaded)) = cache.get(root)
        && at.elapsed() < LOOK_AGAIN
    {
        return Arc::clone(loaded);
    }
    let time = stamp(root);
    if let Some((was, at, loaded)) = cache.get_mut(root)
        && *was == time
    {
        *at = Instant::now();
        return Arc::clone(loaded);
    }
    let mut loaded = load(root);
    loaded.main = main.to_path_buf();
    let loaded = Arc::new(loaded);
    cache.insert(root.to_path_buf(), (time, Instant::now(), Arc::clone(&loaded)));
    loaded
}

/// The config of the checkout `dir` lies in (any checkout or a folder in
/// one): its own breadboard.toml, a branch that changes it uses its own;
/// else the main checkout's. Outside any repo the defaults, as if `dir` were
/// a main checkout.
pub fn of(dir: &Path) -> Arc<Loaded> {
    let known = MAINS.lock().unwrap_or_else(|e| e.into_inner()).get(dir).cloned();
    let (top, main, is_own) = match known {
        Some((top, main, is_own, at)) if at.elapsed() < LOOK_AGAIN => (top, main, is_own),
        known => {
            let (top, main) = match known {
                Some((top, main, ..)) => (top, main),
                None => (
                    dir.ancestors()
                        .find(|at| at.join(".git").exists())
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| dir.to_path_buf()),
                    main_of(dir).unwrap_or_else(|| dir.to_path_buf()),
                ),
            };
            let is_own = top != main && top.join(FILE).is_file();
            MAINS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(dir.to_path_buf(), (top.clone(), main.clone(), is_own, Instant::now()));
            (top, main, is_own)
        }
    };
    if is_own {
        return of_root(&top, &main);
    }
    of_main(&main)
}

/// The repo a process acts on from outside any checkout (`act_on`).
static ACTED_ON: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Says which repo this process acts on when its working directory is in
/// none: `breadboard serve` stands in `/` (#380: `here` read the defaults
/// there, `main` for the default branch). The first one holds.
pub fn act_on(main: &Path) {
    let _ = ACTED_ON.set(main.to_path_buf());
}

/// The config of the repo of the working directory; outside any repo, of
/// the one `act_on` named. What acts on a repo it knows reads `of(main)`.
pub fn here() -> Arc<Loaded> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match ACTED_ON.get() {
        Some(main) if !cwd.ancestors().any(|at| at.join(".git").exists()) => of_main(main),
        _ => of(&cwd),
    }
}

/// Why no command runs in the repo of `dir` (#320): it has no
/// breadboard.toml (the message holds a minimal one with the repo's names), or
/// one that is refused (its key and line). None outside a repo.
pub fn refusal(dir: &Path) -> Option<String> {
    let top = dir.ancestors().find(|at| at.join(".git").exists())?;
    let loaded = of(dir);
    if let Some(error) = &loaded.error {
        return Some(error.clone());
    }
    if top.join(FILE).is_file() || loaded.main.join(FILE).is_file() {
        return None;
    }
    Some(format!(
        "no {FILE} in {}: breadboard reads what it knows about the repo from it. A minimal one:\n\n[repo]\nname = \"{}\"\ndefault_branch = \"{}\"\n\nbreadboard init writes it; every key and its default: docs/config.md of breadboard.",
        loaded.main.display(),
        loaded.name(),
        loaded.branch
    ))
}

/// The main checkout of `dir` from its `.git` alone, no git process: a
/// worktree's `.git` file names `<main>/.git/worktrees/<name>`.
fn main_of(dir: &Path) -> Option<PathBuf> {
    let top = dir.ancestors().find(|at| at.join(".git").exists())?;
    let dot_git = top.join(".git");
    if dot_git.is_dir() {
        return Some(top.to_path_buf());
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let gitdir = if gitdir.is_absolute() { gitdir } else { top.join(gitdir) };
    // <main>/.git/worktrees/<name>
    let common = gitdir.parent()?.parent()?;
    if common.file_name()? == ".git" {
        return common.parent().map(Path::to_path_buf);
    }
    crate::git::main_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The monorepo's own file, None where only this package is (its public mirror).
    fn platform() -> Option<String> {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../breadboard.toml")).ok()
    }

    #[test]
    fn says_a_command_as_the_repo_types_it() {
        // Here, in the repo breadboard is developed in, `pnpm` stays when its file names it.
        let text = "usage: pnpm wt:new <f> · pnpm install · pnpm herdr:install · bb submit, from develop";
        if run_word() == "pnpm" && here().branch == "develop" {
            assert_eq!(in_repo(text), text);
        } else if run_word() == "breadboard" {
            assert!(in_repo(text).starts_with(
                "usage: breadboard wt:new <f> · pnpm install · breadboard herdr:install · breadboard submit"
            ));
        }
    }

    #[test]
    fn the_defaults_hold_without_a_file() {
        let config = Config::parse("").unwrap();
        assert_eq!(config, Config::default());
        assert!(config.stack.is_none());
        assert_eq!(config.command("wt:new 1-x"), "breadboard wt:new 1-x");
        assert_eq!(config.mr.template, ".gitlab/merge_request_templates/Default.md");
    }

    #[test]
    fn reads_platform_s_file() {
        let Some(text) = platform() else { return };
        let config = Config::parse(&text).unwrap();
        assert_eq!(config.branch(), Some("develop"));
        assert_eq!(config.command("wt:new"), "pnpm wt:new");
        let stack = config.stack.unwrap();
        assert_eq!(stack.dir, "products/vita/web");
        assert_eq!(stack.offset_port().unwrap().env.as_deref(), Some("POSTGRES_PORT"));
        assert_eq!(
            stack.ports().iter().map(|port| port.port).collect::<Vec<_>>(),
            [3000, 4983, 5432, 6006, 8080, 8333, 64738]
        );
        // vita's epic and task stacks (#351, #365).
        let epic = stack.epic.unwrap();
        assert_eq!(
            (epic.service.as_str(), epic.template.as_str()),
            ("postgres", "vita_tpl")
        );
        assert_eq!(epic.clone_database().name, "vita");
        assert_eq!(epic.env.get("VITA_NETWORK").map(String::as_str), Some("{network}"));
    }

    #[test]
    fn an_epic_is_refused_without_what_it_needs() {
        let stack = "[stack]\nproject = \"p\"\n[stack.app]\nport = 3000\n\
                     [[stack.services]]\nname = \"postgres\"\nport = 5432\nenv = \"DB_PORT\"\n";
        let migrations = "[stack.migrations]\njournal = \"j\"\nrun = \"migrate\"\n";
        let epic = "[stack.batch]\ntemplate = \"t\"\nstamp = \"db\"\n";
        let clone = "[[stack.batch.databases]]\nname = \"app\"\nowner = \"o\"\nis_clone = true\n";
        assert!(Config::parse(&format!("{stack}{migrations}{epic}{clone}")).is_ok());
        let error = Config::parse(&format!("{stack}{epic}{clone}")).unwrap_err();
        assert!(error.starts_with("stack.batch: needs [stack.migrations]"), "{error}");
        let error = Config::parse(&format!("{stack}{migrations}{epic}")).unwrap_err();
        assert!(error.starts_with("stack.batch.databases: exactly one"), "{error}");
        let error = Config::parse(&format!("{stack}{migrations}{epic}service = \"db\"\n{clone}")).unwrap_err();
        assert!(
            error.starts_with("stack.batch.service: \"db\" is no [[stack.services]]"),
            "{error}"
        );
    }

    #[test]
    fn a_wrong_file_is_refused_with_its_key() {
        let error = Config::parse("[repo]\ndefault_brnch = \"x\"\n").unwrap_err();
        assert!(error.contains("default_brnch"), "{error}");
        let error = Config::parse("[stack]\nproject = \"p\"\n[stack.app]\nport = 3000\n").unwrap_err();
        assert!(error.starts_with("stack: no port has an `env`"), "{error}");
        let error = Config::parse(
            "[stack]\nproject = \"p\"\n[stack.app]\nport = 3000\n[[stack.services]]\nname = \"db\"\nport = 3000\nenv = \"DB_PORT\"\n",
        )
        .unwrap_err();
        assert_eq!(error, "stack (db): port 3000 is app's already");
        let error = Config::parse("[[mr.tests]]\npaths = [\"(\"]\nline = \"x\"\n").unwrap_err();
        assert!(
            error.starts_with("mr.tests[0].paths: \"(\" is no regular expression"),
            "{error}"
        );
        let error = Config::parse("[stack]\nproject = \"a b\"\n[stack.app]\nport = 1\nenv = \"P\"\n").unwrap_err();
        assert!(error.starts_with("stack.project:"), "{error}");
        let error = Config::parse("[main_checkout]\nevery_minutes = 0\n").unwrap_err();
        assert_eq!(error, "main_checkout.every_minutes: must be 1 or more");
        let error = Config::parse("[main_checkout]\ninstall_when = [\"x.lock\"]\n").unwrap_err();
        assert_eq!(error, "main_checkout.install_when: needs an `install` to run");
    }

    #[test]
    fn a_wrong_file_is_refused_with_its_line() {
        let at = |text: &str| Config::located(text).map(|_| ()).unwrap_err();
        let error = at("[repo]\nname = \"a/b\"\n\n[agents]\n# runs\nmax_runs = \"two\"\n");
        assert_eq!(error.0, Some(6));
        assert!(error.1.starts_with("agents.max_runs: invalid type"), "{}", error.1);
        assert_eq!(
            at("[agents]\nmax_runs = 0\n"),
            (Some(2), "agents.max_runs: must be at least 1".into())
        );
        let services = "[stack]\nproject = \"p\"\n[stack.app]\nport = 3000\nenv = \"P\"\n[[stack.services]]\nname = \"db\"\nport = 1\n[[stack.services]]\nname = \" \"\nport = 2\n";
        assert_eq!(
            at(services),
            (Some(10), "stack.services[1].name: must not be empty".into())
        );
        let error = at("[stack]\nproject = \"p\"\n[stack.app]\nport = 3000\n");
        assert_eq!(error.0, Some(1), "{}", error.1);
        assert_eq!(at("[repo]\nname = \n").0, Some(2));
    }

    #[test]
    fn a_product_is_named_by_its_milestone() {
        let config = Config::parse("[products.vita]\npaths = [\"products/vita\"]\n").unwrap();
        assert_eq!(config.products["vita"].paths, ["products/vita"]);
        assert_eq!(
            config.product_of_milestone("vita · Epic 12: The app").as_deref(),
            Some("vita")
        );
        assert_eq!(
            config.product_of_milestone("breadboard · Epic 15: The dev workflow"),
            None
        );
        assert_eq!(config.product_of_milestone("vita"), None);
        let error = Config::parse("[products.vita]\npaths = []\n").unwrap_err();
        assert_eq!(error, "products.vita.paths: must name at least one folder");
        let error = Config::parse("[products.\"a b\"]\npaths = [\"x\"]\n").unwrap_err();
        assert!(error.starts_with("products.a b: must be a name"), "{error}");
        let Some(text) = platform() else { return };
        let products = Config::parse(&text).unwrap().products;
        assert_eq!(products.keys().collect::<Vec<_>>(), ["breadboard", "vita"]);
    }

    #[test]
    fn the_main_checkout_follows_by_default_and_platform_installs_on_its_lockfile() {
        let config = Config::parse("").unwrap();
        assert!(config.main_checkout.follow && config.main_checkout.every_minutes == 2);
        assert!(config.main_checkout.install.is_none());
        assert!(
            !Config::parse("[main_checkout]\nfollow = false\n")
                .unwrap()
                .main_checkout
                .follow
        );
        let Some(text) = platform() else { return };
        let main = Config::parse(&text).unwrap().main_checkout;
        assert_eq!(main.install.as_deref(), Some("pnpm install --frozen-lockfile"));
        assert_eq!(main.install_when, ["pnpm-lock.yaml"]);
    }

    #[test]
    fn a_minimal_stack_takes_its_defaults() {
        let config = Config::parse("[stack]\nproject = \"app\"\n[stack.app]\nport = 8000\nenv = \"PORT\"\n").unwrap();
        let stack = config.stack.unwrap();
        assert_eq!((stack.dir.as_str(), stack.env_file.as_str()), (".", ".env.local"));
        assert_eq!(stack.compose_file, "docker-compose.yml");
        assert_eq!(stack.offset_port().unwrap().name, "app");
    }

    #[test]
    fn finds_the_main_checkout_of_a_worktree_without_git() {
        let root = std::env::temp_dir().join(format!("config-main-{}", std::process::id()));
        let main = root.join("repo");
        let tree = root.join("repo-1-x");
        std::fs::create_dir_all(main.join(".git/worktrees/repo-1-x")).unwrap();
        std::fs::create_dir_all(tree.join("a/b")).unwrap();
        std::fs::write(
            tree.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/repo-1-x").display()),
        )
        .unwrap();
        assert_eq!(main_of(&tree.join("a/b")), Some(tree.parent().unwrap().join("repo")));
        assert_eq!(main_of(&main), Some(main.clone()));
        std::fs::write(main.join(FILE), "[repo]\ndefault_branch = \"trunk\"\n").unwrap();
        assert_eq!(of(&tree).branch, "trunk");
        assert_eq!(of(&tree).base(), "origin/trunk");
        std::fs::write(main.join(FILE), "[repo]\nnope = 1\n").unwrap();
        // A changed file is read again; a wrong one keeps the defaults and says why.
        let filetime = SystemTime::now() + std::time::Duration::from_secs(5);
        let file = std::fs::File::options().write(true).open(main.join(FILE)).unwrap();
        file.set_modified(filetime).unwrap();
        let loaded = of(&tree);
        assert!(loaded.error.as_deref().is_some_and(|error| error.contains("nope")));
        // A worktree with a file of its own reads it, the main checkout keeps its own.
        std::fs::write(tree.join(FILE), "[herdr]\nspace = \"Mine\"\n").unwrap();
        assert_eq!(of(&tree.join("a/b")).space(), "Mine");
        assert_eq!(of(&tree).main, main);
        assert_eq!(of(&main).space(), "repo");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_filled_within_its_time_is_read_again() {
        let main = std::env::temp_dir().join(format!("config-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&main).unwrap();
        let at = SystemTime::now();
        let touch = |text: &str| {
            std::fs::write(main.join(FILE), text).unwrap();
            let file = std::fs::File::options().write(true).open(main.join(FILE)).unwrap();
            file.set_modified(at).unwrap();
        };
        // Caught truncated by git's rewrite, filled within the same tick (#447).
        touch("");
        assert!(of_main(&main).config.products.is_empty());
        touch("[products.breadboard]\npaths = [\"packages/breadboard\"]\n");
        assert!(of_main(&main).config.products.contains_key("breadboard"));
        let _ = std::fs::remove_dir_all(&main);
    }
}
