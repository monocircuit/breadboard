//! The commands that end or rearrange work (#314, group "lifecycle"):
//! `COMMANDS` below, and the handover, which people and
//! the overview call as `wt:orchestrate <name> <issue> --to <agent>`
//! (`--after !<mr>…`): that call comes here, every other `wt:orchestrate`
//! goes on to the setup group. A change for an MR is `wt:action change`, the
//! write group's.
//!
//! Output keeps today's contract: `▸ <step>` on stdout, a failure as
//! `✗ <reason>` at the start of a line on stderr and exit 1 (other processes
//! parse it: the overview's ✕, wt:orchestrate's undo).

mod args;
mod changelog;
pub mod claude;
mod gitlab;
pub mod handover;
pub mod herdr;
pub mod layout;
pub mod plugin;
pub mod proc;
mod prune;
pub mod remove;
pub mod repo;
pub mod retire;

use std::process::ExitCode;

use args::Args;

/// A command: its name, its internal name and what runs it.
type Command = (&'static str, Option<&'static str>, fn(&Args) -> proc::Done);

/// The group's commands, sorted, one per line (#420: two MRs adding one each
/// change different lines, `cli::tests` checks the order), with today's
/// internal name (`worktree.mjs remove|prune`) that the tabs and the Node
/// overview spawn, the same command (spec §0.1). `herdr` is not taken:
/// `breadboard herdr` is left to `pnpm herdr`, and the plugin's restore
/// calls `wt:herdr --no-focus` (rewrite-log 2026-10-02).
const COMMANDS: &[Command] = &[
    ("wt:herdr", None, layout::command),
    ("wt:prune", Some("prune"), prune::command),
    ("wt:remove", Some("remove"), remove::command),
];

/// The group's commands, as `usage` lists them.
pub fn names() -> Vec<&'static str> {
    COMMANDS.iter().map(|(name, ..)| *name).collect()
}

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(command: &str, words: &[String]) -> Option<ExitCode> {
    let command: fn(&Args) -> proc::Done = match command {
        "wt:orchestrate" | "orchestrate" if retire::is_retire(words) => retire::command,
        "wt:orchestrate" | "orchestrate" if handover::is_handover(words) => handover::command,
        _ => {
            let (.., run) = COMMANDS
                .iter()
                .find(|(name, internal, _)| *name == command || *internal == Some(command))?;
            *run
        }
    };
    // Run from the command line: what it waits for is said (never in the overview's screen).
    proc::say_waits();
    Some(proc::finish(command(&Args::new(words))))
}
