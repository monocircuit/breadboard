//! The vita dev stacks (stack.mjs, commands/stack.mjs, commands/control.mjs):
//! `wt:stack` gives a worktree its own, `stack:up|down|reset|ps` act on the
//! one of a port, a feature or the checkout you are in; the summary wt:new
//! and wt:stack print.

use super::herdr;
use super::proc::{self, Args, How, fail, outln, step};
use super::repo::{self, PORTS, VITA};
use breadboard_core::record;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

/// What a new own stack's `.env.local` says: the main checkout's with every
/// port moved by `offset` and a compose project of its own.
pub fn stack_env(source: &str, feature: &str, offset: u16) -> String {
    let skip: Vec<&str> = std::iter::once("COMPOSE_PROJECT_NAME")
        .chain(PORTS.iter().filter_map(|(_, var)| *var))
        .collect();
    let mut lines: Vec<String> = source
        .split('\n')
        .filter(|line| !line.find('=').is_some_and(|at| skip.contains(&&line[..at])))
        .map(|line| {
            let Some(at) = line.find('=').filter(|_| !line.trim_start().starts_with('#')) else {
                return line.to_string();
            };
            let mut value = line[at + 1..].to_string();
            for (port, _) in PORTS {
                value = move_port(&value, port, port + offset);
            }
            format!("{}={value}", &line[..at])
        })
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.push(String::new());
    lines.push(format!(
        "# --- own dev stack of the worktree {feature}, written by breadboard (pnpm wt:new, wt:stack) ---"
    ));
    lines.push(format!("COMPOSE_PROJECT_NAME=vita-{feature}"));
    for (port, var) in PORTS {
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

/// The smallest offset whose ports are free and that no worktree's (stopped) stack claims.
fn free_offset() -> u16 {
    let claimed: Vec<u16> = repo::worktrees()
        .iter()
        .map(|tree| repo::offset_of(&tree.dir))
        .collect();
    (1..50u16)
        .filter(|offset| !claimed.contains(offset))
        .find(|offset| {
            PORTS
                .iter()
                .all(|(port, _)| repo::is_port_free(u32::from(*port) + u32::from(*offset)))
        })
        .unwrap_or_else(|| fail("no free ports found for a stack"))
}

/// The ports of a stack on `offset`, as the lines about it name them.
fn ports_text(offset: u16) -> String {
    let at = |port: u32| port + u32::from(offset);
    format!(
        "app {}, Postgres {}, Keycloak {}, SeaweedFS {}",
        at(3000),
        at(5432),
        at(8080),
        at(8333)
    )
}

/// Moves a worktree from the shared stack to an own one and sets it up. One
/// that has its own already is set up again on it: a `pnpm dev:setup` that
/// failed is finished by running `pnpm wt:stack` once more.
pub fn create_stack(dir: &Path, feature: &str) -> u16 {
    let existing = repo::offset_of(dir);
    if existing != 0 {
        step(&format!(
            "own stack on offset {existing} already: {}, set up again",
            ports_text(existing)
        ));
        dev_setup(dir);
        return existing;
    }
    let source = repo::env_file_of(&repo::main_dir());
    let target = dir.join(VITA).join(".env.local");
    let Ok(text) = fs::read_to_string(&source) else {
        fail(&format!(
            "{} is missing, set up the main checkout first",
            source.display()
        ))
    };
    if fs::read_to_string(&target).is_ok_and(|current| current != text) {
        fail(&format!(
            "{} differs from the one of the main checkout and is no stack's, move it away first",
            target.display()
        ));
    }
    let offset = free_offset();
    step(&format!("own stack on offset {offset}: {}", ports_text(offset)));
    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(error) = fs::write(&target, stack_env(&text, feature, offset)) {
        fail(&format!("couldn't write {}: {error}", target.display()));
    }
    dev_setup(dir);
    offset
}

/// `pnpm dev:setup` in vita: containers, migrations, bucket, Keycloak, seed.
/// It may fail; the hint says how to finish it.
fn dev_setup(dir: &Path) {
    step("pnpm dev:setup (containers, migrations, bucket, Keycloak, seed)");
    let before = proc::hint();
    proc::set_hint(&format!(
        "the stack is only half set up: fix the cause, then run pnpm wt:stack in {}, it sets the stack up again",
        dir.display()
    ));
    proc::run(
        "pnpm",
        &["dev:setup"],
        How {
            cwd: Some(&dir.join(VITA)),
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
        }
    }
}

pub fn summary_text(summary: &Summary, is_agent: bool, label: impl Fn(&str) -> String) -> String {
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
    } = summary;
    let dir = dir.display();
    let port = 3000 + u32::from(*offset);
    let herdr = tab.as_ref().map(|tab| format!(", herdr: {tab}")).unwrap_or_default();
    if let Some(origin) = origin {
        let own = if *offset > 0 {
            format!(", own stack +{offset}")
        } else {
            String::new()
        };
        return format!(
            "\n✓ {branch} pushed, worktree ready for {}{herdr}{own}\n",
            label(origin)
        );
    }
    let mut sweep = String::new();
    if *stale > 0 {
        let (trees, branches) = if *stale == 1 {
            ("worktree", "branch")
        } else {
            ("worktrees", "branches")
        };
        sweep.push_str(&format!(
            "\n  {stale} {trees}/{branches} of merged work left: pnpm wt:prune"
        ));
    }
    if *silent > 0 {
        let issues = if *silent == 1 { "issue" } else { "issues" };
        sweep.push_str(&format!(
            "\n  {silent} {issues} of merged work without a comment since the merge: pnpm wt:issues"
        ));
    }
    if is_agent && *is_stack_only {
        return format!(
            "\n✓ own stack +{offset} ready: app {port}, Postgres {}{herdr}\n",
            5432 + u32::from(*offset)
        );
    }
    if is_agent {
        let enter = match left {
            Some(line) => format!("ExitWorktree(action: \"keep\"), then EnterWorktree(path: \"{dir}\")\n  {line}"),
            None => format!("EnterWorktree(path: \"{dir}\")"),
        };
        let own = if *offset > 0 {
            format!(
                "\n  own stack +{offset}: app {port}, Postgres {}",
                5432 + u32::from(*offset)
            )
        } else {
            String::new()
        };
        return format!("\n✓ {branch} pushed, worktree ready{herdr}\n  {enter}{own}{sweep}\n");
    }
    let base = Path::new(&dir.to_string())
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stack = if *is_stackless {
        "none: the main checkout has no .env.local (pnpm wt:stack once it has)".to_string()
    } else if *offset > 0 {
        format!(
            "own: Postgres {}, Keycloak {}, SeaweedFS {}",
            5432 + u32::from(*offset),
            8080 + u32::from(*offset),
            8333 + u32::from(*offset)
        )
    } else {
        "shared with the main checkout (pnpm wt:stack for an own one)".to_string()
    };
    let head = tab.as_ref().map(|tab| format!("\n▸ herdr: {tab}")).unwrap_or_default();
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
    let vita = if *is_stackless {
        "no stack, so no pnpm dev yet".to_string()
    } else {
        format!("cd {VITA} && pnpm dev{flag}   → http://localhost:{port}{only}")
    };
    format!(
        "{head}\n✓ {base} is ready\n\n  directory  {dir}\n  branch     {branch} (from origin/develop)\n  stack      {stack}\n\n  cd {dir} && claude\n  agent: EnterWorktree(path: \"{dir}\")   ← move the session, don't cd\n  vita:  {vita}{sweep}\n"
    )
}

/// Prints the summary.
pub fn summary(summary: &Summary) {
    let main = repo::main_dir();
    outln!(
        "{}",
        summary_text(summary, proc::is_agent(), |name| record::orchestrator_label(
            &main, name
        ))
    );
}

/// A new own stack's app in the browser, in the tab group of its space (herdr and the extension only).
pub fn open_app(dir: &Path, offset: u16) {
    if offset == 0 || !herdr::is_in_herdr() || !super::browser::can_group_tabs() {
        return;
    }
    let url = format!("http://localhost:{}", 3000 + u32::from(offset));
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
    let feature = repo::feature_of(top);
    let offset = create_stack(top, &feature);
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
        ..Summary::default()
    });
    ExitCode::SUCCESS
}

const ACTIONS: [&str; 4] = ["up", "down", "reset", "ps"];

/// `pnpm stack:up|down|reset|ps [--port <port> | <feature>]`.
pub fn control(action: &str, args: &Args) -> ExitCode {
    let usage = format!("usage: pnpm stack:{} [--port <port> | <feature>]", ACTIONS.join("|"));
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
        repo::worktree_on_port(number)
            .unwrap_or_else(|| fail(&format!("no worktree has a stack on port {port} (pnpm wt:list)")))
    } else if let Some(name) = &name {
        repo::worktrees()
            .into_iter()
            .find(|tree| tree.dir == repo::dir_of(name) || repo::base_of(&tree.dir) == *name)
            .unwrap_or_else(|| fail(&format!("{name} is not a worktree (pnpm wt:list)")))
    } else {
        let here = repo::cwd();
        let all = repo::worktrees();
        let found = all.iter().rev().find(|tree| repo::is_inside(&here, &tree.dir)).cloned();
        found
            .or_else(|| all.into_iter().next())
            .unwrap_or_else(|| fail("not inside the repo"))
    };
    let dir = tree.dir;
    let base = repo::base_of(&dir);
    if !repo::env_file_of(&dir).exists() {
        fail(&format!("{base} has no .env.local, so no stack"));
    }
    super::heal_plugin();
    let offset = if dir == main { 0 } else { repo::offset_of(&dir) };
    let whose = if dir != main && offset == 0 {
        ", that is the shared stack of the main checkout"
    } else {
        ""
    };
    step(&format!(
        "stack {action}: {base} (app {}, Postgres {}){whose}",
        3000 + u32::from(offset),
        5432 + u32::from(offset)
    ));
    let vita = dir.join(VITA);
    let how = How {
        cwd: Some(&vita),
        ..How::default()
    };
    match action {
        "ps" => proc::run(
            "docker",
            &[
                "compose",
                "--env-file",
                ".env.local",
                "-f",
                "docker-compose.local.yml",
                "ps",
            ],
            how,
        ),
        _ => proc::run("pnpm", &[&format!("stack:{action}")], how),
    };
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

    #[test]
    fn moves_every_port_of_the_env() {
        let source = "APP_ORIGIN=http://localhost:3000\nDATABASE_URL=postgres://u@127.0.0.1:5432/vita\nPOSTGRES_PORT=5432\nX=localhost:30001\n# localhost:3000\n\n";
        let env = stack_env(source, "40-x", 3);
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
            summary_text(&origin, false, |name| format!("⇶ {name}")),
            "\n✓ feature/40-x pushed, worktree ready for ⇶ b, herdr: opened, own stack +2\n"
        );
        assert_eq!(
            summary_text(
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
            summary_text(&Summary { stale: 1, ..base }, true, |name| name.into()),
            "\n✓ feature/40-x pushed, worktree ready\n  EnterWorktree(path: \"/r/platform-40-x\")\n  own stack +2: app 3002, Postgres 5434\n  1 worktree/branch of merged work left: pnpm wt:prune\n"
        );
        let human = summary_text(
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
    }
}
