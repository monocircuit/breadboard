//! What wt:new offers to sweep where someone starts the next thing
//! (prune.mjs `prunable`, `staleIssues`), and the push guard it installs
//! (push-guard.mjs, #184).

use super::conversations::{self, session_file};
use super::gitlab;
use super::proc::{git, git_with};
use super::repo;
use breadboard_core::json::{int_of, str_of};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The files of a leftover folder that may be someone's work: not ignored,
/// no symlink, not the binding, and not a blob git already has.
pub fn own_files(dir: &Path) -> Vec<String> {
    let main = repo::main_dir();
    let mut files = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(relative) = stack.pop() {
        let Ok(entries) = fs::read_dir(dir.join(&relative)) else {
            continue;
        };
        let entries: Vec<_> = entries.flatten().collect();
        let paths: Vec<String> = entries
            .iter()
            .map(|entry| {
                let name = Path::new(&relative)
                    .join(entry.file_name())
                    .to_string_lossy()
                    .into_owned();
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();
        let input: String = paths.iter().map(|name| format!("{name}\n")).collect();
        let ignored = git_with(&["check-ignore", "--no-index", "--stdin"], &main, &input).out;
        let ignored: Vec<&str> = ignored.split('\n').collect();
        for (entry, name) in entries.iter().zip(&paths) {
            if ignored.contains(&name.as_str()) {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(name.trim_end_matches('/').to_string());
            } else if kind.is_file() && dir.join(name) != session_file(dir) {
                files.push(name.clone());
            }
        }
    }
    if files.is_empty() {
        return files;
    }
    let input: String = files
        .iter()
        .map(|name| format!("{}\n", dir.join(name).display()))
        .collect();
    let hashes: Vec<String> = git_with(&["hash-object", "--no-filters", "--stdin-paths"], &main, &input)
        .out
        .trim()
        .split('\n')
        .map(String::from)
        .collect();
    let input: String = hashes.iter().map(|hash| format!("{hash}\n")).collect();
    let checked = git_with(&["cat-file", "--batch-check"], &main, &input).out;
    let missing: Vec<&str> = checked
        .split('\n')
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

/// How many worktrees, branches and folders `wt:prune` would close now.
pub fn prunable_count() -> usize {
    let main = repo::main_dir();
    let here = repo::cwd();
    let claudes = conversations::running_claudes();
    let all = repo::worktrees();
    let trees = all.iter().skip(1).filter(|tree| {
        let Some(branch) = &tree.branch else { return false };
        let dir = &tree.dir;
        let bound = conversations::bound_session(dir);
        let is_left = !conversations::left_in(dir, &claudes, bound.as_deref()).is_empty();
        !git(&["rev-parse", "--verify", "--quiet", &format!("origin/{branch}")], dir).is_ok
            && !repo::has_own_commits(branch, dir)
            && git(&["status", "--porcelain"], dir).text().is_empty()
            && !repo::is_inside(&here, dir)
            && !is_left
    });
    let refs = git(
        &[
            "for-each-ref",
            "--format=%(refname:short)|%(upstream:track)|%(worktreepath)",
            "refs/heads",
        ],
        &main,
    );
    let branches = refs.text().split('\n').filter(|line| {
        let parts: Vec<&str> = line.split('|').collect();
        let (name, track, worktree) = (
            parts[0],
            parts.get(1).copied().unwrap_or(""),
            parts.get(2).copied().unwrap_or(""),
        );
        !name.is_empty()
            && worktree.is_empty()
            && track == "[gone]"
            && name != "develop"
            && name != "main"
            && *name != breadboard_core::git::branch(&main)
            && !repo::has_own_commits(name, &main)
    });
    let known: Vec<PathBuf> = all.iter().map(|tree| tree.dir.clone()).collect();
    let prefix = format!("{}-", repo::base_of(&main));
    let parent = main.parent().map(Path::to_path_buf).unwrap_or_default();
    let folders = fs::read_dir(&parent)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.file_type().is_ok_and(|kind| kind.is_dir())
                        && entry.file_name().to_string_lossy().starts_with(&prefix)
                })
                .map(|entry| entry.path())
                .filter(|dir| !known.contains(dir))
                .filter(|dir| own_files(dir).is_empty())
                .count()
        })
        .unwrap_or(0);
    trees.count() + branches.count() + folders
}

static REFERENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?|refs?)\s+#(\d+)").unwrap());

/// Whether an issue's description has a box left open (issues.mjs `boxesOf`, fenced code skipped).
pub fn has_open_box(text: &str) -> bool {
    static BOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+\[([ xX])\]\s+(.*)$").unwrap());
    let mut is_in_code = false;
    for line in text.split('\n') {
        if line.trim_start().starts_with("```") {
            is_in_code = !is_in_code;
            continue;
        }
        if !is_in_code && BOX.captures(line).is_some_and(|found| &found[1] == " ") {
            return true;
        }
    }
    false
}

/// How many issues of merged MRs are open or have a box left open, with no
/// comment since the last of those merges (prune.mjs `staleIssues`).
pub fn stale_issue_count() -> Result<usize, String> {
    let mrs = gitlab::list("projects/:fullpath/merge_requests?state=merged&order_by=merged_at&sort=desc&per_page=100")?;
    let mut last_merge: BTreeMap<String, String> = BTreeMap::new();
    for mr in &mrs {
        let merged_at = str_of(mr, "merged_at").unwrap_or_default().to_string();
        let mut numbers: Vec<String> = Vec::new();
        if let Some(number) = breadboard_core::git::issue_of_branch(str_of(mr, "source_branch").unwrap_or_default()) {
            numbers.push(number.to_string());
        }
        for found in REFERENCE.captures_iter(str_of(mr, "description").unwrap_or_default()) {
            numbers.push(found[1].to_string());
        }
        for number in numbers {
            let known = last_merge.get(&number);
            if known.is_none_or(|known| known < &merged_at) {
                last_merge.insert(number, merged_at.clone());
            }
        }
    }
    if last_merge.is_empty() {
        return Ok(0);
    }
    let numbers: Vec<&String> = last_merge.keys().collect();
    let mut issues: Vec<Value> = Vec::new();
    for chunk in numbers.chunks(50) {
        let iids: Vec<String> = chunk.iter().map(|n| format!("iids[]={n}")).collect();
        issues.extend(gitlab::list(&format!(
            "projects/:fullpath/issues?{}&per_page=50",
            iids.join("&")
        ))?);
    }
    // One nobody commented on (`user_notes_count`, the notes people wrote) is
    // stale without reading its notes: only the commented ones are asked.
    let open: Vec<(i64, String, bool)> = issues
        .iter()
        .filter(|issue| {
            str_of(issue, "state") == Some("opened") || has_open_box(str_of(issue, "description").unwrap_or_default())
        })
        .filter_map(|issue| {
            let iid = int_of(issue, "iid")?;
            let has_comments = issue.get("user_notes_count").and_then(Value::as_u64) != Some(0);
            Some((iid, last_merge.get(&iid.to_string())?.clone(), has_comments))
        })
        .collect();
    let asked: Vec<&(i64, String, bool)> = open.iter().filter(|(_, _, has_comments)| *has_comments).collect();
    let stale = gitlab::in_turns(&asked, 10, |(iid, merged_at, _)| {
        let notes = gitlab::list(&format!(
            "projects/:fullpath/issues/{iid}/notes?sort=desc&order_by=created_at&per_page=100"
        ))?;
        Ok::<bool, String>(!notes.iter().any(|note| {
            !note.get("system").and_then(Value::as_bool).unwrap_or(false)
                && str_of(note, "created_at").is_some_and(|at| at > merged_at.as_str())
        }))
    });
    let mut count = open.len() - asked.len();
    for verdict in stale {
        if verdict? {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_an_open_box_outside_code() {
        assert!(has_open_box("## Work\n- [ ] a\n"));
        assert!(!has_open_box("- [x] a\n```\n- [ ] example\n```\n"));
        assert!(REFERENCE.is_match("Closes #12"));
    }
}
