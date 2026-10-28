//! `breadboard wt:<cmd>` / `stack:<cmd>` and the other commands of the root package.json.
//! One module per group of commands; each owns its commands' parsing and output.

pub mod lifecycle;
mod read;
mod setup;
mod typo;
pub(crate) mod write;

use std::io::Write;
use std::process::ExitCode;

/// The commands main.rs runs itself, as the root package.json names them.
const OWN: &[&str] = &["herdr", "herdr:install", "overview"];

/// Every command, a list per group: each group keeps its own sorted table,
/// so a new command touches its group's file, not this one (#420).
fn groups() -> [Vec<&'static str>; 5] {
    [
        OWN.to_vec(),
        read::names(),
        write::names(),
        lifecycle::names(),
        setup::names(),
    ]
}

/// The folder of breadboard's files (herdr plugin, nvim Lua, Chromium
/// extension) for the repo of the main checkout `main` (init.rs).
pub fn files_dir(main: &std::path::Path) -> std::path::PathBuf {
    setup::init::files_dir(main)
}

/// The usage and every command, a group per line (wrapped at 100 columns).
pub fn usage() -> String {
    let mut lines = Vec::new();
    for group in groups() {
        let mut line = String::new();
        for name in group {
            if !line.is_empty() && line.len() + 2 + name.len() > 100 {
                lines.push(std::mem::take(&mut line));
            }
            line = format!("{line}  {name}");
        }
        lines.push(line);
    }
    format!(
        "usage: breadboard <command> [args…]   (in the repo: pnpm <command> [args…])\n\n{}\n\n  what each does: {run} commands",
        lines.join("\n"),
        run = breadboard_core::config::run_word()
    )
}

/// The command a mistyped one likely is: a typo of it, its start
/// (`wt:act`), or its name without the prefix (`action`, `up`).
fn meant(command: &str) -> Option<&'static str> {
    let all: Vec<&'static str> = groups().into_iter().flatten().collect();
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

/// A command's exit when the repo's breadboard.toml is missing or wrong
/// (#320); `init` alone runs without one, it writes it.
pub fn refused() -> Option<ExitCode> {
    let cwd = std::env::current_dir().ok()?;
    let why = breadboard_core::config::refusal(&cwd)?;
    let _ = writeln!(std::io::stderr(), "\n✗ {why}");
    Some(ExitCode::FAILURE)
}

pub fn main(command: &str, args: &[String]) -> ExitCode {
    if matches!(command, "--help" | "-h" | "help") {
        let _ = writeln!(std::io::stdout(), "{}", usage());
        return ExitCode::SUCCESS;
    }
    if command != "init"
        && groups().iter().flatten().any(|name| *name == command)
        && let Some(code) = refused()
    {
        return code;
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
        assert!(
            usage().contains("\n  status  submit  wt:action  wt:batch  wt:epic  wt:journal  wt:merge-base  wt:mr  wt:note  wt:tick\n  wt:wait\n")
        );
    }

    #[test]
    fn every_group_keeps_its_commands_sorted() {
        for group in groups() {
            assert!(group.windows(2).all(|pair| pair[0] < pair[1]), "{group:?}");
        }
        let all: Vec<&str> = groups().into_iter().flatten().collect();
        let mut unique = all.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), all.len(), "a command in two groups");
    }

    /// #420: two branches that each add a command to the same group, in
    /// neighbouring places of its sorted lists, merge without a conflict.
    #[test]
    fn two_new_commands_merge_cleanly() {
        let base = include_str!("write.rs");
        let add = |lines: &[(&str, &str)]| {
            lines.iter().fold(base.to_string(), |text, (after, line)| {
                assert!(text.contains(after), "{after}");
                text.replacen(after, &format!("{after}\n{line}"), 1)
            })
        };
        let ours = add(&[
            ("mod batch;", "mod bisect;"),
            ("    (\"wt:batch\", batch::run),", "    (\"wt:bisect\", bisect::run),"),
        ]);
        let theirs = add(&[
            ("mod epic;", "mod fold;"),
            ("    (\"wt:epic\", epic::run),", "    (\"wt:fold\", fold::run),"),
        ]);
        let scratch = std::env::temp_dir().join(format!("cli-merge-test-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        for (name, text) in [("ours", &ours), ("base", &base.to_string()), ("theirs", &theirs)] {
            std::fs::write(scratch.join(name), text).unwrap();
        }
        let merged = std::process::Command::new("git")
            .args(["merge-file", "-p", "ours", "base", "theirs"])
            .current_dir(&scratch)
            .output()
            .unwrap();
        std::fs::remove_dir_all(&scratch).unwrap();
        let text = String::from_utf8_lossy(&merged.stdout);
        assert!(merged.status.success(), "{text}");
        for line in [
            "mod bisect;\nmod boxes;",
            "mod fold;\nmod glab;",
            "bisect::run),\n",
            "fold::run),\n",
        ] {
            assert!(text.contains(line), "{line}");
        }
    }
}
