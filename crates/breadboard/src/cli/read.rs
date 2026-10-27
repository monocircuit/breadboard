//! The commands that read, and the command line of `overview` (its view,
//! live or printed, is the TUI's).

mod args;
mod ci_watch;
mod gitlab;
mod issues;
mod list;
mod overview;
mod plan;
mod usage;

use std::process::ExitCode;

// wt:new counts the same stale issues next to the stale worktrees (one function, issues.rs).
#[allow(unused_imports)]
pub use issues::{Stale, stale_issues};

/// A command: its name, what it takes and what runs it.
type Command = (&'static str, &'static args::Takes, fn(&[String]) -> ExitCode);

/// The group's commands, sorted, one per line (#420: two MRs adding one each
/// change different lines, `cli::tests` checks the order): what each takes
/// and what runs it once its command line is fine.
const COMMANDS: &[Command] = &[
    ("ci:watch", &ci_watch::TAKES, |_| ci_watch::run()),
    ("gitlab:usage", &usage::TAKES, usage::run),
    ("wt:issues", &issues::TAKES, |_| issues::run()),
    ("wt:list", &list::TAKES, |_| list::run()),
    ("wt:plan", &plan::TAKES, plan::run),
];

/// The group's commands, as `usage` lists them.
pub fn names() -> Vec<&'static str> {
    COMMANDS.iter().map(|(name, ..)| *name).collect()
}

/// `overview`'s exit code when its command line is refused or asks for help, None to run it.
pub fn overview_refused(args: &[String]) -> Option<ExitCode> {
    overview::refused(args)
}

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let (_, takes, run) = COMMANDS.iter().find(|(name, ..)| *name == command)?;
    Some(args::check(args, takes).unwrap_or_else(|| run(args)))
}
