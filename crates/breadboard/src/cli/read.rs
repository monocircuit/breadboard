//! The commands that read: `wt:list`, `wt:plan`, `wt:issues`, and the
//! command line of `overview` (its view, live or printed, is the TUI's).

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

/// `overview`'s exit code when its command line is refused or asks for help, None to run it.
pub fn overview_refused(args: &[String]) -> Option<ExitCode> {
    overview::refused(args)
}

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let takes = match command {
        "wt:list" => &list::TAKES,
        "wt:plan" => &plan::TAKES,
        "wt:issues" => &issues::TAKES,
        _ => return None,
    };
    if let Some(code) = args::check(args, takes) {
        return Some(code);
    }
    Some(match command {
        "wt:list" => list::run(),
        "wt:plan" => plan::run(args),
        _ => issues::run(),
    })
}
