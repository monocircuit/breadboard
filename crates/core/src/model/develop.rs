//! Two more answers for the view of one worktree (#158, develop.mjs), both
//! local: what develop brought in since the branch started, and whether
//! merging it would conflict; and the migrations the database of the
//! worktree's stack lacks.

use crate::git::git;
use regex::Regex;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

/// The folder of vita's web app in a checkout.
pub const VITA: &str = "products/vita/web";

/// An MR merged on develop's first-parent line since the branch started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub mr: Option<i64>,
    pub title: String,
    pub branch: String,
}

/// DEVELOP of a worktree: the merges newest first, the commits that came in
/// without one, and the files a `git merge origin/develop` would stop at
/// (None when git can't tell, older than 2.38).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Develop {
    pub merges: Vec<Merged>,
    pub direct: usize,
    pub conflicts: Option<Vec<String>>,
}

static MERGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Merge branch '([^']+)' into").unwrap());
static MR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)!(\d+)\s*$").unwrap());
static TRAILER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(Closes|Refs|See merge request|Co-Authored-By)\b").unwrap());

/// What came into develop since `dir` branched off (develop.mjs `developOf`).
pub fn develop_of(dir: &Path) -> Develop {
    // Fields split by 0x1f, entries by 0x1e: a body has newlines.
    let log = git(
        dir,
        &["log", "--first-parent", "--format=%s%x1f%b%x1e", "HEAD..origin/develop"],
    )
    .stdout;
    let entries: Vec<&str> = log
        .split('\u{1e}')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    let mut develop = Develop::default();
    for entry in &entries {
        let (subject, body) = entry.split_once('\u{1f}').unwrap_or((entry, ""));
        let Some(branch) = MERGE.captures(subject).map(|found| found[1].to_string()) else {
            develop.direct += 1;
            continue;
        };
        let mr = MR.captures(body).and_then(|found| found[1].parse().ok());
        let title = body
            .split('\n')
            .map(str::trim)
            .find(|line| !line.is_empty() && !TRAILER.is_match(line))
            .map(String::from)
            .unwrap_or_else(|| branch.clone());
        develop.merges.push(Merged { mr, title, branch });
    }
    develop.conflicts = if entries.is_empty() {
        Some(Vec::new())
    } else {
        let tree = git(
            dir,
            &[
                "merge-tree",
                "--write-tree",
                "--name-only",
                "--no-messages",
                "HEAD",
                "origin/develop",
            ],
        );
        // 0 clean, 1 conflicts (the tree's id, then one file per line), anything else: git can't.
        match tree.status {
            0 => Some(Vec::new()),
            1 => Some(tree.lines().skip(1).map(String::from).collect()),
            _ => None,
        }
    };
    develop
}

/// The migrations the database of a worktree's stack hasn't applied, the way
/// drizzle decides it; `Error` when the database doesn't answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migrations {
    Pending { pending: Vec<String>, applied: usize },
    Error(String),
}

fn pending_since(journal: &Path, newest: i64) -> Option<Migrations> {
    let value = crate::json::read(journal)?;
    let entries = value
        .get("entries")
        .and_then(|entries| entries.as_array())
        .cloned()
        .unwrap_or_default();
    let pending: Vec<String> = entries
        .iter()
        .filter(|entry| entry.get("when").and_then(|when| when.as_i64()).unwrap_or(0) > newest)
        .filter_map(|entry| entry.get("tag").and_then(|tag| tag.as_str()).map(String::from))
        .collect();
    Some(Migrations::Pending {
        applied: entries.len() - pending.len(),
        pending,
    })
}

/// migrationsOf: None without .env.local, journal or psql.
pub fn migrations_of(dir: &Path) -> Option<Migrations> {
    let vita = dir.join(VITA);
    let journal = vita.join("src/server/db/migrations/meta/_journal.json");
    let env = std::fs::read_to_string(vita.join(".env.local")).ok()?;
    if !journal.exists() {
        return None;
    }
    let url = env
        .lines()
        .find_map(|line| line.strip_prefix("MIGRATION_DATABASE_URL="))
        .map(str::trim)?;
    let answer = Command::new("psql")
        .args([
            url,
            "--no-psqlrc",
            "-tAc",
            "select coalesce(max(created_at), 0) from drizzle.__drizzle_migrations",
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if answer.status.success() {
        let newest = String::from_utf8_lossy(&answer.stdout).trim().parse().unwrap_or(0);
        return pending_since(&journal, newest);
    }
    let error = String::from_utf8_lossy(&answer.stderr).into_owned();
    // A fresh database has no drizzle schema yet: everything is pending.
    if error.contains("does not exist") {
        return pending_since(&journal, 0);
    }
    Some(Migrations::Error(
        error
            .lines()
            .find(|line| !line.is_empty())
            .unwrap_or("no answer")
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, run};
    use crate::lock::scratch::Scratch;

    #[test]
    fn names_the_merges_develop_brought_in_and_its_conflicts() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = crate::git::fixture::worktree(&main, "1-one", false);
        assert_eq!(
            develop_of(&dir),
            Develop {
                conflicts: Some(vec![]),
                ..Default::default()
            }
        );
        run(&main, &["switch", "-q", "-c", "feature/2-two"]);
        std::fs::write(main.join("two.md"), "two\n").unwrap();
        run(&main, &["add", "."]);
        run(&main, &["commit", "-q", "-m", "feat: two"]);
        run(&main, &["switch", "-q", "develop"]);
        run(
            &main,
            &[
                "merge",
                "-q",
                "--no-ff",
                "-m",
                "Merge branch 'feature/2-two' into 'develop'\n\nfeat: two\n\nSee merge request monocircuit/platform!42",
                "feature/2-two",
            ],
        );
        std::fs::write(main.join("direct.md"), "x\n").unwrap();
        run(&main, &["add", "."]);
        run(&main, &["commit", "-q", "-m", "chore: direct"]);
        run(&main, &["push", "-q", "origin", "develop"]);
        run(&dir, &["fetch", "-q", "origin"]);
        let develop = develop_of(&dir);
        assert_eq!(develop.direct, 1);
        assert_eq!(
            develop.merges,
            [Merged {
                mr: Some(42),
                title: "feat: two".into(),
                branch: "feature/2-two".into()
            }]
        );
        assert!(develop.conflicts.is_none_or(|conflicts| conflicts.is_empty()));
    }

    #[test]
    fn counts_what_the_journal_has_past_the_newest_applied() {
        let scratch = Scratch::new();
        let journal = scratch.join("_journal.json");
        std::fs::write(
            &journal,
            r#"{"entries":[{"tag":"0001_a","when":10},{"tag":"0002_b","when":20}]}"#,
        )
        .unwrap();
        assert_eq!(
            pending_since(&journal, 10),
            Some(Migrations::Pending {
                pending: vec!["0002_b".into()],
                applied: 1
            })
        );
        assert_eq!(migrations_of(scratch.path()), None);
    }
}
