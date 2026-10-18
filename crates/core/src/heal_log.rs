//! What the tab bar's status and `herdr heal` did about the overview tabs
//! (#401), and why not: `.claude/runs/heal.log` next to the runs' logs, a
//! line each, `<iso> <who>[<pid>] <text>`. herdr logs no `pane run`, so this
//! is where a bare overview tab that stayed bare is explained: the panes
//! seen bare, the lock waited for, where the overview was typed.

use crate::runs::dir_of;
use crate::time::{now_ms, to_iso};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Past this the older half goes.
const MAX_BYTES: u64 = 256 * 1024;

pub fn file_of(main: &Path) -> PathBuf {
    dir_of(main).join("heal.log")
}

/// `text` as a line of `who` (`status`, `heal`); a log that can't be written is no reason to fail.
pub fn note(main: &Path, who: &str, text: &str) {
    let line = format!("{} {who}[{}] {text}", to_iso(now_ms()), std::process::id());
    let _ = append(&file_of(main), &line);
}

fn append(file: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(file).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let text = fs::read_to_string(file)?;
        let half = &text[text.ceil_char_boundary(text.len() / 2)..];
        fs::write(file, half.split_once('\n').map_or("", |(_, rest)| rest))?;
    }
    writeln!(OpenOptions::new().create(true).append(true).open(file)?, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_a_line_each_and_keeps_the_newer_half_past_its_size() {
        let main = std::env::temp_dir().join(format!("heal-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&main);
        note(&main, "status", "bare overview panes: w2A:p2");
        note(&main, "heal", "typed the overview into w2A:p2 (breadboard)");
        let text = fs::read_to_string(file_of(&main)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].ends_with(" bare overview panes: w2A:p2"), "{text}");
        assert!(
            lines[1].contains(&format!(" heal[{}] typed", std::process::id())),
            "{text}"
        );
        fs::write(file_of(&main), "x\n".repeat(MAX_BYTES as usize / 2 + 1)).unwrap();
        note(&main, "heal", "last");
        let text = fs::read_to_string(file_of(&main)).unwrap();
        assert!((text.len() as u64) < MAX_BYTES, "{}", text.len());
        assert!(text.ends_with(" last\n"), "{}", &text[text.len() - 40..]);
        let _ = fs::remove_dir_all(&main);
    }
}
