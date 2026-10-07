//! Running commands and talking to the user, as lib/proc.mjs does: `step`
//! lines on stdout, a failure as `✗ <reason>` on stderr, child processes
//! whose output streams through.

use breadboard_core::git::Out;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::sync::OnceLock;

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

    #[test]
    fn a_missing_program_is_a_failed_answer() {
        let out = output("breadboard-no-such-program", &[], Path::new("/"), None);
        assert!(!out.is_ok);
        assert_eq!(out.status, -1);
    }
}
