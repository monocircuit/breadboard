//! `pnpm snapshot [--json]` (#497): what this node runs, as lab's server
//! will get it (`breadboard_core::node`): its repo's orchestrations,
//! worktrees, agents, runs, actions, checks and batch. `--json` prints the
//! snapshot itself, else a line of counts.

use super::args::{Takes, cwd, fail};
use breadboard_core::node::{Node, RepoFacts, Snapshot, host, snapshot_of};
use breadboard_core::time::now_ms;
use std::process::ExitCode;

pub const TAKES: Takes = Takes {
    valued: &[],
    lists: &[],
    optional: &[],
    switches: &["--json"],
    words: 0,
    about: "What this node runs, as lab's server gets it: orchestrations, worktrees, agents, runs, actions, checks, batch (--json: the snapshot itself).",
    usage: "usage: pnpm snapshot [--json]",
};

pub fn run(args: &[String]) -> ExitCode {
    let now = now_ms();
    let Some(facts) = RepoFacts::read(&cwd(), now) else {
        return fail("not in a git checkout of the platform repo: run it in the main checkout or one of its worktrees");
    };
    let node = Node {
        host: host(),
        user: std::env::var("USER").unwrap_or_default(),
        version: format!("{} {}", env!("CARGO_PKG_VERSION"), crate::source_key())
            .trim_end()
            .to_string(),
    };
    let snapshot = snapshot_of(node, &[facts], now);
    if args.iter().any(|arg| arg == "--json") {
        match serde_json::to_string_pretty(&snapshot) {
            Ok(text) => println!("{text}"),
            Err(error) => return fail(&format!("the snapshot didn't serialize: {error}")),
        }
    } else {
        println!("{}", counts(&snapshot));
    }
    ExitCode::SUCCESS
}

/// `platform: 2 orchestrations · 9 worktrees · 5 agents · 3 runs · 4 actions`.
fn counts(snapshot: &Snapshot) -> String {
    snapshot
        .repos
        .iter()
        .map(|repo| {
            let actions = repo.worktrees.iter().map(|tree| tree.actions.len()).sum::<usize>()
                + repo.orchestrations.iter().map(|it| it.actions.len()).sum::<usize>();
            format!(
                "{}: {} orchestrations · {} worktrees · {} agents · {} runs · {} actions",
                repo.name,
                repo.orchestrations.len(),
                repo.worktrees.len(),
                repo.agents.len(),
                repo.runs.runs.len(),
                actions
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
