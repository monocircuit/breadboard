//! The words after the command, read as worktree.mjs and proc.mjs read them:
//! the first word is the feature unless it is a flag, a flag is a word
//! starting with `--`, a value follows its flag or comes as `--flag=value`.
//! By hand, not clap: five commands with a handful of flags each, whose
//! usage lines and edge cases (a value starting with `--` counts as missing)
//! must stay today's; clap's would differ (rewrite rules 4 and 5).

use super::proc::{Fail, fail};
use crate::cli::typo::nearest;

#[derive(Debug, Clone, Default)]
pub struct Args {
    words: Vec<String>,
}

impl Args {
    pub fn new(words: &[String]) -> Args {
        Args { words: words.to_vec() }
    }

    #[cfg(test)]
    pub fn of(words: &[&str]) -> Args {
        Args {
            words: words.iter().map(|word| word.to_string()).collect(),
        }
    }

    /// The first word that is no flag: the feature, in any place, for a
    /// command whose flags all stand alone (worktree.mjs took only the first
    /// word, `--keep-branch 303-x` was refused).
    pub fn word(&self) -> Option<&str> {
        self.words
            .iter()
            .map(String::as_str)
            .find(|word| !word.starts_with("--"))
    }

    /// Whether the flag is given, exactly as written (`flags.has`).
    pub fn has(&self, flag: &str) -> bool {
        self.words.iter().any(|word| word == flag)
    }

    /// `--name value` or `--name=value`; a value starting with `--` counts as
    /// missing (proc.mjs `flagValue`).
    pub fn value(&self, name: &str) -> Option<&str> {
        if let Some(at) = self.words.iter().position(|word| word == name) {
            return self
                .words
                .get(at + 1)
                .map(String::as_str)
                .filter(|value| !value.starts_with("--"));
        }
        self.words
            .iter()
            .find_map(|word| word.strip_prefix(name)?.strip_prefix('='))
    }

    /// Every word as given.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// The command line held against what the command `takes`, before
    /// anything is read or written (as the read and write groups do, one
    /// wording): `--help` / `-h` prints what it does and its usage on stdout
    /// and answers `Ok(true)`; an unknown flag (a typo of `--keep-branch`),
    /// a valued flag without its value, a switch given one or a word too many
    /// is refused with the usage. Node read the flags it knew and ran on.
    pub fn check(&self, takes: &Takes) -> Result<bool, Fail> {
        if self.words.iter().any(|word| word == "--help" || word == "-h") {
            println!(
                "{}\n\n{}",
                breadboard_core::config::in_repo(takes.about),
                breadboard_core::config::in_repo(takes.usage)
            );
            return Ok(true);
        }
        match problem_of(&self.words, takes) {
            Some(problem) => fail(format!("{problem}\n{}", breadboard_core::config::in_repo(takes.usage))),
            None => Ok(false),
        }
    }
}

/// What a command takes on its command line.
pub struct Takes {
    /// The flags that stand alone.
    pub switches: &'static [&'static str],
    /// The flags followed by a value (`--flag value` or `--flag=value`).
    pub valued: &'static [&'static str],
    /// The flags followed by MRs, the `!N` words behind them (`--after !232 !233`).
    pub lists: &'static [&'static str],
    /// How many words (no flag, no flag's value) at most.
    pub words: usize,
    /// What the command does, for `--help`.
    pub about: &'static str,
    pub usage: &'static str,
}

fn problem_of(words: &[String], takes: &Takes) -> Option<String> {
    let mut count = 0;
    let mut at = 0;
    while at < words.len() {
        let word = words[at].as_str();
        at += 1;
        if !word.starts_with("--") {
            count += 1;
            if count > takes.words {
                return Some(format!("{word} is one argument too many"));
            }
            continue;
        }
        let (name, inline) = match word.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (word, None),
        };
        if takes.lists.contains(&name) {
            while inline.is_none() && words.get(at).is_some_and(|next| next.starts_with('!')) {
                at += 1;
            }
        } else if takes.valued.contains(&name) {
            // `--flag value`, but a next flag is no value (proc.mjs `flagValue`).
            let has_next = words.get(at).is_some_and(|next| !next.starts_with("--"));
            match inline {
                Some("") => return Some(format!("{name} needs a value")),
                Some(_) => {}
                None if has_next => at += 1,
                None => return Some(format!("{name} needs a value")),
            }
        } else if takes.switches.contains(&name) {
            if inline.is_some() {
                return Some(format!("{name} takes no value"));
            }
        } else {
            let flags: Vec<&str> = [takes.switches, takes.valued, takes.lists].concat();
            let meant = nearest(name, &flags)
                .map(|flag| format!(": did you mean {flag}?"))
                .unwrap_or_default();
            return Some(format!("unknown flag {name}{meant}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feature_is_the_first_word_that_is_no_flag() {
        assert_eq!(Args::of(&["12-x", "--fix"]).word(), Some("12-x"));
        assert_eq!(Args::of(&["--fix", "12-x"]).word(), Some("12-x"));
        assert_eq!(Args::of(&["--fix"]).word(), None);
    }

    #[test]
    fn reads_a_value_after_or_inside_its_flag() {
        let args = Args::of(&["a", "--to", "b", "--limit=40"]);
        assert_eq!(args.value("--to"), Some("b"));
        assert_eq!(args.value("--limit"), Some("40"));
        assert_eq!(Args::of(&["--to", "--dry-run"]).value("--to"), None);
        assert_eq!(Args::of(&["--to"]).value("--to"), None);
    }

    const TAKES: Takes = Takes {
        switches: &["--dry-run"],
        valued: &["--to"],
        lists: &["--after"],
        words: 2,
        about: "about",
        usage: "usage: u",
    };

    fn checked(words: &[&str]) -> Result<bool, String> {
        Args::of(words).check(&TAKES).map_err(|refused| refused.0)
    }

    #[test]
    fn refuses_what_the_command_does_not_take_with_the_usage() {
        assert_eq!(
            checked(&["x", "--dryrun"]),
            Err("unknown flag --dryrun: did you mean --dry-run?\nusage: u".into())
        );
        assert_eq!(
            checked(&["x", "--bogus=1"]),
            Err("unknown flag --bogus\nusage: u".into())
        );
        assert_eq!(
            checked(&["a", "b", "c"]),
            Err("c is one argument too many\nusage: u".into())
        );
        assert_eq!(checked(&["--to"]), Err("--to needs a value\nusage: u".into()));
        assert_eq!(
            checked(&["--to", "--dry-run"]),
            Err("--to needs a value\nusage: u".into())
        );
        assert_eq!(checked(&["--to="]), Err("--to needs a value\nusage: u".into()));
        assert_eq!(
            checked(&["--dry-run=yes"]),
            Err("--dry-run takes no value\nusage: u".into())
        );
        assert_eq!(
            checked(&["a", "12", "--to", "b", "--after", "!1", "!2", "--dry-run"]),
            Ok(false)
        );
        assert_eq!(checked(&["a", "12", "--to=b", "--after=!1,!2"]), Ok(false));
    }

    #[test]
    fn hears_the_help_before_any_problem() {
        assert_eq!(checked(&["--help"]), Ok(true));
        assert_eq!(checked(&["--dryrun", "-h"]), Ok(true));
    }
}
