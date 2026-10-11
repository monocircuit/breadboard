//! The epic stack of an orchestration and the task stacks on it (#351): the
//! orchestration's checkout runs a full stack, its Postgres keeps a template
//! database `vita_tpl` (migrated and seeded, never connected to); a task
//! worktree of the orchestration runs Keycloak and SeaweedFS only, on the
//! epic's network, its databases cloned on the epic's Postgres. A task stack
//! starts on demand (`stack:up`, `wt:stack`), never with `wt:new`.

use super::proc::{self, How, fail, outln, step};
use super::repo;
use super::stack;
use breadboard_core::config::Stack;
use breadboard_core::git::orchestrator_checkouts;
use breadboard_core::record;
use std::fs;
use std::path::{Path, PathBuf};

/// The template database on the epic's Postgres.
pub const TEMPLATE: &str = "vita_tpl";
/// The env line that makes a stack a task stack: the orchestration it belongs to.
pub const EPIC_VAR: &str = "BREADBOARD_EPIC";
/// vita's database folder: schema, migrations, seed. Its tree is the stamp of a template.
const DB_FOLDER: &str = "products/vita/web/src/server/db";
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
    /// free worktree, or when the orchestration has no checkout (no herdr).
    pub fn of_task(dir: &Path) -> Option<Epic> {
        let name = record::origin_of(dir)?.name;
        Epic::named(&name)
    }

    /// The epic of the orchestration `name`, when its checkout exists.
    pub fn named(name: &str) -> Option<Epic> {
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

    /// The feature its stack is named after: `vita-epic-<name>`.
    pub fn feature(&self) -> String {
        format!("epic-{}", self.name)
    }

    /// The folder of its checkout the stack's commands run in.
    fn vita(&self) -> PathBuf {
        repo::stack_dir(&self.checkout)
    }
}

/// The repo's stack, refused without one: an epic stack is one of it.
fn stack_of() -> Stack {
    repo::stack().unwrap_or_else(|| fail("the repo has no stack: breadboard.toml has no [stack]"))
}

/// The port of the database a stack on `offset` keeps, the port its offset is read from.
fn database_port(offset: u16) -> u32 {
    stack_of().offset_port().map_or(0, |port| u32::from(port.port)) + u32::from(offset)
}

/// `compose --env-file <env> -f <compose file>`, then `rest`: docker's words for the repo's stack.
fn compose(rest: &[&str]) -> Vec<String> {
    breadboard_core::stack::compose_args(&stack_of(), rest)
}

/// `pnpm stack:*` of a task stack, `vita_<feature>`, `keycloak_<feature>`:
/// lower case, `_` for anything else, at most 63 bytes.
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

/// What a task stack's `.env.local` says: the own stack's (ports moved by
/// `offset`, its own compose project) without a Postgres of its own, on the
/// epic's network, its databases on the epic's Postgres (`epic_port`).
pub fn task_env(
    stack: &Stack,
    source: &str,
    feature: &str,
    offset: u16,
    epic: &str,
    epic_port: u32,
    network: &str,
) -> String {
    let own = stack::stack_env(stack, source, feature, offset);
    let vita = db_name("vita", feature);
    let mut lines: Vec<String> = own
        .split('\n')
        .filter(|line| !line.starts_with(&format!("{EPIC_VAR}=")))
        .map(|line| match line.split_once('=') {
            Some((key @ ("DATABASE_URL" | "MIGRATION_DATABASE_URL"), value)) => {
                format!("{key}={}", point_db(value, epic_port, &vita))
            }
            _ => line.to_string(),
        })
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.push(format!(
        "# a task stack of the epic {epic}: no Postgres of its own, its databases on the epic's (port {epic_port})"
    ));
    lines.push(format!("{EPIC_VAR}={epic}"));
    lines.push("POSTGRES_SCALE=0".into());
    lines.push(format!("VITA_NETWORK={network}"));
    lines.push("VITA_NETWORK_EXTERNAL=true".into());
    lines.push(format!("KEYCLOAK_DB_NAME={}", db_name("keycloak", feature)));
    lines.push(String::new());
    lines.join("\n")
}

/// The epic a stack's `.env.local` text belongs to, None for any other stack.
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

/// The port of the epic's Postgres a task stack's databases lie on, None for any other stack.
pub fn epic_port_of(dir: &Path) -> Option<u32> {
    let epic = Epic::named(&epic_of_stack(dir)?)?;
    Some(database_port(repo::offset_of(&epic.checkout)))
}

/// Whether a task's database is cloned again: the template changed since it
/// was cloned (`clone`), and the task's branch now holds what the template
/// was made from (`base`, the db folder where it meets the epic). A task
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

/// The statements that clone a task's database from the template, one per
/// psql `-c` (CREATE DATABASE runs outside a transaction).
pub fn clone_sql(db: &str, stamp: &str) -> Vec<String> {
    vec![
        format!("drop database if exists {} with (force)", ident(db)),
        format!("create database {} template {TEMPLATE} owner vita_migrator", ident(db)),
        format!("revoke all on database {} from public", ident(db)),
        format!("grant connect on database {} to vita_app", ident(db)),
        format!("comment on database {} is {}", ident(db), literal(stamp)),
    ]
}

/// The epic's Postgres answering `sql` on database `db`: quiet, tuples only.
fn psql(epic: &Epic, db: &str, sql: &[String], can_fail: bool) -> proc::Ran {
    let words = compose(&[]);
    let mut args: Vec<&str> = words.iter().map(String::as_str).collect();
    args.extend([
        "exec",
        "-T",
        "postgres",
        "psql",
        "-X",
        "-q",
        "-t",
        "-A",
        "-v",
        "ON_ERROR_STOP=1",
        "-U",
        "postgres",
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
            cwd: Some(&epic.vita()),
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
    let ran = psql(epic, "postgres", &[sql], true);
    let text = ran.text();
    (ran.is_ok && !ran.out.is_empty()).then(|| text.to_string())
}

/// The tree of vita's db folder at `rev` in `dir`: what a template was made from.
fn db_tree(dir: &Path, rev: &str) -> Option<String> {
    let ran = proc::git(
        &["rev-parse", "--verify", "--quiet", &format!("{rev}:{DB_FOLDER}")],
        dir,
    );
    (ran.is_ok && !ran.text().is_empty()).then(|| ran.text().to_string())
}

/// `pnpm <script>` in a checkout's vita, shown, failing loudly.
fn pnpm(dir: &Path, script: &[&str], env: Vec<(&str, String)>) {
    proc::run(
        "pnpm",
        script,
        How {
            cwd: Some(&repo::stack_dir(dir)),
            env,
            ..How::default()
        },
    );
}

/// The variable `key` of a checkout's `.env.local`.
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

/// The epic's `.env.local`, written when it has none: its offset reserved, nothing started.
pub fn reserve_epic(epic: &Epic) -> u16 {
    if repo::env_file_of(&epic.checkout).exists() {
        return repo::offset_of(&epic.checkout);
    }
    stack::reserve_stack(&epic.checkout, &epic.feature())
}

/// The compose network of the epic's stack, which its task stacks join.
fn network_of(epic: &Epic) -> String {
    let project =
        env_value(&epic.checkout, "COMPOSE_PROJECT_NAME").unwrap_or_else(|| format!("vita-{}", epic.feature()));
    format!("{project}_default")
}

/// A task worktree's `.env.local`: its offset reserved, the epic's too, nothing started.
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
    let stack = stack_of();
    stack::reserve_with(dir, &text, |offset| {
        task_env(
            &stack,
            &text,
            feature,
            offset,
            &epic.name,
            database_port(epic_offset),
            &network,
        )
    })
}

/// The epic's stack running, set up and its template current. A first start
/// is a `pnpm dev:setup`; a stopped one is started and migrated.
pub fn ensure_epic(epic: &Epic) {
    reserve_epic(epic);
    if !epic.checkout.join("node_modules").exists() {
        step(&format!("pnpm install in the checkout of the epic {}", epic.name));
        proc::run(
            "pnpm",
            &["install", "--frozen-lockfile"],
            How {
                cwd: Some(&epic.checkout),
                is_quiet: proc::is_agent(),
                ..How::default()
            },
        );
    }
    if !has_containers(&epic.checkout, false) {
        step(&format!("the epic stack of {} is new: pnpm dev:setup there", epic.name));
        stack::dev_setup(&epic.checkout);
    } else if !has_containers(&epic.checkout, true) {
        step(&format!("start the epic stack of {}", epic.name));
        pnpm(&epic.checkout, &["stack:up"], Vec::new());
    }
    ensure_template(epic);
}

/// The epic's template is what its checkout's db folder says: made again
/// (its own database migrated too) when the stamp differs.
fn ensure_template(epic: &Epic) -> String {
    let Some(stamp) = db_tree(&epic.checkout, "HEAD") else {
        fail(&format!("{} has no {DB_FOLDER}", epic.checkout.display()))
    };
    if stamp_of(epic, TEMPLATE).as_deref() == Some(stamp.as_str()) {
        return stamp;
    }
    step(&format!(
        "the epic {} changed: migrate it, make its template {TEMPLATE} again",
        epic.name
    ));
    pnpm(&epic.checkout, &["pg:migrate"], Vec::new());
    let tpl = ident(TEMPLATE);
    if stamp_of(epic, TEMPLATE).is_some() {
        psql(
            epic,
            "postgres",
            &[
                format!("alter database {tpl} with is_template false allow_connections true"),
                format!("drop database {tpl} with (force)"),
            ],
            false,
        );
    }
    psql(
        epic,
        "postgres",
        &[
            format!("create database {tpl} owner vita_migrator"),
            format!("revoke all on database {tpl} from public"),
            format!("grant connect on database {tpl} to vita_app"),
        ],
        false,
    );
    // What the init script does for `vita` (infra/postgres/init), for the template.
    psql(
        epic,
        TEMPLATE,
        &[
            "revoke all on schema public from public".into(),
            "alter schema public owner to vita_migrator".into(),
            "grant usage on schema public to vita_app".into(),
            "alter default privileges for role vita_migrator in schema public grant select, insert, update, delete on tables to vita_app".into(),
            "alter default privileges for role vita_migrator in schema public grant usage, select on sequences to vita_app".into(),
        ],
        false,
    );
    let port = database_port(repo::offset_of(&epic.checkout));
    let url = |key: &'static str| {
        let value =
            env_value(&epic.checkout, key).unwrap_or_else(|| fail(&format!("the epic's .env.local has no {key}")));
        (key, point_db(&value, port, TEMPLATE))
    };
    pnpm(&epic.checkout, &["pg:migrate"], vec![url("MIGRATION_DATABASE_URL")]);
    pnpm(&epic.checkout, &["pg:seed"], vec![url("DATABASE_URL")]);
    psql(
        epic,
        "postgres",
        &[
            format!("revoke connect on database {tpl} from vita_app"),
            format!("comment on database {tpl} is {}", literal(&stamp)),
            format!("alter database {tpl} with is_template true allow_connections false"),
        ],
        false,
    );
    stamp
}

/// A task's stack up: the epic's running with a current template, the task's
/// databases cloned (again, after it merged the epic in), its containers
/// started, its own migrations, the bucket and Keycloak set up.
pub fn task_up(dir: &Path, feature: &str, epic: &Epic) {
    ensure_epic(epic);
    let template = stamp_of(epic, TEMPLATE).unwrap_or_default();
    let vita = db_name("vita", feature);
    let keycloak = db_name("keycloak", feature);
    let clone = stamp_of(epic, &vita);
    // The db folder where the task's branch meets the epic's.
    let head = proc::git(&["rev-parse", "HEAD"], &epic.checkout);
    let meet = proc::git(&["merge-base", "HEAD", head.text()], dir);
    let base = (head.is_ok && meet.is_ok).then(|| db_tree(dir, meet.text())).flatten();
    if should_reclone(clone.as_deref(), &template, base.as_deref()) {
        let again = if clone.is_some() {
            " again (the epic's migrations came in)"
        } else {
            ""
        };
        step(&format!("clone {vita} from {TEMPLATE} on the epic's Postgres{again}"));
        psql(epic, "postgres", &clone_sql(&vita, &template), false);
    }
    if stamp_of(epic, &keycloak).is_none() {
        step(&format!("create {keycloak} on the epic's Postgres"));
        psql(
            epic,
            "postgres",
            &[format!("create database {} owner keycloak", ident(&keycloak))],
            false,
        );
    }
    step(&format!("start the task stack of {feature} (Keycloak, SeaweedFS)"));
    pnpm(dir, &["stack:up"], Vec::new());
    pnpm(dir, &["pg:migrate"], Vec::new());
    pnpm(dir, &["storage:setup"], Vec::new());
    pnpm(dir, &["keycloak:setup"], Vec::new());
}

/// A task's databases on the epic's Postgres dropped (`wt:remove`, `stack:reset`).
/// The epic's Postgres is started for it when it is down; an epic gone takes them along.
pub fn drop_task(feature: &str, epic_name: &str) {
    let Some(epic) = Epic::named(epic_name).filter(|epic| repo::env_file_of(&epic.checkout).exists()) else {
        return;
    };
    if !has_containers(&epic.checkout, true) {
        let up = proc::run(
            "pnpm",
            &["stack:up"],
            How {
                cwd: Some(&epic.vita()),
                can_fail: true,
                ..How::default()
            },
        );
        if !up.is_ok {
            outln!("! the epic stack of {epic_name} doesn't start, its databases of {feature} stay");
            return;
        }
    }
    let names = [db_name("vita", feature), db_name("keycloak", feature)];
    step(&format!("drop {} on the epic's Postgres", names.join(" and ")));
    let sql: Vec<String> = names
        .iter()
        .map(|name| format!("drop database if exists {} with (force)", ident(name)))
        .collect();
    let ran = psql(&epic, "postgres", &sql, true);
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
            cwd: Some(&epic.vita()),
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
    fn writes_a_task_stack_without_postgres() {
        let source = "APP_ORIGIN=http://localhost:3000\nDATABASE_URL=postgresql://vita_app:a@127.0.0.1:5432/vita\nMIGRATION_DATABASE_URL=postgresql://vita_migrator:m@127.0.0.1:5432/vita\n";
        let stack = breadboard_core::config::Config::parse(
            "[stack]\nproject = \"vita\"\n[stack.app]\nport = 3000\n\
             [[stack.services]]\nname = \"postgres\"\nport = 5432\nenv = \"POSTGRES_PORT\"\n\
             [[stack.services]]\nname = \"keycloak\"\nport = 8080\nenv = \"KEYCLOAK_PORT\"\n\
             [[stack.services]]\nname = \"s3\"\nport = 8333\nenv = \"S3_PORT\"\n\
             [[stack.services]]\nname = \"visualizer\"\nport = 64738\nenv = \"VISUALIZER_PORT\"\n",
        )
        .unwrap()
        .stack
        .unwrap();
        let env = task_env(
            &stack,
            source,
            "40-x",
            3,
            "breadboard",
            5433,
            "vita-epic-breadboard_default",
        );
        assert_eq!(
            env,
            "APP_ORIGIN=http://localhost:3003\nDATABASE_URL=postgresql://vita_app:a@127.0.0.1:5433/vita_40_x\nMIGRATION_DATABASE_URL=postgresql://vita_migrator:m@127.0.0.1:5433/vita_40_x\n\n# --- own dev stack of the worktree 40-x, written by breadboard (pnpm wt:new, wt:stack) ---\nCOMPOSE_PROJECT_NAME=vita-40-x\nPOSTGRES_PORT=5435\nKEYCLOAK_PORT=8083\nS3_PORT=8336\nVISUALIZER_PORT=64741\n# a task stack of the epic breadboard: no Postgres of its own, its databases on the epic's (port 5433)\nBREADBOARD_EPIC=breadboard\nPOSTGRES_SCALE=0\nVITA_NETWORK=vita-epic-breadboard_default\nVITA_NETWORK_EXTERNAL=true\nKEYCLOAK_DB_NAME=keycloak_40_x\n"
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
    fn quotes_names_and_texts() {
        assert_eq!(
            clone_sql("vita_40_x", "ab'c"),
            [
                "drop database if exists \"vita_40_x\" with (force)",
                "create database \"vita_40_x\" template vita_tpl owner vita_migrator",
                "revoke all on database \"vita_40_x\" from public",
                "grant connect on database \"vita_40_x\" to vita_app",
                "comment on database \"vita_40_x\" is 'ab''c'",
            ]
        );
    }
}
