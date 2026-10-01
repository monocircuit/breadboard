//! `wt:prune [--dry-run]` (worktree/commands/prune.mjs, prune.mjs): after a
//! merge nobody remembers `wt:remove`, so worktrees and branches pile up.
//! This closes what GitLab is done with: a worktree whose branch is gone from
//! origin and whose commits are all in develop, the local branches one left
//! behind, the leftover folders of removals that stopped halfway. Everything
//! else is only listed: a branch with own commits is the only copy of them.

use super::args::{Args, Takes};
use super::claude::{bound_session, left_in, left_text, running_claudes, session_file};
use super::plugin;
use super::proc::{Done, fail, git, step, text};
use super::remove::{self, Options};
use super::repo::Repo;
use breadboard_core::git::{is_inside, own_commits};
use std::fs;
use std::path::{Path, PathBuf};

const TAKES: Takes = Takes {
    switches: &["--dry-run"],
    valued: &[],
    lists: &[],
    words: 0,
    about: "Removes what GitLab is done with: worktrees whose branch is gone from origin with all of it in develop, local branches left behind, leftover folders. Everything else is listed with why it stays. --dry-run only lists.",
    usage: "usage: pnpm wt:prune [--dry-run]",
};

/// Commits of `rev` that origin/develop doesn't have, by patch (`cwd`: a checkout of its repo).
pub fn has_own_commits(rev: &str, cwd: &Path) -> bool {
    own_commits(cwd, rev) > 0
}

/// The files of a leftover folder that may be someone's work. Not counted:
/// the binding, symlinks, what .gitignore ignores (not descended into) and a
/// file whose content git has as a blob already (a tracked file a removal
/// left behind unchanged). Sorted.
pub fn own_files(main: &Path, dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk(main, dir, "", &mut files);
    if files.is_empty() {
        return files;
    }
    let input: String = files
        .iter()
        .map(|name| format!("{}\n", text(&dir.join(name))))
        .collect();
    let hashes: Vec<String> = git(main, &["hash-object", "--no-filters", "--stdin-paths"], Some(&input))
        .stdout
        .split('\n')
        .map(String::from)
        .collect();
    let check: String = hashes.iter().map(|hash| format!("{hash}\n")).collect();
    let answer = git(main, &["cat-file", "--batch-check"], Some(&check));
    let missing: Vec<&str> = answer
        .lines()
        .filter(|line| line.ends_with(" missing"))
        .filter_map(|line| line.split(' ').next())
        .collect();
    files
        .into_iter()
        .enumerate()
        .filter(|(at, _)| {
            hashes
                .get(*at)
                .is_none_or(|hash| hash.is_empty() || missing.contains(&hash.as_str()))
        })
        .map(|(_, name)| name)
        .collect()
}

/// One level of `own_files`: git asked once per directory which entries it ignores.
fn walk(main: &Path, dir: &Path, relative: &str, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir.join(relative)) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    // A trailing slash, so `dir/` patterns match a folder the main checkout doesn't have.
    let paths: Vec<(String, std::fs::FileType)> = entries
        .iter()
        .filter_map(|entry| {
            let kind = entry.file_type().ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            Some((
                format!("{relative}{name}{}", if kind.is_dir() { "/" } else { "" }),
                kind,
            ))
        })
        .collect();
    let input: String = paths.iter().map(|(name, _)| format!("{name}\n")).collect();
    let ignored = git(main, &["check-ignore", "--no-index", "--stdin"], Some(&input));
    let ignored: Vec<&str> = ignored.lines().collect();
    for (name, kind) in &paths {
        if ignored.contains(&name.as_str()) {
            continue;
        }
        if kind.is_dir() {
            walk(main, dir, name, files);
        } else if kind.is_file() && dir.join(name) != session_file(dir) {
            files.push(name.clone());
        }
    }
}

/// A worktree, branch or folder `wt:prune` may close, with the reason it
/// stays; `why: None` is the one GitLab is done with.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: PathBuf,
    pub why: Option<String>,
}

pub struct Prunable {
    pub trees: Vec<Entry>,
    pub branches: Vec<Entry>,
    pub folders: Vec<Entry>,
}

/// What `wt:prune` would close (prune.mjs `prunable`). `here`: where it runs.
pub fn prunable(repo: &Repo, here: &Path) -> Prunable {
    let claudes = running_claudes();
    let prefix = format!("{}-", repo.base());
    let worktrees = repo.worktrees();
    let trees = worktrees
        .iter()
        .skip(1)
        .map(|tree| {
            let dir = &tree.dir;
            let name = dir
                .file_name()
                .map(|name| name.to_string_lossy().replacen(&prefix, "", 1))
                .unwrap_or_default();
            let why = match &tree.branch {
                None => Some("detached HEAD".to_string()),
                Some(branch)
                    if git(
                        dir,
                        &["rev-parse", "--verify", "--quiet", &format!("origin/{branch}")],
                        None,
                    )
                    .is_ok =>
                {
                    Some("its branch is still on origin".into())
                }
                Some(branch) if has_own_commits(branch, dir) => Some("commits that are not in develop".into()),
                Some(_) if !git(dir, &["status", "--porcelain"], None).out().is_empty() => {
                    Some("uncommitted changes".into())
                }
                Some(_) if is_inside(here, dir) => Some("you are working in it".into()),
                Some(_) => {
                    let left = left_in(dir, &claudes, bound_session(dir).as_deref());
                    (!left.is_empty())
                        .then(|| format!("{} still stands in it, not in its next worktree yet", left_text(&left)))
                }
            };
            Entry {
                name,
                dir: dir.clone(),
                why,
            }
        })
        .collect();

    let refs = git(
        &repo.main,
        &[
            "for-each-ref",
            "--format=%(refname:short)|%(upstream:track)|%(worktreepath)",
            "refs/heads",
        ],
        None,
    );
    let branches = refs
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('|');
            let (name, track, worktree) = (parts.next()?, parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let is_gone =
                !name.is_empty() && worktree.is_empty() && track == "[gone]" && name != "develop" && name != "main";
            is_gone.then(|| Entry {
                name: name.to_string(),
                dir: repo.main.clone(),
                why: has_own_commits(name, &repo.main)
                    .then(|| "gone from origin but not in develop — the only copy of that work".to_string()),
            })
        })
        .collect();

    // A `platform-*` folder git no longer knows: a removal that stopped halfway.
    let known: Vec<&PathBuf> = worktrees.iter().map(|tree| &tree.dir).collect();
    let parent = repo.main.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut siblings: Vec<_> = fs::read_dir(&parent)
        .map(|entries| entries.flatten().collect())
        .unwrap_or_default();
    siblings.sort_by_key(|entry| entry.file_name());
    let folders = siblings
        .into_iter()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(&prefix)?.to_string();
            let dir = parent.join(&name);
            if known.contains(&&dir) {
                return None;
            }
            let files = own_files(&repo.main, &dir);
            let why = (!files.is_empty())
                .then(|| format!("not a worktree, but {} file(s) git doesn't know in it", files.len()));
            Some(Entry { name: rest, dir, why })
        })
        .collect();
    Prunable {
        trees,
        branches,
        folders,
    }
}

/// `<name> <line>`, the names padded to 32 as today's, or to the longest
/// when one is longer: the lines stay in one column.
fn lines(report: &[(String, String)]) -> Vec<String> {
    let width = report.iter().map(|(name, _)| name.chars().count()).fold(32, usize::max);
    report
        .iter()
        .map(|(name, what)| format!("{name:<width$} {what}"))
        .collect()
}

pub fn command(args: &Args) -> Done {
    if args.check(&TAKES)? {
        return Ok(());
    }
    let is_dry = args.has("--dry-run");
    let repo = Repo::here()?;
    let here = std::env::current_dir().unwrap_or_default();
    step("git fetch --prune");
    git(&repo.main, &["fetch", "--prune"], None);

    let Prunable {
        trees,
        branches,
        folders,
    } = prunable(&repo, &here);
    let mut report = Vec::new();
    let mut failed = 0;
    // The orchestrations whose changelog lists a removed merge: each rewritten once, at the end.
    let mut changelogs: Vec<String> = Vec::new();
    for Entry { name, why, .. } in &trees {
        let what = match why {
            Some(why) => format!("kept, {why}"),
            None if is_dry => "would be removed".into(),
            None => {
                step(&format!("pnpm wt:remove {name}"));
                // A remove that fails is said on its line and the prune goes on: the
                // others are independent (today's stopped the whole prune there).
                match remove::remove(
                    &repo,
                    name,
                    &Options {
                        is_fetched: true,
                        ..Options::default()
                    },
                ) {
                    Ok(changelog) => {
                        changelogs.extend(changelog.filter(|name| !changelogs.contains(name)));
                        "removed".into()
                    }
                    Err(refused) => {
                        failed += 1;
                        format!("kept, wt:remove failed: {}", refused.0)
                    }
                }
            }
        };
        report.push((name.clone(), what));
    }
    for Entry { name, why, .. } in &branches {
        let what = match why {
            Some(why) => format!("kept, {why}"),
            None if is_dry => "would be deleted".into(),
            None if git(&repo.main, &["branch", "-D", name], None).is_ok => "deleted, all of it is in develop".into(),
            None => "kept, git refused".into(),
        };
        report.push((name.clone(), what));
    }
    for Entry { name, dir, why } in &folders {
        let what = match why {
            Some(why) => format!("kept, {why}"),
            None if is_dry => "would be deleted, a leftover folder".into(),
            None => {
                let _ = fs::remove_dir_all(dir);
                "deleted, a leftover folder".into()
            }
        };
        report.push((name.clone(), what));
    }
    // After the removals (the plugin may have been linked from one of them,
    // #230); a dry run asks herdr nothing.
    if !is_dry {
        plugin::heal(&repo.main);
    }
    for name in &changelogs {
        remove::write_changelog(&repo, name);
    }

    println!(
        "\n✓ {}",
        if is_dry {
            "nothing was touched (--dry-run)"
        } else {
            "pruned"
        }
    );
    for entry in lines(&report) {
        println!("  {entry}");
    }
    if report.is_empty() {
        println!("  nothing to do");
    }
    println!();
    if failed > 0 {
        return fail(format!(
            "{failed} worktree(s) could not be removed: the reason is on each line above"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    #[test]
    fn a_leftover_folder_counts_only_files_git_lacks() {
        let root = std::env::temp_dir().join(format!("lifecycle-own-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let main = root.join("platform");
        fs::create_dir_all(&main).unwrap();
        sh(&main, &["init", "--quiet"]);
        fs::write(main.join(".gitignore"), "node_modules/\n").unwrap();
        fs::write(main.join("README.md"), "tracked\n").unwrap();
        sh(&main, &["add", "."]);
        sh(&main, &["commit", "--quiet", "-m", "x"]);
        let left = root.join("platform-9-x");
        fs::create_dir_all(left.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(left.join(".claude")).unwrap();
        fs::write(left.join("node_modules/pkg/index.js"), "ignored\n").unwrap();
        fs::write(left.join("README.md"), "tracked\n").unwrap();
        fs::write(left.join(".claude/herdr-session"), "binding\n").unwrap();
        assert!(own_files(&main, &left).is_empty());
        fs::write(left.join("notes.md"), "someone's work\n").unwrap();
        assert_eq!(own_files(&main, &left), vec!["notes.md"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pads_names_to_32_or_the_longest() {
        let short = |name: &str, what: &str| (name.to_string(), what.to_string());
        assert_eq!(
            lines(&[short("302-wt-mr-mr-s-tests", "kept, x")]),
            vec!["302-wt-mr-mr-s-tests             kept, x"]
        );
        let long = "307-breadboard-best-stack-breadboard-rust";
        assert_eq!(
            lines(&[short("1-x", "removed"), short(long, "kept")]),
            vec![format!("{:<41} removed", "1-x"), format!("{long} kept")]
        );
    }
}
