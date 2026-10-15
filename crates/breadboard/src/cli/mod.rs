//! `breadboard wt:<cmd>` / `stack:<cmd>` and the other commands of the root package.json.
//! One module per group of commands; each owns its commands' parsing and output.

pub mod lifecycle;
mod read;
mod setup;
mod typo;
mod write;

use std::io::Write;
use std::process::ExitCode;

/// The commands, as the root package.json names them, by what they work on.
const COMMANDS: [&[&str]; 7] = [
    &["overview"],
    &[
        "wt:new",
        "wt:orchestrate",
        "wt:plan",
        "wt:list",
        "wt:issues",
        "wt:stack",
        "wt:check",
    ],
    &["wt:mr", "wt:note", "wt:tick", "wt:action", "wt:wait", "wt:journal"],
    &["wt:prune", "wt:remove", "wt:herdr"],
    &["stack:up", "stack:down", "stack:reset", "stack:ps"],
    &[
        "changelog",
        "changelog:add",
        "changelog:unreleased",
        "changelog:release",
        "changelog:notes",
        "changelog:check",
    ],
    &["commands", "browser", "herdr", "herdr:install", "init"],
];

/// The folder of breadboard's files (herdr plugin, nvim Lua, Chromium
/// extension) for the repo of the main checkout `main` (init.rs).
pub fn files_dir(main: &std::path::Path) -> std::path::PathBuf {
    setup::init::files_dir(main)
}

/// The usage and every command, a group per line.
pub fn usage() -> String {
    let lines: Vec<String> = COMMANDS.iter().map(|group| format!("  {}", group.join("  "))).collect();
    format!(
        "usage: breadboard <command> [args…]   (in the repo: pnpm <command> [args…])\n\n{}\n\n  what each does: {run} commands",
        lines.join("\n"),
        run = breadboard_core::config::run_word()
    )
}

/// The command a mistyped one likely is: a typo of it, its start
/// (`wt:act`), or its name without the prefix (`action`, `up`).
fn meant(command: &str) -> Option<&'static str> {
    let all: Vec<&'static str> = COMMANDS.iter().flat_map(|group| group.iter().copied()).collect();
    typo::nearest(command, &all).or_else(|| {
        let mut short = all
            .iter()
            .filter(|name| name.split_once(':').is_some_and(|(_, rest)| rest == command));
        short.next().filter(|_| short.next().is_none()).copied()
    })
}

/// `breadboard` alone: the usage on stderr, exit 1.
pub fn bare() -> ExitCode {
    let _ = writeln!(std::io::stderr(), "{}", usage());
    ExitCode::FAILURE
}

/// `overview`'s exit code when its command line is refused or asks for help;
/// None: the TUI runs it, live or printed once (cli/read/overview.rs).
pub fn overview_refused(args: &[String]) -> Option<ExitCode> {
    read::overview_refused(args)
}

pub fn main(command: &str, args: &[String]) -> ExitCode {
    if matches!(command, "--help" | "-h" | "help") {
        let _ = writeln!(std::io::stdout(), "{}", usage());
        return ExitCode::SUCCESS;
    }
    if let Some(code) = read::run(command, args) {
        return code;
    }
    if let Some(code) = write::run(command, args) {
        return code;
    }
    if let Some(code) = lifecycle::run(command, args) {
        return code;
    }
    if let Some(code) = setup::run(command, args) {
        return code;
    }
    // As lib/proc.mjs `fail`: an empty line, `✗ …`, then what there is.
    let hint = meant(command)
        .map(|name| format!(": did you mean {name}?"))
        .unwrap_or_default();
    let _ = writeln!(std::io::stderr(), "\n✗ unknown command {command}{hint}\n\n{}", usage());
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_which_command_was_meant() {
        assert_eq!(meant("wt:actoin"), Some("wt:action"));
        assert_eq!(meant("overvieww"), Some("overview"));
        assert_eq!(meant("wt:act"), Some("wt:action"));
        assert_eq!(meant("action"), Some("wt:action"));
        assert_eq!(meant("up"), Some("stack:up"));
        assert_eq!(meant("frobnicate"), None);
        assert!(usage().contains("  wt:mr  wt:note  wt:tick  wt:action  wt:wait  wt:journal\n"));
    }
}
