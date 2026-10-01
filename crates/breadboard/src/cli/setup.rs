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

use proc::Args;
use std::process::ExitCode;

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let args = Args::new(args);
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
