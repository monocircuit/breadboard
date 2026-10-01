//! What every command of the group shares: the command line as `worktree.mjs`
//! reads it (`flagValue`), `fail` and the repo the cwd stands in.

use breadboard_core::git;
use std::path::PathBuf;
use std::process::ExitCode;

/// `✗ <message>` on stderr after an empty line, exit 1 (proc.mjs `fail`).
pub fn fail(message: &str) -> ExitCode {
    eprintln!("\n✗ {message}");
    ExitCode::FAILURE
}

/// The value of `--flag <value>` or `--flag=<value>` (proc.mjs `flagValue`):
/// only the first `--flag` counts, and a next argument that is a flag is no
/// value.
pub fn flag_value(args: &[String], name: &str) -> Option<String> {
    if let Some(at) = args.iter().position(|arg| arg == name) {
        return args.get(at + 1).filter(|next| !next.starts_with("--")).cloned();
    }
    let prefix = format!("{name}=");
    args.iter().find_map(|arg| arg.strip_prefix(&prefix)).map(String::from)
}

/// The first argument, unless it is a flag (`worktree.mjs`: the feature).
pub fn first(args: &[String]) -> Option<&str> {
    args.first().map(String::as_str).filter(|arg| !arg.starts_with("--"))
}

/// The directory the command was typed in.
pub fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// The main checkout of the repo the cwd stands in, or the error that says so.
pub fn main_dir() -> Result<PathBuf, ExitCode> {
    git::main_dir(&cwd()).ok_or_else(|| {
        fail("not in a git checkout of the platform repo: run it in the main checkout or one of its worktrees")
    })
}

/// `text` padded with spaces to `width` characters (`padEnd`).
pub fn pad(text: &str, width: usize) -> String {
    let length = text.chars().count();
    format!("{text}{}", " ".repeat(width.saturating_sub(length)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn a_flag_value_is_the_next_argument_or_after_the_equals_sign() {
        let args = strings(&["x", "--why", "because", "--why", "later"]);
        assert_eq!(flag_value(&args, "--why").as_deref(), Some("because"));
        assert_eq!(flag_value(&strings(&["--why=so"]), "--why").as_deref(), Some("so"));
        assert_eq!(flag_value(&strings(&["--why="]), "--why").as_deref(), Some(""));
        assert_eq!(flag_value(&strings(&["--why", "--work"]), "--why"), None);
        assert_eq!(flag_value(&strings(&["--why"]), "--why"), None);
        assert_eq!(flag_value(&strings(&["--whys", "x"]), "--why"), None);
    }

    #[test]
    fn the_first_argument_counts_unless_it_is_a_flag() {
        assert_eq!(first(&strings(&["breadboard", "--work"])), Some("breadboard"));
        assert_eq!(first(&strings(&["--work", "3"])), None);
        assert_eq!(first(&[]), None);
    }

    #[test]
    fn pads_by_characters() {
        assert_eq!(pad("○ x", 5), "○ x  ");
        assert_eq!(pad("long", 2), "long");
    }
}
