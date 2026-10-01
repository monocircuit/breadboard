//! The commands that end or rearrange work (#314, group "lifecycle"):
//! `wt:remove`, `wt:prune`, `wt:herdr`, and the two that today are reached
//! through other commands, `wt:handover` (`wt:orchestrate <name> <issue> --to
//! <agent>`) and `wt:change` (`wt:action change "<what>"`).
//!
//! Output keeps today's contract: `▸ <step>` on stdout, a failure as
//! `✗ <reason>` at the start of a line on stderr and exit 1 (other processes
//! parse it: the overview's ✕, wt:orchestrate's undo).

mod args;
pub mod change;
mod claude;
mod gitlab;
pub mod handover;
mod herdr;
pub mod layout;
mod proc;
mod prune;
pub mod remove;
mod repo;
mod setups;

use std::process::ExitCode;

use args::Args;

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, args: &[String]) -> Option<ExitCode> {
    let args = Args::new(args);
    let done = match command {
        "wt:remove" => remove::command(&args),
        "wt:prune" => prune::command(&args),
        "wt:herdr" => layout::command(&args),
        "wt:handover" => handover::command(&args),
        "wt:change" => change::command(&args),
        _ => return None,
    };
    Some(proc::finish(done))
}
