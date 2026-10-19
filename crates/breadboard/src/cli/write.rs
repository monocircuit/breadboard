//! The commands that write (worktree/commands/*.mjs). They write the files other sessions
//! read (`.claude/actions.json`, the records, `.claude/wait.json`) exactly as
//! today, and GitLab exactly where today's breadboard does (rewrite rules 5, 6).

mod action;
mod batch;
mod boxes;
mod change;
mod common;
mod epic;
mod glab;
mod journal;
mod merge_base;
pub(crate) mod mr;
mod note;
mod tick;
mod wait;

use std::process::ExitCode;

/// A command: its name and what runs it.
type Command = (&'static str, fn(&common::Argv) -> common::Outcome);

/// The group's commands, sorted, one per line (#420: two MRs adding one each
/// change different lines, `cli::tests` checks the order).
const COMMANDS: &[Command] = &[
    ("wt:action", action::run),
    ("wt:batch", batch::run),
    ("wt:epic", epic::run),
    ("wt:journal", journal::run),
    ("wt:merge-base", merge_base::run),
    ("wt:mr", mr::run),
    ("wt:note", note::run),
    ("wt:tick", tick::run),
    ("wt:wait", wait::run),
];

/// The group's commands, as `usage` lists them.
pub fn names() -> Vec<&'static str> {
    COMMANDS.iter().map(|(name, _)| *name).collect()
}

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let (_, run) = COMMANDS.iter().find(|(name, _)| *name == command)?;
    Some(common::finish(run(&common::Argv(args))))
}
