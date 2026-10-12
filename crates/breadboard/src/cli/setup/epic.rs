//! The epic stack of an orchestration and the task stacks on it (#351), as
//! breadboard.toml's `[stack.epic]` describes them (#365): the
//! orchestration's checkout runs a full stack, its database service keeps a
//! template database (migrated and seeded, never connected to); a task
//! worktree of the orchestration runs its stack without that service, on the
//! epic's network, its databases made on the epic's. A task stack starts on
//! demand (`stack:up`, `wt:stack`), never with `wt:new`. Without
//! `[stack.epic]` an orchestration's worktrees get own stacks.

use super::proc::{self, How, fail, outln, step};
use super::repo;
use super::stack;
use breadboard_core::config::{Database, Epic as Config, Stack};
use breadboard_core::git::orchestrator_checkouts;
use breadboard_core::record;
use std::fs;
use std::path::{Path, PathBuf};

/// The env line that makes a stack a task stack: the orchestration it belongs to.
pub const EPIC_VAR: &str = "BREADBOARD_EPIC";
/// Written in an epic's checkout once `[[worktrees.setup]]` ran there.
const SET_UP: &str = ".claude/epic-set-up";
/// The database psql connects to when it works on others: Postgres's own.
const MAINTENANCE: &str = "postgres";
/// The longest name Postgres keeps (NAMEDATALEN - 1).
const MAX_NAME: usize = 63;

/// An orchestration's epic: its name and the checkout its stack runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Epic {
    pub name: String,
    pub checkout: PathBuf,
}

impl Epic {
    /// The epic of the orchestration that started the worktree `dir`; None for a
    /// free worktree, when the orchestration has no checkout (no herdr), or
    /// when the repo's stack has no `[stack.epic]`.
    pub fn of_task(dir: &Path) -> Option<Epic> {
        let name = record::origin_of(dir)?.name;
        Epic::named(&name)
    }

    /// The epic of the orchestration `name`, when its checkout exists and the stack has `[stack.epic]`.
    pub fn named(name: &str) -> Option<Epic> {
        repo::stack()?.epic?;
        let checkout = repo::lifecycle().checkout_of(name);
        orchestrator_checkouts(&repo::main_dir())
            .contains(&checkout)
            .then(|| Epic {
                name: name.to_string(),
                checkout,
            })
    }

    /// The epic whose checkout is `dir`.
    pub fn at(dir: &Path) -> Option<Epic> {
        let name = dir.file_name()?.to_string_lossy().into_owned();
        Epic::named(&name).filter(|epic| epic.checkout == dir)
    }

    /// The feature its stack is named after: `<project>-epic-<name>`.
    pub fn feature(&self) -> String {
        format!("epic-{}", self.name)
    }

    /// The folder of its checkout the stack's commands run in.
    fn stack_dir(&self) -> PathBuf {
        repo::stack_dir(&self.checkout)
    }
}

/// The repo's stack and its `[stack.epic]`, refused without them.
fn stack_of() -> (Stack, Config) {
    let stack = repo::stack().unwrap_or_else(|| fail("the repo has no stack: breadboard.toml has no [stack]"));
    let Some(epic) = stack.epic.clone() else {
        fail("the repo's stack has no epic: breadboard.toml has no [stack.epic]")
    };
    (stack, epic)
}

/// The port of the epic's database service on a stack with `offset`.
fn database_port(offset: u16) -> u32 {
    let (stack, epic) = stack_of();
    stack
        .service(&epic.service)
        .map_or(0, |service| u32::from(service.port))
        + u32::from(offset)
}

/// `compose --env-file <env> -f <compose file>`, then `rest`: docker's words for the repo's stack.
fn compose(rest: &[&str]) -> Vec<String> {
    breadboard_core::stack::compose_args(&stack_of().0, rest)
}

/// What starts a checkout's stack: `[stack] up`, else compose's `up -d --wait`.
fn up_command(stack: &Stack) -> String {
    stack.up.clone().unwrap_or_else(|| {
        let mut words = breadboard_core::stack::compose_args(stack, &["up", "-d", "--wait"]);
        words.insert(0, "docker".into());
        words.join(" ")
    })
}

/// A task's database `<prefix>_<feature>`: lower case, `_` for anything
/// else, at most 63 bytes.
pub fn db_name(prefix: &str, feature: &str) -> String {
    let name: String = format!("{prefix}_{feature}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    name[..name.len().min(MAX_NAME)].to_string()
}

/// A Postgres url pointed at `127.0.0.1:<port>/<db>`, its user, password and query kept.
pub fn point_db(url: &str, port: u32, db: &str) -> String {
    let Some(scheme) = url.find("://").map(|at| at + 3) else {
        return url.to_string();
    };
    let rest = &url[scheme..];
    let path = rest.find('/').unwrap_or(rest.len());
    let user = rest[..path].rfind('@').map(|at| &rest[..=at]).unwrap_or("");
    let query = rest[path..].find('?').map(|at| &rest[path + at..]).unwrap_or("");
    format!("{}{user}127.0.0.1:{port}/{db}{query}", &url[..scheme])
}

/// Where a task stack stands: its feature and ports, and the epic's.
pub struct Task<'a> {
    pub feature: &'a str,
    pub offset: u16,
    pub epic: &'a str,
    pub epic_port: u32,
    pub network: &'a str,
}

/// What a task stack's env file says: the own stack's (ports moved by its
/// offset, its own compose project), its databases' URLs on the epic's
/// service (`epic_port`), then `[stack.epic] env` and its databases' names.
pub fn task_env(stack: &Stack, epic: &Config, source: &str, task: &Task) -> String {
    let own = stack::stack_env(stack, source, task.feature, task.offset);
    let pointed = |key: &str| {
        epic.databases
            .iter()
            .find(|database| database.urls.iter().any(|url| url == key))
            .map(|database| db_name(&database.name, task.feature))
    };
    let mut lines: Vec<String> = own
        .split('\n')
        .filter(|line| !line.starts_with(&format!("{EPIC_VAR}=")))
        .map(|line| match line.split_once('=') {
            Some((key, value)) => match pointed(key) {
                Some(db) => format!("{key}={}", point_db(value, task.epic_port, &db)),
                None => line.to_string(),
            },
            None => line.to_string(),
        })
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.push(format!(
        "# a task stack of the epic {}: no {} of its own, its databases on the epic's (port {})",
        task.epic, epic.service, task.epic_port
    ));
    lines.push(format!("{EPIC_VAR}={}", task.epic));
    for (key, value) in &epic.env {
        lines.push(format!("{key}={}", value.replace("{network}", task.network)));
    }
    for database in &epic.databases {
        if let Some(key) = &database.name_env {
            lines.push(format!("{key}={}", db_name(&database.name, task.feature)));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// The epic a stack's env file text belongs to, None for any other stack.
pub fn epic_in(env: &str) -> Option<String> {
    env.lines()
        .rev()
        .find_map(|line| line.strip_prefix(&format!("{EPIC_VAR}=")))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(String::from)
}

/// The epic of a task stack's checkout, None for any other.
pub fn epic_of_stack(dir: &Path) -> Option<String> {
    epic_in(&fs::read_to_string(repo::env_file_of(dir)).ok()?)
}

/// The port of the epic's database service a task stack's databases lie on, None for any other stack.
pub fn epic_port_of(dir: &Path) -> Option<u32> {
    let epic = Epic::named(&epic_of_stack(dir)?)?;
    Some(database_port(repo::offset_of(&epic.checkout)))
}

/// Whether a task's database is cloned again: the template changed since it
/// was cloned (`clone`), and the task's branch now holds what the template
/// was made from (`base`, the stamp folder where it meets the epic). A task
/// behind the epic keeps its database until it merges the epic in.
pub fn should_reclone(clone: Option<&str>, template: &str, base: Option<&str>) -> bool {
    match clone {
        None => true,
        Some(clone) => clone != template && base == Some(template),
    }
}

/// `"name"`, a quoted identifier.
fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `'text'`, a quoted literal.
fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The statements that make the database `db` of `database`: cloned from the
/// template `template` with its `stamp` as comment, or empty. One per psql
/// `-c` (CREATE DATABASE runs outside a transaction).
pub fn create_sql(database: &Database, db: &str, template: Option<(&str, &str)>) -> Vec<String> {
    let mut sql = vec![format!("drop database if exists {} with (force)", ident(db))];
    sql.push(match template {
        Some((template, _)) => format!(
            "create database {} template {} owner {}",
            ident(db),
            ident(template),
            ident(&database.owner)
        ),
        None => format!("create database {} owner {}", ident(db), ident(&database.owner)),
    });
    sql.extend(grants_sql(database, db));
    if let Some((_, stamp)) = template {
        sql.push(format!("comment on database {} is {}", ident(db), literal(stamp)));
    }
    sql
}

/// Only its owner and its `connect` roles reach a database.
fn grants_sql(database: &Database, db: &str) -> Vec<String> {
    let mut sql = vec![format!("revoke all on database {} from public", ident(db))];
    for role in &database.connect {
        sql.push(format!("grant connect on database {} to {}", ident(db), ident(role)));
    }
    sql
}

/// The epic's database service answering `sql` on database `db`: quiet, tuples only.
fn psql(epic: &Epic, db: &str, sql: &[String], can_fail: bool) -> proc::Ran {
    let (_, config) = stack_of();
    let words = compose(&[]);
    let mut args: Vec<&str> = words.iter().map(String::as_str).collect();
    args.extend([
        "exec",
        "-T",
        config.service.as_str(),
        "psql",
        "-X",
        "-q",
        "-t",
        "-A",
        "-v",
        "ON_ERROR_STOP=1",
        "-U",
        config.superuser.as_str(),
        "-d",
        db,
    ]);
    for statement in sql {
        args.push("-c");
        args.push(statement);
    }
    proc::run(
        "docker",
        &args,
        How {
            cwd: Some(&epic.stack_dir()),
            is_quiet: true,
            can_fail,
            ..How::default()
        },
    )
}

/// The comment of a database (its stamp), None when it doesn't exist.
fn stamp_of(epic: &Epic, db: &str) -> Option<String> {
    let sql = format!(
        "select coalesce(shobj_description(oid, 'pg_database'), '') from pg_database where datname = {}",
        literal(db)
    );
    let ran = psql(epic, MAINTENANCE, &[sql], true);
    let text = ran.text();
    (ran.is_ok && !ran.out.is_empty()).then(|| text.to_string())
}

/// The stamp folder in the stack's `dir`, from the checkout's root.
fn stamp_folder() -> String {
    let (stack, epic) = stack_of();
    let folder = Path::new(&stack.dir).join(&epic.stamp);
    folder
        .components()
        .filter(|part| !matches!(part, std::path::Component::CurDir))
        .collect::<PathBuf>()
        .to_string_lossy()
        .into_owned()
}

/// The tree of the stamp folder at `rev` in `dir`: what a template was made from.
fn db_tree(dir: &Path, rev: &str) -> Option<String> {
    let ran = proc::git(
        &["rev-parse", "--verify", "--quiet", &format!("{rev}:{}", stamp_folder())],
        dir,
    );
    (ran.is_ok && !ran.text().is_empty()).then(|| ran.text().to_string())
}

/// A shell command in a checkout's stack folder, shown, failing loudly.
fn run_in(dir: &Path, command: &str, env: Vec<(&str, String)>) {
    proc::run(
        "sh",
        &["-c", command],
        How {
            cwd: Some(&repo::stack_dir(dir)),
            env,
            ..How::default()
        },
    );
}

/// The variable `key` of a checkout's env file.
fn env_value(dir: &Path, key: &str) -> Option<String> {
    breadboard_core::stack::env_of(dir)?.remove(key)
}

/// Whether any container of a checkout's stack exists (`running` only: runs).
fn has_containers(dir: &Path, is_running: bool) -> bool {
    let words = compose(&["ps", "-q", if is_running { "--status=running" } else { "-a" }]);
    let args: Vec<&str> = words.iter().map(String::as_str).collect();
    let ran = proc::sh("docker", &args, Some(&repo::stack_dir(dir)));
    ran.is_ok && !ran.text().is_empty()
}

/// The epic's env file, written when it has none: its offset reserved, nothing started.
pub fn reserve_epic(epic: &Epic) -> u16 {
    if repo::env_file_of(&epic.checkout).exists() {
        return repo::offset_of(&epic.checkout);
    }
    stack::reserve_stack(&epic.checkout, &epic.feature())
}

/// The compose network of the epic's stack, which its task stacks join.
fn network_of(epic: &Epic) -> String {
    let project = env_value(&epic.checkout, "COMPOSE_PROJECT_NAME")
        .unwrap_or_else(|| format!("{}-{}", stack_of().0.project, epic.feature()));
    format!("{project}_default")
}

/// A task worktree's env file: its offset reserved, the epic's too, nothing started.
pub fn reserve_task(dir: &Path, feature: &str, epic: &Epic) -> u16 {
    let epic_offset = reserve_epic(epic);
    let source = repo::env_file_of(&repo::main_dir());
    let Ok(text) = fs::read_to_string(&source) else {
        fail(&format!(
            "{} is missing, set up the main checkout first",
            source.display()
        ))
    };
    let network = network_of(epic);
    let (stack, config) = stack_of();
    let epic_port = database_port(epic_offset);
    stack::reserve_with(dir, &text, |offset| {
        task_env(
            &stack,
            &config,
            &text,
            &Task {
                feature,
                offset,
                epic: &epic.name,
                epic_port,
                network: &network,
            },
        )
    })
}

/// The epic's checkout set up once as a new worktree is (`[[worktrees.setup]]`).
fn set_up_checkout(epic: &Epic) {
    let marker = epic.checkout.join(SET_UP);
    if marker.exists() {
        return;
    }
    let steps = breadboard_core::config::of(&epic.checkout)
        .config
        .worktrees
        .setup
        .clone();
    for run in &steps {
        step(&format!(
            "the checkout of the epic {}: {}",
            epic.name,
            run.says.as_deref().unwrap_or(&run.run)
        ));
        proc::run(
            "sh",
            &["-c", &run.run],
            How {
                cwd: Some(&epic.checkout),
                is_quiet: run.is_quiet || proc::is_agent(),
                ..How::default()
            },
        );
    }
    if let Some(parent) = marker.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&marker, "");
}

/// The epic's stack running, set up and its template current. A first start
/// is the stack's setup; a stopped one is started and migrated.
pub fn ensure_epic(epic: &Epic) {
    reserve_epic(epic);
    set_up_checkout(epic);
    let (stack, _) = stack_of();
    if !has_containers(&epic.checkout, false) {
        step(&format!("the epic stack of {} is new: its setup", epic.name));
        stack::dev_setup(&epic.checkout);
    } else if !has_containers(&epic.checkout, true) {
        step(&format!("start the epic stack of {}", epic.name));
        run_in(&epic.checkout, &up_command(&stack), Vec::new());
    }
    ensure_template(epic);
}

/// The epic's template is what its checkout's stamp folder says: made again
/// (its own database migrated too) when the stamp differs.
fn ensure_template(epic: &Epic) -> String {
    let (stack, config) = stack_of();
    let template = config.template.as_str();
    let Some(stamp) = db_tree(&epic.checkout, "HEAD") else {
        fail(&format!("{} has no {}", epic.checkout.display(), stamp_folder()))
    };
    if stamp_of(epic, template).as_deref() == Some(stamp.as_str()) {
        return stamp;
    }
    step(&format!(
        "the epic {} changed: migrate it, make its template {template} again",
        epic.name
    ));
    let migrate = stack
        .migrations
        .as_ref()
        .map(|migrations| migrations.run.clone())
        .unwrap_or_default();
    run_in(&epic.checkout, &migrate, Vec::new());
    let tpl = ident(template);
    if stamp_of(epic, template).is_some() {
        psql(
            epic,
            MAINTENANCE,
            &[
                format!("alter database {tpl} with is_template false allow_connections true"),
                format!("drop database {tpl} with (force)"),
            ],
            false,
        );
    }
    let database = config.clone_database();
    psql(epic, MAINTENANCE, &create_sql(database, template, None), false);
    if !config.template_sql.is_empty() {
        psql(epic, template, &config.template_sql, false);
    }
    let port = database_port(repo::offset_of(&epic.checkout));
    let urls: Vec<(&str, String)> = database
        .urls
        .iter()
        .map(|key| {
            let value = env_value(&epic.checkout, key)
                .unwrap_or_else(|| fail(&format!("the epic's {} has no {key}", stack.env_file)));
            (key.as_str(), point_db(&value, port, template))
        })
        .collect();
    run_in(&epic.checkout, &migrate, urls.clone());
    if let Some(seed) = &config.seed {
        run_in(&epic.checkout, seed, urls);
    }
    let mut sql: Vec<String> = database
        .connect
        .iter()
        .map(|role| format!("revoke connect on database {tpl} from {}", ident(role)))
        .collect();
    sql.push(format!("comment on database {tpl} is {}", literal(&stamp)));
    sql.push(format!(
        "alter database {tpl} with is_template true allow_connections false"
    ));
    psql(epic, MAINTENANCE, &sql, false);
    stamp
}

/// A task's stack up: the epic's running with a current template, the task's
/// databases made (its clone again, after it merged the epic in), its
/// containers started, its own migrations applied, then `task_setup`.
pub fn task_up(dir: &Path, feature: &str, epic: &Epic) {
    ensure_epic(epic);
    let (stack, config) = stack_of();
    let template = stamp_of(epic, &config.template).unwrap_or_default();
    // The stamp folder where the task's branch meets the epic's.
    let head = proc::git(&["rev-parse", "HEAD"], &epic.checkout);
    let meet = proc::git(&["merge-base", "HEAD", head.text()], dir);
    let base = (head.is_ok && meet.is_ok).then(|| db_tree(dir, meet.text())).flatten();
    for database in &config.databases {
        let db = db_name(&database.name, feature);
        let known = stamp_of(epic, &db);
        if database.is_clone {
            if should_reclone(known.as_deref(), &template, base.as_deref()) {
                let again = if known.is_some() {
                    " again (the epic's migrations came in)"
                } else {
                    ""
                };
                step(&format!(
                    "clone {db} from {} on the epic's {}{again}",
                    config.template, config.service
                ));
                psql(
                    epic,
                    MAINTENANCE,
                    &create_sql(database, &db, Some((&config.template, &template))),
                    false,
                );
            }
        } else if known.is_none() {
            step(&format!("create {db} on the epic's {}", config.service));
            psql(epic, MAINTENANCE, &create_sql(database, &db, None), false);
        }
    }
    step(&format!("start the task stack of {feature}"));
    run_in(dir, &up_command(&stack), Vec::new());
    if let Some(migrations) = &stack.migrations {
        run_in(dir, &migrations.run, Vec::new());
    }
    for command in &config.task_setup {
        run_in(dir, command, Vec::new());
    }
}

/// A task's databases on the epic's service dropped (`wt:remove`, `stack:reset`).
/// The epic's stack is started for it when it is down; an epic gone takes them along.
pub fn drop_task(feature: &str, epic_name: &str) {
    let Some(epic) = Epic::named(epic_name).filter(|epic| repo::env_file_of(&epic.checkout).exists()) else {
        return;
    };
    let (stack, config) = stack_of();
    if !has_containers(&epic.checkout, true) {
        let up = proc::run(
            "sh",
            &["-c", &up_command(&stack)],
            How {
                cwd: Some(&epic.stack_dir()),
                can_fail: true,
                ..How::default()
            },
        );
        if !up.is_ok {
            outln!("! the epic stack of {epic_name} doesn't start, its databases of {feature} stay");
            return;
        }
    }
    let names: Vec<String> = config
        .databases
        .iter()
        .map(|database| db_name(&database.name, feature))
        .collect();
    step(&format!(
        "drop {} on the epic's {}",
        names.join(" and "),
        config.service
    ));
    let sql: Vec<String> = names
        .iter()
        .map(|name| format!("drop database if exists {} with (force)", ident(name)))
        .collect();
    let ran = psql(&epic, MAINTENANCE, &sql, true);
    if !ran.is_ok {
        outln!("! couldn't drop them: {}", ran.err.trim());
    }
}

/// The epic stack of an ended orchestration stopped, its data deleted: what it says.
pub fn end_epic(name: &str) -> Option<String> {
    let epic = Epic::named(name).filter(|epic| repo::env_file_of(&epic.checkout).exists())?;
    if !has_containers(&epic.checkout, false) {
        return None;
    }
    let words = compose(&["down", "-v"]);
    let args: Vec<&str> = words.iter().map(String::as_str).collect();
    let ran = proc::run(
        "docker",
        &args,
        How {
            cwd: Some(&epic.stack_dir()),
            is_quiet: true,
            can_fail: true,
            ..How::default()
        },
    );
    Some(if ran.is_ok {
        format!("removed the epic stack of {name} with its data")
    } else {
        format!("! the epic stack of {name} stays: {}", ran.err.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// platform's stack as its breadboard.toml has it.
    fn config() -> Stack {
        breadboard_core::config::Config::parse(
            "[stack]\nproject = \"vita\"\ndir = \"products/vita/web\"\n[stack.app]\nport = 3000\n\
             [[stack.services]]\nname = \"postgres\"\nport = 5432\nenv = \"POSTGRES_PORT\"\n\
             [[stack.services]]\nname = \"keycloak\"\nport = 8080\nenv = \"KEYCLOAK_PORT\"\n\
             [[stack.services]]\nname = \"s3\"\nport = 8333\nenv = \"S3_PORT\"\n\
             [[stack.services]]\nname = \"visualizer\"\nport = 64738\nenv = \"VISUALIZER_PORT\"\n\
             [stack.migrations]\njournal = \"j.json\"\nrun = \"pnpm pg:migrate\"\n\
             [stack.epic]\ntemplate = \"vita_tpl\"\nstamp = \"src/server/db\"\nseed = \"pnpm pg:seed\"\n\
             task_setup = [\"pnpm storage:setup\", \"pnpm keycloak:setup\"]\n\
             [stack.epic.env]\nPOSTGRES_SCALE = \"0\"\nVITA_NETWORK = \"{network}\"\nVITA_NETWORK_EXTERNAL = \"true\"\n\
             [[stack.epic.databases]]\nname = \"vita\"\nis_clone = true\nowner = \"vita_migrator\"\n\
             connect = [\"vita_app\"]\nurls = [\"DATABASE_URL\", \"MIGRATION_DATABASE_URL\"]\n\
             [[stack.epic.databases]]\nname = \"keycloak\"\nowner = \"keycloak\"\nname_env = \"KEYCLOAK_DB_NAME\"\n",
        )
        .unwrap()
        .stack
        .unwrap()
    }

    #[test]
    fn names_a_task_database() {
        assert_eq!(db_name("vita", "351-epic-stack"), "vita_351_epic_stack");
        assert_eq!(db_name("keycloak", "Ä-x"), "keycloak___x");
        assert_eq!(db_name("vita", &"x".repeat(80)).len(), 63);
    }

    #[test]
    fn points_a_url_at_another_database() {
        assert_eq!(
            point_db("postgresql://vita_app:p@ss@127.0.0.1:5435/vita", 5433, "vita_40_x"),
            "postgresql://vita_app:p@ss@127.0.0.1:5433/vita_40_x"
        );
        assert_eq!(
            point_db("postgres://localhost:5432/vita?sslmode=disable", 5433, "t"),
            "postgres://127.0.0.1:5433/t?sslmode=disable"
        );
        assert_eq!(point_db("not a url", 1, "t"), "not a url");
    }

    #[test]
    fn writes_a_task_stack_from_its_epic_config() {
        let source = "APP_ORIGIN=http://localhost:3000\nDATABASE_URL=postgresql://vita_app:a@127.0.0.1:5432/vita\nMIGRATION_DATABASE_URL=postgresql://vita_migrator:m@127.0.0.1:5432/vita\n";
        let stack = config();
        let env = task_env(
            &stack,
            stack.epic.as_ref().unwrap(),
            source,
            &Task {
                feature: "40-x",
                offset: 3,
                epic: "breadboard",
                epic_port: 5433,
                network: "vita-epic-breadboard_default",
            },
        );
        // As #351 wrote it for vita, now from [stack.epic].
        assert_eq!(
            env,
            "APP_ORIGIN=http://localhost:3003\nDATABASE_URL=postgresql://vita_app:a@127.0.0.1:5433/vita_40_x\nMIGRATION_DATABASE_URL=postgresql://vita_migrator:m@127.0.0.1:5433/vita_40_x\n\n# --- own dev stack of the worktree 40-x, written by breadboard (pnpm wt:new, wt:stack) ---\nCOMPOSE_PROJECT_NAME=vita-40-x\nPOSTGRES_PORT=5435\nKEYCLOAK_PORT=8083\nS3_PORT=8336\nVISUALIZER_PORT=64741\n# a task stack of the epic breadboard: no postgres of its own, its databases on the epic's (port 5433)\nBREADBOARD_EPIC=breadboard\nPOSTGRES_SCALE=0\nVITA_NETWORK=vita-epic-breadboard_default\nVITA_NETWORK_EXTERNAL=true\nKEYCLOAK_DB_NAME=keycloak_40_x\n"
        );
        assert_eq!(epic_in(&env).as_deref(), Some("breadboard"));
        assert_eq!(epic_in(source), None);
        // Its offset stays its own: POSTGRES_PORT is what the offset is read from.
        assert!(env.contains("POSTGRES_PORT=5435"));
    }

    #[test]
    fn clones_again_only_once_the_epic_is_merged_in() {
        // Never cloned: clone.
        assert!(should_reclone(None, "t2", Some("t1")));
        // Cloned from the template it is now: keep.
        assert!(!should_reclone(Some("t2"), "t2", Some("t2")));
        // The epic moved on, the task hasn't merged it in yet: keep.
        assert!(!should_reclone(Some("t1"), "t2", Some("t1")));
        // Merged in: clone again.
        assert!(should_reclone(Some("t1"), "t2", Some("t2")));
    }

    #[test]
    fn makes_a_clone_and_an_empty_database_quoted() {
        let stack = config();
        let epic = stack.epic.unwrap();
        assert_eq!(
            create_sql(&epic.databases[0], "vita_40_x", Some(("vita_tpl", "ab'c"))),
            [
                "drop database if exists \"vita_40_x\" with (force)",
                "create database \"vita_40_x\" template \"vita_tpl\" owner \"vita_migrator\"",
                "revoke all on database \"vita_40_x\" from public",
                "grant connect on database \"vita_40_x\" to \"vita_app\"",
                "comment on database \"vita_40_x\" is 'ab''c'",
            ]
        );
        assert_eq!(
            create_sql(&epic.databases[1], "keycloak_40_x", None),
            [
                "drop database if exists \"keycloak_40_x\" with (force)",
                "create database \"keycloak_40_x\" owner \"keycloak\"",
                "revoke all on database \"keycloak_40_x\" from public",
            ]
        );
    }
}
