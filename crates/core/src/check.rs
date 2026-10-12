//! Whether a worktree's HEAD is tested (#298): `pnpm wt:check` stamps the tree
//! it tested in `.claude/checked.json` (`{ passed, failed, running }`), the
//! overview shows per worktree where its HEAD stands (worktree/check.mjs).

use crate::git::{self, git};
use crate::json::{self, str_of, strs_of};
use crate::lock::is_alive;
use regex::Regex;
use serde_json::{Map, Value};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub fn check_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("checked.json")
}

/// The record of a worktree's checks, an empty object without one.
pub fn read_check(dir: &Path) -> Map<String, Value> {
    json::read_object(&check_file(dir))
}

/// Sets the given parts of the record (a null clears one), keeps the others.
pub fn write_check(dir: &Path, parts: Map<String, Value>) -> io::Result<Map<String, Value>> {
    let mut record = read_check(dir);
    json::merge(&mut record, parts);
    record.retain(|_, value| !value.is_null());
    json::write_pretty(&check_file(dir), &Value::Object(record.clone()))?;
    Ok(record)
}

/// A task that failed, with the names of its failing tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub package: String,
    pub task: String,
    pub tests: Vec<String>,
}

fn failures_in(value: Option<&Value>) -> Vec<Failure> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|failure| Failure {
                    package: str_of(failure, "package").unwrap_or_default().into(),
                    task: str_of(failure, "task").unwrap_or_default().into(),
                    tests: strs_of(failure, "tests"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Where the HEAD of a worktree stands with its checks.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum CheckState {
    /// wt:check runs, since then.
    Checking { since: Option<String> },
    /// The last check that passed tested HEAD's tree.
    Tested { sha: String, packages: Vec<String> },
    /// The last check of HEAD's tree failed.
    Failed { sha: String, failures: Vec<Failure> },
    /// None of them: commits since the last pass (since develop without one).
    Untested { count: u64 },
    /// Nothing of its own yet.
    #[default]
    None,
}

/// checkStateOf: `record` readCheck's, `head` and `tree` HEAD's, `since(sha)`
/// how many commits HEAD has after `sha` (None when it isn't an ancestor),
/// `ahead` how many it has after develop.
pub fn check_state_of(
    record: &Map<String, Value>,
    head: &str,
    tree: Option<&str>,
    since: impl Fn(&str) -> Option<u64>,
    ahead: u64,
    alive: impl Fn(i64) -> bool,
) -> CheckState {
    let part = |key: &str| record.get(key).filter(|value| value.is_object());
    let (passed, failed, running) = (part("passed"), part("failed"), part("running"));
    if let Some(running) = running
        && json::int_of(running, "pid").is_some_and(&alive)
    {
        return CheckState::Checking {
            since: str_of(running, "since").map(String::from),
        };
    }
    let tree_is = |value: &Value| tree.is_some() && str_of(value, "tree") == tree;
    if let Some(passed) = passed.filter(|passed| tree_is(passed)) {
        return CheckState::Tested {
            sha: head.into(),
            packages: strs_of(passed, "packages"),
        };
    }
    if let Some(failed) = failed.filter(|failed| tree_is(failed)) {
        return CheckState::Failed {
            sha: head.into(),
            failures: failures_in(failed.get("failures")),
        };
    }
    if passed.is_none() && ahead == 0 {
        return CheckState::None;
    }
    let count = passed
        .and_then(|passed| since(str_of(passed, "sha").unwrap_or_default()))
        .unwrap_or(ahead);
    CheckState::Untested { count }
}

/// checkStateOf for the worktree `dir`, from git and its record.
pub fn check_of(dir: &Path) -> CheckState {
    let Some(head) = git::head_of(dir) else {
        return CheckState::None;
    };
    let tree = git::tree_of(dir, "HEAD");
    let since = |sha: &str| {
        if sha.is_empty() || !git(dir, &["merge-base", "--is-ancestor", sha, "HEAD"]).is_ok {
            return None;
        }
        git::count(dir, &format!("{sha}..HEAD"))
    };
    let ahead = git::count(dir, &format!("{}..HEAD", git::base_of(dir).remote)).unwrap_or(0);
    check_state_of(&read_check(dir), &head, tree.as_deref(), since, ahead, is_alive)
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new("\u{1b}\\[[0-9;]*m").unwrap());
static SUMMARY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*Failed:\s+(.+)$").unwrap());
static EXITED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)(\S+#\S+): command .* exited \(\d+\)").unwrap());
static PREFIXED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\S+?):(\w[\w:-]*): (.*)$").unwrap());
static VITEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*FAIL\s+(\S+\.\w+ > .+?)\s*$").unwrap());
static NODE_TEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*✖ (.+?)(?: \([\d.]+m?s\))?\s*$").unwrap());

/// What failed in turbo's output (check.mjs `failuresOf`).
pub fn failures_of(output: &str) -> Vec<Failure> {
    let text = ANSI.replace_all(output, "");
    let mut failed: Vec<(String, Failure)> = Vec::new();
    let add = |failed: &mut Vec<(String, Failure)>, id: &str, test: Option<&str>| {
        let Some((package, task)) = id.split_once('#') else {
            return;
        };
        // `id.split("#")` there: the task is what stands between the first two `#`.
        let task = task.split('#').next().unwrap_or(task);
        if package.is_empty() || task.is_empty() {
            return;
        }
        let at = match failed.iter().position(|(known, _)| known == id) {
            Some(at) => at,
            None => {
                failed.push((
                    id.into(),
                    Failure {
                        package: package.into(),
                        task: task.into(),
                        tests: Vec::new(),
                    },
                ));
                failed.len() - 1
            }
        };
        if let Some(test) = test
            && !failed[at].1.tests.iter().any(|known| known == test)
        {
            failed[at].1.tests.push(test.into());
        }
    };
    for line in text.split('\n') {
        if let Some(summary) = SUMMARY.captures(line) {
            for id in summary[1].split(',') {
                add(&mut failed, id.trim(), None);
            }
        }
        if let Some(exited) = EXITED.captures(line) {
            add(&mut failed, &exited[1], None);
        }
    }
    let known: Vec<(String, String)> = failed
        .iter()
        .map(|(id, failure)| (id.clone(), failure.task.clone()))
        .collect();
    for line in text.split('\n') {
        let Some(prefixed) = PREFIXED.captures(line) else {
            continue;
        };
        let id = format!("{}#{}", &prefixed[1], &prefixed[2]);
        if !known.iter().any(|(known, task)| *known == id && task == "test") {
            continue;
        }
        let body = &prefixed[3];
        if let Some(vitest) = VITEST.captures(body) {
            add(&mut failed, &id, Some(&vitest[1]));
        } else if let Some(node) = NODE_TEST.captures(body)
            && &node[1] != "failing tests:"
        {
            add(&mut failed, &id, Some(&node[1]));
        }
    }
    failed.into_iter().map(|(_, failure)| failure).collect()
}

/// `@monocircuit/vita (frame.test.ts > keeps …, +2 more)`: what failed, in one line.
pub fn failures_text(failures: &[Failure]) -> String {
    failures_line(failures, 2)
}

/// [`failures_text`] with up to `count` tests of a package named (check.mjs
/// `failuresText(failures, count)`: wt:check names 5, its last line 1).
pub fn failures_line(failures: &[Failure], count: usize) -> String {
    failures
        .iter()
        .map(|failure| {
            if failure.task != "test" || failure.tests.is_empty() {
                return format!("{} {}", failure.package, failure.task);
            }
            let shown = failure.tests.iter().take(count).cloned().collect::<Vec<_>>().join(", ");
            let rest = if failure.tests.len() > count {
                format!(", +{} more", failure.tests.len() - count)
            } else {
                String::new()
            };
            format!("{} ({shown}{rest})", failure.package)
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SHA: &str = "1a2b3c4d5e6f";

    fn state(record: Value, since: Option<u64>, ahead: u64, alive: bool) -> CheckState {
        check_state_of(
            record.as_object().unwrap(),
            SHA,
            Some("t2"),
            |_| since,
            ahead,
            |_| alive,
        )
    }

    #[test]
    fn is_checking_while_wt_check_runs() {
        let record = json!({ "running": { "pid": 1, "since": "x" }, "passed": { "tree": "t2" } });
        assert_eq!(
            state(record.clone(), Some(2), 5, true),
            CheckState::Checking {
                since: Some("x".into())
            }
        );
        assert!(matches!(state(record, Some(2), 5, false), CheckState::Tested { .. }));
    }

    #[test]
    fn is_tested_or_failed_by_heads_tree() {
        assert_eq!(
            state(
                json!({ "passed": { "tree": "t2", "packages": ["a"] } }),
                Some(2),
                5,
                true
            ),
            CheckState::Tested {
                sha: SHA.into(),
                packages: vec!["a".into()]
            }
        );
        let record = json!({
            "passed": { "tree": "t1", "sha": "s1" },
            "failed": { "tree": "t2", "failures": [{ "package": "a", "task": "test", "tests": ["x"] }] },
        });
        assert_eq!(
            state(record, Some(2), 5, true),
            CheckState::Failed {
                sha: SHA.into(),
                failures: vec![Failure {
                    package: "a".into(),
                    task: "test".into(),
                    tests: vec!["x".into()]
                }]
            }
        );
    }

    #[test]
    fn counts_the_commits_since_the_last_pass_else_since_develop() {
        assert_eq!(
            state(json!({ "passed": { "tree": "t1", "sha": "s1" } }), Some(2), 5, true),
            CheckState::Untested { count: 2 }
        );
        assert_eq!(state(json!({}), Some(2), 5, true), CheckState::Untested { count: 5 });
        assert_eq!(
            state(json!({ "passed": { "tree": "t1", "sha": "s1" } }), None, 5, true),
            CheckState::Untested { count: 5 }
        );
        assert_eq!(state(json!({}), Some(2), 0, true), CheckState::None);
    }

    #[test]
    fn takes_the_failed_tasks_and_their_failing_tests() {
        let output = [
            "@monocircuit/vita:test:  FAIL  src/timeline/frame.test.ts > frame > keeps the edge",
            "@monocircuit/vita:test:  FAIL  src/timeline/frame.test.ts > frame > keeps the edge",
            "@monocircuit/vita:test:  FAIL  src/a.test.ts > b",
            "@monocircuit/breadboard:test: ✖ refuses a stale HEAD (1.2ms)",
            "@monocircuit/breadboard:test: ✖ failing tests:",
            "@monocircuit/breadboard:lint: ✖ 2 problems (1 error, 1 warning)",
            " Tasks:    3 successful, 6 total",
            "Failed:    @monocircuit/vita#test, @monocircuit/breadboard#test, @monocircuit/breadboard#lint",
        ]
        .join("\n");
        assert_eq!(
            failures_of(&output),
            vec![
                Failure {
                    package: "@monocircuit/vita".into(),
                    task: "test".into(),
                    tests: vec![
                        "src/timeline/frame.test.ts > frame > keeps the edge".into(),
                        "src/a.test.ts > b".into()
                    ]
                },
                Failure {
                    package: "@monocircuit/breadboard".into(),
                    task: "test".into(),
                    tests: vec!["refuses a stale HEAD".into()]
                },
                Failure {
                    package: "@monocircuit/breadboard".into(),
                    task: "lint".into(),
                    tests: vec![]
                },
            ]
        );
        let output =
            "\u{1b}[31m ERROR \u{1b}[0m @monocircuit/vita#typecheck: command (/x) pnpm run typecheck exited (2)";
        assert_eq!(
            failures_of(output),
            vec![Failure {
                package: "@monocircuit/vita".into(),
                task: "typecheck".into(),
                tests: vec![]
            }]
        );
    }

    #[test]
    fn puts_them_in_one_line() {
        let failures = vec![
            Failure {
                package: "@monocircuit/vita".into(),
                task: "test".into(),
                tests: vec!["a > 1".into(), "a > 2".into(), "a > 3".into()],
            },
            Failure {
                package: "@monocircuit/breadboard".into(),
                task: "lint".into(),
                tests: vec![],
            },
        ];
        assert_eq!(
            failures_text(&failures),
            "@monocircuit/vita (a > 1, a > 2, +1 more) · @monocircuit/breadboard lint"
        );
    }
}
