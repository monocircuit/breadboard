//! `wt:orchestrate <name> --changes` (changeset.mjs, #256): what the
//! orchestration's MRs changed in the code, merged and open, each with the
//! commits to diff between, and all together, as one line of JSON for the
//! tab `changes` (nvim/orchestration.lua reads it: the format stays).

use crate::cli::setup::gitlab;
use crate::cli::setup::proc::git;
use crate::cli::setup::repo;
use breadboard_core::json::{int_of, str_of};
use breadboard_core::record;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

fn read(args: &[&str], cwd: &Path) -> Option<String> {
    let mut all = vec!["-c", "core.quotePath=false"];
    all.extend_from_slice(args);
    let done = git(&all, cwd);
    done.is_ok.then(|| done.text().to_string())
}

fn commit_of(reference: &str, cwd: &Path) -> Option<String> {
    read(
        &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")],
        cwd,
    )
    .filter(|sha| !sha.is_empty())
}

/// The files between two commits: `{ path, status, added, removed }`, by path.
pub fn files_between(base: &str, head: &str, cwd: &Path) -> Vec<Value> {
    let mut counts: BTreeMap<String, (Value, Value)> = BTreeMap::new();
    for line in read(&["diff", "--no-renames", "--numstat", base, head], cwd)
        .unwrap_or_default()
        .split('\n')
    {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let count = |text: &str| text.parse::<i64>().map(Value::from).unwrap_or(Value::Null);
        let pair = if parts[0] == "-" {
            (Value::Null, Value::Null)
        } else {
            (count(parts[0]), count(parts[1]))
        };
        counts.insert(parts[2].to_string(), pair);
    }
    let mut files: Vec<Value> = read(&["diff", "--no-renames", "--name-status", base, head], cwd)
        .unwrap_or_default()
        .split('\n')
        .filter_map(|line| line.split_once('\t'))
        .map(|(status, file)| {
            let status = match status.chars().next() {
                Some('A') => "new",
                Some('D') => "deleted",
                _ => "changed",
            };
            let (added, removed) = counts.get(file).cloned().unwrap_or((Value::Null, Value::Null));
            json!({ "path": file, "status": status, "added": added, "removed": removed })
        })
        .collect();
    files.sort_by(|a, b| str_of(a, "path").cmp(&str_of(b, "path")));
    files
}

/// The commits an MR's change lies between, None when git lacks them.
fn range_of(mr: &Value, cwd: &Path) -> Option<(String, String)> {
    if str_of(mr, "state") == Some("merged") {
        let sha = str_of(mr, "merge_commit_sha").or_else(|| str_of(mr, "squash_commit_sha"))?;
        let head = commit_of(sha, cwd)?;
        let base = commit_of(&format!("{head}^1"), cwd)?;
        return Some((base, head));
    }
    let head = str_of(mr, "sha").and_then(|sha| commit_of(sha, cwd)).or_else(|| {
        commit_of(
            &format!("origin/{}", str_of(mr, "source_branch").unwrap_or_default()),
            cwd,
        )
    })?;
    let develop = commit_of(
        &format!(
            "origin/{}",
            str_of(mr, "target_branch").unwrap_or(&breadboard_core::git::branch(cwd))
        ),
        cwd,
    )?;
    let base = read(&["merge-base", &develop, &head], cwd)?;
    Some((base.trim().to_string(), head))
}

/// All files of the groups together, by path (changeset.mjs `combine`).
pub fn combine(groups: &[Value]) -> Vec<Value> {
    struct Entry {
        file: Value,
        base: Value,
        head: Value,
        mrs: Vec<Value>,
        first: String,
        last: Option<String>,
    }
    let mut by_path: Vec<(String, Entry)> = Vec::new();
    for group in groups {
        for file in group
            .get("files")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let path = str_of(&file, "path").unwrap_or_default().to_string();
            let status = str_of(&file, "status").unwrap_or_default().to_string();
            match by_path.iter_mut().find(|(known, _)| *known == path) {
                None => by_path.push((
                    path,
                    Entry {
                        base: group["base"].clone(),
                        head: group["head"].clone(),
                        mrs: vec![group["iid"].clone()],
                        first: status,
                        last: None,
                        file,
                    },
                )),
                Some((_, known)) => {
                    known.head = group["head"].clone();
                    known.mrs.push(group["iid"].clone());
                    known.last = Some(status);
                    for key in ["added", "removed"] {
                        let sum = match (known.file[key].as_i64(), file[key].as_i64()) {
                            (Some(a), Some(b)) => json!(a + b),
                            _ => Value::Null,
                        };
                        known.file[key] = sum;
                    }
                }
            }
        }
    }
    let mut combined: Vec<Value> = by_path
        .into_iter()
        .filter_map(|(_, entry)| {
            let last = entry.last.clone().unwrap_or(entry.first.clone());
            let did_exist = entry.first != "new";
            let is_there = last != "deleted";
            if !did_exist && !is_there {
                return None;
            }
            let status = if !did_exist { "new" } else if !is_there { "deleted" } else { "changed" };
            Some(json!({
                "path": entry.file["path"], "status": status, "added": entry.file["added"], "removed": entry.file["removed"],
                "base": entry.base, "head": entry.head, "mrs": entry.mrs,
            }))
        })
        .collect();
    combined.sort_by(|a, b| str_of(a, "path").cmp(&str_of(b, "path")));
    combined
}

/// What `name` changed: `{ name, title, root, groups, combined, missing }`,
/// None without a record. Err: the branch GitLab didn't answer for after its
/// tries, and why: no set that leaves its MRs out as if there were none (#306).
pub fn read_change_set(name: &str) -> Option<Result<Value, (String, String)>> {
    let main = repo::main_dir();
    let found = record::record_of(&main, name)?;
    git(&["fetch", "--quiet", "--prune", "origin"], &main);
    let mut branches: Vec<String> = Vec::new();
    for issue in &found.issues {
        if let Some(branch) = &issue.branch
            && !branches.contains(branch)
        {
            branches.push(branch.clone());
        }
    }
    let read_mrs = gitlab::all_in_turns(&branches, 12, |branch| {
        gitlab::list(&format!(
            "projects/:fullpath/merge_requests?source_branch={}",
            gitlab::encode(branch)
        ))
        .map_err(|failure| (branch.clone(), failure.reason))
    });
    let read_mrs = match read_mrs {
        Ok(read) => read,
        Err(unanswered) => return Some(Err(unanswered)),
    };
    let mut groups: Vec<Value> = Vec::new();
    let mut missing: Vec<i64> = Vec::new();
    for (branch, mrs) in branches.iter().zip(read_mrs) {
        let issue = found
            .issues
            .iter()
            .find(|entry| entry.branch.as_deref() == Some(branch.as_str()));
        for mr in mrs
            .iter()
            .filter(|mr| matches!(str_of(mr, "state"), Some("merged" | "opened")))
        {
            let Some((base, head)) = range_of(mr, &main) else {
                missing.push(int_of(mr, "iid").unwrap_or_default());
                continue;
            };
            let title = str_of(mr, "title").unwrap_or_default();
            groups.push(json!({
                "iid": mr["iid"], "url": mr["web_url"],
                "title": title.strip_prefix("Draft:").map(str::trim_start).unwrap_or(title),
                "state": mr["state"], "issue": issue.and_then(|issue| issue.number),
                "mergedAt": mr.get("merged_at").cloned().unwrap_or(Value::Null),
                "files": files_between(&base, &head, &main), "base": base, "head": head,
            }));
        }
    }
    groups.sort_by(|a, b| {
        let merged = |group: &Value| str_of(group, "mergedAt").map(String::from);
        let (ma, mb) = (merged(a), merged(b));
        ma.is_none()
            .cmp(&mb.is_none())
            .then(ma.unwrap_or_default().cmp(&mb.unwrap_or_default()))
            .then(int_of(a, "iid").cmp(&int_of(b, "iid")))
    });
    // The keys in today's order: base and head before files.
    let groups: Vec<Value> = groups
        .into_iter()
        .map(|group| {
            json!({
                "iid": group["iid"], "url": group["url"], "title": group["title"], "state": group["state"],
                "issue": group["issue"], "mergedAt": group["mergedAt"], "base": group["base"], "head": group["head"],
                "files": group["files"],
            })
        })
        .collect();
    missing.sort();
    let combined = combine(&groups);
    Some(Ok(json!({
        "name": name, "title": record::title_of(&main, name), "root": main.to_string_lossy(),
        "groups": groups, "combined": combined, "missing": missing,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_what_several_mrs_did_to_a_file() {
        let groups = [
            json!({ "iid": 1, "base": "a", "head": "b", "files": [
                { "path": "x", "status": "new", "added": 3, "removed": 0 },
                { "path": "y", "status": "new", "added": 1, "removed": 0 },
                { "path": "z", "status": "changed", "added": 1, "removed": 1 },
            ] }),
            json!({ "iid": 2, "base": "b", "head": "c", "files": [
                { "path": "x", "status": "changed", "added": 1, "removed": 1 },
                { "path": "y", "status": "deleted", "added": 0, "removed": 1 },
                { "path": "z", "status": "changed", "added": null, "removed": null },
            ] }),
        ];
        let combined = combine(&groups);
        assert_eq!(combined.len(), 2);
        assert_eq!(
            combined[0],
            json!({ "path": "x", "status": "new", "added": 4, "removed": 1, "base": "a", "head": "c", "mrs": [1, 2] })
        );
        assert_eq!(combined[1]["added"], Value::Null);
        assert_eq!(combined[1]["status"], json!("changed"));
    }
}
