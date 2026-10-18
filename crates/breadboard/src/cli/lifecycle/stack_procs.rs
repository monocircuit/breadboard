//! A stack's processes without herdr tabs (#407): while a checkout's stack
//! runs, its app's dev server (`dev`) and the tools that come with the stack
//! (`studio`) run in the background, each under a supervisor of its own
//! (`breadboard supervise`, detached), which restarts it when it crashes and
//! ends it with the stack. Their output goes to `.claude/stack/<name>.log`,
//! which the detail pane's `l` follows; the services' logs it reads from
//! compose on demand. The supervisor's pid is in `.claude/stack/<name>.pid`:
//! the files are the state, so any process (`stack:*`, an overview, the next
//! build of breadboard) finds and ends what another one started.

use super::proc::exe;
use super::repo::{Repo, compose_args, env_file_of, offset_of, stack_dir};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The process of a checkout's app (`pnpm dev`).
pub const DEV: &str = "dev";

/// How long an ended process waits before it starts again, at most: each
/// crash in a row doubles the wait from a second.
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A process that ran this long crashed anew: its wait starts at a second again.
const RAN_WELL: Duration = Duration::from_secs(60);
/// How long a process gets to end after SIGTERM before SIGKILL.
const END_WAIT: Duration = Duration::from_secs(5);
/// How often the supervisor looks at its process and its pid file.
const LOOK_EVERY: Duration = Duration::from_millis(200);

/// The folder of a checkout's stack processes.
pub fn folder(dir: &Path) -> PathBuf {
    dir.join(".claude").join("stack")
}

/// The log of the process `name`.
pub fn log_of(dir: &Path, name: &str) -> PathBuf {
    folder(dir).join(format!("{name}.log"))
}

fn pid_file(dir: &Path, name: &str) -> PathBuf {
    folder(dir).join(format!("{name}.pid"))
}

/// The logs of a checkout's processes that exist, `dev` first.
pub fn logs_of(dir: &Path) -> Vec<PathBuf> {
    let mut logs: Vec<PathBuf> = fs::read_dir(folder(dir))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "log"))
                .collect()
        })
        .unwrap_or_default();
    logs.sort_by_key(|log| (log.file_stem().is_none_or(|stem| stem != DEV), log.clone()));
    logs
}

fn is_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

/// The supervisor of `name` while it runs.
pub fn running(dir: &Path, name: &str) -> Option<i32> {
    let pid: i32 = fs::read_to_string(pid_file(dir, name)).ok()?.trim().parse().ok()?;
    is_alive(pid).then_some(pid)
}

/// The processes a checkout's stack runs: `dev`, then each tool `with_stack`; with their port and command.
fn wanted(dir: &Path, offset: i64) -> Vec<(String, i64, String)> {
    let Some(stack) = breadboard_core::stack::of(dir) else {
        return Vec::new();
    };
    std::iter::once((
        DEV.to_string(),
        i64::from(stack.app.port) + offset,
        stack.app.dev.clone(),
    ))
    .chain(stack.tools.iter().filter(|tool| tool.is_with_stack).map(|tool| {
        (
            tool.name.clone(),
            i64::from(tool.port) + offset,
            tool.command(u16::try_from(offset).unwrap_or(0)),
        )
    }))
    .collect()
}

fn is_port_free(port: i64) -> bool {
    u16::try_from(port).is_ok_and(|port| TcpListener::bind(("127.0.0.1", port)).is_ok())
}

/// Whether the stack of `dir` has a container running.
fn is_up(dir: &Path) -> bool {
    let Some(all) = compose_args(dir, &["ps", "--status", "running", "-q"]) else {
        return false;
    };
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    !super::proc::output("docker", &all, &stack_dir(dir), None)
        .out()
        .is_empty()
}

/// `name` started under a supervisor of its own, detached: whether it runs.
pub fn start(dir: &Path, name: &str, command: &str) -> bool {
    let _ = fs::create_dir_all(folder(dir));
    let cwd = stack_dir(dir);
    let mut supervisor = Command::new(exe());
    supervisor
        .args([
            "supervise",
            &dir.to_string_lossy(),
            name,
            &cwd.to_string_lossy(),
            command,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Not the pane's (#386): what it runs outlives the pane it was started from.
    for name in breadboard_core::herdr::PANE_ENV {
        supervisor.env_remove(name);
    }
    // The supervisor forks and its first half ends at once: no zombie is left here.
    if !supervisor.status().is_ok_and(|status| status.success()) {
        return false;
    }
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        if running(dir, name).is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// `name` ended, its supervisor first: whether one ran.
pub fn stop(dir: &Path, name: &str) -> bool {
    let Some(pid) = running(dir, name) else {
        let _ = fs::remove_file(pid_file(dir, name));
        return false;
    };
    // SAFETY: a plain signal to the supervisor, which ends its process and goes.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let until = Instant::now() + END_WAIT + Duration::from_secs(1);
    while Instant::now() < until && is_alive(pid) {
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// While the stack of `dir` runs, its processes do: each one not running
/// starts while its port is free (an app the user runs by hand keeps it);
/// when the stack is down they end. A worktree on the shared stack has
/// none. What changed: `+dev`, `-studio`.
pub fn sync(repo: &Repo, dir: &Path) -> Vec<String> {
    if !env_file_of(dir).exists() {
        return Vec::new();
    }
    let is_main = dir == repo.main;
    let offset = if is_main { 0 } else { offset_of(dir) };
    if !is_main && offset == 0 {
        return Vec::new();
    }
    let procs = wanted(dir, offset);
    let mut changed = Vec::new();
    if is_up(dir) {
        for (name, port, command) in procs {
            if running(dir, &name).is_none() && is_port_free(port) {
                if start(dir, &name, &command) {
                    changed.push(format!("+{name}"));
                } else {
                    changed.push(format!(
                        "{name} didn't start (its log: {})",
                        log_of(dir, &name).display()
                    ));
                }
            }
        }
        return changed;
    }
    for (name, _, _) in procs {
        if stop(dir, &name) {
            changed.push(format!("-{name}"));
        }
    }
    changed
}

/// Every process of `dir` ended: its stack or its worktree goes.
pub fn stop_all(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder(dir)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_stem()?.to_string_lossy().into_owned();
            (path.extension()? == "pid").then_some(name)
        })
        .collect();
    names.sort_by_key(|name| (name != DEV, name.clone()));
    names
        .into_iter()
        .filter(|name| stop(dir, name))
        .map(|name| format!("-{name}"))
        .collect()
}

// ---------------------------------------------------------------------------
// The supervisor
// ---------------------------------------------------------------------------

static IS_ENDING: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    IS_ENDING.store(true, Ordering::SeqCst);
}

/// The wait before the next start after `crashes` crashes in a row.
pub fn backoff(crashes: u32) -> Duration {
    Duration::from_secs(1u64 << crashes.min(5)).min(BACKOFF_MAX)
}

fn now() -> String {
    breadboard_core::time::to_iso(breadboard_core::time::now_ms())
}

/// `breadboard supervise <dir> <name> <cwd> <command>`: runs `command` in
/// `cwd` (`sh -c`, a process group of its own) with its output in the log,
/// again after it ended, until SIGTERM or its pid file names another; then
/// its process group ends. It detaches first: the caller's wait ends at once.
pub fn supervise(args: &[String]) -> ExitCode {
    let [dir, name, cwd, command] = args else {
        eprintln!("usage: breadboard supervise <dir> <name> <cwd> <command>");
        return ExitCode::FAILURE;
    };
    let (dir, cwd) = (Path::new(dir), Path::new(cwd));
    // SAFETY: nothing runs on another thread yet; the child goes on as the daemon of one process.
    unsafe {
        match libc::fork() {
            -1 => return ExitCode::FAILURE,
            0 => {
                libc::setsid();
                // A worktree removed under it takes no cwd along.
                libc::chdir(c"/".as_ptr());
            }
            _ => return ExitCode::SUCCESS,
        }
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let own = std::process::id().to_string();
    let pids = pid_file(dir, name);
    let _ = fs::create_dir_all(folder(dir));
    let written = pids.with_extension(format!("pid.{own}"));
    if fs::write(&written, format!("{own}\n")).is_err() || fs::rename(&written, &pids).is_err() {
        return ExitCode::FAILURE;
    }
    let is_ours = || fs::read_to_string(&pids).is_ok_and(|text| text.trim() == own);
    let log = log_of(dir, name);
    let say = |line: &str| {
        use std::io::Write;
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&log) {
            let _ = writeln!(file, "── {} {line} ──", now());
        }
    };
    // A new stack, a new log.
    let _ = fs::write(&log, "");
    let mut crashes = 0u32;
    while !IS_ENDING.load(Ordering::SeqCst) && is_ours() {
        let output = fs::OpenOptions::new().create(true).append(true).open(&log);
        let Ok(output) = output else { break };
        say(&format!("breadboard starts {name}: {command}"));
        let started = Instant::now();
        let child = {
            use std::os::unix::process::CommandExt;
            Command::new("sh")
                .args(["-c", command])
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(output.try_clone().map_or_else(|_| Stdio::null(), Stdio::from))
                .stderr(Stdio::from(output))
                .process_group(0)
                .spawn()
        };
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                say(&format!("{name} didn't start: {error}"));
                break;
            }
        };
        let group = i32::try_from(child.id()).unwrap_or(0);
        let status = loop {
            if IS_ENDING.load(Ordering::SeqCst) || !is_ours() {
                end_group(group, &mut child);
                say(&format!("{name} ended with the stack"));
                if is_ours() {
                    let _ = fs::remove_file(&pids);
                }
                return ExitCode::SUCCESS;
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(LOOK_EVERY),
                Err(_) => break std::process::ExitStatus::default(),
            }
        };
        // What it left behind in its group goes with it.
        // SAFETY: a signal to the group it was started in.
        unsafe { libc::kill(-group, libc::SIGTERM) };
        if started.elapsed() >= RAN_WELL {
            crashes = 0;
        }
        let wait = backoff(crashes);
        crashes += 1;
        say(&format!("{name} ended ({status}), again in {} s", wait.as_secs()));
        let until = Instant::now() + wait;
        while Instant::now() < until && !IS_ENDING.load(Ordering::SeqCst) && is_ours() {
            std::thread::sleep(LOOK_EVERY);
        }
    }
    if is_ours() {
        let _ = fs::remove_file(&pids);
    }
    ExitCode::SUCCESS
}

/// The process group `group` ended: SIGTERM, SIGKILL after END_WAIT.
fn end_group(group: i32, child: &mut std::process::Child) {
    // SAFETY: signals to the group the supervisor started.
    unsafe { libc::kill(-group, libc::SIGTERM) };
    let until = Instant::now() + END_WAIT;
    while Instant::now() < until {
        if child.try_wait().ok().flatten().is_some() {
            // Its children may still run in its group.
            // SAFETY: as above.
            unsafe { libc::kill(-group, libc::SIGKILL) };
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: as above.
    unsafe { libc::kill(-group, libc::SIGKILL) };
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_longer_after_each_crash() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(4), Duration::from_secs(16));
        assert_eq!(backoff(9), BACKOFF_MAX);
    }

    #[test]
    fn dev_leads_the_logs() {
        let dir = std::env::temp_dir().join(format!("breadboard-procs-{}", std::process::id()));
        let _ = fs::create_dir_all(folder(&dir));
        for name in ["studio.log", "dev.log", "dev.pid"] {
            fs::write(folder(&dir).join(name), "").unwrap();
        }
        let names: Vec<String> = logs_of(&dir)
            .iter()
            .map(|log| log.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["dev.log", "studio.log"]);
        assert_eq!(running(&dir, DEV), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
