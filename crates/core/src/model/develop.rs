//! Two more answers for the view of one worktree (#158, develop.mjs), both
//! local: what its base (develop or an epic, #349) brought in since the branch started, and whether
//! merging it would conflict; and the migrations the database of the
//! worktree's stack lacks.

use crate::git::{base_of, git};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

/// The folder of vita's web app in a checkout.
pub const VITA: &str = "products/vita/web";

/// An MR merged on develop's first-parent line since the branch started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub mr: Option<i64>,
    pub title: String,
    pub branch: String,
}

/// DEVELOP (its base) of a worktree: the merges newest first, the commits that came in
/// without one, and the files a `git merge` of the base would stop at
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

/// What came into its base since `dir` branched off (develop.mjs `developOf`).
pub fn develop_of(dir: &Path) -> Develop {
    // Fields split by 0x1f, entries by 0x1e: a body has newlines.
    let log = git(
        dir,
        &[
            "log",
            "--first-parent",
            "--format=%s%x1f%b%x1e",
            &format!("HEAD..{}", base_of(dir).remote),
        ],
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
        conflicts_of(dir)
    };
    develop
}

/// The files a `git merge` of its base in `dir` would stop at, empty when
/// it merges cleanly; None when git can't tell (older than 2.38).
pub fn conflicts_of(dir: &Path) -> Option<Vec<String>> {
    let tree = git(
        dir,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            "HEAD",
            &base_of(dir).remote,
        ],
    );
    // 0 clean, 1 conflicts (the tree's id, then one file per line), anything else: git can't.
    match tree.status {
        0 => Some(Vec::new()),
        1 => Some(tree.lines().skip(1).map(String::from).collect()),
        _ => None,
    }
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
        // A database that never answers holds no read (an orchestration asks every worktree's).
        .env("PGCONNECT_TIMEOUT", "3")
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

/// How long a worktree's migrations hold (develop.mjs: 15 s, or until its journal changes).
pub const MIGRATIONS_HOLD_MS: i64 = 15_000;

/// The migrations last asked, by worktree: when, the journal's time then, the answer.
type Asked = (i64, Option<SystemTime>, Option<Migrations>);
static MIGRATIONS: LazyLock<Mutex<HashMap<PathBuf, Asked>>> = LazyLock::new(Mutex::default);

fn journal_time(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join(VITA).join("src/server/db/migrations/meta/_journal.json"))
        .and_then(|meta| meta.modified())
        .ok()
}

/// `migrations_of`, asked again after MIGRATIONS_HOLD_MS or when the journal
/// changed (develop.mjs's cache): the database's state is in no file, so an
/// orchestration's read asks it beside git's facts, not with them.
pub fn migrations_held(dir: &Path, now: i64) -> Option<Migrations> {
    migrations_held_by(dir, now, migrations_of)
}

/// `migrations_held` with its psql given (the tests').
fn migrations_held_by(dir: &Path, now: i64, ask: impl FnOnce(&Path) -> Option<Migrations>) -> Option<Migrations> {
    let journal = journal_time(dir);
    let held = MIGRATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(dir)
        .cloned();
    if let Some((at, was, answer)) = held
        && was == journal
        && (0..MIGRATIONS_HOLD_MS).contains(&(now - at))
    {
        return answer;
    }
    let answer = ask(dir);
    MIGRATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(dir.to_path_buf(), (now, journal, answer.clone()));
    answer
}

/// Forgets what `migrations_held` holds of `dir`: a `pnpm pg:migrate` there ended.
pub fn forget_migrations(dir: &Path) {
    MIGRATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, run};
    use crate::lock::scratch::Scratch;

    #[test]
    fn migrations_are_asked_again_after_a_while_when_the_journal_changes_or_once_forgotten() {
        let scratch = Scratch::new();
        let dir = scratch.path().join("held");
        let journal = dir.join(VITA).join("src/server/db/migrations/meta/_journal.json");
        std::fs::create_dir_all(journal.parent().unwrap()).unwrap();
        std::fs::write(&journal, "{}").unwrap();
        let asked = std::cell::Cell::new(0);
        let ask = |_: &Path| {
            asked.set(asked.get() + 1);
            Some(Migrations::Error(format!("answer {}", asked.get())))
        };
        let answer = |n: usize| Some(Migrations::Error(format!("answer {n}")));
        assert_eq!(migrations_held_by(&dir, 1_000, ask), answer(1));
        assert_eq!(migrations_held_by(&dir, 1_000 + MIGRATIONS_HOLD_MS - 1, ask), answer(1));
        assert_eq!(migrations_held_by(&dir, 1_000 + MIGRATIONS_HOLD_MS, ask), answer(2));
        forget_migrations(&dir);
        assert_eq!(migrations_held_by(&dir, 1_000 + MIGRATIONS_HOLD_MS, ask), answer(3));
        // A new migration in the journal: asked at once.
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&journal)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(migrations_held_by(&dir, 1_000 + MIGRATIONS_HOLD_MS, ask), answer(4));
    }

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
