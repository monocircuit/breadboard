//! How far a worktree is from develop, and where its work collides (#114,
//! diff.mjs): what DIFF of the view of one worktree shows. The work of a
//! worktree is everything between the merge base with origin/develop and its
//! working tree; it collides with develop where develop changed the same file
//! since, and with another worktree where that one's work touches it too.

use crate::git::{git, path_of_status, work_files};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The area a file belongs to: inside a package the folder below `src`
/// (`vita/server`), a file at a package's top the package (`vita`,
/// `vita-core`); outside the packages the top-level folder or file.
pub fn area_of(file: &str) -> String {
    let parts: Vec<&str> = file.split('/').collect();
    if !["products", "packages", "tools"].contains(&parts[0]) || parts.len() < 3 {
        return if parts.len() > 1 {
            parts[0].to_string()
        } else {
            file.to_string()
        };
    }
    // A product holds packages: the web app keeps the product's name, the others are `vita-core`, ….
    let (name, inside) = if parts[0] == "products" && parts.len() > 3 {
        (
            if parts[2] == "web" {
                parts[1].to_string()
            } else {
                format!("{}-{}", parts[1], parts[2])
            },
            &parts[3..],
        )
    } else {
        (parts[1].to_string(), &parts[2..])
    };
    let below = if inside.first() == Some(&"src") {
        &inside[1..]
    } else {
        inside
    };
    if below.len() > 1 {
        format!("{name}/{}", below[0])
    } else {
        name
    }
}

/// A file of the work: how it changed, its lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Change {
    pub file: String,
    /// git's letter: `A` added, `M` modified, `D` deleted, `R` renamed, `T` type, `?` untracked.
    pub status: char,
    pub added: u64,
    pub removed: u64,
    /// Not committed yet: in `git status`.
    pub is_uncommitted: bool,
}

/// An area of the work: its lines, its files, the one most changed (a click opens it).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Area {
    pub name: String,
    pub added: u64,
    pub removed: u64,
    pub files: usize,
    pub top: String,
    top_changed: u64,
    /// Its files, the most lines changed first.
    pub changes: Vec<Change>,
}

/// Another worktree whose work touches files of this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlap {
    pub name: String,
    pub files: Vec<String>,
}

/// DIFF of a worktree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diff {
    pub ahead: u64,
    pub behind: u64,
    pub uncommitted: usize,
    pub added: u64,
    pub removed: u64,
    pub files: usize,
    /// By lines changed, most first.
    pub areas: Vec<Area>,
    /// Its files develop changed too since the merge base.
    pub with_develop: Vec<String>,
    pub with_worktrees: Vec<Overlap>,
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').filter(|line| !line.is_empty())
}

fn add_to(areas: &mut Vec<Area>, change: Change) {
    let (file, added, removed) = (change.file.as_str(), change.added, change.removed);
    let name = area_of(file);
    let at = match areas.iter().position(|area| area.name == name) {
        Some(at) => at,
        None => {
            areas.push(Area {
                name,
                ..Default::default()
            });
            areas.len() - 1
        }
    };
    let area = &mut areas[at];
    area.added += added;
    area.removed += removed;
    area.files += 1;
    // The file of an area with the most lines changed.
    if area.top.is_empty() || added + removed > area.top_changed {
        area.top = file.to_string();
        area.top_changed = added + removed;
    }
    area.changes.push(change);
}

/// `git diff --name-status` of the work: each file's letter by its (new) name.
fn statuses(dir: &Path, base: &str) -> HashMap<String, char> {
    lines(&git(dir, &["diff", "--name-status", base]).stdout)
        .filter_map(|line| {
            let letter = line.chars().next()?;
            let (_, file) = line.rsplit_once('\t')?;
            Some((file.to_string(), letter))
        })
        .collect()
}

/// The diff of `dir` against develop, without the overlaps with other worktrees; None without a merge base.
pub fn own_diff_of(dir: &Path) -> Option<Diff> {
    let develop = crate::git::base(dir);
    let base = git(dir, &["merge-base", &develop, "HEAD"]);
    if !base.is_ok || base.out().is_empty() {
        return None;
    }
    let base = base.out().to_string();
    let status = git(dir, &["status", "--porcelain", "-uall"]).stdout;
    let count = |range: &str| crate::git::count(dir, range).unwrap_or(0);
    let mut areas = Vec::new();
    let (mut added, mut removed) = (0, 0);
    let mut files: Vec<String> = Vec::new();
    let letters = statuses(dir, &base);
    let uncommitted: BTreeSet<String> = lines(&status).map(path_of_status).collect();
    // Against the working tree: the uncommitted changes are part of the work.
    for line in lines(&git(dir, &["diff", "--numstat", &base]).stdout) {
        let mut parts = line.splitn(3, '\t');
        let (plus, minus, file) = (
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        // A binary file has "-": it counts as a file, not as lines.
        let (file_added, file_removed) = (plus.parse().unwrap_or(0), minus.parse().unwrap_or(0));
        let name = renamed(file);
        added += file_added;
        removed += file_removed;
        add_to(
            &mut areas,
            Change {
                status: letters.get(&name).copied().unwrap_or('M'),
                is_uncommitted: uncommitted.contains(&name),
                file: name.clone(),
                added: file_added,
                removed: file_removed,
            },
        );
        files.push(name);
    }
    // New files git doesn't track yet: counted as files, their lines as added.
    for file in lines(&status).filter(|line| line.starts_with("??")).map(path_of_status) {
        if files.contains(&file) {
            continue;
        }
        let numstat = git(dir, &["diff", "--no-index", "--numstat", "/dev/null", &file]).stdout;
        let file_added = numstat
            .split('\t')
            .next()
            .and_then(|count| count.trim().parse().ok())
            .unwrap_or(0);
        added += file_added;
        add_to(
            &mut areas,
            Change {
                file: file.clone(),
                status: '?',
                added: file_added,
                removed: 0,
                is_uncommitted: true,
            },
        );
        files.push(file);
    }
    let on_develop: BTreeSet<String> = lines(&git(dir, &["diff", "--name-only", &base, &develop]).stdout)
        .map(String::from)
        .collect();
    areas.sort_by_key(|area| std::cmp::Reverse(area.added + area.removed));
    for area in &mut areas {
        area.changes
            .sort_by_key(|change| std::cmp::Reverse(change.added + change.removed));
    }
    let mut with_develop: Vec<String> = files
        .iter()
        .filter(|file| on_develop.contains(*file))
        .cloned()
        .collect();
    with_develop.sort();
    Some(Diff {
        ahead: count(&format!("{develop}..HEAD")),
        behind: count(&format!("HEAD..{develop}")),
        uncommitted: lines(&status).count(),
        added,
        removed,
        files: files.len(),
        areas,
        with_develop,
        with_worktrees: Vec::new(),
    })
}

/// A rename's new name: `a/{b => c}/d` → `a/c/d`, `a => b` → `b`.
fn renamed(file: &str) -> String {
    if !file.contains(" => ") {
        return file.to_string();
    }
    if let (Some(open), Some(close)) = (file.find('{'), file.find('}'))
        && open < close
    {
        let inner = &file[open + 1..close];
        let new = inner.split(" => ").last().unwrap_or(inner);
        return format!("{}{new}{}", &file[..open], &file[close + 1..]).replace("//", "/");
    }
    file.rsplit(" => ").next().unwrap_or(file).to_string()
}

/// The diff of `dir` with the overlaps: `others` the other worktrees `(name, files)`.
pub fn diff_of(dir: &Path, others: &[(String, BTreeSet<String>)]) -> Option<Diff> {
    let mut diff = own_diff_of(dir)?;
    let mine = work_files(dir);
    diff.with_worktrees = others
        .iter()
        .map(|(name, files)| Overlap {
            name: name.clone(),
            files: files.intersection(&mine).cloned().collect(),
        })
        .filter(|other| !other.files.is_empty())
        .collect();
    Some(diff)
}

/// A bar of `width` cells for `added` and `removed`, scaled so that `max`
/// fills it: a cell at least for any nonzero part. `(added, removed)` cells.
pub fn bar_of(added: u64, removed: u64, max: u64, width: u64) -> (usize, usize) {
    if max == 0 {
        return (0, 0);
    }
    let cells = |total: u64| {
        if total > 0 {
            ((total as f64 / max as f64) * width as f64).round().max(1.0) as usize
        } else {
            0
        }
    };
    (cells(added), cells(removed))
}

/// Diffs computed once per state of what they depend on: HEAD,
/// origin/develop, the status, and the size and change time of each file in
/// it, so a second edit of a file already ` M` is read too (the status alone
/// stays the same).
#[derive(Debug, Default)]
pub struct DiffCache {
    entries: HashMap<PathBuf, (String, Option<Diff>)>,
}

/// A file's size and when it last changed, as part of a cache key; empty when it is gone.
pub fn stamp_of(path: &Path) -> String {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return String::new();
    };
    let changed = meta
        .modified()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |at| at.as_nanos());
    format!("{} {changed}", meta.len())
}

impl DiffCache {
    pub fn own_diff_of(&mut self, dir: &Path) -> Option<Diff> {
        let status = git(dir, &["status", "--porcelain", "-uall"]).stdout;
        let mut key = git(dir, &["rev-parse", "HEAD", &crate::git::base(dir)]).stdout;
        for line in lines(&status) {
            let file = path_of_status(line);
            key.push_str(&format!("{line}\t{}\n", stamp_of(&dir.join(&file))));
        }
        if let Some((known, diff)) = self.entries.get(dir)
            && *known == key
        {
            return diff.clone();
        }
        let diff = own_diff_of(dir);
        self.entries.insert(dir.to_path_buf(), (key, diff.clone()));
        diff
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, worktree};
    use crate::lock::scratch::Scratch;

    #[test]
    fn names_the_area_of_a_file() {
        assert_eq!(area_of("products/vita/web/src/server/db/schema.ts"), "vita/server");
        assert_eq!(area_of("packages/breadboard/src/lib/tty.mjs"), "breadboard/lib");
        assert_eq!(area_of("products/vita/web/package.json"), "vita");
        assert_eq!(area_of("products/vita/core/src/routes.ts"), "vita-core");
        assert_eq!(area_of("packages/breadboard/src/overview.mjs"), "breadboard");
        assert_eq!(area_of("scripts/check-agents.mjs"), "scripts");
        assert_eq!(area_of("AGENTS.md"), "AGENTS.md");
    }

    #[test]
    fn reads_the_work_of_a_worktree_and_where_it_collides() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let one = worktree(&main, "1-one", false);
        let two = worktree(&main, "2-two", false);
        std::fs::write(one.join("new.md"), "a\nb\n").unwrap();
        let others = vec![("2-two".to_string(), work_files(&two))];
        let diff = diff_of(&one, &others).unwrap();
        assert_eq!(
            (diff.ahead, diff.behind, diff.uncommitted, diff.files, diff.added),
            (1, 0, 1, 2, 3)
        );
        assert_eq!(
            diff.areas.iter().map(|area| area.name.as_str()).collect::<Vec<_>>(),
            ["new.md", "work"]
        );
        assert!(diff.with_worktrees.is_empty());
        let changes: Vec<(&str, char, u64, bool)> = diff
            .areas
            .iter()
            .flat_map(|area| &area.changes)
            .map(|change| (change.file.as_str(), change.status, change.added, change.is_uncommitted))
            .collect();
        assert_eq!(changes, [("new.md", '?', 2, true), ("work/1-one.md", 'A', 1, false)]);
        assert_eq!(bar_of(3, 1, 4, 24), (18, 6));
        assert_eq!(bar_of(1, 0, 1000, 24), (1, 0));
        assert_eq!(renamed("a/{b => c}/d.md"), "a/c/d.md");
        assert_eq!(renamed("old.md => new.md"), "new.md");
        let mut cache = DiffCache::default();
        assert_eq!(cache.own_diff_of(&one).unwrap().files, 2);
        // A second edit of a file the status already lists is read too.
        std::fs::write(one.join("new.md"), "a\nb\nc\nd\n").unwrap();
        assert_eq!(cache.own_diff_of(&one).unwrap().added, 5);
    }
}
