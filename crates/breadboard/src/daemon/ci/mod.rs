//! The CI queue of a repo (#357), run by the daemon while any overview of
//! that repo is open: every EVERY it reads the open MRs and the runner's
//! slots, creates at most one MR pipeline (core's `ci::decide`) and writes
//! where each MR stands to `.claude/ci/queue.json` for the rows. The user
//! asked for these writes (#357, rewrite rule 6): a pipeline created, its
//! superseded one cancelled, nothing else; only for the orchestrations' MRs,
//! one per commit, read back from GitLab right before (#369). What it asked
//! for is kept in `.claude/ci/asked.json`, one daemon per checkout schedules.
//! Off with `git config breadboard.ci push` (the kill switch): pushes start
//! their pipelines themselves then. With `BREADBOARD_CI_DRY` set (a try) it
//! decides and says so, and writes nothing to GitLab.
//!
//! One module per step (#420), so CI features touch different files:
//! `queue` the loop, `start` a pipeline started, `preempt` room made for an
//! MR ahead, `clicks` the run-now clicks, `fix` a failed pipeline (#371); the job watcher (`watch.rs`) and
//! ready (`ready.rs`) are the daemon's own tasks.

mod clicks;
mod fix;
mod preempt;
mod queue;
mod start;

use breadboard_core::ci::{self, MODE_KEY};
use std::path::Path;
use std::time::Duration;

pub use queue::schedule;

/// How often the queue is looked at; `BREADBOARD_CI_EVERY_MS` another (the gauntlet's ticks).
pub const EVERY: Duration = Duration::from_secs(10);

pub(super) fn every() -> Duration {
    std::env::var("BREADBOARD_CI_EVERY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(EVERY)
}
/// Whether `main` leaves its pipelines to the queue (the default) or to the
/// pushes: `breadboard.ci = push`, or an origin that takes no push options
/// (a gauntlet world's), whose pushes never skip CI (`ci::set_push_option`).
pub fn is_queued(main: &Path) -> bool {
    let git = |args: &[&str]| breadboard_core::git::git(main, args);
    let origin = git(&["remote", "get-url", "--push", "origin"]);
    origin.is_ok && ci::is_network_url(origin.out()) && git(&["config", "--get", MODE_KEY]).out() != "push"
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}
