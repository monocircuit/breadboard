//! What a worktree's node says of its work in plain words (#341): the
//! problem, from the `## Goal` / `## Bug` section of its issue, and what its
//! agent is on now, the newer of its note (`pnpm wt:action note`, in
//! `.claude/note.json`) and the first line of its last answer. Read from
//! what is there already: the issue's description of the GitLab read, the
//! worktree's files, the transcript; no model asked, no extra request.

use crate::json::{self, str_of};
use crate::time::{parse_iso, to_iso};
use regex::Regex;
use serde_json::json;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// How long the problem may grow: one or two sentences.
pub const PROBLEM_CHARS: usize = 200;
/// How long the line of what the agent does may grow.
pub const NOW_CHARS: usize = 140;

/// The sections that say what is wrong or wanted, the first one found taken.
const SECTIONS: [&str; 4] = ["goal", "bug", "problem", "why"];

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,6}\s+(.*?)\s*#*\s*$").unwrap());
static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*```").unwrap());
/// A line's lead that is no text: a list's marker, a quote's `>`, a box.
static LEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(?:[-*+]\s+(?:\[[ xX]\]\s+)?|\d+[.)]\s+)?").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!?\[([^\]]*)\]\([^)]*\)").unwrap());
static MARKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*|__|`").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Markdown as the words it shows: links by their text, no `**` or backticks, one space between words.
pub fn plain(text: &str) -> String {
    let text = LINK.replace_all(text, "$1");
    let text = MARKS.replace_all(&text, "");
    SPACES.replace_all(text.trim(), " ").into_owned()
}

/// The problem an issue describes: the first paragraph of its `## Goal`
/// (else `## Bug`, `## Problem`, `## Why`), its first sentences up to
/// `PROBLEM_CHARS`. None without such a section.
pub fn problem_of(description: &str) -> Option<String> {
    let lines: Vec<&str> = description.lines().collect();
    let start = SECTIONS.iter().find_map(|name| {
        lines.iter().position(|line| {
            HEADING
                .captures(line)
                .is_some_and(|found| found[1].trim().trim_end_matches(':').eq_ignore_ascii_case(name))
        })
    })?;
    let mut paragraph = Vec::new();
    for line in &lines[start + 1..] {
        if HEADING.is_match(line) || FENCE.is_match(line) {
            break;
        }
        if line.trim().is_empty() {
            if paragraph.is_empty() {
                continue;
            }
            break;
        }
        paragraph.push(LEAD.replace(line, "").into_owned());
    }
    let text = plain(&paragraph.join(" "));
    (!text.is_empty()).then(|| sentences(&text, PROBLEM_CHARS))
}

/// Where each sentence of `text` ends: after `.`, `!` or `?` (and a closing
/// quote or bracket) that a space or the end follows.
fn sentence_ends(text: &str) -> Vec<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut ends = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i].1, '.' | '!' | '?') {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j].1, '"' | '\'' | ')' | '”' | '’' | '.' | '!' | '?') {
                j += 1;
            }
            if j == chars.len() || chars[j].1.is_whitespace() {
                ends.push(chars.get(j).map_or(text.len(), |(at, _)| *at));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    ends
}

/// The whole sentences of `text` that fit in `max` characters, at least the
/// first. A first sentence of more than twice `max` is cut after the last
/// whole word that fits, then `…`: never in the middle of a word.
pub fn sentences(text: &str, max: usize) -> String {
    let text = text.trim();
    let ends = sentence_ends(text);
    let count = |end: usize| text[..end].chars().count();
    let fitting = ends.iter().copied().take_while(|end| count(*end) <= max).last();
    let first = ends.first().copied().unwrap_or(text.len());
    let end = fitting.unwrap_or(first);
    if count(end) <= max * 2 {
        return text[..end].trim_end().to_string();
    }
    let mut cut = String::new();
    for word in text.split(' ') {
        if cut.chars().count() + word.chars().count() + 1 > max {
            break;
        }
        if !cut.is_empty() {
            cut.push(' ');
        }
        cut.push_str(word);
    }
    let cut = cut.trim_end_matches([',', ';', ':', '-', '—', ' ']);
    format!("{cut} …")
}

/// The first line of a text with something on it, as plain words.
pub fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| plain(&LEAD.replace(line, "")))
        .find(|line| !line.is_empty())
}

/// What an agent chose to say it does (`pnpm wt:action note`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub text: String,
    /// ms since the epoch.
    pub at: i64,
}

/// Where a worktree keeps its agent's note, in its ignored `.claude/`: `{ text, at }`.
pub fn note_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("note.json")
}

/// The note of the worktree `dir`, None without one.
pub fn note_of(dir: &Path) -> Option<Note> {
    let value = json::read(&note_file(dir))?;
    let text = str_of(&value, "text")?.trim();
    let at = parse_iso(str_of(&value, "at")?)?;
    (!text.is_empty()).then(|| Note {
        text: text.to_string(),
        at,
    })
}

/// `text` as the agent's note, the one before replaced: the node shows the latest only.
pub fn set_note(dir: &Path, text: &str, now: i64) -> io::Result<Note> {
    let note = Note {
        text: text.trim().to_string(),
        at: now,
    };
    json::write_pretty(&note_file(dir), &json!({ "text": note.text, "at": to_iso(now) }))?;
    Ok(note)
}

/// What the agent is on now, as its node says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Now {
    pub text: String,
    /// ms since the epoch, None for an answer whose time is unknown.
    pub at: Option<i64>,
    /// Its note (`›`), else the first line of its last answer (`"`).
    pub is_note: bool,
}

/// The newer of the agent's note and the first line of its last answer,
/// cut to `NOW_CHARS`; a note wins a tie and over an answer without a time.
pub fn now_of(note: Option<&Note>, answer: Option<&str>, answer_at: Option<i64>) -> Option<Now> {
    let said = answer.and_then(first_line);
    let is_note_newer = note.is_some_and(|note| said.is_none() || answer_at.is_none_or(|at| note.at >= at));
    if let Some(note) = note.filter(|_| is_note_newer) {
        return Some(Now {
            text: sentences(&plain(&note.text), NOW_CHARS),
            at: Some(note.at),
            is_note: true,
        });
    }
    Some(Now {
        text: sentences(&said?, NOW_CHARS),
        at: answer_at,
        is_note: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::scratch::Scratch;

    #[test]
    fn takes_the_problem_from_the_goal_cut_at_a_sentence_end() {
        let description = "## Goal\n\nEach worktree node in WORKTREES says **what problem it is working on**, in plain words, not only the issue title. The user's words (2026-10-11): \"for the worktrees I'd like to have a better description of the problem the worktree is currently working on.\"\n\nToday a node ends with the title.\n\n## Scope\n\n- [ ] a box";
        assert_eq!(
            problem_of(description).as_deref(),
            Some(
                "Each worktree node in WORKTREES says what problem it is working on, in plain words, not only the issue title."
            )
        );
    }

    #[test]
    fn takes_a_bug_section_and_keeps_two_sentences_that_fit() {
        let description =
            "Some intro.\n\n## Bug\n`wt:new` hangs. It waits for [herdr](https://x) forever.\nMore here.\n\n## Fix\nx";
        assert_eq!(
            problem_of(description).as_deref(),
            Some("wt:new hangs. It waits for herdr forever. More here.")
        );
        assert_eq!(problem_of("## Scope\n- [ ] a"), None);
        assert_eq!(problem_of("## Goal\n\n## Scope"), None);
    }

    #[test]
    fn a_sentence_too_long_is_cut_after_a_whole_word() {
        let long = "word ".repeat(100);
        let cut = sentences(&long, 20);
        assert_eq!(cut, "word word word word …");
        // A version number or a path is no sentence end.
        assert_eq!(sentences("Next 16.3.8 lands. Then more.", 20), "Next 16.3.8 lands.");
        assert_eq!(
            sentences("It said \"done.\" Then more text follows here.", 16),
            "It said \"done.\""
        );
    }

    #[test]
    fn the_newer_of_the_note_and_the_answer_is_what_it_does_now() {
        let note = Note {
            text: "cause found: the cache keeps old issues".into(),
            at: 2_000,
        };
        let answer = "\n**Pushed** the fix.\nMore lines.";
        let now = now_of(Some(&note), Some(answer), Some(1_000)).unwrap();
        assert_eq!(
            (now.text.as_str(), now.at, now.is_note),
            ("cause found: the cache keeps old issues", Some(2_000), true)
        );
        let now = now_of(Some(&note), Some(answer), Some(3_000)).unwrap();
        assert_eq!((now.text.as_str(), now.is_note), ("Pushed the fix.", false));
        // An answer without a time loses to the note.
        assert!(now_of(Some(&note), Some(answer), None).unwrap().is_note);
        assert!(!now_of(None, Some(answer), None).unwrap().is_note);
        assert_eq!(now_of(None, Some("  \n"), Some(1)), None);
        assert_eq!(now_of(None, None, None), None);
    }

    #[test]
    fn keeps_the_note_in_the_worktree() {
        let scratch = Scratch::new();
        assert_eq!(note_of(scratch.path()), None);
        let note = set_note(
            scratch.path(),
            "  fix pushed, waiting for the pipeline ",
            1_760_000_000_000,
        )
        .unwrap();
        assert_eq!(note_of(scratch.path()), Some(note));
        assert_eq!(
            note_of(scratch.path()).unwrap().text,
            "fix pushed, waiting for the pipeline"
        );
    }
}
