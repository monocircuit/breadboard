//! The commands that read: `wt:list`, `wt:plan`, `wt:issues` and the printed
//! overview (`overview --once`, or any overview whose stdout is no terminal).

mod args;
mod gitlab;
mod issues;
mod list;
mod overview;
mod plan;

use std::process::ExitCode;

// wt:new counts the same stale issues next to the stale worktrees (one function, issues.rs).
#[allow(unused_imports)]
pub use issues::{Stale, stale_issues};

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    match command {
        "wt:list" => Some(list::run()),
        "wt:plan" => Some(plan::run(args)),
        "wt:issues" => Some(issues::run()),
        "overview" => overview::run(args),
        _ => None,
    }
}
