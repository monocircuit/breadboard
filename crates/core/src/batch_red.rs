//! A red batch (#413): the full pipeline of a batch failed, and the tasks
//! merged into it since its last green one are the suspects. For a test
//! failure, the tasks whose files the failing job covers (its `rules:
//! changes:` in `.gitlab-ci.yml`) come first.

use crate::git::{git, issue_of_branch};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

/// Top-level keys of `.gitlab-ci.yml` that are no jobs.
const GLOBAL: [&str; 5] = ["workflow", "default", "stages", "variables", "include"];

/// What the band and the batch row say.
pub fn headline(job: &str) -> String {
    format!("batch red: {job}")
}

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub sha: String,
    pub subject: String,
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
            Some(Merged {
                mr: mr_of(body),
                issue: branch_of(&subject).and_then(issue_of_branch),
                sha,
                subject,
                files,
            })
        })
        .collect()
}

/// `See merge request monocircuit/platform!412` → 412.
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

/// The suspects of a red batch, newest first; for a test failure (the job's
/// script failed, no runner or timeout), those whose files `globs` cover
/// ahead of the others. Without globs the job runs for any change, and
/// covers every task.
pub fn suspects(merged: Vec<Merged>, globs: Option<&[String]>, is_test_failure: bool) -> Vec<Suspect> {
    let mut suspects: Vec<Suspect> = merged
        .into_iter()
        .map(|merged| Suspect {
            is_covered: globs.is_none_or(|globs| merged.files.iter().any(|file| covers(globs, file))),
            merged,
        })
        .collect();
    if is_test_failure {
        suspects.sort_by_key(|suspect| !suspect.is_covered);
    }
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
    fn reads_the_repo_s_own_ci_file() {
        let ci = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../.gitlab-ci.yml")).unwrap();
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
        assert_eq!(headline("check:vita"), "batch red: check:vita");
    }

    fn merged(sha: &str, files: &[&str]) -> Merged {
        Merged {
            sha: sha.into(),
            subject: String::new(),
            mr: None,
            issue: None,
            files: files.iter().map(|file| file.to_string()).collect(),
        }
    }

    #[test]
    fn puts_the_tasks_the_failing_test_covers_first() {
        let globs = vec!["packages/breadboard/**/*".to_string()];
        let tasks = || {
            vec![
                merged("c", &["products/vita/a.ts"]),
                merged("b", &["packages/breadboard/x.rs"]),
                merged("a", &["README.md"]),
            ]
        };
        let order = |suspects: Vec<Suspect>| suspects.iter().map(|one| one.merged.sha.clone()).collect::<Vec<_>>();
        assert_eq!(order(suspects(tasks(), Some(&globs), true)), ["b", "c", "a"]);
        // The runner failed: no task is more likely than another, newest first.
        let kept = suspects(tasks(), Some(&globs), false);
        assert_eq!(order(kept.clone()), ["c", "b", "a"]);
        assert!(kept[1].is_covered && !kept[0].is_covered);
        // A job for any change covers each.
        assert!(suspects(tasks(), None, true).iter().all(|one| one.is_covered));
    }

    #[test]
    fn lists_the_tasks_merged_since_the_last_green_pipeline() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        run(&main, &["checkout", "--quiet", "-b", "batch/breadboard-1"]);
        let green = run(&main, &["rev-parse", "HEAD"]);
        for (number, file) in [(411, "packages/breadboard/a.rs"), (412, ".gitlab-ci.yml")] {
            let branch = format!("feature/{number}-task");
            run(&main, &["checkout", "--quiet", "-b", &branch, &green]);
            std::fs::create_dir_all(main.join(file).parent().unwrap()).unwrap();
            std::fs::write(main.join(file), "x\n").unwrap();
            run(&main, &["add", "."]);
            run(&main, &["commit", "--quiet", "-m", "feat: task"]);
            run(&main, &["checkout", "--quiet", "batch/breadboard-1"]);
            let subject = format!("Merge branch '{branch}' into 'batch/breadboard-1'");
            let body = format!("See merge request monocircuit/platform!{}", number - 20);
            run(
                &main,
                &["merge", "--quiet", "--no-ff", "-m", &subject, "-m", &body, &branch],
            );
        }
        let red = run(&main, &["rev-parse", "HEAD"]);
        let tasks = merged_since(&main, &green, &red);
        let found: Vec<_> = tasks.iter().map(|one| (one.issue, one.mr, one.files.clone())).collect();
        assert_eq!(
            found,
            [
                (Some(412), Some(392), vec![".gitlab-ci.yml".to_string()]),
                (Some(411), Some(391), vec!["packages/breadboard/a.rs".to_string()]),
            ]
        );
        assert!(merged_since(&main, &red, &red).is_empty());
    }
}
