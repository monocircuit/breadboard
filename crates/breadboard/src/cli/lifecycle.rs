//! The commands that end or rearrange work (#314, group "lifecycle"):
//! `wt:remove`, `wt:prune`, `wt:herdr`, and the handover, which people and
//! the overview call as `wt:orchestrate <name> <issue> --to <agent>`
//! (`--after !<mr>…`): that call comes here, every other `wt:orchestrate`
//! goes on to the setup group. A change for an MR is `wt:action change`, the
//! write group's.
//!
//! Output keeps today's contract: `▸ <step>` on stdout, a failure as
//! `✗ <reason>` at the start of a line on stderr and exit 1 (other processes
//! parse it: the overview's ✕, wt:orchestrate's undo).

mod args;
mod claude;
mod gitlab;
pub mod handover;
mod herdr;
pub mod layout;
pub mod plugin;
mod proc;
mod prune;
pub mod remove;
mod repo;
mod setups;

use std::process::ExitCode;

use args::Args;

/// The command's exit code, or None when it isn't one of this group's.
/// Today's internal names (`worktree.mjs remove|prune|orchestrate`), which
/// the tabs and the Node overview spawn, are the same commands (spec §0.1).
/// `herdr` is not taken: `breadboard herdr` is left to `pnpm herdr`, and the
/// plugin's restore calls `wt:herdr --no-focus` (rewrite-log 2026-10-02).
pub fn run(command: &str, words: &[String]) -> Option<ExitCode> {
    let args = Args::new(words);
    let done = match command {
        "wt:remove" | "remove" => remove::command(&args),
        "wt:prune" | "prune" => prune::command(&args),
        "wt:herdr" => layout::command(&args),
        "wt:orchestrate" | "orchestrate" if handover::is_handover(words) => handover::command(&args),
        _ => return None,
    };
    Some(proc::finish(done))
}
