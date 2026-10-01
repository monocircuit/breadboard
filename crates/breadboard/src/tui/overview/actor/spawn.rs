//! What the overview starts and doesn't wait for (auto-actions.mjs
//! `removeWorktree`, workers.mjs `runInBackground`, end.mjs, actions.mjs
//! `startStack`): a command of breadboard itself or pnpm, detached into a
//! process group of its own so quitting the overview never breaks it
//! halfway, its output into a log in `$TMPDIR` the row shows the last line
//! of; its end comes back to the view as an effect.

use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where a background run stands, for its row: armed by a first click,
/// running (its log), or failed (why).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    pub is_armed: bool,
    pub is_running: bool,
    pub log: Option<PathBuf>,
    pub error: Option<String>,
}

impl Run {
    pub fn armed() -> Run {
        Run {
            is_armed: true,
            ..Run::default()
        }
    }
    pub fn running(log: &Path) -> Run {
        Run {
            is_running: true,
            log: Some(log.to_path_buf()),
            ..Run::default()
        }
    }
    /// The last line its log has, for its row.
    pub fn last_line(&self) -> String {
        self.log.as_deref().map(last_line).unwrap_or_default()
    }
}

/// The program that is breadboard: the commands it starts are its own (`wt:remove`, `wt:orchestrate`, `stack:up`).
pub fn breadboard() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("breadboard"))
}

/// `$TMPDIR/<name>`: where a background run writes.
pub fn log_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// What a background command is: its program, arguments, where it runs, its env.
pub struct Background<'a> {
    pub program: &'a Path,
    pub args: Vec<String>,
    pub cwd: &'a Path,
    pub log: &'a Path,
    /// Variables set besides `NO_COLOR=1`.
    pub env: Vec<(&'a str, String)>,
    /// Without the overview's pane: a start must not take the overview's tab for its own.
    pub is_paneless: bool,
}

/// Starts `background` detached, its output into its log (truncated). Its end
/// — the exit code and the last line of its log — goes to `on_exit`, on a
/// thread of its own. Err: it couldn't start, why.
pub fn start(background: Background, on_exit: impl FnOnce(Option<i32>, String) + Send + 'static) -> Result<(), String> {
    let out = File::create(background.log).map_err(|error| format!("{}: {error}", background.log.display()))?;
    let err = out.try_clone().map_err(|error| error.to_string())?;
    let mut command = Command::new(background.program);
    command
        .args(&background.args)
        .current_dir(background.cwd)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0);
    for (key, value) in &background.env {
        command.env(key, value);
    }
    if background.is_paneless {
        command.env_remove("HERDR_PANE_ID").env_remove("HERDR_TAB_ID");
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let log = background.log.to_path_buf();
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|status| status.code());
        on_exit(code, last_line(&log));
    });
    Ok(())
}

/// The last line a log has that says something, without colours and redraws (actions.mjs `lastLine`).
pub fn last_line(log: &Path) -> String {
    let Ok(bytes) = std::fs::read(log) else {
        return String::new();
    };
    let tail = &bytes[bytes.len().saturating_sub(4096)..];
    let text = strip_escapes(&String::from_utf8_lossy(tail));
    text.split(['\r', '\n'])
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// `text` without terminal escapes (`ESCAPES` of tty.mjs: CSI and OSC sequences).
pub fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
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

/// Why a run failed, for its row: its last log line without a leading `✗ `, else `<what> ended with <code>`.
pub fn error_of(code: Option<i32>, last: &str, what: &str) -> Option<String> {
    if code == Some(0) {
        return None;
    }
    let last = last.trim_start_matches("✗ ").trim();
    Some(if last.is_empty() {
        format!(
            "{what}ended with {}",
            code.map_or_else(|| "a signal".to_string(), |code| code.to_string())
        )
    } else {
        last.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_last_line_without_colours() {
        assert_eq!(strip_escapes("\x1b[32m✓ up\x1b[0m"), "✓ up");
        assert_eq!(strip_escapes("\x1b]8;;http://x\x07link\x1b]8;;\x07"), "link");
        let dir = std::env::temp_dir().join(format!("breadboard-spawn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log");
        std::fs::write(&log, "one\n\x1b[2Kstarting\rtwo  \n\n").unwrap();
        assert_eq!(last_line(&log), "two");
        assert_eq!(last_line(&dir.join("none")), "");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn says_why_a_run_failed() {
        assert_eq!(error_of(Some(0), "done", "wt:remove "), None);
        assert_eq!(error_of(Some(1), "✗ it has changes", ""), Some("it has changes".into()));
        assert_eq!(
            error_of(Some(2), "", "wt:remove "),
            Some("wt:remove ended with 2".into())
        );
    }

    #[test]
    fn runs_detached_and_reports_its_end() {
        let dir = std::env::temp_dir().join(format!("breadboard-start-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("run.log");
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            Background {
                program: Path::new("sh"),
                args: vec!["-c".into(), "echo first; echo \"$NO_COLOR last\"; exit 3".into()],
                cwd: &dir,
                log: &log,
                env: vec![],
                is_paneless: true,
            },
            move |code, last| sender.send((code, last)).unwrap(),
        )
        .unwrap();
        let (code, last) = receiver.recv().unwrap();
        assert_eq!((code, last.as_str()), (Some(3), "1 last"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
