//! A file's changes inline under its row (lazygit's diff view): a click or
//! Enter on a file of DIFF or DEVELOP shows its hunks against the merge base
//! with develop, `+` green and `−` red, a second one or Esc closes them, a
//! click on a line opens nvim there. Read once per change of the file's
//! counts, not per frame.

use super::super::{Act, look};
use crate::tui::Scene;
use crate::tui::theme::{self, Tone};
use breadboard_core::git::git;
use ratatui::style::Style;
use ratatui::text::Span;
use std::cell::RefCell;
use std::path::Path;

/// How many lines of a file's hunks show before `… N more lines`.
pub const LINES: usize = 30;
/// Where the hunks' lines start: under a file's letter.
const AT: u16 = 2;

/// Which file's hunks show, under which row, and what was read for them.
#[derive(Debug, Default)]
pub struct Shown {
    pub file: String,
    pub at: String,
    /// The file's counts it was read at, and what it read.
    read: RefCell<Option<(Key, Read)>>,
}

/// A file's counts: when they change, its hunks are read again.
pub type Key = (u64, u64, bool);

/// What a read gives: the lines, or why there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    Lines(Vec<Line>),
    Note(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Head,
    Added,
    Removed,
    Context,
}

/// A line of a hunk and the line of the working file a click opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    pub at: u64,
}

impl Shown {
    pub fn new(file: String, at: String) -> Self {
        Self {
            file,
            at,
            read: RefCell::new(None),
        }
    }

    /// Whether these hunks go under the row `at` naming `file`.
    pub fn is_under(&self, at: &str, file: &str) -> bool {
        self.at == at && self.file == file
    }

    /// The hunks, read again when the file's counts changed since.
    fn read(&self, dir: &Path, key: Key, is_untracked: bool) -> Read {
        let mut read = self.read.borrow_mut();
        match &*read {
            Some((was, lines)) if *was == key => lines.clone(),
            _ => {
                let lines = read_hunks(dir, &self.file, is_untracked);
                *read = Some((key, lines.clone()));
                lines
            }
        }
    }
}

/// `git diff -U3 <merge base> -- <file>`, from /dev/null for a file git doesn't track.
fn read_hunks(dir: &Path, file: &str, is_untracked: bool) -> Read {
    let out = if is_untracked {
        git(dir, &["diff", "--no-index", "-U3", "--", "/dev/null", file])
    } else {
        let base = git(dir, &["merge-base", "origin/develop", "HEAD"]);
        if !base.is_ok || base.out().is_empty() {
            return Read::Failed("no merge base with origin/develop".into());
        }
        git(dir, &["diff", "-U3", base.out(), "--", file])
    };
    // `--no-index` answers 1 when the files differ.
    if !out.is_ok && out.stdout.is_empty() {
        let why = out.stderr.lines().next().unwrap_or("git diff failed").to_string();
        return Read::Failed(why);
    }
    if out.stdout.lines().any(|line| line.starts_with("Binary files")) {
        return Read::Note("a binary file: no lines to show".into());
    }
    let lines = parse(&out.stdout);
    if lines.is_empty() {
        return Read::Note("no lines changed against develop".into());
    }
    Read::Lines(lines)
}

/// The hunks of a `git diff`: each line with the line of the new file it stands at.
pub fn parse(diff: &str) -> Vec<Line> {
    let mut lines = vec![];
    let mut new = 0u64;
    let mut is_in_hunk = false;
    for text in diff.lines() {
        if let Some(head) = text.strip_prefix("@@ ") {
            is_in_hunk = true;
            // `-a,b +c,d @@ context`: the new file's lines start at c.
            new = head
                .split_whitespace()
                .find_map(|part| part.strip_prefix('+'))
                .and_then(|part| part.split(',').next()?.parse().ok())
                .unwrap_or(1);
            lines.push(Line {
                kind: Kind::Head,
                text: text.to_string(),
                at: new.max(1),
            });
            continue;
        }
        if !is_in_hunk {
            continue;
        }
        let (kind, rest) = match text.chars().next() {
            Some('+') => (Kind::Added, &text[1..]),
            Some('-') => (Kind::Removed, &text[1..]),
            Some(' ') => (Kind::Context, &text[1..]),
            Some('\\') => continue,
            None => (Kind::Context, ""),
            // The header of a next file (a rename): not a hunk any more.
            _ => {
                is_in_hunk = false;
                continue;
            }
        };
        lines.push(Line {
            kind,
            text: rest.replace('\t', "    "),
            at: new.max(1),
        });
        if kind != Kind::Removed {
            new += 1;
        }
    }
    lines
}

/// The hunks of `shown` under the row that opened them: a dim line with the
/// file's path, then each line of the diff as a block that opens nvim at it,
/// a line too long for the pane wrapping under its text.
pub fn draw(scene: &mut Scene<Act>, shown: &Shown, dir: &Path, key: Key, is_untracked: bool) {
    let file = &shown.file;
    let open_at = |line: u64| Act::OpenFile {
        dir: dir.to_path_buf(),
        file: file.clone(),
        line: Some(line),
    };
    let close = Act::Hunks {
        file: file.clone(),
        at: shown.at.clone(),
    };
    let lines = match shown.read(dir, key, is_untracked) {
        Read::Lines(lines) => lines,
        Read::Note(note) => return say(scene, close, &note, theme::dim()),
        Read::Failed(why) => return say(scene, close, &why, theme::tone(Tone::Red)),
    };
    let gutter = lines
        .iter()
        .map(|line| line.at.to_string().len())
        .max()
        .unwrap_or(1)
        .max(3) as u16;
    let text_at = AT + gutter + 3;
    scene.block(close, |scene| {
        scene.pad_to(AT);
        scene.indent(AT);
        scene.atom_gap(vec![look::dim(file)], 0);
        look::say(scene, "· a line: nvim there · Esc closes", theme::dim());
        scene.indent(0);
    });
    for line in lines.iter().take(LINES) {
        scene.block(open_at(line.at), |scene| {
            scene.pad_to(AT);
            if line.kind == Kind::Head {
                scene.indent(AT + 2);
                scene.atom_gap(vec![look::span(&line.text, theme::tone(Tone::Cyan))], 0);
                scene.indent(0);
                return;
            }
            let (sign, style) = match line.kind {
                Kind::Added => ("+", theme::tone(Tone::Green)),
                Kind::Removed => ("−", theme::tone(Tone::Red)),
                _ => (" ", Style::new()),
            };
            let number = if line.kind == Kind::Removed {
                String::new()
            } else {
                line.at.to_string()
            };
            scene.pad_to(AT + gutter - number.len() as u16);
            if !number.is_empty() {
                scene.atom_gap(vec![look::dim(number)], 0);
            }
            scene.pad_to(AT + gutter + 1);
            scene.atom_gap(vec![look::span(sign, style)], 0);
            scene.pad_to(text_at);
            scene.indent(text_at);
            code(scene, &line.text, style);
            scene.indent(0);
        });
    }
    if let Some(next) = lines.get(LINES) {
        let more = lines.len() - LINES;
        scene.block(open_at(next.at), |scene| {
            scene.indent(AT);
            let rows = if more == 1 { "line" } else { "lines" };
            scene.text(&format!("… {more} more {rows}, click: nvim there"), theme::dim());
            scene.indent(0);
        });
    }
}

/// A one-line answer in place of hunks; a click closes it.
fn say(scene: &mut Scene<Act>, close: Act, text: &str, style: Style) {
    scene.block(close, |scene| {
        scene.pad_to(AT);
        scene.indent(AT);
        scene.atom_gap(vec![], 0);
        look::say(scene, text, style);
        scene.indent(0);
    });
}

/// A line of code as it is: its spaces kept, broken at the edge of the pane
/// when it is wider, never cut (#304).
fn code(scene: &mut Scene<Act>, text: &str, style: Style) {
    let room = scene.width().saturating_sub(scene.x()).max(1) as usize;
    let mut piece = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let char_width = Span::raw(ch.to_string()).width();
        if width + char_width > room && !piece.is_empty() {
            scene.atom_gap(vec![look::span(std::mem::take(&mut piece), style)], 0);
            scene.newline();
            width = 0;
        }
        piece.push(ch);
        width += char_width;
    }
    if !piece.is_empty() {
        scene.atom_gap(vec![look::span(piece, style)], 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::scene::Target;

    fn text(scene: &Scene<Act>) -> Vec<String> {
        scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .map(|line| line.trim_end().to_string())
            .collect()
    }

    const DIFF: &str = "diff --git a/a.ts b/a.ts\nindex 1..2 100644\n--- a/a.ts\n+++ b/a.ts\n@@ -1,3 +1,3 @@ fn x\n one\n-two\n+zwei\n three\n\\ No newline at end of file\n";

    #[test]
    fn a_diff_parses_to_lines_at_the_new_files_lines() {
        let lines = parse(DIFF);
        let kinds: Vec<(Kind, &str, u64)> = lines
            .iter()
            .map(|line| (line.kind, line.text.as_str(), line.at))
            .collect();
        assert_eq!(
            kinds,
            [
                (Kind::Head, "@@ -1,3 +1,3 @@ fn x", 1),
                (Kind::Context, "one", 1),
                (Kind::Removed, "two", 2),
                (Kind::Added, "zwei", 2),
                (Kind::Context, "three", 3),
            ]
        );
        let new_file = parse("--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+x\n");
        assert_eq!(
            new_file[1],
            Line {
                kind: Kind::Added,
                text: "x".into(),
                at: 1
            }
        );
    }

    #[test]
    fn hunks_show_under_their_row_wrap_whole_and_open_nvim_at_their_line() {
        let shown = Shown::new("src/a.ts".into(), "row".into());
        let mut lines = parse(DIFF);
        lines.push(Line {
            kind: Kind::Added,
            text: "a line of code that is longer than the pane is wide".into(),
            at: 4,
        });
        *shown.read.borrow_mut() = Some(((1, 1, false), Read::Lines(lines)));
        let mut scene = Scene::new(40);
        draw(&mut scene, &shown, Path::new("/w"), (1, 1, false), false);
        scene.finish();
        assert_eq!(
            text(&scene),
            [
                "  src/a.ts · a line: nvim there · Esc",
                "  closes",
                "    @@ -1,3 +1,3 @@ fn x",
                "    1   one",
                "      − two",
                "    2 + zwei",
                "    3   three",
                "    4 + a line of code that is longer th",
                "        an the pane is wide",
            ]
        );
        let act_at = |y: u16| {
            scene
                .regions
                .iter()
                .rev()
                .find(|region| region.contains(10, y) && matches!(region.target, Target::Block(_)))
                .map(|region| region.action().clone())
        };
        let open = |line| {
            Some(Act::OpenFile {
                dir: "/w".into(),
                file: "src/a.ts".into(),
                line: Some(line),
            })
        };
        assert_eq!(act_at(4), open(2));
        assert_eq!(act_at(8), open(4));
        assert_eq!(
            act_at(0),
            Some(Act::Hunks {
                file: "src/a.ts".into(),
                at: "row".into()
            })
        );
    }

    #[test]
    fn more_lines_than_shown_end_in_a_row_that_counts_them() {
        let shown = Shown::new("a".into(), "row".into());
        let lines: Vec<Line> = (1..=LINES as u64 + 5)
            .map(|at| Line {
                kind: Kind::Added,
                text: "x".into(),
                at,
            })
            .collect();
        *shown.read.borrow_mut() = Some(((0, 0, true), Read::Lines(lines)));
        let mut scene = Scene::new(60);
        draw(&mut scene, &shown, Path::new("/w"), (0, 0, true), true);
        scene.finish();
        assert_eq!(text(&scene).last().unwrap(), "  … 5 more lines, click: nvim there");
    }
}
