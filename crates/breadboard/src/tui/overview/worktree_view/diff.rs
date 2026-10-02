//! DIFF and the lines of file names (render-worktree.mjs `pushDiff`,
//! files.mjs `fileLine` / `fileAt`): how far the work is from develop, where
//! it lies, which files collide; every file a click away from its hunks
//! (hunks.rs), each line of them a click away from nvim.

use super::super::{Act, look};
use super::hunks::{self, Shown};
use crate::tui::Scene;
use crate::tui::scene::{Region, Target};
use crate::tui::theme::{self, Tone};
use breadboard_core::model::counted;
use breadboard_core::model::diff::{Area, Change, Diff, bar_of};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::path::Path;

/// How many areas DIFF names, the rest summed up.
const AREAS: usize = 6;
/// The bars of DIFF, at their widest.
const BAR: u64 = 24;
/// How many files of an area DIFF lists under it.
const FILES: usize = 5;
/// Where a file's path starts in DIFF: after its letter.
const FILE_AT: u16 = 5;
/// How many names a collision line shows (`someFiles`).
pub const NAMES: usize = 3;
/// How many conflicting files DEVELOP names (actions.mjs `count: 4`).
pub const CONFLICTS: usize = 4;
/// Where a wrapped line of names goes on (#304).
pub const WRAP: u16 = 4;
/// The narrowest the last cell of a DIFF row gets (lib/tty.mjs `MIN_WRAP`).
const MIN_WRAP: u16 = 16;
/// What the last cell of a DIFF row gets at most before the bars narrow for it.
const LAST: u16 = 24;
/// The narrowest bar of DIFF.
const MIN_BAR: u64 = 6;
/// The mark of a file with changes not yet committed.
const UNCOMMITTED: &str = "uncommitted";

/// A file's hunks under the row `at`, or closed again.
pub fn show(file: &str, at: &str) -> Act {
    Act::Hunks {
        file: file.to_string(),
        at: at.to_string(),
    }
}

/// Where the files of DIFF and DEVELOP are: the worktree, its diff (the
/// counts of each file), the file whose hunks show.
#[derive(Clone, Copy)]
pub struct Files<'a> {
    pub dir: &'a Path,
    pub diff: Option<&'a Diff>,
    pub shown: Option<&'a Shown>,
}

impl Files<'_> {
    /// The hunks of `file` after the row `at`, when they are the ones shown.
    fn under(&self, scene: &mut Scene<Act>, at: &str, file: &str) {
        let Some(shown) = self.shown.filter(|shown| shown.is_under(at, file)) else {
            return;
        };
        let change = self.diff.and_then(|diff| {
            diff.areas
                .iter()
                .flat_map(|area| &area.changes)
                .find(|change| change.file == file)
        });
        let key = change.map_or((0, 0, false), |change| {
            (change.added, change.removed, change.is_uncommitted)
        });
        let is_untracked = change.is_some_and(|change| change.status == '?');
        hunks::draw(scene, shown, self.dir, key, is_untracked);
    }
}

/// `prefix`, then file names as Node's `fileLine`: up to `count` by name,
/// the rest as `+N more`, which stands for the first one it hides. The names
/// wrap between names, never inside one, the next line at `indent` (#304).
/// A name lights and opens its file, and so does the gap after it (`fileAt`:
/// the nearest name to the left); anywhere else on the line opens the first.
/// `files` are (shown, path) pairs; the line is `at` for the hunks a click
/// shows under it.
#[allow(clippy::too_many_arguments)]
pub fn file_line(
    scene: &mut Scene<Act>,
    prefix: impl FnOnce(&mut Scene<Act>),
    files: &[(String, String)],
    count: usize,
    suffix: Option<&str>,
    at: &str,
    place: &Files,
    indent: u16,
) {
    let Some((_, first)) = files.first() else {
        return;
    };
    scene.block(show(first, at), |scene| {
        prefix(scene);
        scene.indent(indent);
        let shown = files.len().min(count);
        // Each name where it was drawn and what it opens.
        let mut placed: Vec<(Vec<Rect>, String, String)> = vec![];
        for (i, (name, file)) in files.iter().take(shown).enumerate() {
            let mut spans = vec![look::span(name, Style::new().add_modifier(Modifier::UNDERLINED))];
            if i + 1 < shown {
                spans.push(Span::raw(","));
            }
            placed.push((scene.atom(spans), name.clone(), file.clone()));
        }
        if let Some((_, hidden)) = files.get(shown) {
            let more = format!("+{} more", files.len() - shown);
            placed.push((scene.atom(vec![look::dim(&more)]), more, hidden.clone()));
        }
        if let Some(suffix) = suffix {
            scene.atom_gap(vec![look::dim(suffix)], 2);
        }
        let width = scene.width();
        for (i, (rects, label, file)) in placed.iter().enumerate() {
            let next = placed.get(i + 1).and_then(|(rects, ..)| rects.first());
            for (j, rect) in rects.iter().enumerate() {
                let mut area = *rect;
                // The gap after a name is its own; the last one gets the slack of a button.
                let end = match next {
                    Some(next) if j + 1 == rects.len() && next.y == rect.y => next.x,
                    _ => (rect.x + rect.width + 1).min(width),
                };
                area.width = end.saturating_sub(rect.x).max(rect.width);
                let target = Target::Button {
                    label: label.clone(),
                    tone: Tone::Cyan,
                    action: show(file, at),
                };
                scene.regions.push(Region { area, target });
            }
        }
        scene.indent(0);
    });
    if let Some(shown) = place
        .shown
        .filter(|shown| files.iter().any(|(_, file)| shown.is_under(at, file)))
    {
        place.under(scene, at, &shown.file);
    }
}

/// `(shown, path)` pairs of paths shown whole.
pub fn whole(files: &[String]) -> Vec<(String, String)> {
    files.iter().map(|file| (file.clone(), file.clone())).collect()
}

/// `✗ a merge conflicts in a, b, c, d +N more`: red, a merge that will fail.
pub fn conflicts(scene: &mut Scene<Act>, files: &[String], place: &Files) {
    let prefix = |scene: &mut Scene<Act>| {
        scene.atom(vec![look::span("✗", theme::tone(Tone::Red))]);
        look::say(scene, "a merge conflicts in", Style::new());
    };
    file_line(scene, prefix, &whole(files), CONFLICTS, None, "conflicts", place, WRAP);
}

/// `⚠ <what> a, b, c +N more  <suffix>`: yellow, an overlap to keep in mind.
fn collision(
    scene: &mut Scene<Act>,
    what: Vec<Span<'static>>,
    files: &[String],
    suffix: Option<&str>,
    at: &str,
    place: &Files,
) {
    let prefix = |scene: &mut Scene<Act>| {
        scene.atom(vec![look::span("⚠", theme::tone(Tone::Yellow))]);
        look::flow(scene, &what);
    };
    file_line(scene, prefix, &whole(files), NAMES, suffix, at, place, WRAP);
}

/// DIFF's title after its name: where it stands against develop.
pub fn note(diff: &Diff) -> String {
    let mut counts = vec![format!("{} ahead", diff.ahead), format!("{} behind", diff.behind)];
    if diff.uncommitted > 0 {
        counts.push(format!("{} uncommitted", diff.uncommitted));
    }
    // Parts the title drops from the end in a narrow pane: the hint first.
    format!(
        "against develop  {} · click a file: its changes, a line of them: nvim",
        counts.join(" · ")
    )
}

pub fn diff_of(scene: &mut Scene<Act>, diff: &Diff, dir: &Path, shown: Option<&Shown>) {
    let place = Files {
        dir,
        diff: Some(diff),
        shown,
    };
    if diff.files == 0 {
        scene.text("nothing changed yet", theme::dim());
    } else {
        table(scene, diff, &place);
    }
    if !diff.with_develop.is_empty() || !diff.with_worktrees.is_empty() {
        scene.blank();
    }
    if !diff.with_develop.is_empty() {
        collision(
            scene,
            vec![look::span("also changed on develop:", Style::new())],
            &diff.with_develop,
            Some("merge develop in while it is small"),
            "develop",
            &place,
        );
    }
    for other in &diff.with_worktrees {
        let what = vec![
            look::span("also changed in ", Style::new()),
            look::bold(format!("{}:", other.name)),
        ];
        collision(scene, what, &other.files, None, &format!("tree:{}", other.name), &place);
    }
}

/// The total, then the biggest areas as a table, an area's files under it
/// when it has more than one (lazygit's files panel), the rest summed up.
/// The name column fits the area names and the files' paths while the bars
/// keep `MIN_BAR`; a path longer than it puts its counts on the next line,
/// in their columns. An area of one file names it after its bar when the
/// name fits there whole, else under it as a file row: nothing breaks a name.
fn table(scene: &mut Scene<Act>, diff: &Diff, place: &Files) {
    scene.atom(vec![added_span(diff.added), Span::raw(" "), removed_span(diff.removed)]);
    scene.text(&format!("in {}", counted(diff.files as u64, "file")), Style::new());
    scene.atom_gap(bar(diff.added, diff.removed, diff.added + diff.removed, BAR), 2);
    scene.blank();
    let shown = &diff.areas[..diff.areas.len().min(AREAS)];
    let rest = &diff.areas[shown.len()..];
    let more = (!rest.is_empty()).then(|| format!("… {}", counted(rest.len() as u64, "area")));
    let (rest_added, rest_removed) = (
        rest.iter().map(|area| area.added).sum::<u64>(),
        rest.iter().map(|area| area.removed).sum::<u64>(),
    );
    let numbers = shown
        .iter()
        .flat_map(|area| {
            std::iter::once((area.added, area.removed))
                .chain(area.changes.iter().map(|change| (change.added, change.removed)))
        })
        .chain(more.as_ref().map(|_| (rest_added, rest_removed)));
    let (plus, minus) = numbers.fold((0, 0), |(plus, minus), (added, removed)| {
        (
            width(&format!("+{added}")).max(plus),
            width(&format!("−{removed}")).max(minus),
        )
    });
    let named: Vec<String> = shown.iter().map(named_last).collect();
    // What follows the bar gets what its widest wants, at least MIN_WRAP, at most LAST.
    let want = named
        .iter()
        .map(|last| width(last))
        .max()
        .unwrap_or(0)
        .clamp(MIN_WRAP, LAST);
    let counts_width = 2 + plus + 2 + minus + 2;
    let line = scene.width();
    // Names and paths set the column while a narrow bar and the last cell fit beside them.
    let name_cap = line.saturating_sub(counts_width + MIN_BAR as u16 + 2 + MIN_WRAP);
    let path_cap = line.saturating_sub(counts_width + MIN_BAR as u16 + 2 + want);
    let names = shown
        .iter()
        .map(|area| width(&area.name))
        .chain(more.as_deref().map(width))
        .map(|name| name.min(name_cap));
    let paths = shown
        .iter()
        .flat_map(|area| files_of(area, false))
        .map(|(path, _)| FILE_AT + width(&path))
        .filter(|path| *path <= path_cap);
    let name = names.chain(paths).max().unwrap_or(0);
    let biggest = shown.iter().map(|area| area.added + area.removed).max().unwrap_or(0);
    let bar_at = name + counts_width;
    // The bars narrower in a narrow pane, so the last cell keeps its room.
    let cells = (line.saturating_sub(bar_at + 2 + want) as u64)
        .clamp(MIN_BAR, BAR)
        .min(line.saturating_sub(bar_at + 2 + MIN_WRAP).max(1) as u64);
    let columns = Columns {
        plus: name + 2,
        minus: name + 2 + plus + 2,
        bar: bar_at,
        last: bar_at + cells as u16 + 2,
    };
    // What goes on a row's line after its counts: the bar and a last cell.
    let after = cells as u16 + 2 + MIN_WRAP;
    for (area, named) in shown.iter().zip(&named) {
        // One file's name after the bar when it fits there whole, else as a row under it.
        let is_named = area.changes.len() < 2 && columns.last + width(named) <= line;
        let files = files_of(area, !is_named);
        let last = if is_named {
            named.clone()
        } else {
            counted(area.files as u64, "file")
        };
        scene.block(show(&area.top, "area"), |scene| {
            scene.atom_gap(vec![look::bold(&area.name)], 0);
            counts(scene, &columns, area.added, area.removed, after);
            scene.atom_gap(bar(area.added, area.removed, biggest, cells), 0);
            columns.to(scene, columns.last, width(&last).min(MIN_WRAP));
            let parts: Vec<&str> = last.split(" · ").collect();
            scene.parts(&parts, theme::dim());
            scene.indent(0);
        });
        if files.is_empty() {
            place.under(scene, "area", &area.top);
        }
        for (path, change) in &files {
            file_row(scene, path, change, &columns, area.changes.len() > 1);
            place.under(scene, "file", &change.file);
            // The area's row shows its top file's changes under that file's row.
            if change.file == area.top {
                place.under(scene, "area", &area.top);
            }
        }
        if area.changes.len() > FILES {
            let hidden: Vec<(String, String)> = area.changes[FILES..]
                .iter()
                .map(|change| (basename(&change.file).to_string(), change.file.clone()))
                .collect();
            let prefix = |scene: &mut Scene<Act>| {
                scene.pad_to(FILE_AT);
                scene.atom_gap(vec![look::dim(format!("… {} more:", hidden.len()))], 0);
            };
            let at = format!("more:{}", area.name);
            file_line(scene, prefix, &hidden, NAMES, None, &at, place, FILE_AT);
        }
    }
    if let Some(more) = more {
        // The hidden areas' files by name, the biggest area's first.
        let hidden: Vec<(String, String)> = rest
            .iter()
            .flat_map(|area| area.changes.iter())
            .map(|change| (basename(&change.file).to_string(), change.file.clone()))
            .collect();
        let files: usize = rest.iter().map(|area| area.files).sum();
        let prefix = |scene: &mut Scene<Act>| {
            scene.atom_gap(vec![look::dim(&more)], 0);
            counts(scene, &columns, rest_added, rest_removed, after);
            columns.to(scene, columns.last, MIN_WRAP);
            scene.atom_gap(vec![look::dim(format!("{}:", counted(files as u64, "file")))], 0);
        };
        if hidden.is_empty() {
            prefix(scene);
            scene.end_line();
        } else {
            // Under the files column while it leaves room for names, else as a collision line.
            let indent = if scene.width() >= columns.last + 30 {
                columns.last
            } else {
                WRAP
            };
            file_line(scene, prefix, &hidden, NAMES, None, "rest", place, indent);
        }
    }
}

/// What follows the bar of an area of one file: `1 file · name`; `N files` for more.
fn named_last(area: &Area) -> String {
    let count = counted(area.files as u64, "file");
    if area.changes.len() < 2 {
        format!("{count} · {}", basename(&area.top))
    } else {
        count
    }
}

/// The files listed under an area, by their path below the area's folder;
/// one file only `is_listed`, when its row has no room for its name.
fn files_of(area: &Area, is_listed: bool) -> Vec<(String, &Change)> {
    if area.changes.len() < 2 && !is_listed {
        return vec![];
    }
    let folder = common_folder(area.changes.iter().map(|change| change.file.as_str()));
    area.changes
        .iter()
        .take(FILES)
        .map(|change| (change.file[folder.len()..].to_string(), change))
        .collect()
}

fn basename(file: &str) -> &str {
    file.rsplit('/').next().unwrap_or(file)
}

/// `█` added green, `░` removed red, scaled so `max` fills `cells`.
fn bar(added: u64, removed: u64, max: u64, cells: u64) -> Vec<Span<'static>> {
    let (plus, minus) = bar_of(added, removed, max, cells);
    vec![
        look::span("█".repeat(plus), theme::tone(Tone::Green)),
        look::span("░".repeat(minus), theme::tone(Tone::Red)),
    ]
}

/// git's letter of a file in its colour, as lazygit's files panel: new
/// green, changed yellow, gone red, moved cyan.
fn status_mark(status: char) -> Span<'static> {
    let tone = match status {
        'A' | '?' => Tone::Green,
        'D' => Tone::Red,
        'R' | 'C' => Tone::Cyan,
        _ => Tone::Yellow,
    };
    look::span(status.to_string(), theme::tone(tone))
}

/// `+12` green, `+0` quiet.
fn added_span(added: u64) -> Span<'static> {
    let style = if added > 0 {
        theme::tone(Tone::Green)
    } else {
        theme::dim()
    };
    look::span(format!("+{added}"), style)
}

/// `−3` red (U+2212), `−0` quiet.
fn removed_span(removed: u64) -> Span<'static> {
    let style = if removed > 0 {
        theme::tone(Tone::Red)
    } else {
        theme::dim()
    };
    look::span(format!("−{removed}"), style)
}

/// Where DIFF's columns start: added, removed, the bar, what follows the bar.
struct Columns {
    plus: u16,
    minus: u16,
    bar: u16,
    last: u16,
}

impl Columns {
    /// At `column`; past it, two after what ran into it while `need` columns
    /// fit there, else on the next line at `column`. The cell wraps where it starts.
    fn to(&self, scene: &mut Scene<Act>, column: u16, need: u16) {
        let at = if scene.x() < column {
            column
        } else if scene.x() + 2 + need <= scene.width() || column + need > scene.width() {
            scene.x() + 2
        } else {
            scene.newline();
            column
        };
        scene.pad_to(at);
        scene.indent(at);
    }
}

/// `+a  −r` in their columns, the line ready for what follows them, of
/// which `after` columns stay on the counts' line. A name that ran into the
/// counts' column puts them on the next line, in it: the column stays straight.
fn counts(scene: &mut Scene<Act>, columns: &Columns, added: u64, removed: u64, after: u16) {
    let (plus, minus) = (added_span(added), removed_span(removed));
    let minus_width = minus.width() as u16;
    if scene.x() + 2 > columns.plus && columns.plus < scene.width() {
        scene.newline();
    }
    scene.pad_to(columns.plus);
    scene.indent(columns.plus);
    scene.atom_gap(vec![plus], 0);
    columns.to(scene, columns.minus, minus_width + after);
    scene.atom_gap(vec![minus], 0);
    columns.to(scene, columns.bar, after.saturating_sub(2));
}

/// A file of an area, as lazygit's files panel has it: its letter, its path
/// below the area's folder (wrapping at a `/` when it is wider than the
/// line), its lines (not for the one file of an area: its row has them),
/// whether it is committed; a click shows its hunks under it.
fn file_row(scene: &mut Scene<Act>, path: &str, change: &Change, columns: &Columns, has_counts: bool) {
    scene.block(show(&change.file, "file"), |scene| {
        scene.pad_to(2);
        scene.atom_gap(vec![status_mark(change.status)], 0);
        scene.pad_to(FILE_AT);
        scene.indent(FILE_AT);
        let (folder, name) = match path.rfind('/') {
            Some(at) => path.split_at(at + 1),
            None => ("", path),
        };
        for piece in folder.split_inclusive('/') {
            scene.atom_gap(vec![look::dim(piece)], 0);
        }
        scene.atom_gap(vec![look::span(name, Style::new())], 0);
        let after = if change.is_uncommitted {
            2 + width(UNCOMMITTED)
        } else {
            0
        };
        if has_counts {
            counts(scene, columns, change.added, change.removed, after);
        }
        if change.is_uncommitted {
            scene.atom_gap(vec![look::dim(UNCOMMITTED)], if has_counts { 0 } else { 2 });
        }
        scene.indent(0);
    });
}

/// The folder all `files` are in, with its `/`; empty when they share none.
fn common_folder<'a>(mut files: impl Iterator<Item = &'a str>) -> String {
    let Some(first) = files.next() else {
        return String::new();
    };
    let mut folder = &first[..first.rfind('/').map_or(0, |at| at + 1)];
    for file in files {
        while !file.starts_with(folder) {
            let trimmed = folder.trim_end_matches('/');
            folder = &folder[..trimmed.rfind('/').map_or(0, |at| at + 1)];
        }
    }
    folder.to_string()
}

fn width(text: &str) -> u16 {
    Span::raw(text).width() as u16
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

    /// What a click at (x, y) runs: a button before its block.
    fn act_at(scene: &Scene<Act>, x: u16, y: u16) -> Option<&Act> {
        let inside = || scene.regions.iter().filter(move |region| region.contains(x, y));
        inside()
            .find(|region| region.is_button())
            .or_else(|| inside().find(|region| matches!(region.target, Target::Block(_))))
            .map(|region| region.action())
    }

    /// The screen column `needle` starts at in `line`.
    fn column_of(line: &str, needle: &str) -> u16 {
        Span::raw(&line[..line.find(needle).unwrap()]).width() as u16
    }

    fn opens(file: &str) -> Option<Act> {
        Some(show(file, "conflicts"))
    }

    fn shows(file: &str, at: &str) -> Option<Act> {
        Some(show(file, at))
    }

    fn place() -> Files<'static> {
        Files {
            dir: Path::new("/w"),
            diff: None,
            shown: None,
        }
    }

    fn change(file: &str, status: char, added: u64, is_uncommitted: bool) -> Change {
        Change {
            file: file.into(),
            status,
            added,
            removed: 0,
            is_uncommitted,
        }
    }

    fn area_of(name: &str, changes: Vec<Change>) -> Area {
        let mut area = Area::default();
        area.name = name.into();
        area.added = changes.iter().map(|change| change.added).sum();
        area.files = changes.len();
        area.top = changes[0].file.clone();
        area.changes = changes;
        area
    }

    #[test]
    fn a_conflict_is_red_names_four_and_every_name_opens_its_file() {
        let files: Vec<String> = ["a.ts", "b.ts", "c.ts", "d.ts", "e.ts", "f.ts"]
            .iter()
            .map(|file| format!("src/{file}"))
            .collect();
        let mut scene = Scene::new(80);
        conflicts(&mut scene, &files, &place());
        scene.finish();
        let line = "✗ a merge conflicts in src/a.ts, src/b.ts, src/c.ts, src/d.ts +2 more";
        assert_eq!(text(&scene), [line]);
        assert_eq!(scene.lines[0].spans[0].style, theme::tone(Tone::Red));
        let column = |needle: &str| column_of(line, needle);
        assert_eq!(act_at(&scene, column("src/b.ts"), 0), opens("src/b.ts").as_ref());
        // The gap after a name is that name's.
        assert_eq!(act_at(&scene, column(", src/c.ts") + 1, 0), opens("src/b.ts").as_ref());
        // `+2 more` stands for the first hidden one, with a column of slack.
        assert_eq!(act_at(&scene, column("+2 more"), 0), opens("src/e.ts").as_ref());
        assert_eq!(act_at(&scene, column("+2 more") + 7, 0), opens("src/e.ts").as_ref());
        // Off the names: the first.
        assert_eq!(act_at(&scene, 0, 0), opens("src/a.ts").as_ref());
        assert_eq!(act_at(&scene, 78, 0), opens("src/a.ts").as_ref());
    }

    #[test]
    fn names_wrap_between_names_and_keep_their_clicks() {
        let files = whole(&[
            "packages/ui/src/button/Button.tsx".into(),
            "work/304-overview.md".into(),
        ]);
        let mut scene = Scene::new(48);
        file_line(
            &mut scene,
            |scene| scene.text("⚠ also changed on develop:", Style::new()),
            &files,
            NAMES,
            Some("merge develop in while it is small"),
            "conflicts",
            &place(),
            WRAP,
        );
        scene.finish();
        assert_eq!(
            text(&scene),
            [
                "⚠ also changed on develop:",
                "    packages/ui/src/button/Button.tsx,",
                "    work/304-overview.md",
                "    merge develop in while it is small",
            ]
        );
        assert_eq!(act_at(&scene, 6, 2), opens("work/304-overview.md").as_ref());
        assert_eq!(
            act_at(&scene, 6, 1),
            opens("packages/ui/src/button/Button.tsx").as_ref()
        );
    }

    #[test]
    fn an_area_of_many_files_lists_them_below_its_folder_one_of_one_file_names_it() {
        let area = area_of(
            "breadboard/overview",
            vec![
                change("packages/breadboard/src/overview/files.mjs", 'M', 3, false),
                change("packages/breadboard/src/overview/lib/tty.mjs", '?', 1, true),
            ],
        );
        let readme = area_of("README.md", vec![change("README.md", 'M', 1, true)]);
        let diff = Diff {
            ahead: 1,
            behind: 2,
            uncommitted: 2,
            added: 5,
            files: 3,
            areas: vec![area, readme],
            ..Diff::default()
        };
        let mut scene = Scene::new(100);
        diff_of(&mut scene, &diff, Path::new("/w"), None);
        scene.finish();
        assert_eq!(
            text(&scene)[2..],
            [
                "breadboard/overview  +4  −0  ████████████████████████  2 files",
                "  M  files.mjs       +3  −0",
                "  ?  lib/tty.mjs     +1  −0  uncommitted",
                "README.md            +1  −0  ██████                    1 file · README.md",
            ]
        );
        assert_eq!(
            act_at(&scene, 3, 4),
            shows("packages/breadboard/src/overview/lib/tty.mjs", "file").as_ref()
        );
        assert_eq!(act_at(&scene, 3, 5), shows("README.md", "area").as_ref());
    }

    #[test]
    fn the_hidden_areas_are_summed_in_the_columns_and_their_files_open() {
        let areas: Vec<Area> = (1..=8)
            .rev()
            .map(|i| {
                area_of(
                    &format!("area{i}"),
                    vec![change(&format!("area{i}/f{i}.ts"), '?', i * 10, true)],
                )
            })
            .collect();
        let diff = Diff {
            added: 360,
            files: 8,
            areas,
            ..Diff::default()
        };
        let mut scene = Scene::new(100);
        diff_of(&mut scene, &diff, Path::new("/w"), None);
        scene.finish();
        let lines = text(&scene);
        let last = lines.last().unwrap();
        assert_eq!(
            last,
            "… 2 areas  +30  −0                            2 files: f2.ts, f1.ts"
        );
        let row = &lines[lines.len() - 2];
        assert_eq!(column_of(row, "+30"), column_of(last, "+30"), "{row}");
        assert_eq!(column_of(row, "1 file"), column_of(last, "2 files"), "{row}");
        let y = lines.len() as u16 - 1;
        assert_eq!(
            act_at(&scene, column_of(last, "f1.ts"), y),
            shows("area1/f1.ts", "rest").as_ref()
        );
        assert_eq!(act_at(&scene, 2, y), shows("area2/f2.ts", "rest").as_ref());
    }

    /// The rich world's table (the critic's, round 3): one long path, areas of one file.
    fn rich() -> Diff {
        let long = "packages/breadboard/src/overview/render-worktree-with-a-really-long-file-name-for-wrapping.mjs";
        let areas = vec![
            area_of(
                "monolithium/components",
                vec![change("packages/monolithium/src/components/Button.tsx", '?', 40, true)],
            ),
            area_of(
                "breadboard/overview",
                vec![
                    change("packages/breadboard/src/overview/gx-render.mjs", 'A', 15, false),
                    change("packages/breadboard/src/overview/view.mjs", '?', 12, true),
                    change(long, '?', 1, true),
                ],
            ),
            area_of("vita", vec![change("products/vita/web/.env.local", '?', 3, true)]),
        ];
        Diff {
            added: 71,
            files: 5,
            areas,
            ..Diff::default()
        }
    }

    #[test]
    fn a_long_path_runs_into_the_counts_and_never_breaks_a_last_cell() {
        let mut scene = Scene::new(120);
        diff_of(&mut scene, &rich(), Path::new("/w"), None);
        scene.finish();
        assert_eq!(
            text(&scene)[2..],
            [
                "monolithium/components                                              +40  −0  ██████████████████████  1 file · Button.tsx",
                "breadboard/overview                                                 +28  −0  ███████████████         3 files",
                "  A  gx-render.mjs                                                  +15  −0",
                "  ?  view.mjs                                                       +12  −0  uncommitted",
                "  ?  render-worktree-with-a-really-long-file-name-for-wrapping.mjs  +1   −0  uncommitted",
                "vita                                                                +3   −0  ██                      1 file · .env.local",
            ]
        );
        let mut scene = Scene::new(64);
        // Narrower: the bars give way to the last cell, the path breaks after its `.`.
        diff_of(&mut scene, &rich(), Path::new("/w"), None);
        scene.finish();
        let lines = text(&scene);
        assert_eq!(
            lines[2..],
            [
                "monolithium/components  +40  −0  ██████████  1 file · Button.tsx",
                "breadboard/overview     +28  −0  ███████     3 files",
                "  A  gx-render.mjs      +15  −0",
                "  ?  view.mjs           +12  −0  uncommitted",
                "  ?  render-worktree-with-a-really-long-file-name-for-wrapping.",
                "     mjs                +1   −0  uncommitted",
                "vita                    +3   −0  █           1 file · .env.local",
            ]
        );
        // Both lines of the long file's row open it.
        assert_eq!(
            act_at(&scene, 3, 6),
            shows(&rich().areas[1].changes[2].file, "file").as_ref()
        );
        assert_eq!(
            act_at(&scene, 30, 7),
            shows(&rich().areas[1].changes[2].file, "file").as_ref()
        );
    }

    #[test]
    fn the_folder_files_share() {
        assert_eq!(common_folder(["a/b/c.ts", "a/b/d/e.ts"].into_iter()), "a/b/");
        assert_eq!(common_folder(["a/b/c.ts", "a/x.ts"].into_iter()), "a/");
        assert_eq!(common_folder(["ab/c.ts", "a/x.ts"].into_iter()), "");
        assert_eq!(common_folder(["README.md"].into_iter()), "");
        assert_eq!(common_folder(std::iter::empty()), "");
    }
}
