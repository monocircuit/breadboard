//! The orchestration's journal (#382): what the orchestrator keeps outside
//! its chat, `.claude/orchestrations/<name>.journal.md` next to its record.
//! Decisions taken for the user, open threads, who waits for what: one line
//! each, `- <day> <time> <text>`. The overview shows its last lines, a fresh
//! orchestrator starts from it.

use crate::record::records_dir;
use crate::time::{day, hm};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn journal_file(main: &Path, name: &str) -> PathBuf {
    records_dir(main).join(format!("{name}.journal.md"))
}

/// One line of the journal: whitespace of any kind folded into single
/// spaces, so a pasted paragraph stays one entry.
pub fn entry_of(text: &str, at: i64) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then(|| format!("- {} {} {text}", day(at), hm(at)))
}

/// `text` appended as an entry at `at`, the file made with its heading
/// first: the entry written.
pub fn append(main: &Path, name: &str, text: &str, at: i64) -> io::Result<String> {
    let entry = entry_of(text, at).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "an empty line"))?;
    let file = journal_file(main, name);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let is_new = !file.exists();
    let mut out = OpenOptions::new().create(true).append(true).open(&file)?;
    if is_new {
        writeln!(out, "# Journal of the orchestration {name}\n")?;
    }
    writeln!(out, "{entry}")?;
    Ok(entry)
}

/// The entries of the journal, oldest first; none without one.
pub fn entries(main: &Path, name: &str) -> Vec<String> {
    fs::read_to_string(journal_file(main, name))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(String::from)
        .collect()
}

/// Its last `count` entries, oldest first.
pub fn last(main: &Path, name: &str, count: usize) -> Vec<String> {
    let all = entries(main, name);
    all[all.len().saturating_sub(count)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_a_paragraph_into_one_entry_and_keeps_the_order() {
        let main = std::env::temp_dir().join(format!("journal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&main);
        assert!(entry_of("  \n ", 0).is_none());
        append(&main, "o", "decided: #3 waits\nfor !12", 0).unwrap();
        append(&main, "o", "open: ask about #4", 60_000).unwrap();
        let text = fs::read_to_string(journal_file(&main, "o")).unwrap();
        assert!(text.starts_with("# Journal of the orchestration o\n\n- "), "{text}");
        let all = entries(&main, "o");
        assert_eq!(all.len(), 2);
        assert!(all[0].ends_with(" decided: #3 waits for !12"), "{all:?}");
        assert_eq!(last(&main, "o", 1), all[1..].to_vec());
        assert_eq!(last(&main, "o", 5), all);
        assert!(entries(&main, "none").is_empty());
        let _ = fs::remove_dir_all(&main);
    }
}
