//! Running commands and talking to the user, as lib/proc.mjs does: `step`
//! lines on stdout, a failure as `✗ <reason>` on stderr, child processes
//! whose output streams through.

use breadboard_core::git::Out;
use std::io::{IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Child, Command, ExitCode, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Why a command stops: printed as `✗ <reason>` (proc.mjs `fail`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail(pub String);

pub type Done = Result<(), Fail>;

/// A failure with `reason`.
pub fn fail<T>(reason: impl Into<String>) -> Result<T, Fail> {
    Err(Fail(reason.into()))
}

/// Whether a stream gets colours: a terminal, and no NO_COLOR (the overview
/// sets it for the commands it runs, it reads their last line).
fn is_coloured(is_terminal: bool, no_color: Option<&std::ffi::OsStr>) -> bool {
    is_terminal && no_color.is_none_or(|value| value.is_empty())
}

/// Whether stdout gets colours, asked once.
fn is_stdout_coloured() -> bool {
    static IS_COLOURED: OnceLock<bool> = OnceLock::new();
    *IS_COLOURED.get_or_init(|| is_coloured(std::io::stdout().is_terminal(), std::env::var_os("NO_COLOR").as_deref()))
}

/// The SGR codes of what a line says.
pub mod tone {
    pub const DONE: &str = "1;32";
    pub const FAILED: &str = "1;31";
    pub const STEP: &str = "36";
    pub const KEPT: &str = "33";
    pub const WOULD: &str = "36";
    pub const BOLD: &str = "1";
}

/// `text` in the SGR `code` when `is_on`.
fn painted(code: &str, text: &str, is_on: bool) -> String {
    if is_on {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// `text` in the SGR `code` when stdout gets colours.
pub fn paint(code: &str, text: &str) -> String {
    painted(code, text, is_stdout_coloured())
}

/// The exit code of a command's outcome, its failure said on stderr.
pub fn finish(done: Done) -> ExitCode {
    let _ = std::io::stdout().flush();
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail(reason)) => {
            let is_on = is_coloured(std::io::stderr().is_terminal(), std::env::var_os("NO_COLOR").as_deref());
            eprintln!("\n{}", painted(tone::FAILED, &format!("✗ {reason}"), is_on));
            ExitCode::FAILURE
        }
    }
}

/// `▸ <message>` after a blank line: what the command does next.
pub fn step(message: &str) {
    println!("\n{} {message}", paint(tone::STEP, "▸"));
}

/// `✓ <message>` after a blank line: what the command did.
pub fn done(message: &str) {
    println!("\n{} {}", paint(tone::DONE, "✓"), paint(tone::BOLD, message));
}

/// `<program> <args>` in `cwd` with its output on this terminal (proc.mjs
/// `run`, not quiet); whether it exited 0.
pub fn run_inherit(program: &str, args: &[&str], cwd: &Path) -> bool {
    let _ = std::io::stdout().flush();
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `<program> <args>` in `cwd`, quiet, with `input` on its stdin: its whole
/// answer, never failing (proc.mjs `run` quiet + allowFail).
pub fn output(program: &str, args: &[&str], cwd: &Path, input: Option<&str>) -> Out {
    let spawned = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            return Out {
                is_ok: false,
                status: -1,
                stdout: String::new(),
                stderr: error.to_string(),
            };
        }
    };
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        // A writer thread: a long input must not block on a full stdout pipe.
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    match child.wait_with_output() {
        Ok(result) => Out {
            is_ok: result.status.success(),
            status: result.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        },
        Err(error) => Out {
            is_ok: false,
            status: -1,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

/// What a call to another machine waits for, said once it takes longer
/// than `notice`, and how long it may take before it counts as unanswered.
#[derive(Debug, Clone)]
pub struct Wait {
    /// `GitLab (the MRs of fix/303-x)`: the line says `waiting for <what>`.
    pub what: String,
    pub notice: Duration,
    pub limit: Duration,
}

/// Whether waits are said: a command of this group run from the command
/// line, never the overview, whose screen a line would break (it calls the
/// layout in its own process).
static IS_SAYING_WAITS: AtomicBool = AtomicBool::new(false);

/// From here on, this process says what it waits for (`lifecycle::run`).
pub fn say_waits() {
    IS_SAYING_WAITS.store(true, Ordering::Relaxed);
}

/// `line` on stdout at once, when this process says its waits.
pub fn say_wait(line: &str) {
    if IS_SAYING_WAITS.load(Ordering::Relaxed) {
        println!("{line}");
        let _ = std::io::stdout().flush();
    }
}

/// How long a command may be silent before it says what it waits for.
pub const NOTICE: Duration = Duration::from_millis(1500);

impl Wait {
    pub fn new(what: impl Into<String>, limit: Duration) -> Wait {
        Wait {
            what: what.into(),
            notice: NOTICE,
            limit,
        }
    }

    /// The line said once the wait is longer than `notice`.
    pub fn line(&self) -> String {
        format!("  waiting for {}, at most {} s", self.what, self.limit.as_secs())
    }
}

/// `child` waited for, its output read beside it: `waiting for …` said once
/// it takes longer than `wait.notice`, killed after `wait.limit` (None then).
/// Today's waited without a word and without end for a GitLab or an origin
/// that accepts and never answers.
pub fn wait_within(mut child: Child, wait: &Wait) -> std::io::Result<Option<Output>> {
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = reader(child.stdout.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>));
    let stderr = reader(child.stderr.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>));
    let started = Instant::now();
    let mut is_said = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        let waited = started.elapsed();
        if waited >= wait.limit {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        if !is_said && waited >= wait.notice {
            say_wait(&wait.line());
            is_said = true;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A grandchild may hold a pipe open past the kill: read what is there, never block on it.
    let stdout = if status.is_some() {
        stdout.join().unwrap_or_default()
    } else {
        Vec::new()
    };
    let stderr = if status.is_some() {
        stderr.join().unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(status.map(|status| Output { status, stdout, stderr }))
}

/// [`output`] of a call to another machine, given at most `wait.limit`:
/// past it, a failed answer whose stderr says `timeout`.
pub fn output_within(program: &str, args: &[&str], cwd: &Path, wait: &Wait) -> Out {
    let unanswered = |stderr: String| Out {
        is_ok: false,
        status: -1,
        stdout: String::new(),
        stderr,
    };
    let spawned = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    match spawned.and_then(|child| wait_within(child, wait)) {
        Ok(Some(result)) => Out {
            is_ok: result.status.success(),
            status: result.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        },
        Ok(None) => unanswered(format!("timeout: no answer in {} s", wait.limit.as_secs())),
        Err(error) => unanswered(error.to_string()),
    }
}

/// `git <args>` in `dir`, quiet, never failing, `input` on its stdin.
pub fn git(dir: &Path, args: &[&str], input: Option<&str>) -> Out {
    output("git", args, dir, input)
}

/// A shell word: single quotes, a `'` inside as `'\''` (proc.mjs `quote`).
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// A path as text, for messages and child processes.
pub fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// This binary, for the commands a tab or a child process runs: the
/// rewrite's tabs run `breadboard …` where today's ran `node …/*.mjs`.
pub fn exe() -> String {
    std::env::current_exe()
        .map(|exe| text(&exe))
        .unwrap_or_else(|_| "breadboard".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_only_a_terminal_without_no_color() {
        use std::ffi::OsStr;
        assert!(is_coloured(true, None));
        assert!(is_coloured(true, Some(OsStr::new(""))));
        assert!(!is_coloured(true, Some(OsStr::new("1"))));
        assert!(!is_coloured(false, None));
        assert_eq!(painted(tone::DONE, "✓", true), "\x1b[1;32m✓\x1b[0m");
        assert_eq!(painted(tone::DONE, "✓", false), "✓");
    }

    #[test]
    fn quotes_a_word_for_any_shell() {
        assert_eq!(quote("/a b/c"), "'/a b/c'");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn feeds_input_and_reads_output() {
        let out = output("cat", &[], Path::new("/"), Some("a\nb\n"));
        assert!(out.is_ok);
        assert_eq!(out.stdout, "a\nb\n");
    }

    fn quick(limit: u64) -> Wait {
        Wait {
            what: "a test".into(),
            notice: Duration::from_millis(10),
            limit: Duration::from_millis(limit),
        }
    }

    #[test]
    fn a_call_that_answers_in_time_is_its_answer() {
        let out = output_within(
            "sh",
            &["-c", "echo yes; echo no >&2; exit 3"],
            Path::new("/"),
            &quick(5000),
        );
        assert!(!out.is_ok);
        assert_eq!(out.status, 3);
        assert_eq!(out.stdout, "yes\n");
        assert_eq!(out.stderr, "no\n");
        assert!(output_within("true", &[], Path::new("/"), &quick(5000)).is_ok);
    }

    #[test]
    fn a_call_that_does_not_answer_is_given_up() {
        let started = Instant::now();
        let out = output_within("sleep", &["5"], Path::new("/"), &quick(200));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!out.is_ok);
        assert!(out.stderr.starts_with("timeout"));
        assert_eq!(
            Wait::new("GitLab (the MRs of fix/3-x)", Duration::from_secs(15)).line(),
            "  waiting for GitLab (the MRs of fix/3-x), at most 15 s"
        );
    }

    #[test]
    fn a_missing_program_is_a_failed_answer() {
        let out = output("breadboard-no-such-program", &[], Path::new("/"), None);
        assert!(!out.is_ok);
        assert_eq!(out.status, -1);
    }
}
