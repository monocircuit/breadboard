//! Running commands and talking to the user (lib/proc.mjs): the arguments as
//! today's commands read them, `step`, `fail` with its hint, child processes
//! with their output shown or kept.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// The arguments after the command, read as worktree.mjs and `flagValue` do.
#[derive(Debug, Clone, Default)]
pub struct Args {
    pub all: Vec<String>,
}

impl Args {
    pub fn new(args: &[String]) -> Args {
        Args { all: args.to_vec() }
    }

    /// The first argument unless it is a flag: a flag before the feature
    /// leaves none (`wt:new --fix 12-x` is a usage error, as today).
    pub fn first(&self) -> Option<&str> {
        self.all
            .first()
            .map(String::as_str)
            .filter(|arg| !arg.starts_with("--"))
    }

    /// Whether `--name` stands on the command line as it is (`--x=y` counts
    /// only spelled so, as the Set of flags in worktree.mjs).
    pub fn has(&self, name: &str) -> bool {
        self.all.iter().any(|arg| arg == name)
    }

    /// `--name <value>` or `--name=<value>`; a value that starts with `--`
    /// counts as missing.
    pub fn value(&self, name: &str) -> Option<String> {
        if let Some(at) = self.all.iter().position(|arg| arg == name) {
            return self.all.get(at + 1).filter(|value| !value.starts_with("--")).cloned();
        }
        let prefix = format!("{name}=");
        self.all
            .iter()
            .find_map(|arg| arg.strip_prefix(&prefix).map(String::from))
    }

    /// The words that are no flag and no value of one of `valued`.
    pub fn words(&self, valued: &[&str]) -> Vec<String> {
        self.all
            .iter()
            .enumerate()
            .filter(|(at, arg)| !arg.starts_with("--") && !(*at > 0 && valued.contains(&self.all[at - 1].as_str())))
            .map(|(_, arg)| arg.clone())
            .collect()
    }
}

static HINT: Mutex<String> = Mutex::new(String::new());
/// Files this process holds and drops when it fails (setup claims).
static HELD: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// What `fail` adds under its message from here on.
pub fn set_hint(text: &str) {
    if let Ok(mut hint) = HINT.lock() {
        *hint = text.to_string();
    }
}

/// A file removed when this process fails or ends through `fail`.
pub fn hold(file: &Path) {
    if let Ok(mut held) = HELD.lock() {
        held.push(file.to_path_buf());
    }
}

/// The file no longer held.
pub fn unhold(file: &Path) {
    if let Ok(mut held) = HELD.lock() {
        held.retain(|known| known != file);
    }
}

/// Every held file let go, as the exit handlers of Node do.
pub fn release_held() {
    let files = HELD
        .lock()
        .map(|mut held| std::mem::take(&mut *held))
        .unwrap_or_default();
    for file in files {
        let _ = std::fs::remove_file(&file);
    }
}

/// `\n✗ <message>` (and the hint) on stderr, exit 1. Other processes read
/// the `✗ ` at the start of a line (wt:orchestrate, the overview).
pub fn fail(message: &str) -> ! {
    let hint = HINT.lock().map(|hint| hint.clone()).unwrap_or_default();
    let _ = std::io::stdout().flush();
    if hint.is_empty() {
        eprintln!("\n✗ {message}");
    } else {
        eprintln!("\n✗ {message}\n  {hint}");
    }
    release_held();
    std::process::exit(1)
}

/// `\n▸ <message>` on stdout.
pub fn step(message: &str) {
    println!("\n▸ {message}");
}

/// What a child process answered.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    pub is_ok: bool,
    pub out: String,
    pub err: String,
}

impl Ran {
    /// The output trimmed, as proc.mjs gives it.
    pub fn text(&self) -> &str {
        self.out.trim()
    }
}

/// How a child runs: in `cwd`, its output shown (`is_quiet` false) or kept.
#[derive(Debug, Clone, Default)]
pub struct How<'a> {
    pub cwd: Option<&'a Path>,
    pub is_quiet: bool,
    pub can_fail: bool,
    pub input: Option<&'a str>,
    pub env: Vec<(&'a str, String)>,
}

/// `command args` (proc.mjs `run`): shown or quiet; a failure fails unless
/// `can_fail`.
pub fn run(command: &str, args: &[&str], how: How) -> Ran {
    let _ = std::io::stdout().flush();
    let mut child = Command::new(command);
    child.args(args);
    if let Some(cwd) = how.cwd {
        child.current_dir(cwd);
    }
    for (key, value) in &how.env {
        child.env(key, value);
    }
    child.stdin(if how.input.is_some() {
        Stdio::piped()
    } else if how.is_quiet {
        Stdio::null()
    } else {
        Stdio::inherit()
    });
    if how.is_quiet {
        child.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        child.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    }
    let ran = match child.spawn() {
        Ok(mut process) => {
            if let (Some(input), Some(mut stdin)) = (how.input, process.stdin.take()) {
                let _ = stdin.write_all(input.as_bytes());
            }
            match process.wait_with_output() {
                Ok(output) => Ran {
                    is_ok: output.status.success(),
                    out: String::from_utf8_lossy(&output.stdout).into_owned(),
                    err: String::from_utf8_lossy(&output.stderr).into_owned(),
                },
                Err(error) => Ran {
                    err: error.to_string(),
                    ..Ran::default()
                },
            }
        }
        Err(error) => Ran {
            err: format!("{command}: {error}"),
            ..Ran::default()
        },
    };
    if !ran.is_ok && !how.can_fail {
        let quiet = if how.is_quiet {
            format!(":\n{}", ran.err)
        } else {
            String::new()
        };
        fail(&format!("{command} {} failed{quiet}", args.join(" ")));
    }
    ran
}

/// `command args` quiet, never failing: for reading.
pub fn sh(command: &str, args: &[&str], cwd: Option<&Path>) -> Ran {
    run(
        command,
        args,
        How {
            cwd,
            is_quiet: true,
            can_fail: true,
            ..How::default()
        },
    )
}

/// `git args` in `cwd`, quiet, never failing (proc.mjs `git`).
pub fn git(args: &[&str], cwd: &Path) -> Ran {
    sh("git", args, Some(cwd))
}

/// `git args` with `input` on its stdin.
pub fn git_with(args: &[&str], cwd: &Path, input: &str) -> Ran {
    run(
        "git",
        args,
        How {
            cwd: Some(cwd),
            is_quiet: true,
            can_fail: true,
            input: Some(input),
            ..How::default()
        },
    )
}

/// A shell word: single quotes, a `'` inside as `'\''`.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The non-empty lines of a text.
pub fn lines(text: &str) -> Vec<String> {
    text.split('\n')
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

/// Whether claude runs this (Claude Code sets CLAUDECODE): an agent stays in its tab.
pub fn is_agent() -> bool {
    std::env::var_os("CLAUDECODE").is_some_and(|value| !value.is_empty())
}

/// This binary, for a command that calls another one in a child process.
pub fn entry() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("breadboard"))
}

/// The folder of the breadboard package this binary was built in
/// (`packages/breadboard`): its `nvim/` and `chromium/` lie there. Found
/// above the binary, else in the main checkout.
pub fn package_dir(main: &Path) -> PathBuf {
    entry()
        .ancestors()
        .find(|dir| dir.join("nvim").join("changes.lua").is_file())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| main.join("packages").join("breadboard"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        Args::new(&list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn reads_flags_as_today() {
        let parsed = args(&["12-x", "--origin", "breadboard", "--shared"]);
        assert_eq!(parsed.first(), Some("12-x"));
        assert!(parsed.has("--shared"));
        assert_eq!(parsed.value("--origin").as_deref(), Some("breadboard"));
        assert_eq!(args(&["--fix", "12-x"]).first(), None);
        assert_eq!(args(&["--origin", "--shared"]).value("--origin"), None);
        assert_eq!(args(&["--since=2w"]).value("--since").as_deref(), Some("2w"));
        assert_eq!(args(&["a", "--type", "fixed", "b"]).words(&["--type"]), vec!["a", "b"]);
    }

    #[test]
    fn quotes_for_every_shell() {
        assert_eq!(quote("it's"), "'it'\\''s'");
    }
}
