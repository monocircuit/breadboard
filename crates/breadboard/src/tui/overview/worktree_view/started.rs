//! Commands a click starts (actions.mjs `startStack`, `migrate`,
//! `removeWorktree`): detached, in a process group of their own, their output
//! in a log file instead of a pipe, so quitting the overview never breaks one
//! halfway. The view shows the last line they wrote and, after a failure, why.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// How much of the end of a log the last line is looked for in.
const TAIL: u64 = 4096;

/// A command started from the view: while it runs, and how it ended.
#[derive(Debug, Clone)]
pub struct Started {
    log: PathBuf,
    /// Its exit code once it ended, -1 for a signal.
    end: Arc<Mutex<Option<i32>>>,
}

impl Started {
    /// `command` detached, stdout and stderr into `log` (truncated first).
    pub fn spawn(mut command: Command, log: PathBuf) -> io::Result<Started> {
        let out = File::create(&log)?;
        command
            .stdin(Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .env("NO_COLOR", "1")
            .process_group(0);
        let mut child = command.spawn()?;
        let end = Arc::new(Mutex::new(None));
        let ended = end.clone();
        // Waited for on a thread of its own: no zombie, the view never blocks on it.
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|status| status.code()).unwrap_or(-1);
            if let Ok(mut end) = ended.lock() {
                *end = Some(code);
            }
        });
        Ok(Started { log, end })
    }

    /// A start that failed before it ran: shown like one that ended badly.
    pub fn failed(log: PathBuf) -> Started {
        Started {
            log,
            end: Arc::new(Mutex::new(Some(-1))),
        }
    }

    fn code(&self) -> Option<i32> {
        self.end.lock().ok().and_then(|end| *end)
    }

    pub fn is_running(&self) -> bool {
        self.code().is_none()
    }

    /// It ran to the end and said so.
    pub fn is_done(&self) -> bool {
        self.code() == Some(0)
    }

    /// Why it failed, once it ended badly: its last line, else `<what> ended with <code>`.
    pub fn error(&self, what: &str) -> Option<String> {
        let code = self.code().filter(|code| *code != 0)?;
        let line = self.last_line();
        Some(if line.is_empty() {
            format!("{what} ended with {code}")
        } else {
            line
        })
    }

    /// The last line it wrote, without colours and progress redraws.
    pub fn last_line(&self) -> String {
        last_line_of(&self.log)
    }
}

/// The last non-empty line of the end of `file`, escapes stripped, `\r` redraws split.
pub fn last_line_of(file: &Path) -> String {
    let Ok(mut file) = File::open(file) else {
        return String::new();
    };
    let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(length.saturating_sub(TAIL)));
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    last_line_in(&String::from_utf8_lossy(&bytes))
}

fn last_line_in(text: &str) -> String {
    strip_escapes(text)
        .split(['\r', '\n'])
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

/// `text` without ANSI escapes: CSI (`ESC [ … final`), OSC (`ESC ] … BEL|ESC \`) and two-byte ones.
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for ch in chars.by_ref() {
                    if ('@'..='~').contains(&ch) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(ch) = chars.next() {
                    if ch == '\u{7}' {
                        break;
                    }
                    if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// `$TMPDIR/overview-<kind>-<basename of dir>.log`.
pub fn log_of(kind: &str, dir: &Path) -> PathBuf {
    let base = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("overview-{kind}-{base}.log"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn the_last_line_drops_colours_and_progress() {
        assert_eq!(
            last_line_in("one\n\u{1b}[32mtwo\u{1b}[0m\r 50%\r100% done\n\n"),
            "100% done"
        );
        assert_eq!(last_line_in("\u{1b}]8;;http://x\u{7}link\u{1b}]8;;\u{7}\n"), "link");
        assert_eq!(last_line_in(""), "");
    }

    fn wait_for(started: &Started) {
        let until = Instant::now() + Duration::from_secs(5);
        while started.is_running() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_command_that_fails_says_why() {
        let dir = std::env::temp_dir().join(format!("started-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "echo working; echo 'it broke' >&2; exit 3"]);
        let started = Started::spawn(command, dir.join("fail.log")).unwrap();
        wait_for(&started);
        assert_eq!(started.error("thing").as_deref(), Some("it broke"));
        let mut command = Command::new("sh");
        command.args(["-c", "exit 4"]);
        let silent = Started::spawn(command, dir.join("silent.log")).unwrap();
        wait_for(&silent);
        assert_eq!(silent.error("thing").as_deref(), Some("thing ended with 4"));
        let mut command = Command::new("sh");
        command.args(["-c", "echo fine"]);
        let fine = Started::spawn(command, dir.join("fine.log")).unwrap();
        wait_for(&fine);
        assert!(fine.is_done() && fine.error("thing").is_none());
        assert_eq!(fine.last_line(), "fine");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
