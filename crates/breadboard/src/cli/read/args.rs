//! What every command of the group shares: the command line as `worktree.mjs`
//! reads it (`flagValue`), `fail` and the repo the cwd stands in.

use breadboard_core::git;
use std::path::PathBuf;
use std::process::ExitCode;

/// `✗ <message>` on stderr after an empty line, exit 1 (proc.mjs `fail`).
pub fn fail(message: &str) -> ExitCode {
    eprintln!("\n✗ {}", breadboard_core::config::in_repo(message));
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

/// What a command takes on its command line.
pub struct Takes {
    /// The flags followed by a value (`--flag value` or `--flag=value`).
    pub valued: &'static [&'static str],
    /// The flags followed by a list, up to the next flag (`--work 3 4`); no `=` form.
    pub lists: &'static [&'static str],
    /// The flags whose value may be left out (`--worktree` alone is the cwd).
    pub optional: &'static [&'static str],
    /// The flags that stand alone.
    pub switches: &'static [&'static str],
    /// How many words (no flag, no flag's value) at most.
    pub words: usize,
    /// What the command does, for `--help`.
    pub about: &'static str,
    pub usage: &'static str,
}

/// The command line held against what the command takes, before anything is
/// read: `--help` / `-h` prints what it does and its usage (exit 0); an
/// unknown flag, a valued flag without its value, a switch given one, a
/// list flag in the `=` form or a word too many is refused with the usage
/// (exit 1). None: run it. Node read the flags it knew and ignored the rest:
/// `wt:plan x --work=3` wrote nothing and said nothing.
pub fn check(args: &[String], takes: &Takes) -> Option<ExitCode> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "{}\n\n{}",
            breadboard_core::config::in_repo(takes.about),
            breadboard_core::config::in_repo(takes.usage)
        );
        return Some(ExitCode::SUCCESS);
    }
    problem_of(args, takes)
        .map(|problem| fail(&format!("{problem}\n{}", breadboard_core::config::in_repo(takes.usage))))
}

fn problem_of(args: &[String], takes: &Takes) -> Option<String> {
    let mut words = 0;
    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        at += 1;
        if !arg.starts_with("--") {
            words += 1;
            if words > takes.words {
                return Some(format!("{arg} is one argument too many"));
            }
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (arg, None),
        };
        if takes.lists.contains(&name) {
            if let Some(value) = inline {
                return Some(format!("{name} takes its issues as separate arguments: {name} {value}"));
            }
            while args.get(at).is_some_and(|next| !next.starts_with("--")) {
                at += 1;
            }
        } else if takes.optional.contains(&name) {
            if inline.is_none() && args.get(at).is_some_and(|next| !next.starts_with("--")) {
                at += 1;
            }
        } else if takes.valued.contains(&name) {
            // `--flag value`, but a next flag is no value (proc.mjs `flagValue`).
            let has_next = args.get(at).is_some_and(|next| !next.starts_with("--"));
            if inline.is_none() && !has_next {
                return Some(format!("{name} needs a value"));
            }
            if inline.is_none() {
                at += 1;
            }
        } else if takes.switches.contains(&name) {
            if inline.is_some() {
                return Some(format!("{name} takes no value"));
            }
        } else {
            return Some(format!("unknown flag {name}"));
        }
    }
    None
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

    const TAKES: Takes = Takes {
        valued: &["--why"],
        lists: &["--work"],
        optional: &["--at"],
        switches: &["--once"],
        words: 1,
        about: "",
        usage: "usage: x",
    };

    #[test]
    fn refuses_what_the_command_does_not_take() {
        let problem = |args: &[&str]| problem_of(&strings(args), &TAKES);
        assert_eq!(problem(&["b", "--work", "3", "4", "--why", "so", "--once"]), None);
        assert_eq!(problem(&["b", "--why=so", "--work"]), None);
        assert_eq!(problem(&["--at", "--once", "b", "--at", "c"]).as_deref(), None);
        assert_eq!(problem(&["b", "--bogus"]).as_deref(), Some("unknown flag --bogus"));
        assert_eq!(problem(&["b", "c"]).as_deref(), Some("c is one argument too many"));
        assert_eq!(problem(&["b", "--why"]).as_deref(), Some("--why needs a value"));
        assert_eq!(problem(&["--once=1"]).as_deref(), Some("--once takes no value"));
        assert_eq!(
            problem(&["b", "--work=3"]).as_deref(),
            Some("--work takes its issues as separate arguments: --work 3")
        );
    }

    #[test]
    fn pads_by_characters() {
        assert_eq!(pad("○ x", 5), "○ x  ");
        assert_eq!(pad("long", 2), "long");
    }
}
