//! What the commands of this group share: the command line as Node read it
//! (lib/proc.mjs `flagValue`, worktree.mjs's `flags`), `fail` as an error
//! value, the checkout the command runs in.

use super::super::typo::nearest;
use breadboard_core::git;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

/// A command's end: its exit code, or the message of `✗` (stderr, exit 1).
pub type Outcome = Result<ExitCode, String>;

/// Prints the failure as lib/proc.mjs `fail` does: an empty line, `✗ <message>`, on stderr.
pub fn finish(outcome: Outcome) -> ExitCode {
    match outcome {
        Ok(code) => code,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "\n✗ {}", breadboard_core::config::in_repo(&message));
            ExitCode::FAILURE
        }
    }
}

/// `console.log(text)`: the text and a newline on stdout. A reader that went
/// away (a closed pipe) is no reason to panic.
pub fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", breadboard_core::config::in_repo(text));
    let _ = out.flush();
}

/// The arguments after the command.
#[derive(Clone, Copy)]
pub struct Argv<'a>(pub &'a [String]);

impl<'a> Argv<'a> {
    /// The value of `--flag <value>` or `--flag=<value>`: the first
    /// occurrence counts, a next argument that is a flag is no value
    /// (proc.mjs `flagValue`).
    pub fn value(&self, name: &str) -> Option<&'a str> {
        if let Some(at) = self.0.iter().position(|arg| arg == name) {
            return self
                .0
                .get(at + 1)
                .map(String::as_str)
                .filter(|next| !next.starts_with("--"));
        }
        let prefix = format!("{name}=");
        self.0.iter().find_map(|arg| arg.strip_prefix(&prefix))
    }

    /// The value of a flag when it is set to something (JavaScript's truthiness of it).
    pub fn given(&self, name: &str) -> Option<&'a str> {
        self.value(name).filter(|value| !value.is_empty())
    }

    /// Whether the flag is on the command line as it is (`flags.has`).
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|arg| arg == name)
    }

    /// The first argument unless it is a flag (worktree.mjs's `feature`).
    pub fn first(&self) -> Option<&'a str> {
        self.0
            .first()
            .map(String::as_str)
            .filter(|first| !first.starts_with("--"))
    }

    pub fn all(&self) -> &'a [String] {
        self.0
    }

    /// The command line held against what the command takes, before anything
    /// is read or written: `Ok(true)` asks for the help (`--help` anywhere,
    /// `-h` as the first argument: later it is a word of a text, as Node read
    /// it, `wt:action look check -h output`), an
    /// unknown flag, a valued flag without its value, a switch given a value
    /// or one word too many is refused with the usage. Node read only the
    /// flags it knew and ran on: a typo of `--dry-run` opened the MR (rule 6).
    pub fn check(&self, takes: &Takes) -> Result<bool, String> {
        let mut problem = None;
        let mut words = 0;
        let mut at = 0;
        while at < self.0.len() {
            let arg = self.0[at].as_str();
            at += 1;
            if arg == "--help" || (arg == "-h" && at == 1) {
                return Ok(true);
            }
            if problem.is_some() {
                continue;
            }
            if !arg.starts_with("--") {
                words += 1;
                if takes.words.is_some_and(|most| words > most) {
                    problem = Some(format!("{arg} is one argument too many"));
                }
                continue;
            }
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (arg, None),
            };
            if takes.valued.contains(&name) {
                // `--flag value`, but a next flag is no value (proc.mjs `flagValue`).
                let has_next = self.0.get(at).is_some_and(|next| !next.starts_with("--"));
                if inline.is_none() && !has_next {
                    problem = Some(format!("{name} needs a value"));
                } else if inline.is_none() {
                    at += 1;
                }
            } else if takes.switches.contains(&name) {
                if inline.is_some() {
                    problem = Some(format!("{name} takes no value"));
                }
            } else {
                let flags: Vec<&str> = takes.valued.iter().chain(takes.switches).copied().collect();
                problem = Some(match nearest(name, &flags) {
                    Some(meant) => format!("unknown flag {name}: did you mean {meant}?"),
                    None => format!("unknown flag {name}"),
                });
            }
        }
        match problem {
            Some(problem) => Err(format!("{problem}\n{}", takes.after_error())),
            None => Ok(false),
        }
    }
}

/// What a command takes on its command line.
pub struct Takes {
    /// The flags followed by a value.
    pub valued: &'static [&'static str],
    /// The flags that stand alone.
    pub switches: &'static [&'static str],
    /// How many words (no flag, no flag's value) at most; None: any number.
    pub words: Option<usize>,
    pub usage: &'static str,
    /// Its name, `wt:mr`, for `pnpm wt:mr --help`.
    pub command: &'static str,
}

impl Takes {
    /// What follows an error of the command line: the usage while it is
    /// short, else where to read it (wt:action's has 11 lines: one wrong flag
    /// was a screen of text).
    pub fn after_error(&self) -> String {
        if self.usage.lines().count() <= 3 {
            self.usage.to_string()
        } else {
            format!("  pnpm {} --help shows every form", self.command)
        }
    }
}

/// `--help`: what the command does and its usage, on stdout, exit 0.
pub fn help(about: &str, usage: &str) -> Outcome {
    say(&format!("{about}\n\n{usage}"));
    Ok(ExitCode::SUCCESS)
}

/// Where a command runs: the cwd, the checkout it lies in (`git rev-parse
/// --show-toplevel`) and the repo's main checkout (worktrees.mjs `mainDir`).
pub struct Here {
    pub cwd: PathBuf,
    pub top: Option<PathBuf>,
    pub main: Option<PathBuf>,
}

impl Here {
    pub fn find() -> Here {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let out = git::git(&cwd, &["rev-parse", "--show-toplevel"]);
        let top = Some(out.out())
            .filter(|top| out.is_ok && !top.is_empty())
            .map(PathBuf::from);
        let main = top.as_ref().and_then(|top| git::main_dir(top));
        Here { cwd, top, main }
    }

    /// The checkout of a worktree, None in the main checkout or outside a repo.
    pub fn worktree(&self) -> Option<&Path> {
        self.top.as_deref().filter(|top| Some(*top) != self.main.as_deref())
    }

    /// Where git and glab run: the checkout, else the cwd (Node's `cwd: ""`).
    pub fn dir(&self) -> &Path {
        self.top.as_deref().unwrap_or(&self.cwd)
    }

    /// The branch checked out in the checkout, "" when HEAD is detached.
    pub fn branch(&self) -> String {
        git::git(self.dir(), &["branch", "--show-current"]).out().to_string()
    }

    /// A path as the command line meant it: relative to the cwd.
    pub fn resolve(&self, file: &str) -> PathBuf {
        normalize(&self.cwd.join(file))
    }

    /// A path as it is worth reading: relative inside the checkout, as given
    /// outside (issues.mjs `shown`).
    pub fn shown(&self, file: &str) -> String {
        let base = self.top.as_deref().unwrap_or(&self.cwd);
        match self.resolve(file).strip_prefix(base) {
            Ok(inside) => inside.to_string_lossy().into_owned(),
            Err(_) => file.to_string(),
        }
    }
}

/// `.` and `..` taken out, as `path.resolve` does, without asking the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `#307` or `307` → `307`; None when it is no number (`/^\d+$/` after the `#`).
pub fn number_of(given: &str) -> Option<&str> {
    let number = given.strip_prefix('#').unwrap_or(given);
    (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then_some(number)
}

/// `n thing` or `n things`.
pub fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// A folder of its own in `$TMPDIR`, `<prefix>XXXXXX` (`fs.mkdtempSync`).
pub fn temp_dir(prefix: &str) -> std::io::Result<PathBuf> {
    let base = std::env::temp_dir();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    for attempt in 0..100u32 {
        let dir = base.join(format!(
            "{prefix}{:06x}",
            (seed ^ std::process::id().rotate_left(8)).wrapping_add(attempt) & 0xff_ffff
        ));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("no free temp folder"))
}

#[cfg(test)]
pub mod scratch {
    //! A folder of its own per test, gone after it.
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    pub struct Scratch(PathBuf);

    impl Scratch {
        pub fn new() -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "breadboard-write-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn reads_a_flag_value_as_flag_value_does() {
        let args = argv(&["look", "--url", "/a", "--why=", "--run", "--done", "3", "--url=/b"]);
        let argv = Argv(&args);
        assert_eq!(argv.value("--url"), Some("/a"));
        assert_eq!(argv.value("--why"), Some(""));
        assert_eq!(argv.given("--why"), None);
        assert_eq!(argv.value("--run"), None);
        assert_eq!(argv.value("--done"), Some("3"));
        assert_eq!(argv.value("--on"), None);
        assert_eq!(argv.first(), Some("look"));
        assert_eq!(Argv(&argv_of_flag()).first(), None);
    }

    fn argv_of_flag() -> Vec<String> {
        argv(&["--list"])
    }

    const TAKES: Takes = Takes {
        valued: &["--description-file"],
        switches: &["--update", "--dry-run"],
        words: Some(1),
        usage: "usage: x",
        command: "wt:x",
    };

    fn checked(args: &[&str]) -> Result<bool, String> {
        Argv(&argv(args)).check(&TAKES)
    }

    #[test]
    fn refuses_what_the_command_does_not_take_and_hears_the_help() {
        assert_eq!(checked(&["10", "--update", "--description-file", "f"]), Ok(false));
        assert_eq!(checked(&["--description-file=f", "--dry-run"]), Ok(false));
        assert_eq!(checked(&[]), Ok(false));
        assert_eq!(
            checked(&["--dryrun"]),
            Err("unknown flag --dryrun: did you mean --dry-run?\nusage: x".into())
        );
        assert_eq!(checked(&["--bogus=1"]), Err("unknown flag --bogus\nusage: x".into()));
        assert_eq!(
            checked(&["10", "11"]),
            Err("11 is one argument too many\nusage: x".into())
        );
        assert_eq!(
            checked(&["--description-file", "--update"]),
            Err("--description-file needs a value\nusage: x".into())
        );
        assert_eq!(
            checked(&["--description-file"]),
            Err("--description-file needs a value\nusage: x".into())
        );
        assert_eq!(
            checked(&["--update=yes"]),
            Err("--update takes no value\nusage: x".into())
        );
        // A flag's value is no word, and the help wins over any problem.
        assert_eq!(checked(&["10", "--description-file", "-h"]), Ok(false));
        assert_eq!(checked(&["--dryrun", "--help"]), Ok(true));
        assert_eq!(checked(&["-h"]), Ok(true));
        // Later, -h is a word: of a text, or one too many.
        assert_eq!(
            checked(&["10", "-h"]),
            Err("-h is one argument too many\nusage: x".into())
        );
        let text = argv(&["look", "check", "-h", "output"]);
        let any = Takes { words: None, ..TAKES };
        assert_eq!(Argv(&text).check(&any), Ok(false));
        // A long usage gives way to where to read it.
        let long = Takes {
            usage: "usage: a\n  b\n  c\n  d",
            ..TAKES
        };
        assert_eq!(
            Argv(&argv(&["--updat"])).check(&long),
            Err("unknown flag --updat: did you mean --update?\n  pnpm wt:x --help shows every form".into())
        );
    }

    #[test]
    fn takes_a_number_with_or_without_its_hash() {
        assert_eq!(number_of("#307"), Some("307"));
        assert_eq!(number_of("307"), Some("307"));
        assert_eq!(number_of("#"), None);
        assert_eq!(number_of("3a"), None);
    }

    #[test]
    fn makes_a_temp_folder_of_its_own() {
        let one = temp_dir("wt-test-").unwrap();
        let two = temp_dir("wt-test-").unwrap();
        assert_ne!(one, two);
        std::fs::remove_dir_all(one).unwrap();
        std::fs::remove_dir_all(two).unwrap();
    }
}
