//! The herdr CLI of the session this runs in, as JSON results (lib/herdr.mjs).
//! Outside herdr, or when herdr is old or unhappy, every call answers None,
//! never fails: the layout is comfort, the work goes on without it.
//!
//! Reads are remembered until the next write: a layout asks the same
//! `workspace list` and `tab list` dozens of times between two changes
//! (~240 reads a run in Node, rewrite rule 7), and a write makes every later
//! read ask herdr again, so nothing stale is ever acted on.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

/// The answers of reads since the last write, by their arguments.
static READS: Mutex<Option<HashMap<String, Option<Value>>>> = Mutex::new(None);

/// Whether this runs inside herdr (`HERDR_ENV=1`).
pub fn is_in() -> bool {
    std::env::var("HERDR_ENV").is_ok_and(|value| value == "1")
}

/// The commands that only read (the gauntlet's fake herdr knows the same list).
fn is_read(args: &[&str]) -> bool {
    let reads = [
        ["workspace", "list"],
        ["tab", "list"],
        ["pane", "list"],
        ["pane", "current"],
        ["pane", "process-info"],
        ["pane", "read"],
        ["worktree", "list"],
        ["agent", "list"],
    ];
    reads
        .iter()
        .any(|read| args.len() >= 2 && args[0] == read[0] && args[1] == read[1])
}

fn spawn(args: &[&str]) -> Option<std::process::Output> {
    Command::new("herdr")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()
}

/// `herdr <args>` → its `result`, None when herdr is missing, failed or
/// answered no JSON.
pub fn call(args: &[&str]) -> Option<Value> {
    let key = args.join("\u{1f}");
    let is_read = is_read(args);
    if is_read && let Some(known) = READS.lock().ok().and_then(|reads| reads.as_ref()?.get(&key).cloned()) {
        return known;
    }
    if !is_read {
        forget();
    }
    let output = spawn(args).filter(|output| output.status.success());
    let result = output
        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok())
        .and_then(|answer| answer.get("result").cloned())
        .filter(|result| !result.is_null());
    if is_read && let Ok(mut reads) = READS.lock() {
        reads.get_or_insert_with(HashMap::new).insert(key, result.clone());
    }
    result
}

/// `herdr <args>` for its effect: whether herdr did it (some answer nothing).
pub fn did(args: &[&str]) -> bool {
    forget();
    spawn(args).is_some_and(|output| output.status.success())
}

/// Every read asks herdr again.
pub fn forget() {
    if let Ok(mut reads) = READS.lock() {
        *reads = None;
    }
}

/// A string field of herdr's JSON, "" without one.
pub fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The string at `path` in herdr's JSON (`move_result.created_tab.tab_id`).
pub fn id_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut at = value;
    for key in path {
        at = at.get(key)?;
    }
    at.as_str().filter(|id| !id.is_empty()).map(String::from)
}

/// Whether a pane runs an agent (herdr's `agent` set).
pub fn has_agent(pane: &Value) -> bool {
    breadboard_core::json::is_truthy(pane.get("agent"))
}

fn list(args: &[&str], key: &str) -> Vec<Value> {
    call(args)
        .and_then(|result| result.get(key)?.as_array().cloned())
        .unwrap_or_default()
}

pub fn spaces() -> Vec<Value> {
    list(&["workspace", "list"], "workspaces")
}

pub fn tabs_of(workspace: &str) -> Vec<Value> {
    list(&["tab", "list", "--workspace", workspace], "tabs")
}

pub fn panes_of(workspace: &str) -> Vec<Value> {
    list(&["pane", "list", "--workspace", workspace], "panes")
}

/// The panes of every space.
pub fn all_panes() -> Vec<Value> {
    list(&["pane", "list"], "panes")
}

/// The space with this label.
pub fn space_labelled(label: &str) -> Option<Value> {
    spaces().into_iter().find(|space| s(space, "label") == label)
}

/// The shell of a pane, its pid.
fn shell_of(pane: &str) -> Option<i64> {
    call(&["pane", "process-info", "--pane", pane])?
        .get("process_info")?
        .get("shell_pid")?
        .as_i64()
}

/// A pane whose shell has the terminal to itself: nothing runs in it (tabs.mjs `isIdle`).
pub fn is_idle(pane: &str) -> bool {
    let Some(info) =
        call(&["pane", "process-info", "--pane", pane]).and_then(|result| result.get("process_info").cloned())
    else {
        return false;
    };
    let shell = info.get("shell_pid").and_then(Value::as_i64);
    shell.is_some_and(|shell| {
        shell != 0 && info.get("foreground_process_group_id").and_then(Value::as_i64) == Some(shell)
    })
}

/// The pids from `pid` up to init, from Linux's /proc; only `pid` without it.
fn ancestors_of(pid: i64) -> Vec<i64> {
    let mut chain = Vec::new();
    let mut id = pid;
    while id > 1 && chain.len() < 64 {
        chain.push(id);
        // `<pid> (<comm>) <state> <ppid> …`: the name may hold spaces and parentheses.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{id}/stat")) else {
            break;
        };
        let Some(rest) = stat.rfind(')').map(|at| &stat[at + 1..]) else {
            break;
        };
        match rest.split_whitespace().nth(1).and_then(|ppid| ppid.parse().ok()) {
            Some(parent) => id = parent,
            None => break,
        }
    }
    chain
}

/// The pane running this: the one whose shell is an ancestor of this
/// process. Not trusting HERDR_PANE_ID alone: a pane that moved got a new
/// id, and herdr answers a gone id with the focused pane (#287). Without
/// HERDR_PANE_ID this process has no pane (a detached ✕ of the overview):
/// None, where today's asked herdr and took whichever pane is focused.
pub fn current_pane() -> Option<Value> {
    if !is_in() || std::env::var_os("HERDR_PANE_ID").is_none() {
        return None;
    }
    let current = call(&["pane", "current", "--current"]).and_then(|result| result.get("pane").cloned());
    let chain = ancestors_of(i64::from(std::process::id()));
    if chain.len() < 2 {
        return current;
    }
    let is_ours = |pane: &Value| shell_of(s(pane, "pane_id")).is_some_and(|shell| chain.contains(&shell));
    if let Some(current) = current.filter(is_ours) {
        return Some(current);
    }
    all_panes().into_iter().find(is_ours)
}

/// A method of herdr's socket API the CLI has no command for
/// (`workspace.move_block`) → its `result`; None outside herdr, on an error
/// or after two seconds.
pub fn api(method: &str, params: Value) -> Option<Value> {
    let socket = std::env::var_os("HERDR_SOCKET_PATH").filter(|_| is_in())?;
    forget();
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let request = json!({ "id": "breadboard", "method": method, "params": params });
    stream.write_all(format!("{request}\n").as_bytes()).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str::<Value>(&line)
        .ok()?
        .get("result")
        .cloned()
        .filter(|result| !result.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_reads_from_writes() {
        assert!(is_read(&["workspace", "list"]));
        assert!(is_read(&["pane", "process-info", "--pane", "p"]));
        assert!(!is_read(&["tab", "close", "t"]));
        assert!(!is_read(&["pane", "run", "p", "x"]));
    }

    #[test]
    fn walks_up_to_init() {
        let chain = ancestors_of(i64::from(std::process::id()));
        assert_eq!(chain.first(), Some(&i64::from(std::process::id())));
        assert!(chain.len() >= 2);
    }
}
