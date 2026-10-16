//! An agent's uncommitted work kept as a ref before breadboard ends or
//! restarts its claude (#395): jj's "the working copy is always a commit",
//! on git. A snapshot is a stash commit (`git stash create`: HEAD, the index,
//! the tracked changes) with the untracked files as its third parent, built
//! in a temporary index, under `refs/breadboard/snapshots/<feature>/<utc
//! time>`. Nothing in the worktree changes, its index neither. The refs are
//! shared by every worktree of the clone, so a snapshot outlives its
//! worktree; no refspec pushes `refs/breadboard/`. `git stash apply` brings
//! one back, untracked files included (`restore`).

use crate::git::{feature_of, git, main_dir};
use crate::time::parse_iso;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Where the snapshots live, a folder per feature.
pub const PREFIX: &str = "refs/breadboard/snapshots/";
/// How long a snapshot is kept.
pub const KEEP_MS: i64 = 14 * 24 * 3_600_000;
/// Who a snapshot's commits are by: never the user's identity, and no
/// `user.name` needed.
const BY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "breadboard"),
    ("GIT_AUTHOR_EMAIL", "breadboard@localhost"),
    ("GIT_COMMITTER_NAME", "breadboard"),
    ("GIT_COMMITTER_EMAIL", "breadboard@localhost"),
];

/// One snapshot: its feature, its time as the ref names it
/// (`20261016T142233Z`), when that is, its commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub feature: String,
    pub time: String,
    pub at: i64,
    pub sha: String,
}

impl Snapshot {
    pub fn reference(&self) -> String {
        format!("{PREFIX}{}/{}", self.feature, self.time)
    }
}

/// `ms` as a ref's time: `20261016T142233Z` (no `:`, which refs refuse).
pub fn time_of(ms: i64) -> String {
    let iso = crate::time::to_iso(ms);
    let (date, clock) = (&iso[..10], &iso[11..19]);
    format!("{}T{}Z", date.replace('-', ""), clock.replace(':', ""))
}

/// The ms of a ref's time, None for anything else.
pub fn at_of(time: &str) -> Option<i64> {
    let (date, clock) = time.strip_suffix('Z')?.split_once('T')?;
    let is_digits = |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    if !is_digits(date, 8) || !is_digits(clock, 6) {
        return None;
    }
    parse_iso(&format!(
        "{}-{}-{}T{}:{}:{}Z",
        &date[..4],
        &date[4..6],
        &date[6..],
        &clock[..2],
        &clock[2..4],
        &clock[4..]
    ))
}

/// `git <args>` in `dir` with `env`, `input` on stdin: its stdout trimmed, or why it failed.
fn run(dir: &Path, args: &[&str], env: &[(&str, &str)], input: Option<&[u8]>) -> Result<String, String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(dir)
        .envs(env.iter().copied())
        .envs(BY)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| format!("git {}: {error}", args[0]))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(input);
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("git {}: {error}", args[0]))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The untracked files of `dir` (ignored ones not), NUL-separated as
/// `update-index -z --stdin` takes them; a nested repo (`dir/`) left out.
fn untracked_of(dir: &Path) -> Result<Vec<u8>, String> {
    let listed = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("git ls-files: {error}"))?;
    if !listed.status.success() {
        return Err(format!(
            "git ls-files: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        ));
    }
    Ok(listed
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty() && !path.ends_with(b"/"))
        .flat_map(|path| path.iter().copied().chain([0]))
        .collect())
}

/// The commit of the untracked files of `dir` as `git stash -u` makes it
/// (a tree of them alone, no parent), None when there are none. Built in a
/// temporary index: the worktree's own stays as it is.
fn untracked_commit(dir: &Path, label: &str) -> Result<Option<String>, String> {
    let paths = untracked_of(dir)?;
    if paths.is_empty() {
        return Ok(None);
    }
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let index = std::env::temp_dir().join(format!(
        "breadboard-snapshot-{}-{}.index",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let index_env = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_env.as_str())];
    let built = run(dir, &["update-index", "--add", "-z", "--stdin"], &env, Some(&paths))
        .and_then(|_| run(dir, &["write-tree"], &env, None));
    let _ = std::fs::remove_file(&index);
    let tree = built?;
    run(
        dir,
        &["commit-tree", &tree, "-m", &format!("untracked files on {label}")],
        &[],
        None,
    )
    .map(Some)
}

/// The worktree `dir`'s uncommitted state, tracked and untracked, kept as
/// `refs/breadboard/snapshots/<feature>/<time of now_ms>`. None when there
/// is nothing uncommitted, or nothing new since its newest snapshot.
pub fn take(dir: &Path, feature: &str, now_ms: i64) -> Result<Option<Snapshot>, String> {
    let head = run(dir, &["rev-parse", "--verify", "HEAD"], &[], None)?;
    let label = format!("{feature}: breadboard snapshot");
    let untracked = untracked_commit(dir, &label)?;
    // HEAD, the index, the tracked changes: empty when none of them differ from HEAD.
    let stashed = run(dir, &["stash", "create", &label], &[], None)?;
    let (tree, index) = if stashed.is_empty() {
        let Some(_) = &untracked else { return Ok(None) };
        let tree = run(dir, &["rev-parse", "HEAD^{tree}"], &[], None)?;
        let index = run(
            dir,
            &["commit-tree", &tree, "-p", &head, "-m", &format!("index on {label}")],
            &[],
            None,
        )?;
        (tree, index)
    } else {
        (
            run(dir, &["rev-parse", &format!("{stashed}^{{tree}}")], &[], None)?,
            run(dir, &["rev-parse", &format!("{stashed}^2")], &[], None)?,
        )
    };
    let main = main_dir(dir).unwrap_or_else(|| dir.to_path_buf());
    if let Some(newest) = list(&main, Some(feature)).into_iter().next()
        && is_same(dir, &newest.sha, &tree, &index, untracked.as_deref())
    {
        return Ok(None);
    }
    let mut args = vec!["commit-tree", tree.as_str(), "-p", head.as_str(), "-p", index.as_str()];
    if let Some(untracked) = &untracked {
        args.extend(["-p", untracked.as_str()]);
    }
    let message = format!("WIP on {label}");
    args.extend(["-m", message.as_str()]);
    let sha = run(dir, &args, &[], None)?;
    let snapshot = Snapshot {
        feature: feature.to_string(),
        time: time_of(now_ms),
        at: now_ms.div_euclid(1000) * 1000,
        sha,
    };
    // A ref of that second already there is kept: `""` as the old value refuses to replace it.
    run(
        dir,
        &["update-ref", &snapshot.reference(), &snapshot.sha, ""],
        &[],
        None,
    )?;
    Ok(Some(snapshot))
}

/// Whether the snapshot `sha` holds the same state: its tree, its index's
/// tree, its untracked files' tree.
fn is_same(dir: &Path, sha: &str, tree: &str, index: &str, untracked: Option<&str>) -> bool {
    let tree_of = |rev: &str| run(dir, &["rev-parse", &format!("{rev}^{{tree}}")], &[], None).ok();
    tree_of(sha).as_deref() == Some(tree)
        && tree_of(&format!("{sha}^2")) == tree_of(index)
        && tree_of(&format!("{sha}^3")) == untracked.and_then(tree_of)
}

/// Before breadboard ends a claude standing in `cwd`: a snapshot of the
/// worktree it stands in (the main checkout is no agent's), the old ones
/// pruned. What the line says, None when nothing was kept.
pub fn before_ending(cwd: &Path, now_ms: i64) -> Option<String> {
    let dir = PathBuf::from(git(cwd, &["rev-parse", "--show-toplevel"]).out().trim());
    let main = main_dir(cwd)?;
    if dir.as_os_str().is_empty() || dir == main {
        return None;
    }
    let feature = feature_of(&main, &dir);
    prune(&main, now_ms);
    match take(&dir, &feature, now_ms) {
        Ok(Some(snapshot)) => Some(format!("its uncommitted work is kept as {}", snapshot.reference())),
        Ok(None) => None,
        Err(why) => Some(format!("its uncommitted work could not be kept: {why}")),
    }
}

/// The snapshots of the clone of `dir` (of `feature` alone when given), newest first.
pub fn list(dir: &Path, feature: Option<&str>) -> Vec<Snapshot> {
    let pattern = match feature {
        Some(feature) => format!("{PREFIX}{feature}/"),
        None => PREFIX.to_string(),
    };
    let mut snapshots: Vec<Snapshot> = git(dir, &["for-each-ref", "--format=%(objectname) %(refname)", &pattern])
        .lines()
        .filter_map(|line| {
            let (sha, name) = line.split_once(' ')?;
            let (feature, time) = name.strip_prefix(PREFIX)?.rsplit_once('/')?;
            Some(Snapshot {
                feature: feature.to_string(),
                time: time.to_string(),
                at: at_of(time)?,
                sha: sha.to_string(),
            })
        })
        .collect();
    snapshots.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.feature.cmp(&b.feature)));
    snapshots
}

/// The newest snapshot of `feature`.
pub fn newest(dir: &Path, feature: &str) -> Option<Snapshot> {
    list(dir, Some(feature)).into_iter().next()
}

/// The snapshot applied to the worktree `dir` (`git stash apply`: the
/// tracked changes and the untracked files; the index only as changes).
/// Refused by git where it would overwrite a change or a file there now.
pub fn restore(dir: &Path, snapshot: &Snapshot) -> Result<(), String> {
    run(dir, &["stash", "apply", &snapshot.sha], &[], None).map(|_| ())
}

/// The snapshots that go: older than `KEEP_MS`, or of a feature that is
/// done (its worktree gone and no local branch `*/<feature>` left: merged
/// and removed by `wt:remove`). Removed; what went.
pub fn prune(main: &Path, now_ms: i64) -> Vec<Snapshot> {
    let branches: Vec<String> = git(main, &["for-each-ref", "--format=%(refname:short)", "refs/heads/"])
        .lines()
        .map(String::from)
        .collect();
    let worktrees: Vec<PathBuf> = crate::git::all_worktrees(main)
        .into_iter()
        .map(|tree| tree.dir)
        .collect();
    let is_done = |feature: &str| {
        let has_branch = branches
            .iter()
            .any(|branch| branch == feature || branch.ends_with(&format!("/{feature}")));
        let has_worktree = worktrees.iter().any(|dir| feature_of(main, dir) == feature);
        !has_branch && !has_worktree
    };
    let gone: Vec<Snapshot> = list(main, None)
        .into_iter()
        .filter(|snapshot| now_ms - snapshot.at > KEEP_MS || is_done(&snapshot.feature))
        .collect();
    for snapshot in &gone {
        let _ = run(
            main,
            &["update-ref", "-d", &snapshot.reference(), &snapshot.sha],
            &[],
            None,
        );
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, run as sh, worktree};
    use crate::lock::scratch::Scratch;
    use std::fs;

    const NOW: i64 = 1_792_160_553_000; // 2026-10-16T14:22:33Z

    #[test]
    fn names_its_time_without_colons_and_reads_it_back() {
        assert_eq!(time_of(NOW + 456), "20261016T142233Z");
        assert_eq!(at_of("20261016T142233Z"), Some(NOW));
        assert_eq!(at_of("2026-10-16T14:22:33Z"), None);
        assert_eq!(at_of("20261016T1422Z"), None);
    }

    #[test]
    fn keeps_tracked_and_untracked_work_without_touching_the_worktree() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "7-x", false);
        fs::write(dir.join("README.md"), "# changed\n").unwrap();
        fs::write(dir.join("staged.md"), "staged\n").unwrap();
        sh(&dir, &["add", "staged.md"]);
        fs::write(dir.join("new.md"), "new\n").unwrap();
        fs::create_dir_all(dir.join(".claude")).unwrap();
        fs::write(dir.join(".claude/ignored"), "x\n").unwrap();
        let status = sh(&dir, &["status", "--porcelain"]);

        let snapshot = take(&dir, "7-x", NOW).unwrap().unwrap();
        assert_eq!(snapshot.reference(), "refs/breadboard/snapshots/7-x/20261016T142233Z");
        assert_eq!(sh(&dir, &["status", "--porcelain"]), status);
        assert_eq!(fs::read_to_string(dir.join("new.md")).unwrap(), "new\n");
        assert_eq!(list(&main, None), vec![snapshot.clone()]);
        // Nothing new: no second ref.
        assert_eq!(take(&dir, "7-x", NOW + 60_000).unwrap(), None);

        sh(&dir, &["reset", "--quiet", "--hard"]);
        fs::remove_file(dir.join("new.md")).unwrap();
        restore(&dir, &newest(&main, "7-x").unwrap()).unwrap();
        assert_eq!(fs::read_to_string(dir.join("README.md")).unwrap(), "# changed\n");
        assert_eq!(fs::read_to_string(dir.join("staged.md")).unwrap(), "staged\n");
        assert_eq!(fs::read_to_string(dir.join("new.md")).unwrap(), "new\n");
        assert!(!sh(&dir, &["ls-tree", "-r", "--name-only", &format!("{}^3", snapshot.sha)]).contains("ignored"));
    }

    #[test]
    fn keeps_untracked_files_alone_and_nothing_when_clean() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "8-y", false);
        assert_eq!(take(&dir, "8-y", NOW).unwrap(), None);
        fs::write(dir.join("only.md"), "only\n").unwrap();
        let snapshot = take(&dir, "8-y", NOW).unwrap().unwrap();
        fs::remove_file(dir.join("only.md")).unwrap();
        restore(&dir, &snapshot).unwrap();
        assert_eq!(fs::read_to_string(dir.join("only.md")).unwrap(), "only\n");
        assert_eq!(sh(&dir, &["status", "--porcelain"]), "?? only.md");
    }

    #[test]
    fn says_what_it_kept_before_ending_a_claude_in_a_worktree_only() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "9-z", false);
        fs::write(dir.join("work/more.md"), "more\n").unwrap();
        assert_eq!(
            before_ending(&dir.join("work"), NOW).as_deref(),
            Some("its uncommitted work is kept as refs/breadboard/snapshots/9-z/20261016T142233Z")
        );
        fs::write(main.join("dirty.md"), "x\n").unwrap();
        assert_eq!(before_ending(&main, NOW), None);
    }

    #[test]
    fn prunes_old_ones_and_those_of_removed_branches() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let kept = worktree(&main, "10-a", false);
        let done = worktree(&main, "11-b", false);
        for dir in [&kept, &done] {
            fs::write(dir.join("wip.md"), "wip\n").unwrap();
        }
        let old = take(&kept, "10-a", NOW - KEEP_MS - 1000).unwrap().unwrap();
        fs::write(kept.join("wip.md"), "wip 2\n").unwrap();
        let young = take(&kept, "10-a", NOW).unwrap().unwrap();
        let removed = take(&done, "11-b", NOW).unwrap().unwrap();
        assert_eq!(prune(&main, NOW), vec![old]);
        assert_eq!(list(&main, None), vec![young.clone(), removed.clone()]);

        sh(&main, &["worktree", "remove", "--force", done.to_str().unwrap()]);
        assert_eq!(prune(&main, NOW), Vec::<Snapshot>::new());
        sh(&main, &["branch", "--quiet", "-D", "feature/11-b"]);
        assert_eq!(prune(&main, NOW), vec![removed]);
        assert_eq!(list(&main, None), vec![young]);
    }
}
