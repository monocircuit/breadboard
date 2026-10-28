//! A red batch (#413): the full pipeline of a shipped batch's MR into
//! develop failed, and the tasks merged into it are the suspects. Those
//! whose files a failed job covers (its `rules: changes:` in
//! `.gitlab-ci.yml`) come first.

use crate::git::{git, issue_of_branch};
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{LazyLock, Mutex};

/// Top-level keys of `.gitlab-ci.yml` that are no jobs.
const GLOBAL: [&str; 5] = ["workflow", "default", "stages", "variables", "include"];

/// The job of a pipeline's job name: `check:breadboard-gauntlet 2/3` (a
/// copy of `parallel`) and `deploy: [test]` (a matrix's) are their job's.
pub fn job_of(name: &str) -> &str {
    static COPY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(: \[.*\]| \d+/\d+)$").unwrap());
    COPY.find(name).map_or(name, |found| &name[..found.start()])
}

/// Every job of `.gitlab-ci.yml` with the files it covers: the globs of its
/// `rules: changes:`, aliases resolved. A job whose rules name no files is
/// missing: it runs for any change. No YAML parser, the file's own shape is
/// enough (scripts/check-ci-tiers.mjs reads it the same way).
pub fn job_files(ci: &str) -> BTreeMap<String, Vec<String>> {
    let lines: Vec<&str> = ci.lines().collect();
    let anchors = anchors_of(&lines);
    let mut jobs = BTreeMap::new();
    for (at, line) in lines.iter().enumerate() {
        let Some(job) = top_key(line) else {
            continue;
        };
        if job.starts_with('.') || GLOBAL.contains(&job) {
            continue;
        }
        let block = block_after(&lines, at);
        let Some(rules) = block
            .iter()
            .position(|line| indent_of(line) == 2 && line.trim().starts_with("rules:"))
        else {
            continue;
        };
        let rules: Vec<&str> = match alias_of(value_of(block[rules])) {
            Some(alias) => anchors.get(alias).cloned().unwrap_or_default(),
            None => list_after(&block, rules),
        };
        let mut globs: Vec<String> = Vec::new();
        for (at, line) in rules.iter().enumerate() {
            if !line.trim().trim_start_matches("- ").starts_with("changes:") {
                continue;
            }
            let items = match alias_of(value_of(line)) {
                Some(alias) => anchors.get(alias).cloned().unwrap_or_default(),
                None => list_after(&rules, at),
            };
            for glob in globs_of(&items, &anchors) {
                if !globs.contains(&glob) {
                    globs.push(glob);
                }
            }
        }
        if !globs.is_empty() {
            jobs.insert(job.to_string(), globs);
        }
    }
    jobs
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn top_key(line: &str) -> Option<&str> {
    static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([^\s#-]\S*?):(\s|$)").unwrap());
    KEY.captures(line).map(|key| key.get(1).unwrap().as_str())
}

/// What follows the key on its line: `changes: *x` → `*x`.
fn value_of(line: &str) -> &str {
    line.split_once(':').map_or("", |(_, value)| value.trim())
}

fn alias_of(value: &str) -> Option<&str> {
    value.strip_prefix('*').map(str::trim)
}

/// The lines of a top-level block, up to the next top-level key.
fn block_after<'a>(lines: &[&'a str], at: usize) -> Vec<&'a str> {
    let mut block = vec![lines[at]];
    block.extend(lines[at + 1..].iter().take_while(|line| top_key(line).is_none()));
    block
}

/// The lines below `lines[at]` deeper than it, blank lines and comments left out.
fn list_after<'a>(lines: &[&'a str], at: usize) -> Vec<&'a str> {
    let depth = indent_of(lines[at]);
    lines[at + 1..]
        .iter()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .take_while(|line| indent_of(line) > depth)
        .copied()
        .collect()
}

/// Every `&name` that ends its line, with the lines it anchors.
fn anchors_of<'a>(lines: &[&'a str]) -> BTreeMap<&'a str, Vec<&'a str>> {
    static ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&([\w.-]+)\s*$").unwrap());
    lines
        .iter()
        .enumerate()
        .filter_map(|(at, line)| {
            let name = ANCHOR.captures(line)?.get(1)?.as_str();
            Some((name, list_after(lines, at)))
        })
        .collect()
}

/// The items of a list of globs, `- *alias` items resolved.
fn globs_of(items: &[&str], anchors: &BTreeMap<&str, Vec<&str>>) -> Vec<String> {
    let depth = items.first().map_or(0, |line| indent_of(line));
    let mut globs = Vec::new();
    for line in items.iter().filter(|line| indent_of(line) == depth) {
        let Some(item) = line.trim().strip_prefix("- ") else {
            continue;
        };
        let item = item.split(" #").next().unwrap_or(item).trim();
        match alias_of(item) {
            Some(alias) => globs.extend(globs_of(anchors.get(alias).map_or(&[][..], Vec::as_slice), anchors)),
            None => globs.push(item.trim_matches(['"', '\'']).to_string()),
        }
    }
    globs
}

/// Whether `path` matches one of `globs` the way GitLab's `changes:` does:
/// `**` any depth, `*` and `?` within a directory, `{a,b}` either.
pub fn covers(globs: &[String], path: &str) -> bool {
    globs
        .iter()
        .any(|glob| Regex::new(&format!("^{}$", regex_of(glob))).is_ok_and(|regex| regex.is_match(path)))
}

fn regex_of(glob: &str) -> String {
    let mut regex = String::new();
    let mut chars = glob.chars().peekable();
    while let Some(char) = chars.next() {
        match char {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    regex.push_str("(?:.*/)?");
                } else {
                    regex.push_str(".*");
                }
            }
            '*' => regex.push_str("[^/]*"),
            '?' => regex.push_str("[^/]"),
            '{' => regex.push_str("(?:"),
            '}' => regex.push(')'),
            ',' => regex.push('|'),
            other => regex.push_str(&regex::escape(&other.to_string())),
        }
    }
    regex
}

/// A task merged into the batch: one merge commit on its first-parent line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Merged {
    pub sha: String,
    pub subject: String,
    /// Its MR's title: the first line of GitLab's message after the subject.
    pub title: String,
    /// Its task MR, from GitLab's `See merge request …!N`.
    pub mr: Option<i64>,
    pub issue: Option<u64>,
    /// The files it changed in the batch.
    pub files: Vec<String>,
}

/// The tasks merged between `green` (the commit of the last green batch
/// pipeline, or where the batch left develop) and `red`, newest first.
pub fn merged_since(dir: &Path, green: &str, red: &str) -> Vec<Merged> {
    let range = format!("{green}..{red}");
    let log = git(
        dir,
        &[
            "log",
            "--first-parent",
            "--merges",
            "--format=%H%x1f%s%x1f%b%x1e",
            &range,
        ],
    );
    if !log.is_ok {
        return Vec::new();
    }
    log.stdout
        .split('\x1e')
        .filter_map(|entry| {
            let mut fields = entry.trim_start_matches('\n').splitn(3, '\x1f');
            let sha = fields.next().filter(|sha| !sha.is_empty())?.to_string();
            let subject = fields.next().unwrap_or("").to_string();
            let body = fields.next().unwrap_or("");
            let files = git(dir, &["diff", "--name-only", &format!("{sha}^1"), &sha])
                .lines()
                .map(str::to_string)
                .collect();
            // A task's merge by GitLab; not develop merged in to keep up.
            let mr = mr_of(body)?;
            Some(Merged {
                mr: Some(mr),
                issue: branch_of(&subject).and_then(issue_of_branch),
                title: title_of(body),
                sha,
                subject,
                files,
            })
        })
        .collect()
}

/// A batch shipped to develop as git has it (#413): the tasks merged into
/// it, the files each CI job covers at its head.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Shipped {
    pub branch: String,
    /// Newest first.
    pub tasks: Vec<Merged>,
    pub jobs: BTreeMap<String, Vec<String>>,
}

/// The shipped batch `branch` on origin, the tasks it has that develop
/// hasn't; read once per head of it. None while origin has no such branch.
pub fn shipped_of(main: &Path, branch: &str, develop: &str) -> Option<Shipped> {
    static READ: LazyLock<Mutex<HashMap<(String, String), Shipped>>> = LazyLock::new(Mutex::default);
    let remote = format!("origin/{branch}");
    let head = git(main, &["rev-parse", "--verify", "--quiet", &remote]);
    if !head.is_ok {
        return None;
    }
    let key = (branch.to_string(), head.out().to_string());
    if let Some(found) = READ.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Some(found.clone());
    }
    let ci = git(main, &["show", &format!("{remote}:.gitlab-ci.yml")]);
    let shipped = Shipped {
        branch: branch.to_string(),
        tasks: merged_since(main, &format!("origin/{develop}"), &remote),
        jobs: if ci.is_ok {
            job_files(&ci.stdout)
        } else {
            BTreeMap::new()
        },
    };
    READ.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, shipped.clone());
    Some(shipped)
}

/// The MR's title in GitLab's merge message: its first line that names no issue or MR.
fn title_of(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("Closes ") && !line.starts_with("See merge request "))
        .unwrap_or_default()
        .to_string()
}

/// `See merge request group/project!412` → 412.
fn mr_of(body: &str) -> Option<i64> {
    let (_, after) = body.split_once("See merge request ")?;
    let (_, number) = after.lines().next()?.rsplit_once('!')?;
    number.trim().parse().ok()
}

/// `Merge branch 'feature/413-x' into 'batch/breadboard-1'` → `feature/413-x`.
fn branch_of(subject: &str) -> Option<&str> {
    let (_, after) = subject.split_once("Merge branch '")?;
    after.split_once('\'').map(|(branch, _)| branch)
}

/// A task that may have turned the batch red.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suspect {
    pub merged: Merged,
    /// It changed a file the failing job covers.
    pub is_covered: bool,
}

/// The suspects of a red batch, newest first, those with a file one of the
/// failed jobs covers ahead of the others. `jobs`: the globs of the failed
/// jobs that name files; one that runs on any change ranks no task.
pub fn suspects(tasks: &[Merged], jobs: &[&[String]]) -> Vec<Suspect> {
    let mut suspects: Vec<Suspect> = tasks
        .iter()
        .map(|merged| Suspect {
            is_covered: jobs
                .iter()
                .any(|globs| merged.files.iter().any(|file| covers(globs, file))),
            merged: merged.clone(),
        })
        .collect();
    suspects.sort_by_key(|suspect| !suspect.is_covered);
    suspects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::*;
    use crate::lock::scratch::Scratch;

    const CI: &str = r#"
stages:
  - check

.vita-changes: &vita-changes
  - products/vita/**/*
  - packages/{monolithium,utilities}/**/*
  - pnpm-lock.yaml # the install

.into-batch: &into-batch
  if: $CI_MERGE_REQUEST_TARGET_BRANCH_NAME =~ /^batch\//
  when: never

check:vita:
  script:
    - pnpm test
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
      changes: *vita-changes
    - if: $CI_COMMIT_BRANCH =~ /^epic\//
      changes: *vita-changes

check:breadboard-rs:
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
      changes: &breadboard-rs-changes
        - packages/breadboard/**/*

check:breadboard-gauntlet:
  parallel: 3
  rules: &gauntlet-rules
    - *into-batch
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
      changes:
        - *breadboard-rs-changes
        - "packages/breadboard-gauntlet/**/*"

check:breadboard-budgets:
  rules: *gauntlet-rules

build:breadboard:
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
"#;

    #[test]
    fn reads_the_files_each_job_covers_with_its_aliases() {
        let jobs = job_files(CI);
        assert_eq!(
            jobs["check:vita"],
            [
                "products/vita/**/*",
                "packages/{monolithium,utilities}/**/*",
                "pnpm-lock.yaml"
            ]
        );
        assert_eq!(jobs["check:breadboard-rs"], ["packages/breadboard/**/*"]);
        let gauntlet = ["packages/breadboard/**/*", "packages/breadboard-gauntlet/**/*"];
        assert_eq!(jobs["check:breadboard-gauntlet"], gauntlet);
        assert_eq!(jobs["check:breadboard-budgets"], gauntlet);
        // No files: it runs for any change.
        assert!(!jobs.contains_key("build:breadboard"));
        assert!(!jobs.contains_key("stages"));
    }

    #[test]
    fn reads_the_ci_file_of_the_repo_it_is_developed_in() {
        // The mirror has none: there the fixture above stands in for it.
        let Ok(ci) = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../.gitlab-ci.yml")) else {
            return;
        };
        let jobs = job_files(&ci);
        assert!(covers(
            &jobs["check:breadboard-rs"],
            "packages/breadboard/crates/core/src/batch_red.rs"
        ));
        assert!(covers(&jobs["check:vita"], "packages/monolithium/src/Button.tsx"));
        assert!(!covers(&jobs["check:vita"], "packages/breadboard/Cargo.toml"));
    }

    #[test]
    fn matches_paths_like_gitlab_s_changes() {
        let globs = [
            "packages/{monolithium,utilities}/**/*".to_string(),
            "*.yaml".to_string(),
        ];
        assert!(covers(&globs, "packages/utilities/src/cn.ts"));
        assert!(covers(&globs, "packages/monolithium/README.md"));
        assert!(covers(&globs, "pnpm-lock.yaml"));
        assert!(!covers(&globs, "packages/tanstack/src/x.ts"));
        assert!(!covers(&globs, "products/vita/x.yaml"));
    }

    #[test]
    fn names_the_job_of_a_parallel_copy_or_a_matrix() {
        assert_eq!(job_of("check:breadboard-gauntlet 2/3"), "check:breadboard-gauntlet");
        assert_eq!(job_of("deploy: [test]"), "deploy");
        assert_eq!(job_of("check:vita"), "check:vita");
    }

    fn merged(sha: &str, files: &[&str]) -> Merged {
        Merged {
            sha: sha.into(),
            subject: String::new(),
            title: String::new(),
            mr: None,
            issue: None,
            files: files.iter().map(|file| file.to_string()).collect(),
        }
    }

    #[test]
    fn puts_the_tasks_a_failed_job_covers_first() {
        let rs = vec!["packages/breadboard/**/*".to_string()];
        let readme = vec!["README.md".to_string()];
        let tasks = vec![
            merged("c", &["products/vita/a.ts"]),
            merged("b", &["packages/breadboard/x.rs"]),
            merged("a", &["README.md"]),
        ];
        let order = |suspects: Vec<Suspect>| suspects.iter().map(|one| one.merged.sha.clone()).collect::<Vec<_>>();
        let found = suspects(&tasks, &[&rs]);
        assert_eq!(order(found.clone()), ["b", "c", "a"]);
        assert!(found[0].is_covered && !found[1].is_covered);
        // Two failed jobs: what either covers, newest first.
        assert_eq!(order(suspects(&tasks, &[&rs, &readme])), ["b", "a", "c"]);
        // Only jobs of any change: newest first, none likelier.
        let found = suspects(&tasks, &[]);
        assert_eq!(order(found.clone()), ["c", "b", "a"]);
        assert!(found.iter().all(|one| !one.is_covered));
    }

    #[test]
    fn reads_a_shipped_batch_s_tasks_and_its_ci_jobs_from_origin() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        run(&main, &["checkout", "--quiet", "-b", "batch/breadboard-1"]);
        let start = run(&main, &["rev-parse", "HEAD"]);
        let ci = "check:rs:\n  rules:\n    - changes:\n        - packages/breadboard/**/*\n";
        let merge = |number: i64, file: &str, text: &str| {
            let branch = format!("feature/{number}-task");
            run(&main, &["checkout", "--quiet", "-b", &branch, &start]);
            std::fs::create_dir_all(main.join(file).parent().unwrap()).unwrap();
            std::fs::write(main.join(file), text).unwrap();
            run(&main, &["add", "."]);
            run(&main, &["commit", "--quiet", "-m", "feat: task"]);
            run(&main, &["checkout", "--quiet", "batch/breadboard-1"]);
            let subject = format!("Merge branch '{branch}' into 'batch/breadboard-1'");
            let body = format!(
                "feat: task {number}\n\nCloses #{number}\n\nSee merge request group/project!{}",
                number - 20
            );
            run(
                &main,
                &["merge", "--quiet", "--no-ff", "-m", &subject, "-m", &body, &branch],
            );
        };
        merge(411, "packages/breadboard/a.rs", "x\n");
        // develop merged in to keep up: no task.
        run(&main, &["checkout", "--quiet", "develop"]);
        std::fs::write(main.join("README.md"), "# hotfix\n").unwrap();
        run(&main, &["commit", "--quiet", "-am", "fix: hotfix"]);
        run(&main, &["push", "--quiet", "origin", "develop"]);
        run(&main, &["checkout", "--quiet", "batch/breadboard-1"]);
        run(
            &main,
            &[
                "merge",
                "--quiet",
                "--no-ff",
                "-m",
                "Merge branch 'develop' into 'batch/breadboard-1'",
                "develop",
            ],
        );
        merge(412, ".gitlab-ci.yml", ci);
        run(&main, &["push", "--quiet", "origin", "batch/breadboard-1"]);
        run(&main, &["fetch", "--quiet", "origin"]);
        let shipped = shipped_of(&main, "batch/breadboard-1", "develop").unwrap();
        let found: Vec<_> = shipped
            .tasks
            .iter()
            .map(|one| (one.issue, one.mr, one.title.as_str(), one.files.clone()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    Some(412),
                    Some(392),
                    "feat: task 412",
                    vec![".gitlab-ci.yml".to_string()]
                ),
                (
                    Some(411),
                    Some(391),
                    "feat: task 411",
                    vec!["packages/breadboard/a.rs".to_string()]
                ),
            ]
        );
        assert_eq!(shipped.jobs["check:rs"], ["packages/breadboard/**/*"]);
        assert_eq!(shipped_of(&main, "batch/breadboard-9", "develop"), None);
        assert!(merged_since(&main, "origin/batch/breadboard-1", "origin/batch/breadboard-1").is_empty());
    }
}
