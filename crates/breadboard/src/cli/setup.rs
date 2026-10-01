//! The commands that set up (#314): `wt:new`, `wt:orchestrate`, `wt:stack`,
//! `stack:up|down|reset|ps`, `wt:check`, `changelog [--watch]`,
//! `changelog:add|unreleased|release|notes|check`, `commands`, `browser`.
//! Names, flags and wording as today's Node commands (rewrite rules 2, 3, 5);
//! what other groups and core should share is in their own modules here
//! until breadboard-core offers it (herdr, GitLab, local time).

mod browser;
mod changelog;
mod check;
mod commands;
mod conversations;
mod fragments;
mod gitlab;
mod herdr;
mod localtime;
mod new;
mod nvim;
mod orchestrate;
mod proc;
mod repo;
mod stack;
mod sweep;
mod view;

use proc::{Args, refuse_unknown};
use std::process::ExitCode;

/// The flags each command takes and its usage: anything else is refused
/// before the command does anything (a typo of `--shared` would make a
/// worktree on its own stack and push its claim).
fn flags_of(command: &str) -> Option<(&'static [&'static str], String)> {
    Some(match command {
        "wt:new" => (&["--shared", "--fix", "--origin"], new::USAGE.to_string()),
        "wt:orchestrate" => (
            &[
                "--title",
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
                // What the overview's own start passes; `--cap` says it is gone (#229).
                "--now",
                "--cap",
            ],
            orchestrate::USAGE.to_string(),
        ),
        "wt:stack" => (&[], "usage: pnpm wt:stack".to_string()),
        "wt:check" => (&[], "usage: pnpm wt:check".to_string()),
        "stack:up" | "stack:down" | "stack:reset" | "stack:ps" => (
            &["--port"],
            "usage: pnpm stack:up|down|reset|ps [--port <port> | <feature>]".to_string(),
        ),
        "changelog" => (
            &["--since", "--watch"],
            "usage: pnpm changelog [--since 30d|2w|<date>] [--watch]".to_string(),
        ),
        "changelog:add" => (&["--type", "--area"], fragments::USAGE.to_string()),
        "changelog:check" => (&["--mr"], fragments::USAGE.to_string()),
        "changelog:unreleased" | "changelog:release" | "changelog:notes" => (&[], fragments::USAGE.to_string()),
        "commands" => (&["--all"], "usage: pnpm commands [--all]".to_string()),
        "browser" => (&[], "usage: pnpm browser".to_string()),
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

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let (command, args) = public_of(command, args).unwrap_or_else(|| (command.to_string(), args));
    let command = command.as_str();
    let args = Args::new(args);
    if let Some((known, usage)) = flags_of(command) {
        refuse_unknown(&args, known, &usage);
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
            let (_, usage) = flags_of(command).unwrap();
            assert!(usage.starts_with("usage:"), "{command}");
        }
    }
}
