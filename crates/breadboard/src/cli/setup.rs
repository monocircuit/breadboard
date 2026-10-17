//! The commands that set up (#314): `wt:new`, `wt:orchestrate`, `wt:stack`,
//! `stack:up|down|reset|ps`, `wt:check`, `changelog [--watch]`,
//! `changelog:add|unreleased|release|notes|check`, `commands`, `browser`,
//! `init` (#342).
//! Names, flags and wording as today's Node commands (rewrite rules 2, 3, 5);
//! what other groups and core should share is in their own modules here
//! until breadboard-core offers it (herdr, GitLab, local time).

mod browser;
pub(in crate::cli) mod changelog;
mod check;
mod commands;
mod conversations;
pub(in crate::cli) mod epic;
mod fragments;
mod gitlab;
mod herdr;
mod hook;
pub(crate) mod init;
mod new;
mod orchestrate;
mod proc;
mod repo;
mod stack;
mod sweep;
mod view;

pub(super) use proc::PLUGIN_CHECKED;
use proc::{Args, Shape, outln, refuse_misuse, wants_help};
use std::process::ExitCode;

/// What each command takes (flags, those with a value, how many words) and
/// its usage: anything else is refused before the command does anything (a
/// typo of `--shared` would make a worktree on its own stack and push its
/// claim; `wt:check foo` would check what `foo` doesn't name).
fn shape_of(command: &str) -> Option<Shape> {
    let shape = |flags, valued, words, usage: &str| Shape {
        flags,
        valued,
        words,
        usage: usage.to_string(),
    };
    Some(match command {
        "wt:new" => shape(
            &["--shared", "--fix", "--origin", "--base"],
            &["--origin", "--base"],
            Some(1),
            new::USAGE,
        ),
        "wt:orchestrate" => shape(
            &[
                "--title",
                "--auto",
                "--epic",
                "--product",
                "--shared",
                "--peer",
                "--brief",
                "--queue",
                "--after",
                "--dry-run",
                "--to",
                "--limit",
                "--end",
                "--changelog",
                "--watch",
                "--changes",
                "--fresh",
                // What the overview's own start passes; `--cap` says it is gone (#229).
                "--now",
                "--cap",
                // The overview's retirement of a full agent at its merge (#381).
                "--retire",
                "--mr",
            ],
            &[
                "--title",
                "--auto",
                "--epic",
                "--product",
                "--peer",
                "--brief",
                "--to",
                "--limit",
                "--retire",
                "--mr",
            ],
            None,
            orchestrate::USAGE,
        ),
        "wt:stack" => shape(&[], &[], Some(0), "usage: pnpm wt:stack"),
        "wt:check" => shape(&[], &[], Some(0), "usage: pnpm wt:check"),
        "stack:up" | "stack:down" | "stack:reset" | "stack:ps" => shape(
            &["--port"],
            &["--port"],
            Some(1),
            "usage: pnpm stack:up|down|reset|ps [--port <port> | <feature>]",
        ),
        "changelog" => shape(
            &["--since", "--watch"],
            &["--since"],
            Some(0),
            "usage: pnpm changelog [--since 30d|2w|<date>] [--watch]",
        ),
        "changelog:add" => shape(&["--type", "--area"], &["--type", "--area"], None, fragments::USAGE),
        "changelog:check" => shape(&["--mr"], &[], Some(0), fragments::USAGE),
        "changelog:unreleased" => shape(&[], &[], Some(0), fragments::USAGE),
        "changelog:release" | "changelog:notes" => shape(&[], &[], Some(1), fragments::USAGE),
        "commands" => shape(&["--all"], &[], Some(0), "usage: pnpm commands [--all]"),
        "browser" => shape(&[], &[], Some(0), "usage: pnpm browser"),
        "hook" => shape(&[], &[], Some(3), hook::USAGE),
        "init" => shape(&["--check"], &[], Some(0), init::USAGE),
        _ => return None,
    })
}

/// The name of the root package.json for the internal names that the herdr
/// plugin, tabs and the Node overview spawn today (`worktree.mjs control up`,
/// `platform-changelog check`, `new`, …), with the arguments after it.
fn public_of<'a>(command: &str, args: &'a [String]) -> Option<(String, &'a [String])> {
    let sub = || {
        args.first()
            .filter(|sub| !sub.starts_with("--"))
            .cloned()
            .unwrap_or_default()
    };
    let rest = args.get(1..).unwrap_or_default();
    Some(match command {
        "new" | "orchestrate" | "stack" | "check" => (format!("wt:{command}"), args),
        "control" if !sub().is_empty() => (format!("stack:{}", sub()), rest),
        "control" => ("stack:".to_string(), args),
        "platform-changelog" if !sub().is_empty() => (format!("changelog:{}", sub()), rest),
        "platform-changelog" => ("changelog:".to_string(), args),
        _ => return None,
    })
}

/// #230: a `wt:*` or `stack:*` run inside herdr finds the plugin linked
/// from a path that is gone and links it again (lifecycle's heal, once).
/// Each command calls it once nothing refused: a usage error asks herdr
/// nothing; a child of this binary (`proc::child`) leaves it to its parent.
pub fn heal_plugin() {
    if std::env::var_os(proc::PLUGIN_CHECKED).is_none()
        && let Some(main) = repo::find_main()
    {
        super::lifecycle::plugin::heal(&main);
    }
}

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let (command, args) = public_of(command, args).unwrap_or_else(|| (command.to_string(), args));
    let command = command.as_str();
    let args = Args::new(args);
    if let Some(shape) = shape_of(command) {
        if wants_help(&args) {
            outln!("{}", breadboard_core::config::in_repo(&shape.usage));
            return Some(ExitCode::SUCCESS);
        }
        refuse_misuse(&args, &shape);
    }
    Some(match command {
        "wt:new" => new::command(&args),
        "wt:orchestrate" => orchestrate::command(&args),
        "wt:stack" => stack::wt_stack(),
        _ if command.starts_with("stack:") => stack::control(&command["stack:".len()..], &args),
        "wt:check" => check::command(),
        "changelog" => changelog::overall_command(&args),
        _ if command.starts_with("changelog:") => fragments::command(&command["changelog:".len()..], &args),
        "commands" => commands::command(args.has("--all")),
        "browser" => browser::command(),
        "hook" => hook::command(&args.all),
        "init" => init::command(&args),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn takes_the_internal_names_of_today() {
        let args = strings(&["up", "--port", "3002"]);
        let (name, rest) = public_of("control", &args).unwrap();
        assert_eq!((name.as_str(), rest), ("stack:up", &args[1..]));
        let args = strings(&["check", "--mr"]);
        assert_eq!(public_of("platform-changelog", &args).unwrap().0, "changelog:check");
        let args = strings(&["12-x", "--shared"]);
        assert_eq!(public_of("new", &args).unwrap(), ("wt:new".to_string(), &args[..]));
        assert_eq!(public_of("control", &[]).unwrap().0, "stack:");
        assert!(public_of("wt:new", &args).is_none());
    }

    #[test]
    fn every_command_names_its_flags() {
        for command in [
            "wt:new",
            "wt:orchestrate",
            "wt:stack",
            "wt:check",
            "stack:up",
            "changelog",
            "changelog:add",
            "changelog:check",
            "changelog:release",
            "commands",
            "browser",
        ] {
            let shape = shape_of(command).unwrap();
            assert!(shape.usage.starts_with("usage:"), "{command}");
            assert!(shape.valued.iter().all(|flag| shape.flags.contains(flag)), "{command}");
        }
    }
}
