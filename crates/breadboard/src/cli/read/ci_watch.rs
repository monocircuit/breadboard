//! `breadboard ci:watch` (#379): what the CI job watcher sees now, from the
//! history the daemon keeps and GitLab's newest pipelines: the jobs that hold
//! pipelines up and the ones hung now. It writes and posts nothing; the
//! daemon of the overviews does that every minute.

use super::args::{Takes, main_dir};
use crate::daemon::watch;
use breadboard_core::ci_watch::Report;
use std::process::ExitCode;

pub const TAKES: Takes = Takes {
    valued: &[],
    lists: &[],
    optional: &[],
    switches: &[],
    words: 0,
    about: "What holds the project's pipelines up: the CI job watcher's flags and hung jobs, now.",
    usage: "usage: pnpm ci:watch",
};

/// The report as lines.
pub fn lines_of(report: &Report) -> Vec<String> {
    let mut lines: Vec<String> = report.flags.iter().map(|flag| format!("⚠ {}", flag.line())).collect();
    lines.extend(report.hung.iter().map(|hung| format!("⏳ {}: hung?", hung.line())));
    if lines.is_empty() {
        lines.push("✓ no job holds the pipelines up".into());
    }
    lines
}

pub fn run() -> ExitCode {
    let main = match main_dir() {
        Ok(main) => main,
        Err(code) => return code,
    };
    let Some(report) = watch::once(&main) else {
        eprintln!("✗ GitLab didn't answer: glab api projects/:fullpath/pipelines");
        return ExitCode::FAILURE;
    };
    for line in lines_of(&report) {
        println!("{line}");
    }
    ExitCode::SUCCESS
}
