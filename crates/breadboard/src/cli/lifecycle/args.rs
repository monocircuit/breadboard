//! The words after the command, read as worktree.mjs and proc.mjs read them:
//! the first word is the feature unless it is a flag, a flag is a word
//! starting with `--`, a value follows its flag or comes as `--flag=value`.
//! By hand, not clap: five commands with a handful of flags each, whose
//! usage lines and edge cases (a value starting with `--` counts as missing)
//! must stay today's; clap's would differ (rewrite rules 4 and 5).

use super::proc::{Fail, fail};

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

    /// The first word, unless it is a flag (worktree.mjs: `feature`).
    pub fn first(&self) -> Option<&str> {
        self.words
            .first()
            .map(String::as_str)
            .filter(|word| !word.starts_with("--"))
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

    /// The words that are no flag and no flag's value, `valued` the flags that take one.
    pub fn positionals(&self, valued: &[&str]) -> Vec<&str> {
        let mut words = Vec::new();
        let mut is_value = false;
        for word in &self.words {
            if is_value {
                is_value = false;
                if !word.starts_with("--") {
                    continue;
                }
            }
            if word.starts_with("--") {
                is_value = valued.contains(&word.as_str());
            } else {
                words.push(word.as_str());
            }
        }
        words
    }

    /// Every word as given.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Refuses a flag the command doesn't know: a typo of `--keep-branch`
    /// must not pass silently. `flags`: the plain ones, `valued`: those with
    /// a value. The message ends with the usage.
    pub fn check(&self, flags: &[&str], valued: &[&str], usage: &str) -> Result<(), Fail> {
        for word in self.words.iter().filter(|word| word.starts_with("--")) {
            let name = word.split('=').next().unwrap_or(word);
            let is_known = flags.contains(&word.as_str()) || valued.contains(&name);
            if !is_known {
                return fail(format!("{word} is no flag of this command: {usage}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_word_is_the_feature_unless_a_flag() {
        assert_eq!(Args::of(&["12-x", "--fix"]).first(), Some("12-x"));
        assert_eq!(Args::of(&["--fix", "12-x"]).first(), None);
    }

    #[test]
    fn reads_a_value_after_or_inside_its_flag() {
        let args = Args::of(&["a", "--to", "b", "--limit=40"]);
        assert_eq!(args.value("--to"), Some("b"));
        assert_eq!(args.value("--limit"), Some("40"));
        assert_eq!(Args::of(&["--to", "--dry-run"]).value("--to"), None);
        assert_eq!(Args::of(&["--to"]).value("--to"), None);
    }

    #[test]
    fn positionals_skip_the_values_of_flags() {
        let args = Args::of(&["name", "12", "--to", "agent", "--dry-run", "--peer=x"]);
        assert_eq!(args.positionals(&["--to", "--peer"]), vec!["name", "12"]);
    }

    #[test]
    fn refuses_an_unknown_flag_with_the_usage() {
        let args = Args::of(&["x", "--keep-brnach"]);
        let refused = args.check(&["--keep-branch"], &[], "usage: u").unwrap_err();
        assert_eq!(refused.0, "--keep-brnach is no flag of this command: usage: u");
        assert!(Args::of(&["x", "--limit=4"]).check(&[], &["--limit"], "u").is_ok());
    }
}
