//! `pnpm wt:check` (#298, commands/check.mjs, check.mjs): lint, typecheck and
//! test of every package the branch changes against develop and of those
//! depending on them, through turbo; on success the tree it tested is
//! stamped in `.claude/checked.json` (`{ passed, failed, running }`), which
//! the pre-push hook reads: it lets a commit of that tree through. The
//! stamp's format stays (rule 5): Node's hook and overview read it.

use super::proc::{Stream, fail, git, outln, step, write_bytes};
use super::repo;
use breadboard_core::time::{now_ms, to_iso};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

/// What wt:check runs, through turbo.
const TASKS: [&str; 3] = ["lint", "typecheck", "test"];

fn ci_only(package: &str) -> Option<&'static str> {
    (package == "@monocircuit/vita")
        .then_some("vita's integration tests (test:int) need the stack: they run in CI only (check:vita:int)")
}

pub fn check_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("checked.json")
}

pub fn read_check(dir: &Path) -> Map<String, Value> {
    breadboard_core::json::read(&check_file(dir))
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

/// Sets the given parts (a null clears one), keeps the others; 2-space JSON as Node writes it.
pub fn write_check(dir: &Path, parts: Value) {
    let mut record = read_check(dir);
    if let Value::Object(parts) = parts {
        for (key, value) in parts {
            record.insert(key, value);
        }
    }
    record.retain(|_, value| !value.is_null());
    let _ = breadboard_core::json::write_pretty(&check_file(dir), &Value::Object(record));
}

/// The tree of the working copy as a commit of everything would have it, built in a copy of the index.
pub fn working_tree_of(dir: &Path) -> Option<String> {
    let real = git(&["rev-parse", "--path-format=absolute", "--git-path", "index"], dir)
        .text()
        .to_string();
    let copy = std::env::temp_dir().join(format!("breadboard-check-{}-{}", std::process::id(), now_ms()));
    if !real.is_empty() && Path::new(&real).exists() {
        let _ = fs::copy(&real, &copy);
    }
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_INDEX_FILE", &copy)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
    };
    let tree = match run(&["add", "-A"]) {
        Ok(added) if added.status.success() => match run(&["write-tree"]) {
            Ok(tree) if tree.status.success() => Some(String::from_utf8_lossy(&tree.stdout).trim().to_string()),
            _ => None,
        },
        _ => None,
    };
    let _ = fs::remove_file(&copy);
    tree
}

/// The turbo of the worktree, else the main checkout's.
pub fn turbo_of(dir: &Path) -> Option<PathBuf> {
    [dir.to_path_buf(), repo::main_dir()]
        .into_iter()
        .map(|root| root.join("node_modules/.bin/turbo"))
        .find(|file| file.exists())
}

/// The packages turbo would check, from its dry run as JSON: `(package, tasks)`, sorted.
pub fn planned_of(dry_run: &Value) -> Vec<(String, Vec<String>)> {
    let mut planned: Vec<(String, Vec<String>)> = Vec::new();
    for task in dry_run
        .get("tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let name = task.get("task").and_then(Value::as_str).unwrap_or_default();
        let package = task.get("package").and_then(Value::as_str).unwrap_or_default();
        let command = task.get("command").and_then(Value::as_str).unwrap_or_default();
        if !TASKS.contains(&name) || package == "//" || command == "<NONEXISTENT>" {
            continue;
        }
        match planned.iter_mut().find(|(known, _)| known == package) {
            Some((_, tasks)) => tasks.push(name.to_string()),
            None => planned.push((package.to_string(), vec![name.to_string()])),
        }
    }
    planned.sort_by(|a, b| a.0.cmp(&b.0));
    planned
}

/// What turbo would check for `filter`, or why it can't tell.
pub fn plan_of(turbo: &Path, dir: &Path, filter: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut args: Vec<String> = vec!["run".into()];
    args.extend(TASKS.iter().map(|task| task.to_string()));
    args.extend([format!("--filter={filter}"), "--dry-run=json".into()]);
    let output = Command::new(turbo)
        .args(&args)
        .current_dir(dir)
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("turbo can't tell which packages changed: {error}"))?;
    match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(dry_run) => Ok(planned_of(&dry_run)),
        Err(_) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let text = if stderr.trim().is_empty() { stdout } else { stderr };
            let why = text.trim().lines().last().unwrap_or_default().to_string();
            Err(format!(
                "turbo can't tell which packages changed{}",
                if why.is_empty() {
                    String::new()
                } else {
                    format!(": {why}")
                }
            ))
        }
    }
}

/// A task that failed, with its failing tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub package: String,
    pub task: String,
    pub tests: Vec<String>,
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());
static SUMMARY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*Failed:\s+(.+)$").unwrap());
static EXITED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)(\S+#\S+): command .* exited \(\d+\)").unwrap());
static PREFIXED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\S+?):(\w[\w:-]*): (.*)$").unwrap());
static VITEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*FAIL\s+(\S+\.\w+ > .+?)\s*$").unwrap());
static NODE_TEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*✖ (.+?)(?: \([\d.]+m?s\))?\s*$").unwrap());

/// What failed in turbo's output (check.mjs `failuresOf`).
pub fn failures_of(output: &str) -> Vec<Failure> {
    let text = ANSI.replace_all(output, "");
    let mut failed: Vec<Failure> = Vec::new();
    let add = |id: &str, test: Option<&str>, failed: &mut Vec<Failure>| {
        let Some((package, task)) = id.split_once('#') else {
            return;
        };
        let task = task.split('#').next().unwrap_or(task);
        if package.is_empty() || task.is_empty() {
            return;
        }
        let at = match failed
            .iter()
            .position(|known| known.package == package && known.task == task)
        {
            Some(at) => at,
            None => {
                failed.push(Failure {
                    package: package.into(),
                    task: task.into(),
                    tests: Vec::new(),
                });
                failed.len() - 1
            }
        };
        if let Some(test) = test
            && !failed[at].tests.iter().any(|known| known == test)
        {
            failed[at].tests.push(test.to_string());
        }
    };
    for line in text.split('\n') {
        if let Some(summary) = SUMMARY.captures(line) {
            for id in summary[1].split(',') {
                add(id.trim(), None, &mut failed);
            }
        }
        if let Some(exited) = EXITED.captures(line) {
            add(&exited[1], None, &mut failed);
        }
    }
    for line in text.split('\n') {
        let Some(prefixed) = PREFIXED.captures(line) else {
            continue;
        };
        let id = format!("{}#{}", &prefixed[1], &prefixed[2]);
        let is_test = failed
            .iter()
            .any(|known| format!("{}#{}", known.package, known.task) == id && known.task == "test");
        if !is_test {
            continue;
        }
        let body = &prefixed[3];
        if let Some(vitest) = VITEST.captures(body) {
            add(&id, Some(&vitest[1]), &mut failed);
        } else if let Some(node) = NODE_TEST.captures(body).filter(|node| &node[1] != "failing tests:") {
            add(&id, Some(&node[1]), &mut failed);
        }
    }
    failed
}

/// `@monocircuit/vita (frame.test.ts > keeps …, +2 more)`: what failed, in one line.
pub fn failures_text(failures: &[Failure], count: usize) -> String {
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

/// turbo, its output shown as it comes and kept: `(code, output)`.
fn run_turbo(turbo: &Path, args: &[String], dir: &Path) -> (Option<i32>, String) {
    let child = Command::new(turbo)
        .args(args)
        .current_dir(dir)
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return (None, String::new());
    };
    let kept = Arc::new(Mutex::new(String::new()));
    let pump = |stream: Box<dyn Read + Send>, is_err: bool| {
        let kept = Arc::clone(&kept);
        thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = Vec::new();
            while reader.read_until(b'\n', &mut line).is_ok_and(|read| read > 0) {
                write_bytes(if is_err { Stream::Err } else { Stream::Out }, &line);
                if let Ok(mut kept) = kept.lock() {
                    kept.push_str(&String::from_utf8_lossy(&line));
                }
                line.clear();
            }
        })
    };
    let out = child.stdout.take().map(|stream| pump(Box::new(stream), false));
    let err = child.stderr.take().map(|stream| pump(Box::new(stream), true));
    let status = child.wait().ok().and_then(|status| status.code());
    for handle in [out, err].into_iter().flatten() {
        let _ = handle.join();
    }
    let output = kept.lock().map(|kept| kept.clone()).unwrap_or_default();
    (status, output)
}

static STOPPED: AtomicBool = AtomicBool::new(false);
static RUNNING_IN: Mutex<Option<PathBuf>> = Mutex::new(None);

extern "C" fn on_signal(_: libc::c_int) {
    STOPPED.store(true, Ordering::SeqCst);
}

/// SIGINT/SIGTERM while turbo runs: `running` cleared, exit 130 (turbo gets the signal too).
fn watch_signals() {
    // SAFETY: the handler only stores into an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    thread::spawn(|| {
        loop {
            if STOPPED.load(Ordering::SeqCst) {
                if let Some(dir) = RUNNING_IN.lock().ok().and_then(|dir| dir.clone()) {
                    write_check(&dir, json!({ "running": null }));
                }
                std::process::exit(130);
            }
            thread::sleep(std::time::Duration::from_millis(50));
        }
    });
}

pub fn command() -> ExitCode {
    let dir = git(&["rev-parse", "--show-toplevel"], &repo::cwd()).text().to_string();
    if dir.is_empty() {
        fail("wt:check runs in a checkout of the repo");
    }
    let dir = PathBuf::from(dir);
    let Some(turbo) = turbo_of(&dir) else {
        fail("turbo isn't installed here: pnpm install first")
    };
    let base = git(&["merge-base", "origin/develop", "HEAD"], &dir).text().to_string();
    if base.is_empty() {
        fail("no merge base with origin/develop: git fetch origin first");
    }
    super::heal_plugin();
    let sha = git(&["rev-parse", "HEAD"], &dir).text().to_string();
    let Some(tree) = working_tree_of(&dir) else {
        fail("couldn't read the working tree (git add -A in a copy of the index failed)")
    };
    let filter = format!("...[{base}]");
    let planned = plan_of(&turbo, &dir, &filter).unwrap_or_else(|error| fail(&error));
    let started = now_ms();
    let packages: Vec<String> = planned.iter().map(|(package, _)| package.clone()).collect();
    let short = |text: &str| text.chars().take(8).collect::<String>();
    if planned.is_empty() {
        write_check(
            &dir,
            json!({
                "passed": { "sha": sha, "tree": tree, "packages": packages, "at": to_iso(now_ms()), "duration": 0 },
                "failed": null, "running": null,
            }),
        );
        outln!(
            "\n✓ nothing to check: the branch changes no package against develop ({}); {} may be pushed\n",
            short(&base),
            short(&sha)
        );
        return ExitCode::SUCCESS;
    }
    step(&format!(
        "{} of {} package{}, changed against develop ({}) or depending on one:",
        TASKS.join(", "),
        planned.len(),
        if planned.len() == 1 { "" } else { "s" },
        short(&base)
    ));
    for (package, tasks) in &planned {
        outln!("  {package}  {}", tasks.join(" "));
    }
    for package in &packages {
        if let Some(why) = ci_only(package) {
            outln!("  ({why})");
        }
    }
    outln!();
    write_check(
        &dir,
        json!({ "running": { "sha": sha, "tree": tree, "pid": std::process::id(), "since": to_iso(now_ms()) } }),
    );
    if let Ok(mut running) = RUNNING_IN.lock() {
        *running = Some(dir.clone());
    }
    watch_signals();
    let mut args: Vec<String> = vec!["run".into()];
    args.extend(TASKS.iter().map(|task| task.to_string()));
    args.extend([
        format!("--filter={filter}"),
        "--continue".into(),
        "--output-logs=errors-only".into(),
        "--log-order=grouped".into(),
    ]);
    let (code, output) = run_turbo(&turbo, &args, &dir);
    let duration = ((now_ms() - started) as f64 / 1000.0).round() as i64;
    let at = to_iso(now_ms());
    if code != Some(0) {
        let failures = failures_of(&output);
        let listed: Vec<Value> = failures
            .iter()
            .map(|failure| json!({ "package": failure.package, "task": failure.task, "tests": failure.tests }))
            .collect();
        write_check(
            &dir,
            json!({
                "running": null,
                "failed": { "sha": sha, "tree": tree, "packages": packages, "failures": listed, "at": at, "duration": duration },
            }),
        );
        let what = if failures.is_empty() {
            "see turbo's output above".into()
        } else {
            failures_text(&failures, 5)
        };
        fail(&format!(
            "wt:check failed after {duration} s: {what}\n  Fix it, commit, run pnpm wt:check again: the push waits for it.\n  Red on develop too: it is fixed once, in its own fix MR; wait for that with pnpm wt:wait !<mr>."
        ));
    }
    if working_tree_of(&dir).as_deref() != Some(tree.as_str()) {
        write_check(&dir, json!({ "running": null }));
        fail("the files changed while wt:check ran: what passed isn't what is there now. Run it again.");
    }
    write_check(
        &dir,
        json!({
            "running": null, "failed": null,
            "passed": { "sha": sha, "tree": tree, "packages": packages, "at": at, "duration": duration },
        }),
    );
    let head = git(&["rev-parse", "HEAD"], &dir).text().to_string();
    let is_dirty = !git(&["status", "--porcelain"], &dir).text().is_empty();
    let next = if is_dirty {
        "commit these changes as they are, then push: the commit is stamped as checked".to_string()
    } else {
        format!("{} may be pushed", short(&head))
    };
    outln!("\n✓ checked in {duration} s: {}\n  {next}\n", packages.join(", "));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_failed() {
        let output = "\x1b[31m@m/vita:test: FAIL  src/a.test.ts > keeps it\x1b[0m\n@m/vita:test: ✖ other (12ms)\n@m/vita:test: ✖ failing tests:\n Failed:  @m/vita#test, @m/ui#lint\n";
        let failures = failures_of(output);
        assert_eq!(
            failures,
            vec![
                Failure {
                    package: "@m/vita".into(),
                    task: "test".into(),
                    tests: vec!["src/a.test.ts > keeps it".into(), "other".into()]
                },
                Failure {
                    package: "@m/ui".into(),
                    task: "lint".into(),
                    tests: vec![]
                },
            ]
        );
        assert_eq!(
            failures_text(&failures, 1),
            "@m/vita (src/a.test.ts > keeps it, +1 more) · @m/ui lint"
        );
    }

    #[test]
    fn plans_from_turbos_dry_run() {
        let dry = json!({ "tasks": [
            { "task": "test", "package": "b", "command": "vitest" },
            { "task": "lint", "package": "a", "command": "eslint" },
            { "task": "test", "package": "a", "command": "<NONEXISTENT>" },
            { "task": "lint", "package": "//", "command": "x" },
            { "task": "build", "package": "a", "command": "x" },
        ] });
        assert_eq!(
            planned_of(&dry),
            vec![
                ("a".to_string(), vec!["lint".to_string()]),
                ("b".to_string(), vec!["test".to_string()])
            ]
        );
    }
}
