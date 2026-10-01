//! What the commands of this group share: the command line as Node read it
//! (lib/proc.mjs `flagValue`, worktree.mjs's `flags`), `fail` as an error
//! value, the checkout the command runs in.

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
            let _ = writeln!(std::io::stderr(), "\n✗ {message}");
            ExitCode::FAILURE
        }
    }
}

/// `console.log(text)`: the text and a newline on stdout. A reader that went
/// away (a closed pipe) is no reason to panic.
pub fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
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
