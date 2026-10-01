//! The commands that read: `wt:list`, `wt:plan`, `wt:issues` and the printed
//! overview (`overview --once`, or any overview whose stdout is no terminal).

mod args;
mod cache;
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
    let takes = match command {
        "wt:list" => &list::TAKES,
        "wt:plan" => &plan::TAKES,
        "wt:issues" => &issues::TAKES,
        "overview" => return overview::run(args),
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
