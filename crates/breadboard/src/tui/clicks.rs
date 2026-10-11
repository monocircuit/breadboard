//! The log of the overview's clicks (#303, overview/clicks-log.mjs): where
//! each one landed and what ran, so a click that did nothing can be traced
//! afterwards. One line each in `$TMPDIR/overview-clicks.log`, shared by
//! every overview running, Node's and this one, in the same form.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Past this size the older half of the log goes.
const MAX_BYTES: u64 = 256 * 1024;

/// One click: the screen cell (from 0), the row of the page if it hit one,
/// what ran (`button <label>`, `line`, `nothing`, or a note such as
/// `release without its press`), and the row as drawn, plain.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub row: u16,
    pub column: u16,
    pub view_row: Option<u16>,
    pub ran: String,
    pub line: String,
}

pub fn path() -> PathBuf {
    std::env::temp_dir().join("overview-clicks.log")
}

/// The line of the log, rows and columns from 1 as the terminal counts.
pub fn line(entry: &Entry, at: SystemTime, pid: u32) -> String {
    let mut parts = vec![
        iso(at),
        format!("pid {pid}"),
        format!("row {} col {}", entry.row + 1, entry.column + 1),
    ];
    if let Some(row) = entry.view_row {
        parts.push(format!("view {}", row + 1));
    }
    parts.push(format!("→ {}", entry.ran));
    let line = entry.line.trim();
    if !line.is_empty() {
        parts.push(format!("| {}", line.chars().take(160).collect::<String>()));
    }
    parts.join(" ")
}

/// `entry` appended to the log; a log that can't be written is no reason to fail a click.
pub fn log(entry: &Entry) {
    let _ = append(&path(), &line(entry, SystemTime::now(), std::process::id()));
}

fn append(file: &Path, line: &str) -> std::io::Result<()> {
    if fs::metadata(file).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let text = fs::read_to_string(file)?;
        let half = &text[text.ceil_char_boundary(text.len() / 2)..];
        fs::write(file, half.split_once('\n').map_or("", |(_, rest)| rest))?;
    }
    writeln!(OpenOptions::new().create(true).append(true).open(file)?, "{line}")
}

/// `2026-10-01T14:57:03.120Z`, as JavaScript's `toISOString`.
fn iso(at: SystemTime) -> String {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (days, seconds) = ((since.as_secs() / 86_400) as i64, since.as_secs() % 86_400);
    // Days to a civil date (Howard Hinnant's days_from_civil, inverted).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        since.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_line_as_node_writes_it() {
        let at = UNIX_EPOCH + Duration::from_millis(1_790_000_000_120);
        let entry = Entry {
            row: 14,
            column: 2,
            view_row: Some(40),
            ran: "button looks good".into(),
            line: "  looks good   broken  ".into(),
        };
        assert_eq!(
            line(&entry, at, 42),
            "2026-09-21T14:13:20.120Z pid 42 row 15 col 3 view 41 → button looks good | looks good   broken"
        );
        let nothing = Entry {
            ran: "nothing".into(),
            ..Entry::default()
        };
        assert_eq!(
            line(&nothing, UNIX_EPOCH, 1),
            "1970-01-01T00:00:00.000Z pid 1 row 1 col 1 → nothing"
        );
    }

    #[test]
    fn leap_days_and_year_ends() {
        let day = |days: u64| iso(UNIX_EPOCH + Duration::from_secs(days * 86_400))[..10].to_string();
        assert_eq!(day(11_016), "2000-02-29");
        assert_eq!(day(11_017), "2000-03-01");
        assert_eq!(day(20_453), "2025-12-31");
    }

    #[test]
    fn the_log_keeps_its_newer_half() {
        let dir = std::env::temp_dir().join(format!("breadboard-clicks-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("overview-clicks.log");
        fs::write(&file, "old line\n".repeat(40_000)).unwrap();
        append(&file, "new").unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.len() < 200_000 && text.ends_with("old line\nnew\n"));
        fs::remove_dir_all(dir).unwrap();
    }
}
