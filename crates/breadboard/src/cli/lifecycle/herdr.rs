//! The herdr CLI of the session this runs in, as JSON results (lib/herdr.mjs).
//! Outside herdr, or when herdr is old or unhappy, every call answers None,
//! never fails: the layout is comfort, the work goes on without it.
//!
//! Reads are remembered only inside `remembering` (a layout, under its lock),
//! and there only until the next write: a layout asks the same `workspace
//! list` and `tab list` dozens of times between two changes (~240 reads a run
//! in Node, rewrite rule 7). Outside it every read asks herdr: an agent's
//! state changes on its own, during a wt:new of minutes or a wait (#272), and
//! a cache cleared only by this process's writes would act on a stale answer.

use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::Duration;

thread_local! {
    /// How deep this thread is in `remembering`.
    static REMEMBERING: Cell<u32> = const { Cell::new(0) };
    /// The answers of reads since the last write inside `remembering`, by their arguments.
    static READS: RefCell<HashMap<String, Option<Value>>> = RefCell::new(HashMap::new());
}

/// `run` with herdr's reads remembered until a write: for a stretch of
/// herdr calls that no child process and no wait interrupts (a layout under
/// its lock). Its first read asks herdr, its end forgets.
pub fn remembering<T>(run: impl FnOnce() -> T) -> T {
    if REMEMBERING.get() == 0 {
        forget();
    }
    REMEMBERING.set(REMEMBERING.get() + 1);
    let result = run();
    REMEMBERING.set(REMEMBERING.get() - 1);
    if REMEMBERING.get() == 0 {
        forget();
    }
    result
}

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
        ["plugin", "list"],
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
    answer(args, || {
        spawn(args)
            .filter(|output| output.status.success())
            .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok())
            .and_then(|answer| answer.get("result").cloned())
            .filter(|result| !result.is_null())
    })
}

/// The answer to `args` from `ask`, or the remembered one of the same read
/// inside `remembering`; a write forgets every read.
fn answer(args: &[&str], ask: impl FnOnce() -> Option<Value>) -> Option<Value> {
    if !is_read(args) {
        forget();
        return ask();
    }
    if REMEMBERING.get() == 0 {
        return ask();
    }
    let key = args.join("\u{1f}");
    if let Some(known) = READS.with_borrow(|reads| reads.get(&key).cloned()) {
        return known;
    }
    let result = ask();
    READS.with_borrow_mut(|reads| reads.insert(key, result.clone()));
    result
}

/// `herdr <args>` for its effect: whether herdr did it (some answer nothing).
pub fn did(args: &[&str]) -> bool {
    forget();
    spawn(args).is_some_and(|output| output.status.success())
}

/// Every read asks herdr again.
pub fn forget() {
    READS.with_borrow_mut(HashMap::clear);
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

    /// An answer that counts up: each time herdr is really asked, a new one.
    fn counter() -> (Cell<i64>, impl Fn(&Cell<i64>) -> Option<Value>) {
        (Cell::new(0), |asked: &Cell<i64>| {
            asked.set(asked.get() + 1);
            Some(json!(asked.get()))
        })
    }

    #[test]
    fn a_read_after_a_setup_sees_what_changed_meanwhile() {
        // #272: the agent was idle before wt:new and works after it; the
        // re-check must not get the answer from before.
        let (asked, ask) = counter();
        let panes = ["pane", "list"];
        assert_eq!(answer(&panes, || ask(&asked)), Some(json!(1)));
        assert_eq!(answer(&panes, || ask(&asked)), Some(json!(2)));
    }

    #[test]
    fn remembering_keeps_a_read_until_a_write() {
        let (asked, ask) = counter();
        let tabs = ["tab", "list", "--workspace", "w1"];
        remembering(|| {
            assert_eq!(answer(&tabs, || ask(&asked)), Some(json!(1)));
            assert_eq!(answer(&tabs, || ask(&asked)), Some(json!(1)));
            answer(&["tab", "rename", "t1", "root"], || None);
            assert_eq!(answer(&tabs, || ask(&asked)), Some(json!(2)));
            // Nested: still the same stretch.
            remembering(|| assert_eq!(answer(&tabs, || ask(&asked)), Some(json!(2))));
            assert_eq!(answer(&tabs, || ask(&asked)), Some(json!(2)));
        });
        assert_eq!(answer(&tabs, || ask(&asked)), Some(json!(3)));
        remembering(|| assert_eq!(answer(&tabs, || ask(&asked)), Some(json!(4))));
    }

    #[test]
    fn walks_up_to_init() {
        let chain = ancestors_of(i64::from(std::process::id()));
        assert_eq!(chain.first(), Some(&i64::from(std::process::id())));
        assert!(chain.len() >= 2);
    }
}
