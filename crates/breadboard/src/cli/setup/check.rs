//! `pnpm wt:check` (#298, commands/check.mjs, check.mjs): lint, typecheck and
//! test of every package the branch changes against develop and of those
//! depending on them, through turbo; on success the tree it tested is
//! stamped in `.claude/checked.json` (`{ passed, failed, running }`), which
//! the pre-push hook reads: it lets a commit of that tree through. The
//! stamp's format stays (rule 5): Node's hook and overview read it.

use super::proc::{Stream, fail, git, outln, step, write_bytes};
use super::repo;
use breadboard_core::check::{self, failures_line, failures_of};
pub use breadboard_core::check::{Failure, read_check};
use breadboard_core::time::{now_ms, to_iso};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// What wt:check runs, through turbo.
const TASKS: [&str; 3] = ["lint", "typecheck", "test"];
/// breadboard's cargo checks (#336): only when the branch changes its Rust
/// sources, never because develop was merged in or a package it depends on
/// changed. They run through herdr/cargo.sh, in the clone's shared target/.
const RUST_TASKS: [&str; 2] = ["lint:rs", "test:rs"];
/// The Rust sources of breadboard, as the launcher keys its builds on them
/// (herdr/breadboard.sh `sources`: the crates and what they embed).
const RUST_SOURCES: [&str; 11] = [
    "packages/breadboard/crates",
    "packages/breadboard/hooks",
    "packages/breadboard/claude",
    "packages/breadboard/herdr/herdr-plugin.toml",
    "packages/breadboard/herdr/keys.toml",
    "packages/breadboard/herdr/config.toml",
    "packages/breadboard/herdr/breadboard.sh",
    "packages/breadboard/nvim",
    "packages/breadboard/chromium",
    "packages/breadboard/Cargo.toml",
    "packages/breadboard/Cargo.lock",
];

/// What a check runs: turbo's tasks and the filters of the packages they run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub tasks: Vec<String>,
    pub filters: Vec<String>,
}

/// The `--filter`s of the packages a change affects, from turbo's
/// `affectedPackages` and its package paths (`query`'s answer). Not turbo's
/// `...[<base>]`: the root may depend on a package (#299), and turbo
/// counts a change in a package the root depends on as a change of every
/// package (`RootInternalDepChanged`), so any vita change ran breadboard's
/// cargo checks (#336). Such a package counts as itself and the packages in
/// its directory with their dependents; the root (`//`) is no package to check.
pub fn filters_of(answer: &Value) -> Vec<String> {
    let items = |path: &str| {
        answer
            .pointer(path)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let path_of = |name: &str| {
        items("/data/packages/items")
            .iter()
            .find(|package| package.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|package| package.get("path").and_then(Value::as_str).map(String::from))
    };
    let mut filters: Vec<String> = Vec::new();
    let mut add = |filter: String| {
        if !filters.contains(&filter) {
            filters.push(filter);
        }
    };
    for item in items("/data/affectedPackages/items") {
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        let reason = item.get("reason");
        let kind = reason
            .and_then(|reason| reason.get("__typename"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "RootInternalDepChanged" {
            let dep = reason
                .and_then(|reason| reason.get("rootInternalDep"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            match path_of(dep) {
                Some(path) if !path.is_empty() => {
                    add(format!("{{./{path}}}"));
                    add(format!("...{{./{path}/*}}"));
                }
                _ if !dep.is_empty() => add(dep.to_string()),
                _ => {}
            }
        } else if !name.is_empty() && name != "//" {
            add(name.to_string());
        }
    }
    filters
}

/// The query of what changed between `base` and `head` (the working tree without one).
fn affected_query(base: &str, head: Option<&str>) -> String {
    let head = head.map(|head| format!(", head: \"{head}\"")).unwrap_or_default();
    format!(
        "query {{ affectedPackages(base: \"{base}\"{head}) {{ items {{ name reason {{ __typename ... on RootInternalDepChanged {{ rootInternalDep }} }} }} }} packages {{ items {{ name path }} }} }}"
    )
}

/// Whether the change from `base` to `head` (the working tree, untracked
/// files too, without one) touches breadboard's Rust sources.
pub fn is_rust_changed(dir: &Path, base: &str, head: Option<&str>) -> bool {
    let mut diff = vec!["diff", "--name-only", base];
    if let Some(head) = head {
        diff.push(head);
    }
    diff.push("--");
    diff.extend(RUST_SOURCES);
    if !git(&diff, dir).text().is_empty() {
        return true;
    }
    if head.is_some() {
        return false;
    }
    let mut untracked = vec!["ls-files", "--others", "--exclude-standard", "--"];
    untracked.extend(RUST_SOURCES);
    !git(&untracked, dir).text().is_empty()
}

/// What a check of the change from `base` to `head` runs (#336): the
/// packages turbo says it affects, breadboard's cargo checks only with its
/// Rust sources changed.
pub fn scope_of(turbo: &Path, dir: &Path, base: &str, head: Option<&str>) -> Result<Scope, String> {
    let output = Command::new(turbo)
        .args(["query", &affected_query(base, head)])
        .current_dir(dir)
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("turbo can't tell which packages changed: {error}"))?;
    let answer: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let why = stderr.trim().lines().last().unwrap_or_default().to_string();
        format!(
            "turbo can't tell which packages changed{}",
            if why.is_empty() {
                String::new()
            } else {
                format!(": {why}")
            }
        )
    })?;
    if answer.get("errors").is_some_and(|errors| !errors.is_null()) {
        return Err(format!("turbo can't tell which packages changed: {}", answer["errors"]));
    }
    let mut tasks: Vec<String> = TASKS.iter().map(|task| task.to_string()).collect();
    if is_rust_changed(dir, base, head) {
        tasks.extend(RUST_TASKS.iter().map(|task| task.to_string()));
    }
    Ok(Scope {
        tasks,
        filters: filters_of(&answer),
    })
}

/// What breadboard.toml says of a package whose tests need the stack (`[[check.stack_tests]]`).
fn ci_only(dir: &Path, package: &str) -> Option<String> {
    breadboard_core::config::of(dir)
        .config
        .check
        .stack_tests
        .iter()
        .find(|tests| tests.package == package)
        .map(|tests| tests.says.clone())
}

/// breadboard's own package in the repo it is developed in, by the name in its package.json.
fn breadboard_package(dir: &Path) -> Option<String> {
    let package = breadboard_core::json::read(&dir.join("packages/breadboard/package.json"))?;
    breadboard_core::json::str_of(&package, "name").map(String::from)
}

/// wt:check without turbo: `[check] run` once over the branch, stamped like
/// turbo's; without it nothing to check, stamped at once.
fn plain_check(dir: &Path, sha: &str, tree: &str) -> ExitCode {
    let loaded = breadboard_core::config::of(dir);
    let short: String = sha.chars().take(8).collect();
    let Some(run) = loaded.config.check.plain().map(String::from) else {
        write_check(
            dir,
            json!({
                "passed": { "sha": sha, "tree": tree, "packages": [], "at": to_iso(now_ms()), "duration": 0 },
                "failed": null, "running": null,
            }),
        );
        outln!("\n✓ nothing to check: breadboard.toml names no [check]; {short} may be pushed\n");
        return ExitCode::SUCCESS;
    };
    step(&format!("{run} (breadboard.toml's [check] run)"));
    write_check(
        dir,
        json!({ "running": { "sha": sha, "tree": tree, "pid": std::process::id(), "since": to_iso(now_ms()) } }),
    );
    let started = now_ms();
    let status = Command::new("sh").args(["-c", &run]).current_dir(dir).status();
    let duration = ((now_ms() - started) as f64 / 1000.0).round() as i64;
    let at = to_iso(now_ms());
    if !status.is_ok_and(|status| status.success()) {
        write_check(
            dir,
            json!({
                "running": null,
                "failed": { "sha": sha, "tree": tree, "packages": [], "failures": [], "at": at, "duration": duration },
            }),
        );
        fail(&format!(
            "wt:check failed after {duration} s: {run}\n  Fix it, commit, run {} again: the push waits for it.",
            loaded.command("wt:check")
        ));
    }
    if working_tree_of(dir).as_deref() != Some(tree) {
        write_check(dir, json!({ "running": null }));
        fail("the files changed while wt:check ran: what passed isn't what is there now. Run it again.");
    }
    write_check(
        dir,
        json!({
            "running": null, "failed": null,
            "passed": { "sha": sha, "tree": tree, "packages": [], "at": at, "duration": duration },
        }),
    );
    outln!("\n✓ checked in {duration} s: {run}\n  {short} may be pushed\n");
    ExitCode::SUCCESS
}

/// core's `write_check` (`.claude/checked.json`, one reader and writer for
/// wt:check, the pre-push hook and the overview) with the parts as JSON:
/// a null clears one, the others stay.
pub fn write_check(dir: &Path, parts: Value) {
    let _ = check::write_check(dir, parts.as_object().cloned().unwrap_or_default());
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

/// The packages turbo would check, from its dry run as JSON: `(package,
/// tasks)` of the `tasks` asked for, sorted.
pub fn planned_of(dry_run: &Value, tasks: &[String]) -> Vec<(String, Vec<String>)> {
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
        if !tasks.iter().any(|task| task == name) || package == "//" || command == "<NONEXISTENT>" {
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

/// turbo's `run <tasks> --filter=…` of a scope.
fn run_args(scope: &Scope) -> Vec<String> {
    let mut args: Vec<String> = vec!["run".into()];
    args.extend(scope.tasks.iter().cloned());
    args.extend(scope.filters.iter().map(|filter| format!("--filter={filter}")));
    args
}

/// What turbo would check of `scope`, or why it can't tell. Nothing affected: nothing.
pub fn plan_of(turbo: &Path, dir: &Path, scope: &Scope) -> Result<Vec<(String, Vec<String>)>, String> {
    if scope.filters.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = run_args(scope);
    args.push("--dry-run=json".into());
    let output = Command::new(turbo)
        .args(&args)
        .current_dir(dir)
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("turbo can't tell which packages changed: {error}"))?;
    match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(dry_run) => Ok(planned_of(&dry_run, &scope.tasks)),
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

/// How turbo ended: its exit code, or the signal that ended it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    Code(i32),
    Signal(i32),
    /// It didn't start, or its end couldn't be read.
    Unknown,
}

/// turbo, its output shown as it comes and kept. At low priority (#336):
/// an agent's check never freezes the machine the others work on.
fn run_turbo(turbo: &Path, args: &[String], dir: &Path) -> (Ended, String) {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    let mut command = Command::new(turbo);
    command
        .args(args)
        .current_dir(dir)
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: nice(2) is async-signal-safe; its result (the new priority, or -1) doesn't matter.
    unsafe {
        command.pre_exec(|| {
            libc::nice(10);
            Ok(())
        });
    }
    let child = command.spawn();
    let Ok(mut child) = child else {
        return (Ended::Unknown, String::new());
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
    let ended = match child.wait() {
        Ok(status) => match (status.code(), status.signal()) {
            (Some(code), _) => Ended::Code(code),
            (None, Some(signal)) => Ended::Signal(signal),
            _ => Ended::Unknown,
        },
        Err(_) => Ended::Unknown,
    };
    for handle in [out, err].into_iter().flatten() {
        let _ = handle.join();
    }
    let output = kept.lock().map(|kept| kept.clone()).unwrap_or_default();
    (ended, output)
}

/// Whether a run was stopped rather than judged: Ctrl-C reaches turbo and
/// this process together, so turbo's end then says nothing about the code
/// (it dies of the signal, or exits non-zero on its way out). A stop is no
/// failure: nothing is stamped (spec §4, `commands/check.mjs` `stop`).
fn was_stopped(is_stopped: bool, ended: Ended) -> bool {
    is_stopped || matches!(ended, Ended::Signal(libc::SIGINT | libc::SIGTERM))
}

static STOPPED: AtomicBool = AtomicBool::new(false);
static RUNNING_IN: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Held while the stamp is judged and written: a stop never cuts a write in
/// half, and one seen after turbo's end is never written as a failure.
static STAMPING: Mutex<()> = Mutex::new(());

extern "C" fn on_signal(_: libc::c_int) {
    STOPPED.store(true, Ordering::SeqCst);
}

/// The stopped run's end: `running` cleared, exit 130.
fn stop() -> ! {
    if let Some(dir) = RUNNING_IN.lock().ok().and_then(|dir| dir.clone()) {
        write_check(&dir, json!({ "running": null }));
    }
    std::process::exit(130);
}

/// SIGINT/SIGTERM while turbo runs: `running` cleared, exit 130, also when
/// turbo hasn't ended yet (it gets Ctrl-C too, and a SIGTERM sent to this
/// process alone leaves it to end on its own, as today's).
fn watch_signals() {
    // SAFETY: the handler only stores into an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    thread::spawn(|| {
        loop {
            if STOPPED.load(Ordering::SeqCst) {
                let _stamping = STAMPING.lock();
                stop();
            }
            thread::sleep(std::time::Duration::from_millis(50));
        }
    });
}

/// drizzle's journal in the checkout, as breadboard.toml's `[stack.migrations]` places it.
fn journal_of(dir: &Path) -> Option<String> {
    let stack = breadboard_core::stack::of(dir)?;
    let journal = Path::new(&stack.dir).join(&stack.migrations?.journal);
    Some(journal.to_string_lossy().trim_start_matches("./").to_string())
}

/// A journal's migrations: `(tag, when)`.
fn journal_entries(text: &str) -> Vec<(String, i64)> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|journal| journal.get("entries")?.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| Some((entry.get("tag")?.as_str()?.to_string(), entry.get("when")?.as_i64()?)))
        .collect()
}

/// The branch's own migrations made before the base's newest: drizzle skips a
/// migration older than the last one applied, so after the merge it never
/// runs. `ours` and `base` are the journals (#351).
pub fn older_migrations(ours: &str, base: &str) -> Option<String> {
    let base = journal_entries(base);
    let (newest, at) = base.iter().max_by_key(|(_, when)| *when)?;
    let stale: Vec<String> = journal_entries(ours)
        .into_iter()
        .filter(|(tag, when)| !base.iter().any(|(known, _)| known == tag) && when <= at)
        .map(|(tag, _)| tag)
        .collect();
    (!stale.is_empty()).then(|| {
        format!(
            "the migration {} is older than {newest} of the base, drizzle would skip it once merged: delete it, merge the base in and run pnpm pg:generate again (skill db-migration)",
            stale.join(", ")
        )
    })
}

/// The refusal of `older_migrations` for the working tree against `base`'s tip.
fn stale_migrations(dir: &Path, base: &str) -> Option<String> {
    let journal = journal_of(dir)?;
    let ours = fs::read_to_string(dir.join(&journal)).ok()?;
    let theirs = git(&["show", &format!("{base}:{journal}")], dir);
    older_migrations(&ours, theirs.is_ok.then_some(theirs.out.as_str())?)
}

pub fn command() -> ExitCode {
    let dir = git(&["rev-parse", "--show-toplevel"], &repo::cwd()).text().to_string();
    if dir.is_empty() {
        fail("wt:check runs in a checkout of the repo");
    }
    let dir = PathBuf::from(dir);
    let turbo = turbo_of(&dir);
    if turbo.is_none() && breadboard_core::config::of(&dir).config.check.turbo {
        fail("turbo isn't installed here: pnpm install first")
    };
    let against = breadboard_core::git::base_of(&dir);
    let base = git(&["merge-base", &against.remote, "HEAD"], &dir).text().to_string();
    if base.is_empty() {
        fail(&format!(
            "no merge base with {}: git fetch origin first",
            against.remote
        ));
    }
    // develop's, and for an orchestration's task its epic's: what the checkout of the epic holds (#351).
    let epic =
        super::epic::Epic::of_task(&dir).map(|epic| git(&["rev-parse", "HEAD"], &epic.checkout).text().to_string());
    for base in std::iter::once(breadboard_core::git::Base::of(&breadboard_core::git::branch(&dir)).remote)
        .chain(epic.filter(|head| !head.is_empty()))
    {
        if let Some(refusal) = stale_migrations(&dir, &base) {
            fail(&refusal);
        }
    }
    super::heal_plugin();
    let sha = git(&["rev-parse", "HEAD"], &dir).text().to_string();
    let Some(tree) = working_tree_of(&dir) else {
        fail("couldn't read the working tree (git add -A in a copy of the index failed)")
    };
    let Some(turbo) = turbo else {
        return plain_check(&dir, &sha, &tree);
    };
    let scope = scope_of(&turbo, &dir, &base, None).unwrap_or_else(|error| fail(&error));
    let planned = plan_of(&turbo, &dir, &scope).unwrap_or_else(|error| fail(&error));
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
            "\n✓ nothing to check: the branch changes no package against {} ({}); {} may be pushed\n",
            against.local,
            short(&base),
            short(&sha)
        );
        return ExitCode::SUCCESS;
    }
    step(&format!(
        "{} of {} package{}, changed against {} ({}) or depending on one:",
        scope.tasks.join(", "),
        planned.len(),
        if planned.len() == 1 { "" } else { "s" },
        against.local,
        short(&base)
    ));
    for (package, tasks) in &planned {
        outln!("  {package}  {}", tasks.join(" "));
    }
    for package in &packages {
        if let Some(why) = ci_only(&dir, package) {
            outln!("  ({why})");
        }
    }
    if breadboard_package(&dir).is_some_and(|own| packages.contains(&own))
        && !scope.tasks.iter().any(|task| task.ends_with(":rs"))
    {
        outln!("  (breadboard's cargo checks: only when the branch changes its Rust sources, CI runs them anyway)");
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
    let mut args = run_args(&scope);
    args.extend([
        "--continue".into(),
        "--output-logs=errors-only".into(),
        "--log-order=grouped".into(),
    ]);
    let (ended, output) = run_turbo(&turbo, &args, &dir);
    let _stamping = STAMPING.lock();
    if was_stopped(STOPPED.load(Ordering::SeqCst), ended) {
        stop();
    }
    let duration = ((now_ms() - started) as f64 / 1000.0).round() as i64;
    let at = to_iso(now_ms());
    if ended != Ended::Code(0) {
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
            failures_line(&failures, 5)
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
    fn refuses_a_migration_older_than_the_base_s_newest() {
        let journal = |entries: &[(&str, i64)]| {
            let entries: Vec<Value> = entries
                .iter()
                .map(|(tag, when)| json!({ "tag": tag, "when": when }))
                .collect();
            json!({ "entries": entries }).to_string()
        };
        let base = journal(&[("0000_a", 1), ("0001_epic", 5)]);
        // Made after the base's newest: fine.
        assert_eq!(
            older_migrations(&journal(&[("0000_a", 1), ("0001_mine", 6)]), &base),
            None
        );
        // Made before it, on a base that didn't have it yet: drizzle would skip it.
        assert_eq!(
            older_migrations(&journal(&[("0000_a", 1), ("0001_epic", 5), ("0002_mine", 4)]), &base).as_deref(),
            Some(
                "the migration 0002_mine is older than 0001_epic of the base, drizzle would skip it once merged: delete it, merge the base in and run pnpm pg:generate again (skill db-migration)"
            )
        );
        // No journal on the base, or none here: nothing to say.
        assert_eq!(older_migrations(&base, "not json"), None);
    }

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
            failures_line(&failures, 1),
            "@m/vita (src/a.test.ts > keeps it, +1 more) · @m/ui lint"
        );
    }

    #[test]
    fn a_stopped_run_is_no_failure() {
        // Ctrl-C: turbo dies of the same SIGINT, often before this process's watcher looks.
        assert!(was_stopped(false, Ended::Signal(libc::SIGINT)));
        assert!(was_stopped(false, Ended::Signal(libc::SIGTERM)));
        // turbo caught it and exited non-zero: the flag decides.
        assert!(was_stopped(true, Ended::Code(1)));
        assert!(was_stopped(true, Ended::Code(0)));
        assert!(!was_stopped(false, Ended::Code(1)));
        assert!(!was_stopped(false, Ended::Signal(libc::SIGKILL)));
        assert!(!was_stopped(false, Ended::Unknown));
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
        let tasks: Vec<String> = TASKS.iter().map(|task| task.to_string()).collect();
        assert_eq!(
            planned_of(&dry, &tasks),
            vec![
                ("a".to_string(), vec!["lint".to_string()]),
                ("b".to_string(), vec!["test".to_string()])
            ]
        );
    }

    /// turbo's answer to `affected_query`: `(name, reason, root dependency)`.
    fn answer(items: &[(&str, &str, &str)]) -> Value {
        let affected: Vec<Value> = items
            .iter()
            .map(|(name, kind, dep)| {
                let mut reason = json!({ "__typename": kind });
                if !dep.is_empty() {
                    reason["rootInternalDep"] = json!(dep);
                }
                json!({ "name": name, "reason": reason })
            })
            .collect();
        json!({ "data": {
            "affectedPackages": { "items": affected },
            "packages": { "items": [
                { "name": "//", "path": "" },
                { "name": "@m/breadboard", "path": "packages/breadboard" },
                { "name": "@m/vita", "path": "products/vita" },
                { "name": "@m/vita-ui", "path": "products/vita/ui" },
            ] },
        } })
    }

    // #336: the vita branches after a develop merge. turbo's own answer for
    // a change of the root's dependency @m/vita is every package, breadboard
    // too; the check takes vita and the packages in its directory.
    #[test]
    fn a_change_of_a_root_dependency_checks_that_package_not_everything() {
        let vita = answer(&[
            ("//", "RootInternalDepChanged", "@m/vita"),
            ("@m/breadboard", "RootInternalDepChanged", "@m/vita"),
            ("@m/vita", "RootInternalDepChanged", "@m/vita"),
            ("@m/vita-ui", "RootInternalDepChanged", "@m/vita"),
        ]);
        assert_eq!(filters_of(&vita), vec!["{./products/vita}", "...{./products/vita/*}"]);
        // A change inside vita-ui: vita-ui and what depends on it.
        let ui = answer(&[
            ("@m/vita-ui", "FileChanged", ""),
            ("@m/vita-web", "DependencyChanged", ""),
        ]);
        assert_eq!(filters_of(&ui), vec!["@m/vita-ui", "@m/vita-web"]);
        // The root alone (a CI file, a lockfile): nothing to check.
        assert!(filters_of(&answer(&[("//", "FileChanged", "")])).is_empty());
        // turbo.json: every package, as turbo says.
        assert_eq!(
            filters_of(&answer(&[
                ("//", "GlobalDepsChanged", ""),
                ("@m/breadboard", "GlobalDepsChanged", ""),
            ])),
            vec!["@m/breadboard"]
        );
    }

    #[test]
    fn asks_turbo_for_the_change_up_to_a_commit_or_the_working_tree() {
        assert!(affected_query("abc", Some("def")).contains("affectedPackages(base: \"abc\", head: \"def\")"));
        assert!(affected_query("abc", None).contains("affectedPackages(base: \"abc\")"));
        let scope = Scope {
            tasks: vec!["lint".into(), "lint:rs".into()],
            filters: vec!["@m/a".into(), "...{./p/*}".into()],
        };
        assert_eq!(
            run_args(&scope),
            vec!["run", "lint", "lint:rs", "--filter=@m/a", "--filter=...{./p/*}"]
        );
    }

    /// A repo with breadboard's Rust sources and a vita file, `origin/develop` at its first commit.
    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("check-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for file in [
            "packages/breadboard/crates/core/lib.rs",
            "products/vita/ui/a.ts",
            "README.md",
        ] {
            fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
            fs::write(dir.join(file), "1\n").unwrap();
        }
        let run = |args: &[&str]| {
            let done = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(&dir)
                .env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap())
                .output()
                .unwrap();
            assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
        };
        run(&["init", "--quiet", "--initial-branch=develop"]);
        run(&["add", "-A"]);
        run(&["commit", "--quiet", "-m", "base"]);
        run(&["update-ref", "refs/remotes/origin/develop", "HEAD"]);
        dir
    }

    fn commit_all(dir: &Path) -> String {
        for args in [vec!["add", "-A"], vec!["commit", "--quiet", "-m", "x"]] {
            let mut full = vec!["-c", "user.name=t", "-c", "user.email=t@t"];
            full.extend(args);
            assert!(git(&full, dir).is_ok);
        }
        git(&["rev-parse", "HEAD"], dir).text().to_string()
    }

    // #336: a vita branch runs no cargo, also after develop brought the crate.
    #[test]
    fn cargo_checks_only_when_the_rust_sources_change() {
        let dir = repo("rust");
        let base = git(&["rev-parse", "HEAD"], &dir).text().to_string();
        assert!(!is_rust_changed(&dir, &base, None));
        fs::write(dir.join("products/vita/ui/a.ts"), "2\n").unwrap();
        let vita = commit_all(&dir);
        assert!(!is_rust_changed(&dir, &base, Some(&vita)));
        assert!(!is_rust_changed(&dir, &base, None));
        // Uncommitted, untracked: the working tree counts.
        fs::write(dir.join("packages/breadboard/crates/core/new.rs"), "1\n").unwrap();
        assert!(is_rust_changed(&dir, &base, None));
        assert!(!is_rust_changed(&dir, &base, Some(&vita)));
        let rust = commit_all(&dir);
        assert!(is_rust_changed(&dir, &base, Some(&rust)));
        // develop moved on and was merged in: the merge base moves with it.
        assert!(!is_rust_changed(&dir, &rust, None));
        let _ = fs::remove_dir_all(&dir);
    }
}
