//! A file's changes inline under its row (lazygit's diff view): a click or
//! Enter on a file of DIFF or DEVELOP shows its hunks against the merge base
//! with develop, `+` green and `−` red, a second one or Esc closes them. A
//! file develop changed too (a conflict, `also changed on develop`) shows
//! both sides, `here` and `on develop`, as lazygit's merge view explains a
//! conflict. Each hunk of this checkout is one stop of Tab (Enter: nvim at
//! its first change), each of its lines a click away from nvim at that line.
//! Read again when the file's counts or the file itself changed (its size
//! and change time, looked at on each build): an edit shows within one local
//! read, and a line's nvim target is always the line on disk.

use super::super::{Act, look};
use crate::tui::Scene;
use crate::tui::scene::{Region, Target};
use crate::tui::theme::{self, Tone};
use breadboard_core::git::{Out, base_of, git};
use breadboard_core::model::diff::stamp_of;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use std::cell::RefCell;
use std::path::Path;

/// How many lines of a file's hunks show before `… N more lines`, a click
/// on which shows them all.
pub const LINES: usize = 30;
/// Where the hunks' lines start: under a file's letter.
const AT: u16 = 2;
/// The `at` of a line's step (diff.rs `step`): the next name of the line.
pub const NEXT: &str = "next:";
/// The rows whose files develop changed too: their hunks show both sides.
const BOTH: [&str; 2] = ["conflicts", "develop"];

/// Which file's hunks show, under which row, and what was read for them.
#[derive(Debug, Default)]
pub struct Shown {
    pub file: String,
    pub at: String,
    /// All of its lines, not only the first LINES.
    pub is_whole: bool,
    /// Shown because the pick moved onto its row (acts.rs `follow`): its
    /// hunks are no stops of Tab, so Tab goes on to the next file, until
    /// Enter on the row.
    pub is_followed: bool,
    /// The file's counts and stamp it was read at, and what it read.
    read: RefCell<Option<(Key, String, Sides)>>,
    /// The files of the line of names it shows under, as last drawn: where a step goes next.
    line: RefCell<Vec<String>>,
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

/// This checkout's changes of the file, and develop's where they meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sides {
    pub here: Read,
    pub develop: Option<Read>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Head,
    Added,
    Removed,
    Context,
}

/// A line of a hunk and the line of its side's file it stands at.
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
            is_whole: false,
            is_followed: false,
            read: RefCell::new(None),
            line: RefCell::new(Vec::new()),
        }
    }

    /// It shows under a line of these names.
    pub fn on_line(&self, files: Vec<String>) {
        *self.line.borrow_mut() = files;
    }

    /// The name after this one on its line: Some(None) after the last, None off a line.
    pub fn next_on_line(&self) -> Option<Option<String>> {
        let line = self.line.borrow();
        let at = line.iter().position(|file| *file == self.file)?;
        Some(line.get(at + 1).cloned())
    }

    /// Whether these hunks go under the row `at` naming `file`.
    pub fn is_under(&self, at: &str, file: &str) -> bool {
        self.at == at && self.file == file
    }

    /// Whether develop's side shows too: the row says develop changed the file.
    pub fn is_both(&self) -> bool {
        BOTH.contains(&self.at.as_str())
    }

    /// The hunks, read again when the file's counts or the file changed since.
    fn read(&self, dir: &Path, key: Key, is_untracked: bool) -> Sides {
        let stamp = stamp_of(&dir.join(&self.file));
        let mut read = self.read.borrow_mut();
        match &*read {
            Some((was, at, sides)) if *was == key && *at == stamp => sides.clone(),
            _ => {
                let sides = Sides {
                    here: read_hunks(dir, &self.file, is_untracked),
                    develop: self.is_both().then(|| read_develop(dir, &self.file)),
                };
                *read = Some((key, stamp, sides.clone()));
                sides
            }
        }
    }
}

/// Where the branch left its base.
fn merge_base(dir: &Path) -> Result<String, Read> {
    breadboard_core::git::merge_base(dir)
        .ok_or_else(|| Read::Failed(format!("no merge base with {}", base_of(dir).remote)))
}

/// `git diff -U3 <merge base> -- <file>`, from /dev/null for a file git doesn't track.
fn read_hunks(dir: &Path, file: &str, is_untracked: bool) -> Read {
    let out = if is_untracked {
        git(dir, &["diff", "--no-index", "-U3", "--", "/dev/null", file])
    } else {
        match merge_base(dir) {
            Ok(base) => git(dir, &["diff", "-U3", &base, "--", file]),
            Err(why) => return why,
        }
    };
    lines_of(&out, &format!("no lines changed against {}", base_of(dir).local))
}

/// `git diff -U3 <merge base> origin/<base> -- <file>`: what its base did to it since.
fn read_develop(dir: &Path, file: &str) -> Read {
    let against = base_of(dir);
    match merge_base(dir) {
        Ok(base) => lines_of(
            &git(dir, &["diff", "-U3", &base, &against.remote, "--", file]),
            &format!("{} changed no line of it", against.local),
        ),
        Err(why) => why,
    }
}

/// The lines of a diff, or why there are none.
fn lines_of(out: &Out, unchanged: &str) -> Read {
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
        return Read::Note(unchanged.into());
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
/// file's path (plain, so Enter and the pick stay on the row), then each
/// hunk as a block that opens nvim at its first change, each of its lines a
/// button that opens nvim at itself (a file deleted here has none to open),
/// a line too long for the pane wrapping under its text. Develop's side, when
/// it shows, opens nothing: its lines aren't in this checkout.
pub fn draw(scene: &mut Scene<Act>, shown: &Shown, dir: &Path, key: Key, is_untracked: bool) {
    let file = &shown.file;
    let sides = shown.read(dir, key, is_untracked);
    let is_gone = !dir.join(file).is_file();
    scene.end_line();
    scene.pad_to(AT);
    scene.indent(AT);
    scene.atom_gap(vec![look::dim(file)], 0);
    let hint = if is_gone {
        "· deleted here"
    } else if sides.develop.is_some() {
        "· a line here: nvim there"
    } else {
        "· a line: nvim there"
    };
    // The keys say themselves here too: a narrow band drops `Tab Enter pick a row`.
    let tab = match (is_gone, shown.is_followed) {
        (true, _) => "",
        (false, true) => " · Tab: next file · Enter: Tab into its hunks",
        (false, false) => " · Tab: hunk by hunk",
    };
    let next = if !shown.is_followed && shown.line.borrow().len() > 1 {
        " · Enter on its line: next name"
    } else {
        ""
    };
    look::say(scene, &format!("{hint}{tab}{next} · Esc closes"), theme::dim());
    scene.indent(0);
    scene.end_line();
    let numbers = [Some(&sides.here), sides.develop.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|read| match read {
            Read::Lines(lines) => Some(lines),
            _ => None,
        })
        .flatten()
        .map(|line| line.at.to_string().len());
    let gutter = numbers.max().unwrap_or(1).max(3) as u16;
    let mut side = Side {
        dir,
        file,
        gutter,
        left: if shown.is_whole { usize::MAX } else { LINES },
        hidden: 0,
        is_stop: !shown.is_followed,
    };
    match &sides.develop {
        None => side.draw(scene, &sides.here, !is_gone),
        Some(develop) => {
            heading(scene, "here");
            side.draw(scene, &sides.here, !is_gone);
            heading(scene, &format!("on {}", base_of(dir).local));
            side.draw(scene, develop, false);
        }
    }
    if side.hidden > 0 {
        let more = side.hidden;
        let whole = Act::Hunks {
            file: file.clone(),
            at: shown.at.clone(),
            is_whole: true,
        };
        let rows = if more == 1 { "line" } else { "lines" };
        let text = format!("… {more} more {rows}, click: all of them");
        let draw = |scene: &mut Scene<Act>| {
            scene.indent(AT);
            scene.text(&text, theme::dim());
            scene.indent(0);
        };
        if side.is_stop {
            scene.block(whole, draw);
        } else {
            line_button(scene, AT, format!("{file}: all lines"), whole, draw);
        }
    }
}

/// `here` / `on develop` over a side's hunks, dim.
fn heading(scene: &mut Scene<Act>, text: &str) {
    scene.end_line();
    scene.pad_to(AT);
    scene.atom_gap(vec![look::dim(text)], 0);
    scene.end_line();
}

/// The hunks of one side as they are drawn: where, how wide the numbers
/// are, how many lines may still show and how many didn't.
struct Side<'a> {
    dir: &'a Path,
    file: &'a str,
    gutter: u16,
    left: usize,
    hidden: usize,
    /// Each hunk a stop of Tab; else (followed hunks) only its lines click.
    is_stop: bool,
}

impl Side<'_> {
    fn open_at(&self, line: u64) -> Act {
        Act::OpenFile {
            dir: self.dir.to_path_buf(),
            file: self.file.to_string(),
            line: Some(line),
        }
    }

    /// A side's hunks, or its one-line answer; `is_open`: they are this
    /// checkout's, a hunk and a line open nvim.
    fn draw(&mut self, scene: &mut Scene<Act>, read: &Read, is_open: bool) {
        let lines = match read {
            Read::Lines(lines) => lines,
            Read::Note(note) => return say(scene, note, theme::dim()),
            Read::Failed(why) => return say(scene, why, theme::tone(Tone::Red)),
        };
        for hunk in hunks_of(lines) {
            let shown = hunk.len().min(self.left);
            self.left -= shown;
            self.hidden += hunk.len() - shown;
            if shown == 0 {
                continue;
            }
            let hunk = &hunk[..shown];
            if !is_open {
                hunk.iter().for_each(|line| draw_line(scene, line, self.gutter));
                continue;
            }
            let first = first_change(hunk);
            let lines = |scene: &mut Scene<Act>| {
                for line in hunk {
                    // The `@@` row is the hunk's own: as its block, or a button to its first change.
                    if line.kind == Kind::Head && self.is_stop {
                        draw_line(scene, line, self.gutter);
                        continue;
                    }
                    let at = if line.kind == Kind::Head { first } else { line.at };
                    let label = format!("{}:{at}", self.file);
                    line_button(scene, AT, label, self.open_at(at), |scene| {
                        draw_line(scene, line, self.gutter)
                    });
                }
            };
            if self.is_stop {
                scene.block(self.open_at(first), lines);
            } else {
                lines(scene);
            }
        }
    }
}

/// The hunks of a diff: each from its `@@` line to the next.
fn hunks_of(lines: &[Line]) -> Vec<&[Line]> {
    let mut hunks = vec![];
    let mut from = 0;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.kind == Kind::Head {
            hunks.push(&lines[from..i]);
            from = i;
        }
    }
    if from < lines.len() {
        hunks.push(&lines[from..]);
    }
    hunks
}

/// Where a hunk's change starts in the working file (files.mjs: a pure
/// deletion gives the line before it, at least 1).
fn first_change(hunk: &[Line]) -> u64 {
    let added = hunk.iter().find(|line| line.kind == Kind::Added);
    let removed = hunk.iter().find(|line| line.kind == Kind::Removed);
    match (added, removed) {
        (Some(line), _) => line.at,
        (None, Some(line)) => line.at.saturating_sub(1).max(1),
        (None, None) => hunk.first().map_or(1, |line| line.at),
    }
}

/// The rows `draw` writes as one button from column `from` to the edge: a
/// click on any of them runs `action`, it is no stop of Tab (the block
/// around it is), and the click log names it `label`.
pub(super) fn line_button(
    scene: &mut Scene<Act>,
    from: u16,
    label: String,
    action: Act,
    draw: impl FnOnce(&mut Scene<Act>),
) {
    scene.end_line();
    let top = scene.y();
    draw(scene);
    scene.end_line();
    let bottom = scene.y();
    if bottom > top {
        let area = Rect::new(from, top, scene.width().saturating_sub(from), bottom - top);
        let target = Target::Button {
            label,
            tone: Tone::Cyan,
            action,
        };
        scene.regions.push(Region { area, target });
    }
}

/// A line of a hunk: its number in its side's file (none for a removed
/// one), its sign, its text in the sign's colour; the line ended.
fn draw_line(scene: &mut Scene<Act>, line: &Line, gutter: u16) {
    scene.pad_to(AT);
    if line.kind == Kind::Head {
        scene.indent(AT + 2);
        scene.atom_gap(vec![look::span(&line.text, theme::tone(Tone::Cyan))], 0);
        scene.indent(0);
        scene.end_line();
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
    let text_at = AT + gutter + 3;
    scene.pad_to(text_at);
    scene.indent(text_at);
    code(scene, &line.text, style);
    scene.indent(0);
    scene.end_line();
}

/// A one-line answer in place of hunks.
fn say(scene: &mut Scene<Act>, text: &str, style: Style) {
    scene.end_line();
    scene.pad_to(AT);
    scene.indent(AT);
    look::say(scene, text, style);
    scene.indent(0);
    scene.end_line();
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

    fn text(scene: &Scene<Act>) -> Vec<String> {
        scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .map(|line| line.trim_end().to_string())
            .collect()
    }

    /// What a click at (x, y) runs, as the page's `hit`: a button, else the innermost block.
    fn act_at(scene: &Scene<Act>, x: u16, y: u16) -> Option<Act> {
        let inside = || scene.regions.iter().filter(move |region| region.contains(x, y));
        inside()
            .find(|region| region.is_button())
            .or_else(|| inside().min_by_key(|region| region.area.height))
            .map(|region| region.action().clone())
    }

    /// The stops of Tab: the blocks, not the buttons.
    fn stops(scene: &Scene<Act>) -> Vec<Act> {
        scene
            .regions
            .iter()
            .filter(|region| !region.is_button())
            .map(|region| region.action().clone())
            .collect()
    }

    fn stored(shown: &Shown, dir: &Path, key: Key, here: Read, develop: Option<Read>) {
        let stamp = stamp_of(&dir.join(&shown.file));
        *shown.read.borrow_mut() = Some((key, stamp, Sides { here, develop }));
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

    fn open(dir: &Path, line: u64) -> Option<Act> {
        Some(Act::OpenFile {
            dir: dir.to_path_buf(),
            file: "src/a.ts".into(),
            line: Some(line),
        })
    }

    #[test]
    fn hunks_wrap_whole_a_hunk_is_one_stop_and_each_line_opens_nvim_at_itself() {
        let shown = Shown::new("src/a.ts".into(), "row".into());
        let mut lines = parse(DIFF);
        lines.push(Line {
            kind: Kind::Added,
            text: "a line of code that is longer than the pane is wide".into(),
            at: 4,
        });
        lines.extend(parse("@@ -9,2 +10 @@\n ten\n-gone\n"));
        let dir = std::env::temp_dir().join(format!("breadboard-hunks-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.ts"), "one\nzwei\nthree\n").unwrap();
        stored(&shown, &dir, (1, 1, false), Read::Lines(lines), None);
        let mut scene = Scene::new(40);
        draw(&mut scene, &shown, &dir, (1, 1, false), false);
        scene.finish();
        assert_eq!(
            text(&scene),
            [
                "  src/a.ts · a line: nvim there · Tab:",
                "  hunk by hunk · Esc closes",
                "    @@ -1,3 +1,3 @@ fn x",
                "    1   one",
                "      − two",
                "    2 + zwei",
                "    3   three",
                "    4 + a line of code that is longer th",
                "        an the pane is wide",
                "    @@ -9,2 +10 @@",
                "   10   ten",
                "      − gone",
            ]
        );
        // The file's line is no stop and runs nothing: Enter and the pick stay on the row.
        assert_eq!(act_at(&scene, 10, 0), None);
        // A line opens nvim at itself, a wrapped one on each of its rows.
        assert_eq!(act_at(&scene, 10, 5), open(&dir, 2));
        assert_eq!(act_at(&scene, 10, 8), open(&dir, 4));
        // The hunk's head opens its first change; a pure deletion the line before it.
        assert_eq!(act_at(&scene, 10, 2), open(&dir, 2));
        assert_eq!(act_at(&scene, 10, 9), open(&dir, 10));
        // Tab stops at each hunk, not at each line.
        assert_eq!(stops(&scene), [open(&dir, 2).unwrap(), open(&dir, 10).unwrap()]);
        // Deleted here: nothing to open in nvim, the header says so.
        std::fs::remove_dir_all(&dir).unwrap();
        let mut scene = Scene::new(60);
        draw(&mut scene, &shown, &dir, (1, 1, false), false);
        scene.finish();
        assert_eq!(text(&scene)[0], "  src/a.ts · deleted here · Esc closes");
        assert!(scene.regions.is_empty());
    }

    #[test]
    fn hunks_that_follow_the_pick_are_lines_to_click_not_stops_of_tab() {
        let mut shown = Shown::new("src/a.ts".into(), "row".into());
        shown.is_followed = true;
        let dir = std::env::temp_dir().join(format!("breadboard-followed-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.ts"), "one\nzwei\nthree\n").unwrap();
        let mut lines = parse(DIFF);
        lines.extend(parse("@@ -9,2 +10 @@\n ten\n-gone\n"));
        stored(&shown, &dir, (1, 1, false), Read::Lines(lines), None);
        let mut scene = Scene::new(120);
        draw(&mut scene, &shown, &dir, (1, 1, false), false);
        scene.finish();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            text(&scene)[0],
            "  src/a.ts · a line: nvim there · Tab: next file · Enter: Tab into its hunks · Esc closes"
        );
        // No stops: Tab goes on to the next file's row.
        assert_eq!(stops(&scene), []);
        // Every line still clicks, the `@@` row to the hunk's first change.
        assert_eq!(act_at(&scene, 10, 1), open(&dir, 2));
        assert_eq!(act_at(&scene, 10, 4), open(&dir, 2));
        assert_eq!(act_at(&scene, 10, 6), open(&dir, 10));
    }

    #[test]
    fn a_file_develop_changed_too_shows_both_sides_and_only_ours_opens() {
        let shown = Shown::new("README.md".into(), "conflicts".into());
        assert!(shown.is_both());
        assert!(Shown::new("README.md".into(), "develop".into()).is_both());
        assert!(!Shown::new("README.md".into(), "file".into()).is_both());
        let here = parse("@@ -1 +1,2 @@\n-# platform\n+# platform (this branch)\n+more\n");
        let develop = parse("@@ -1 +1 @@\n-# platform\n+# platform (develop)\n");
        let dir = std::env::temp_dir().join(format!("breadboard-sides-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The heading names the base: the repo's default branch, breadboard.toml's.
        std::fs::write(dir.join("breadboard.toml"), "[repo]\ndefault_branch = \"develop\"\n").unwrap();
        std::fs::write(dir.join("README.md"), "# platform (this branch)\nmore\n").unwrap();
        stored(
            &shown,
            &dir,
            (2, 1, false),
            Read::Lines(here),
            Some(Read::Lines(develop)),
        );
        let mut scene = Scene::new(70);
        draw(&mut scene, &shown, &dir, (2, 1, false), false);
        scene.finish();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            text(&scene),
            [
                "  README.md · a line here: nvim there · Tab: hunk by hunk · Esc closes",
                "  here",
                "    @@ -1 +1,2 @@",
                "      − # platform",
                "    1 + # platform (this branch)",
                "    2 + more",
                "  on develop",
                "    @@ -1 +1 @@",
                "      − # platform",
                "    1 + # platform (develop)",
            ]
        );
        let readme = |line| {
            Some(Act::OpenFile {
                dir: dir.clone(),
                file: "README.md".into(),
                line: Some(line),
            })
        };
        assert_eq!(act_at(&scene, 10, 5), readme(2));
        // Develop's lines aren't in this checkout: no click, no stop.
        assert_eq!(act_at(&scene, 10, 9), None);
        assert_eq!(stops(&scene), [readme(1).unwrap()]);
    }

    #[test]
    fn an_edit_of_the_file_reads_its_hunks_again_with_the_same_counts() {
        let shown = Shown::new("a.md".into(), "file".into());
        let dir = std::env::temp_dir().join(format!("breadboard-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "one\n").unwrap();
        stored(
            &shown,
            &dir,
            (1, 0, true),
            Read::Lines(parse("@@ -0,0 +1 @@\n+one\n")),
            None,
        );
        let drawn = |shown: &Shown| {
            let mut scene = Scene::new(60);
            draw(&mut scene, shown, &dir, (1, 0, true), false);
            scene.finish();
            text(&scene)
        };
        assert_eq!(drawn(&shown)[2], "    1 + one");
        // Edited: read again (here outside a repo, so git says why), not the lines of before.
        std::fs::write(dir.join("a.md"), "one\ntwo\n").unwrap();
        let lines = drawn(&shown);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!lines.iter().any(|line| line.contains("+ one")), "{lines:?}");
    }

    #[test]
    fn more_lines_than_shown_end_in_a_row_that_counts_them() {
        let shown = Shown::new("a".into(), "row".into());
        let mut lines = vec![Line {
            kind: Kind::Head,
            text: "@@ -0,0 +1,35 @@".into(),
            at: 1,
        }];
        lines.extend((1..=LINES as u64 + 4).map(|at| Line {
            kind: Kind::Added,
            text: "x".into(),
            at,
        }));
        stored(&shown, Path::new("/w"), (0, 0, true), Read::Lines(lines), None);
        let mut scene = Scene::new(60);
        draw(&mut scene, &shown, Path::new("/w"), (0, 0, true), true);
        scene.finish();
        assert_eq!(text(&scene).last().unwrap(), "  … 5 more lines, click: all of them");
        let more = scene.lines.len() as u16 - 1;
        assert_eq!(
            act_at(&scene, 4, more),
            Some(Act::Hunks {
                file: "a".into(),
                at: "row".into(),
                is_whole: true
            })
        );
        // Whole: every line, no count.
        let mut whole = Shown::new("a".into(), "row".into());
        whole.is_whole = true;
        *whole.read.borrow_mut() = shown.read.borrow().clone();
        let mut scene = Scene::new(60);
        draw(&mut scene, &whole, Path::new("/w"), (0, 0, true), true);
        scene.finish();
        assert_eq!(text(&scene).len(), 1 + LINES + 5);
        assert!(!text(&scene).last().unwrap().contains("more"));
    }
}
