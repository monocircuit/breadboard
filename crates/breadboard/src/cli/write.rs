//! The commands that write: `wt:action`, `wt:tick`, `wt:note`, `wt:mr`,
//! `wt:wait`, `wt:journal` (worktree/commands/*.mjs). They write the files other sessions
//! read (`.claude/actions.json`, the records, `.claude/wait.json`) exactly as
//! today, and GitLab exactly where today's breadboard does (rewrite rules 5, 6).

mod action;
mod boxes;
mod change;
mod common;
mod epic;
mod glab;
mod journal;
pub(super) mod mr;
mod note;
mod tick;
mod wait;

use std::process::ExitCode;

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let argv = common::Argv(args);
    let outcome = match command {
        "wt:action" => action::run(&argv),
        "wt:tick" => tick::run(&argv),
        "wt:note" => note::run(&argv),
        "wt:mr" => mr::run(&argv),
        "wt:wait" => wait::run(&argv),
        "wt:epic" => epic::run(&argv),
        "wt:journal" => journal::run(&argv),
        _ => return None,
    };
    Some(common::finish(outcome))
}
