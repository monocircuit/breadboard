//! The dev stacks of breadboard.toml's `[stack]` (stack.mjs, commands/stack.mjs, commands/control.mjs):
//! `wt:stack` gives a worktree its own, `stack:up|down|reset|ps` act on the
//! one of a port, a feature or the checkout you are in; the summary wt:new
//! and wt:stack print.

use super::epic::{self, Epic};
use super::herdr;
use super::proc::{self, Args, How, Shape, fail, outln, step};
use super::repo;
use breadboard_core::config::Stack;
use breadboard_core::record;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

/// What `stack:up|down|reset|ps` take.
pub const SHAPE: Shape = Shape::new(
    &["--port"],
    &["--port"],
    Some(1),
    "usage: pnpm stack:up|down|reset|ps [--port <port> | <feature>]",
);
/// What `wt:stack` takes: nothing.
pub const WT_SHAPE: Shape = Shape::new(&[], &[], Some(0), "usage: pnpm wt:stack");

/// What a new own stack's env file says: the main checkout's with every
/// port moved by `offset` and a compose project of its own.
pub fn stack_env(stack: &Stack, source: &str, feature: &str, offset: u16) -> String {
    let ports: Vec<(u16, Option<String>)> = stack.ports().into_iter().map(|port| (port.port, port.env)).collect();
    let skip: Vec<&str> = std::iter::once(breadboard_core::stack::PROJECT)
        .chain(ports.iter().filter_map(|(_, var)| var.as_deref()))
        .collect();
    let mut lines: Vec<String> = source
        .split('\n')
        .filter(|line| !line.find('=').is_some_and(|at| skip.contains(&&line[..at])))
        .map(|line| {
            let Some(at) = line.find('=').filter(|_| !line.trim_start().starts_with('#')) else {
                return line.to_string();
            };
            let mut value = line[at + 1..].to_string();
            for (port, _) in &ports {
                value = move_port(&value, *port, port + offset);
            }
            format!("{}={value}", &line[..at])
        })
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.push(String::new());
    lines.push(format!(
        "# --- own dev stack of the worktree {feature}, written by breadboard ({run} wt:new, wt:stack) ---",
        run = breadboard_core::config::run_word()
    ));
    lines.push(format!(
        "{}={}-{feature}",
        breadboard_core::stack::PROJECT,
        stack.project
    ));
    for (port, var) in &ports {
        if let Some(var) = var {
            lines.push(format!("{var}={}", port + offset));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// `localhost:<port>` and `127.0.0.1:<port>` (a whole number, `\b`) moved to `to`.
fn move_port(value: &str, port: u16, to: u16) -> String {
    let mut out = String::new();
    let mut rest = value;
    'scan: while !rest.is_empty() {
        for host in ["localhost:", "127.0.0.1:"] {
            let wanted = format!("{host}{port}");
            if let Some(after) = rest.strip_prefix(&wanted)
                && !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            {
                out.push_str(&format!("{host}{to}"));
                rest = after;
                continue 'scan;
            }
        }
        let next = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&rest[..next]);
        rest = &rest[next..];
    }
    out
}

/// The smallest offset whose ports are free and that no checkout's (stopped)
/// stack claims: the worktrees' and the orchestrations' epic stacks.
fn free_offset(stack: &Stack) -> u16 {
    let claimed: Vec<u16> = breadboard_core::git::all_worktrees(&repo::main_dir())
        .iter()
        .map(|tree| repo::offset_of(&tree.dir))
        .collect();
    (1..50u16)
        .filter(|offset| !claimed.contains(offset))
        .find(|offset| {
            stack
                .ports()
                .iter()
                .all(|port| repo::is_port_free(u32::from(port.port) + u32::from(*offset)))
        })
        .unwrap_or_else(|| fail("no free ports found for a stack"))
}

/// The services a line names, by label (`Postgres 5434`): those with a `label`.
fn named(stack: &Stack, offset: u16) -> Vec<String> {
    stack
        .services
        .iter()
        .filter_map(|service| {
            Some(format!(
                "{} {}",
                service.label.as_ref()?,
                u32::from(service.port) + u32::from(offset)
            ))
        })
        .collect()
}

/// The ports of a stack on `offset`, as the lines about it name them.
fn ports_text(stack: &Stack, offset: u16) -> String {
    std::iter::once(format!("app {}", u32::from(stack.app.port) + u32::from(offset)))
        .chain(named(stack, offset))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The app and the first named service: `app 3002, Postgres 5434`.
fn short_text(stack: &Stack, offset: u16) -> String {
    std::iter::once(format!("app {}", u32::from(stack.app.port) + u32::from(offset)))
        .chain(named(stack, offset).into_iter().take(1))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The repo's stack, refused without one.
fn stack_or_fail() -> Stack {
    repo::stack().unwrap_or_else(|| fail("the repo has no stack: breadboard.toml has no [stack]"))
}

/// Moves a worktree from the shared stack to an own one and sets it up. One
/// that has its own already is set up again on it: a setup that failed is
/// finished by running `wt:stack` once more.
pub fn create_stack(dir: &Path, feature: &str) -> u16 {
    let stack = stack_or_fail();
    let existing = repo::offset_of(dir);
    if existing != 0 {
        step(&format!(
            "own stack on offset {existing} already: {}, set up again",
            ports_text(&stack, existing)
        ));
        dev_setup(dir);
        return existing;
    }
    let offset = reserve_stack(dir, feature);
    dev_setup(dir);
    offset
}

/// A checkout's own stack written to its env file, nothing started: the
/// main checkout's with the ports moved and a compose project of its own.
pub fn reserve_stack(dir: &Path, feature: &str) -> u16 {
    let stack = stack_or_fail();
    let source = repo::env_file_of(&repo::main_dir());
    let Ok(text) = fs::read_to_string(&source) else {
        fail(&format!(
            "{} is missing, set up the main checkout first",
            source.display()
        ))
    };
    reserve_with(dir, &text, |offset| stack_env(&stack, &text, feature, offset))
}

/// A free offset taken by writing `env(offset)` as the env file of `dir`,
/// which has none or the main checkout's (`source`).
pub fn reserve_with(dir: &Path, source: &str, env: impl Fn(u16) -> String) -> u16 {
    let stack = stack_or_fail();
    let target = repo::env_file_of(dir);
    if fs::read_to_string(&target).is_ok_and(|current| current != source) {
        fail(&format!(
            "{} differs from the one of the main checkout and is no stack's, move it away first",
            target.display()
        ));
    }
    let offset = free_offset(&stack);
    step(&format!("own stack on offset {offset}: {}", ports_text(&stack, offset)));
    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(error) = fs::write(&target, env(offset)) {
        fail(&format!("couldn't write {}: {error}", target.display()));
    }
    offset
}

/// The stack's setup (`[stack.setup]`: containers, migrations, seed), else
/// compose's `up -d`, in its folder. It may fail; the hint says how to finish it.
pub fn dev_setup(dir: &Path) {
    let stack = stack_or_fail();
    let compose = || {
        let mut args = breadboard_core::stack::compose_args(&stack, &["up", "-d"]);
        args.insert(0, "docker".into());
        args.join(" ")
    };
    let (run, says) = match &stack.setup {
        Some(setup) => (
            setup.run.clone(),
            setup.says.clone().unwrap_or_else(|| setup.run.clone()),
        ),
        None => (compose(), compose()),
    };
    step(&says);
    let before = proc::hint();
    proc::set_hint(&format!(
        "the stack is only half set up: fix the cause, then run {} in {}, it sets the stack up again",
        breadboard_core::config::of(dir).command("wt:stack"),
        dir.display()
    ));
    proc::run(
        "sh",
        &["-c", &run],
        How {
            cwd: Some(&repo::stack_dir(dir)),
            ..How::default()
        },
    );
    proc::set_hint(&before);
}

/// What wt:new and wt:stack say at the end: one line for wt:orchestrate,
/// the facts for an agent, the whole picture for a human.
#[derive(Debug)]
pub struct Summary<'a> {
    pub dir: &'a Path,
    pub branch: &'a str,
    pub offset: u16,
    pub tab: Option<String>,
    pub stale: usize,
    pub silent: usize,
    pub left: Option<String>,
    pub origin: Option<&'a str>,
    /// wt:stack's: nothing was pushed, the agent stands in the worktree.
    pub is_stack_only: bool,
    /// The main checkout had no .env.local: the worktree has no stack, shared or own.
    pub is_stackless: bool,
    /// origin refused the claim: the line says so, never `pushed` (round 7;
    /// today's said `pushed` under its own `! origin refused the branch`).
    pub is_unpushed: bool,
    /// A task stack (#351): the port of its epic's database; it starts on `stack:up`.
    pub epic_port: Option<u32>,
}

impl Default for Summary<'_> {
    fn default() -> Self {
        Summary {
            dir: Path::new(""),
            branch: "",
            offset: 0,
            tab: None,
            stale: 0,
            silent: 0,
            left: None,
            origin: None,
            is_stack_only: false,
            is_stackless: false,
            is_unpushed: false,
            epic_port: None,
        }
    }
}

pub fn summary_text(
    repo: &breadboard_core::config::Loaded,
    summary: &Summary,
    is_agent: bool,
    label: impl Fn(&str) -> String,
) -> String {
    let stack = repo.config.stack.as_ref();
    let Summary {
        dir,
        branch,
        offset,
        tab,
        stale,
        silent,
        left,
        origin,
        is_stack_only,
        is_stackless,
        is_unpushed,
        epic_port,
    } = summary;
    // The worktree's base: its epic's, else the repo's default branch (breadboard.toml).
    let from = breadboard_core::record::base_in(dir).map_or_else(|| repo.base(), |base| format!("origin/{base}"));
    let dir = dir.display();
    let claim = if *is_unpushed {
        format!("{branch} not pushed (git push -u origin {branch})")
    } else {
        format!("{branch} pushed")
    };
    let port = stack.map_or(0, |stack| u32::from(stack.app.port)) + u32::from(*offset);
    let short = stack.map(|stack| short_text(stack, *offset)).unwrap_or_default();
    // A task stack (#351): its database lies on its epic's, the first named service.
    let database = stack
        .and_then(|stack| stack.services.iter().find(|service| service.label.is_some()))
        .map(|service| service.label().to_string())
        .unwrap_or_else(|| "database".into());
    let task = epic_port.filter(|_| *offset > 0);
    let up = repo.command("stack:up");
    let herdr = tab.as_ref().map(|tab| format!(", herdr: {tab}")).unwrap_or_default();
    if let Some(origin) = origin {
        let own = if let Some(epic) = task {
            format!(", task stack +{offset} on the epic's {database} {epic} ({up} starts it)")
        } else if *offset > 0 {
            format!(", own stack +{offset}")
        } else {
            String::new()
        };
        return format!("\n✓ {claim}, worktree ready for {}{herdr}{own}\n", label(origin));
    }
    let mut sweep = String::new();
    if *stale > 0 {
        let (trees, branches) = if *stale == 1 {
            ("worktree", "branch")
        } else {
            ("worktrees", "branches")
        };
        sweep.push_str(&format!(
            "\n  {stale} {trees}/{branches} of merged work left: {run} wt:prune",
            run = breadboard_core::config::run_word()
        ));
    }
    if *silent > 0 {
        let issues = if *silent == 1 { "issue" } else { "issues" };
        sweep.push_str(&format!(
            "\n  {silent} {issues} of merged work without a comment since the merge: {run} wt:issues",
            run = breadboard_core::config::run_word()
        ));
    }
    if is_agent && *is_stack_only {
        if let Some(epic) = task {
            return format!("\n✓ task stack +{offset} ready: app {port}, {database} {epic}{herdr}\n");
        }
        return format!("\n✓ own stack +{offset} ready: {short}{herdr}\n");
    }
    if is_agent {
        let enter = match left {
            Some(line) => format!("ExitWorktree(action: \"keep\"), then EnterWorktree(path: \"{dir}\")\n  {line}"),
            None => format!("EnterWorktree(path: \"{dir}\")"),
        };
        let own = if let Some(epic) = task {
            format!("\n  task stack +{offset}: app {port}, {database} {epic} (the epic's), {up} starts it")
        } else if *offset > 0 {
            format!("\n  own stack +{offset}: {short}")
        } else {
            String::new()
        };
        return format!("\n✓ {claim}, worktree ready{herdr}\n  {enter}{own}{sweep}\n");
    }
    let base = Path::new(&dir.to_string())
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let env = stack.map_or(".env.local", |stack| stack.env_file.as_str());
    let wt_stack = repo.command("wt:stack");
    let stack_line = if stack.is_none() {
        "none: breadboard.toml has no [stack]".to_string()
    } else if *is_stackless {
        format!("none: the main checkout has no {env} ({wt_stack} once it has)")
    } else if let Some(epic) = task {
        let others: Vec<String> = stack
            .map(|stack| named(stack, *offset).into_iter().skip(1).collect())
            .unwrap_or_default();
        let others: String = others.iter().map(|named| format!(", {named}")).collect();
        format!("task: {database} {epic} (the epic's){others}, started by {up}")
    } else if *offset > 0 {
        format!(
            "own: {}",
            stack.map(|stack| named(stack, *offset).join(", ")).unwrap_or_default()
        )
    } else {
        format!("shared with the main checkout ({wt_stack} for an own one)")
    };
    let head = tab.as_ref().map(|tab| format!("\n▸ herdr: {tab}")).unwrap_or_default();
    let unpushed = if *is_unpushed {
        format!(", not pushed: git push -u origin {branch}")
    } else {
        String::new()
    };
    let flag = if *offset > 0 {
        format!(" --port {port}")
    } else {
        String::new()
    };
    let only = if *offset > 0 {
        ""
    } else {
        " (only one checkout at a time)"
    };
    let app = match stack {
        None => String::new(),
        Some(stack) if *is_stackless => format!("\n  {}:  no stack, so no {} yet", stack.project, stack.app.dev),
        Some(stack) => {
            let cd = if stack.dir == "." {
                String::new()
            } else {
                format!("cd {} && ", stack.dir)
            };
            format!(
                "\n  {}:  {cd}{}{flag}   → http://localhost:{port}{only}",
                stack.project, stack.app.dev
            )
        }
    };
    format!(
        "{head}\n✓ {base} is ready\n\n  directory  {dir}\n  branch     {branch} (from {}{unpushed})\n  stack      {stack_line}\n\n  cd {dir} && claude\n  agent: EnterWorktree(path: \"{dir}\")   ← move the session, don't cd{app}{sweep}\n",
        from
    )
}

/// Prints the summary.
pub fn summary(summary: &Summary) {
    let main = repo::main_dir();
    let loaded = breadboard_core::config::of(&main);
    outln!(
        "{}",
        summary_text(&loaded, summary, proc::is_agent(), |name| record::orchestrator_label(
            &main, name
        ))
    );
}

/// A new own stack's app in the browser, in the tab group of its space (herdr and the extension only).
pub fn open_app(dir: &Path, offset: u16) {
    if offset == 0 || !herdr::is_in_herdr() || !super::browser::can_group_tabs() {
        return;
    }
    let Some(stack) = repo::stack() else { return };
    let url = format!("http://localhost:{}", u32::from(stack.app.port) + u32::from(offset));
    let label = herdr::space_label(dir);
    if super::browser::open_in_group(&url, &label) {
        step(&format!("browser: {url} in the tab group {label}"));
    }
}

/// `pnpm wt:stack`: the worktree you are in gets its own stack.
pub fn wt_stack() -> ExitCode {
    let top = proc::git(&["rev-parse", "--show-toplevel"], &repo::cwd())
        .text()
        .to_string();
    let top = Path::new(&top);
    if top == repo::main_dir() {
        fail("run it inside a worktree, the main checkout has the default stack");
    }
    super::heal_plugin();
    if let Some(epic) = Epic::at(top) {
        epic::ensure_epic(&epic);
        let offset = repo::offset_of(top);
        outln!(
            "\n✓ epic stack of {} ready: {}\n",
            epic.name,
            ports_text(&stack_or_fail(), offset)
        );
        return ExitCode::SUCCESS;
    }
    let feature = repo::feature_of(top);
    let offset = match task_epic(top) {
        Some(epic) => {
            if repo::offset_of(top) == 0 {
                epic::reserve_task(top, &feature, &epic);
            }
            epic::task_up(top, &feature, &epic);
            repo::offset_of(top)
        }
        None => create_stack(top, &feature),
    };
    let changed = if herdr::is_in_herdr() {
        let space = herdr::worktree_space(top, false);
        herdr::sync_stack_tabs(top, space)
    } else {
        Vec::new()
    };
    open_app(top, offset);
    let branch = proc::git(&["branch", "--show-current"], top).text().to_string();
    summary(&Summary {
        dir: top,
        branch: &branch,
        offset,
        tab: (!changed.is_empty()).then(|| format!("stack tabs {}", changed.join(" "))),
        is_stack_only: true,
        epic_port: epic::epic_port_of(top),
        ..Summary::default()
    });
    ExitCode::SUCCESS
}

/// The epic a worktree's stack runs on: its stack is a task stack, or it has
/// none yet and an orchestration with a checkout started it. A worktree with
/// a full own stack keeps it.
pub fn task_epic(dir: &Path) -> Option<Epic> {
    match epic::epic_of_stack(dir) {
        Some(name) => Some(Epic::named(&name).unwrap_or_else(|| {
            fail(&format!(
                "the stack of {} is a task stack of the epic {name}, which has no checkout any more",
                dir.display()
            ))
        })),
        None if repo::offset_of(dir) == 0 => Epic::of_task(dir),
        None => None,
    }
}

const ACTIONS: [&str; 4] = ["up", "down", "reset", "ps"];

/// `pnpm stack:up|down|reset|ps [--port <port> | <feature>]`.
pub fn control(action: &str, args: &Args) -> ExitCode {
    let usage = format!(
        "usage: {run} stack:{} [--port <port> | <feature>]",
        ACTIONS.join("|"),
        run = breadboard_core::config::run_word()
    );
    if !ACTIONS.contains(&action) {
        fail(&usage);
    }
    let port = args.value("--port");
    let name = args.words(&["--port"]).into_iter().next();
    if args.has("--port") && port.is_none() {
        fail(&usage);
    }
    let main = repo::main_dir();
    let tree = if let Some(port) = &port {
        let Some(number) = port
            .parse::<u32>()
            .ok()
            .filter(|_| port.chars().all(|c| c.is_ascii_digit()))
        else {
            fail(&format!("--port {port} is not a port"))
        };
        repo::worktree_on_port(number).unwrap_or_else(|| {
            fail(&format!(
                "no worktree has a stack on port {port} ({run} wt:list)",
                run = breadboard_core::config::run_word()
            ))
        })
    } else if let Some(name) = &name {
        breadboard_core::git::all_worktrees(&main)
            .into_iter()
            .find(|tree| tree.dir == repo::dir_of(name) || repo::base_of(&tree.dir) == *name)
            .unwrap_or_else(|| {
                fail(&format!(
                    "{name} is not a worktree ({run} wt:list)",
                    run = breadboard_core::config::run_word()
                ))
            })
    } else {
        let here = repo::cwd();
        // The orchestrations' checkouts too: one of them lies inside the main checkout.
        let all = breadboard_core::git::all_worktrees(&main);
        let found = all.iter().rev().find(|tree| repo::is_inside(&here, &tree.dir)).cloned();
        found
            .or_else(|| all.into_iter().next())
            .unwrap_or_else(|| fail("not inside the repo"))
    };
    let dir = tree.dir;
    let base = repo::base_of(&dir);
    let Some(stack) = repo::stack() else {
        fail("the repo has no stack: breadboard.toml has no [stack]")
    };
    if !repo::env_file_of(&dir).exists() {
        fail(&format!("{base} has no {}, so no stack", stack.env_file));
    }
    super::heal_plugin();
    let offset = if dir == main { 0 } else { repo::offset_of(&dir) };
    let whose = if dir != main && offset == 0 {
        ", that is the shared stack of the main checkout"
    } else {
        ""
    };
    step(&format!(
        "stack {action}: {base} ({}){whose}",
        short_text(&stack, offset)
    ));
    let how = How {
        cwd: Some(&repo::stack_dir(&dir)),
        ..How::default()
    };
    let compose = |args: &[&str]| {
        let mut all = breadboard_core::stack::compose_args(&stack, args);
        all.insert(0, "docker".into());
        all.join(" ")
    };
    let run = |action: &str| match action {
        "ps" => compose(&["ps"]),
        "up" => stack.up.clone().unwrap_or_else(|| compose(&["up", "-d", "--wait"])),
        "down" => stack.down.clone().unwrap_or_else(|| compose(&["down"])),
        _ => stack
            .reset
            .clone()
            .unwrap_or_else(|| format!("{} && {}", compose(&["down", "-v"]), compose(&["up", "-d", "--wait"]))),
    };
    // A task stack (#351) starts on its epic's database, an epic's checkout brings the epic's up.
    let feature = repo::feature_of(&dir);
    let task = epic::epic_of_stack(&dir).map(|_| task_epic(&dir).unwrap_or_else(|| fail("no epic")));
    match (action, &task, Epic::at(&dir)) {
        ("up", Some(epic), _) => epic::task_up(&dir, &feature, epic),
        ("reset", Some(epic), _) => {
            proc::run("sh", &["-c", &compose(&["down", "--volumes"])], how);
            epic::drop_task(&feature, &epic.name);
            epic::task_up(&dir, &feature, epic);
        }
        ("up", None, Some(epic)) => epic::ensure_epic(&epic),
        _ => {
            proc::run("sh", &["-c", &run(action)], how);
        }
    }
    if herdr::is_in_herdr() && action != "ps" {
        let changed = herdr::sync_stack_tabs(&dir, None);
        if !changed.is_empty() {
            step(&format!("herdr: {}", changed.join(" ")));
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::config::{Config, Loaded};

    /// A repo configured as platform is.
    fn repo() -> Loaded {
        let config = Config::parse(
            "[repo]\ndefault_branch = \"develop\"\nrun = \"pnpm\"\n\
             [stack]\ndir = \"products/vita/web\"\nproject = \"vita\"\n[stack.app]\nport = 3000\n\
             [[stack.services]]\nname = \"postgres\"\nlabel = \"Postgres\"\nport = 5432\nenv = \"POSTGRES_PORT\"\n\
             [[stack.services]]\nname = \"keycloak\"\nlabel = \"Keycloak\"\nport = 8080\nenv = \"KEYCLOAK_PORT\"\n\
             [[stack.services]]\nname = \"s3\"\nlabel = \"SeaweedFS\"\nport = 8333\nenv = \"S3_PORT\"\n\
             [[stack.services]]\nname = \"visualizer\"\nport = 64738\nenv = \"VISUALIZER_PORT\"\n\
             [[stack.tools]]\nname = \"studio\"\nport = 4983\nrun = \"x\"\n\
             [[stack.tools]]\nname = \"storybook\"\nport = 6006\nrun = \"y\"\n",
        )
        .unwrap();
        Loaded {
            config,
            main: "/r/platform".into(),
            branch: "develop".into(),
            error: None,
        }
    }

    #[test]
    fn moves_every_port_of_the_env() {
        let source = "APP_ORIGIN=http://localhost:3000\nDATABASE_URL=postgres://u@127.0.0.1:5432/vita\nPOSTGRES_PORT=5432\nX=localhost:30001\n# localhost:3000\n\n";
        let env = stack_env(repo().config.stack.as_ref().unwrap(), source, "40-x", 3);
        assert_eq!(
            env,
            "APP_ORIGIN=http://localhost:3003\nDATABASE_URL=postgres://u@127.0.0.1:5435/vita\nX=localhost:30001\n# localhost:3000\n\n# --- own dev stack of the worktree 40-x, written by breadboard (pnpm wt:new, wt:stack) ---\nCOMPOSE_PROJECT_NAME=vita-40-x\nPOSTGRES_PORT=5435\nKEYCLOAK_PORT=8083\nS3_PORT=8336\nVISUALIZER_PORT=64741\n"
        );
    }

    #[test]
    fn says_what_is_ready() {
        let dir = Path::new("/r/platform-40-x");
        let base = Summary {
            dir,
            branch: "feature/40-x",
            offset: 2,
            ..Summary::default()
        };
        let origin = Summary {
            origin: Some("b"),
            tab: Some("opened".into()),
            ..Summary {
                dir,
                branch: "feature/40-x",
                offset: 2,
                ..Summary::default()
            }
        };
        assert_eq!(
            summary_text(&repo(), &origin, false, |name| format!("⇶ {name}")),
            "\n✓ feature/40-x pushed, worktree ready for ⇶ b, herdr: opened, own stack +2\n"
        );
        assert_eq!(
            summary_text(
                &repo(),
                &Summary {
                    is_stack_only: true,
                    tab: Some("stack tabs +postgres".into()),
                    dir,
                    branch: "feature/40-x",
                    offset: 2,
                    ..Summary::default()
                },
                true,
                |name| name.into()
            ),
            "\n✓ own stack +2 ready: app 3002, Postgres 5434, herdr: stack tabs +postgres\n"
        );
        assert_eq!(
            summary_text(&repo(), &Summary { stale: 1, ..base }, true, |name| name.into()),
            "\n✓ feature/40-x pushed, worktree ready\n  EnterWorktree(path: \"/r/platform-40-x\")\n  own stack +2: app 3002, Postgres 5434\n  1 worktree/branch of merged work left: pnpm wt:prune\n"
        );
        let human = summary_text(
            &repo(),
            &Summary {
                dir,
                branch: "feature/40-x",
                ..Summary::default()
            },
            false,
            |name| name.into(),
        );
        assert!(human.starts_with("\n✓ platform-40-x is ready\n\n  directory  /r/platform-40-x\n"));
        assert!(human.ends_with("→ http://localhost:3000 (only one checkout at a time)\n"));
        // Without the main checkout's .env.local: no stack is promised.
        let stackless = summary_text(
            &repo(),
            &Summary {
                dir,
                branch: "feature/40-x",
                is_stackless: true,
                ..Summary::default()
            },
            false,
            |name| name.into(),
        );
        assert!(stackless.contains("  stack      none: the main checkout has no .env.local"));
        assert!(stackless.ends_with("  vita:  no stack, so no pnpm dev yet\n"));
        assert!(!stackless.contains("shared with the main checkout") && !stackless.contains("localhost"));
        // origin refused the claim: never `pushed`.
        let unpushed = || Summary {
            dir,
            branch: "feature/40-x",
            is_unpushed: true,
            ..Summary::default()
        };
        let agent = summary_text(&repo(), &unpushed(), true, |name| name.into());
        assert!(agent.starts_with("\n✓ feature/40-x not pushed (git push -u origin feature/40-x), worktree ready\n"));
        let human = summary_text(&repo(), &unpushed(), false, |name| name.into());
        assert!(human.contains(
            "  branch     feature/40-x (from origin/develop, not pushed: git push -u origin feature/40-x)\n"
        ));
    }
}
