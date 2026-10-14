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
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::proc::{Wait, wait_within};

thread_local! {
    /// How deep this thread is in `remembering`.
    static REMEMBERING: Cell<u32> = const { Cell::new(0) };
    /// The answers of reads since the last write inside `remembering`, by their arguments.
    static READS: RefCell<HashMap<String, Option<Value>>> = RefCell::new(HashMap::new());
    /// What this thread asked herdr to do inside `journaled`, None outside it.
    static JOURNAL: RefCell<Option<Journal>> = const { RefCell::new(None) };
}

/// The herdr writes of a stretch of code, for whoever says what it did (the
/// overview's log, lazygit's command log): each write herdr did, word for
/// word, and the labels of the tabs it read meanwhile, so a `tab close
/// <id>` can be named by the tab it closed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Journal {
    /// The writes herdr did (exit 0), in order; a socket call as `socket <method> <params>`.
    pub writes: Vec<Vec<String>>,
    /// tab id → its label.
    pub tabs: HashMap<String, String>,
    /// pane id → the id of its tab.
    pub panes: HashMap<String, String>,
}

impl Journal {
    /// The label of a tab, or of the tab a pane stands in, as last read.
    pub fn tab_label(&self, id: &str) -> Option<&str> {
        let tab = self.panes.get(id).map_or(id, String::as_str);
        self.tabs.get(tab).map(String::as_str)
    }
}

/// `run` with what it made herdr do written down (a journal inside another one adds to it).
pub fn journaled<T>(run: impl FnOnce() -> T) -> (T, Journal) {
    let outer = JOURNAL.replace(Some(Journal::default()));
    let result = run();
    let journal = JOURNAL.replace(outer).unwrap_or_default();
    JOURNAL.with_borrow_mut(|outer| {
        if let Some(outer) = outer {
            outer.writes.extend(journal.writes.iter().cloned());
            outer.tabs.extend(journal.tabs.clone());
            outer.panes.extend(journal.panes.clone());
        }
    });
    (result, journal)
}

/// A call herdr answered, into the journal when one is kept: a write it did, the tabs and panes a read named.
fn note(args: &[&str], output: &std::process::Output) {
    JOURNAL.with_borrow_mut(|journal| {
        let Some(journal) = journal else { return };
        if !output.status.success() {
            return;
        }
        if !is_read(args) {
            journal.writes.push(args.iter().map(|arg| arg.to_string()).collect());
            return;
        }
        let Some(result) = serde_json::from_slice::<Value>(&output.stdout)
            .ok()
            .and_then(|answer| answer.get("result").cloned())
        else {
            return;
        };
        let items = |key: &str| result.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        for tab in items("tabs") {
            journal
                .tabs
                .insert(s(&tab, "tab_id").to_string(), s(&tab, "label").to_string());
        }
        for pane in items("panes") {
            journal
                .panes
                .insert(s(&pane, "pane_id").to_string(), s(&pane, "tab_id").to_string());
        }
    });
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

/// How long one herdr call may take: they answer in milliseconds.
pub const LIMIT: Duration = Duration::from_secs(10);

/// After a call went past [`LIMIT`], herdr counts as gone this long: every
/// call answers None at once (a layout makes ~80, a removal ~20), then it is
/// asked again (the overview runs for days).
const SILENCE: Duration = Duration::from_secs(60);

/// Since when herdr did not answer, if it did not.
static SILENT_SINCE: Mutex<Option<Instant>> = Mutex::new(None);

/// Set on the commands this one starts while herdr is silent: they don't
/// wait for it again (a handover's undo, a removal's changelog, 10 s each).
const SILENT_ENV: &str = "BREADBOARD_HERDR_SILENT";

/// Whether herdr went silent less than [`SILENCE`] ago, here or in the
/// command that started this one: its calls answer None.
pub fn is_silent() -> bool {
    std::env::var_os(SILENT_ENV).is_some_and(|value| value == "1")
        || SILENT_SINCE
            .lock()
            .ok()
            .and_then(|since| *since)
            .is_some_and(|since| since.elapsed() < SILENCE)
}

/// `command` (a child of this one) told that herdr is silent, when it is.
pub fn pass_silence(command: &mut Command) -> &mut Command {
    if is_silent() {
        command.env(SILENT_ENV, "1");
    }
    command
}

/// `herdr <args>`, given at most [`LIMIT`]: a herdr server that hangs never
/// holds a removal that is done (today's waited without end).
fn spawn(args: &[&str]) -> Option<std::process::Output> {
    if is_silent() {
        return None;
    }
    let child = Command::new("herdr")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let what = format!("herdr ({})", args.iter().take(2).copied().collect::<Vec<_>>().join(" "));
    let output = wait_within(child, &Wait::new(what, LIMIT)).ok()?;
    if let Some(output) = &output {
        note(args, output);
    }
    if output.is_none()
        && let Ok(mut since) = SILENT_SINCE.lock()
    {
        *since = Some(Instant::now());
    }
    output
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

/// `herdr <args>` with its whole answer, within the limit of every call;
/// None when herdr didn't start or didn't answer. Forgets every read.
pub fn raw(args: &[&str]) -> Option<std::process::Output> {
    forget();
    spawn(args)
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

/// The panes of every space as herdr answers, or why it gave none: an
/// empty list is an answer, a herdr that hung or failed is not (a handover
/// must not take it for "no claude runs the agent").
pub fn all_panes_answered() -> Result<Vec<Value>, String> {
    let args = ["pane", "list"];
    forget();
    let Some(output) = spawn(&args) else {
        return Err(if is_silent() {
            format!("herdr did not answer in {} s (herdr pane list)", LIMIT.as_secs())
        } else {
            "herdr could not be started (herdr pane list)".into()
        });
    };
    panes_answered(&output)
}

/// The panes in herdr's answer to `pane list`, or why it is none.
fn panes_answered(output: &std::process::Output) -> Result<Vec<Value>, String> {
    let result = serde_json::from_slice::<Value>(&output.stdout)
        .ok()
        .and_then(|answer| answer.get("result").cloned())
        .filter(|_| output.status.success());
    match result {
        // A result without panes is none (lib/herdr.mjs read it so).
        Some(result) => Ok(result
            .get("panes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()),
        None => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first = stderr.lines().map(str::trim).find(|line| !line.is_empty());
            Err(match first {
                Some(line) => format!("herdr pane list failed: {line}"),
                None => "herdr pane list answered no JSON".into(),
            })
        }
    }
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
pub fn ancestors_of(pid: i64) -> Vec<i64> {
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

/// What `current_pane` found in this process: the id of the pane whose
/// shell is an ancestor, or None when no pane's is. A shell started before
/// this process; so a pane found none of stays none for the run, and one
/// found is asked about once (a move gives it a new id).
static FOUND: Mutex<Option<Option<String>>> = Mutex::new(None);

/// How many panes are asked for their shell at once: herdr answers each in
/// milliseconds, 94 one after another took 3.4 s (rewrite-log 21:05 (4)).
const AT_ONCE: usize = 16;

/// The first pane, in herdr's order, whose shell is in `chain`, asked
/// [`AT_ONCE`] at a time; and whether herdr told the shell of every pane
/// it asked about (else "none" is no answer).
fn first_ours(panes: &[Value], chain: &[i64]) -> (Option<Value>, bool) {
    let mut has_every_shell = true;
    for batch in panes.chunks(AT_ONCE) {
        let shells: Vec<Option<i64>> = std::thread::scope(|scope| {
            let asked: Vec<_> = batch
                .iter()
                .map(|pane| scope.spawn(|| shell_of(s(pane, "pane_id"))))
                .collect();
            asked.into_iter().map(|one| one.join().ok().flatten()).collect()
        });
        has_every_shell &= shells.iter().all(Option::is_some);
        if let Some(at) = shells
            .iter()
            .position(|shell| shell.is_some_and(|shell| chain.contains(&shell)))
        {
            return (Some(batch[at].clone()), true);
        }
    }
    (None, has_every_shell)
}

/// The pane running this: the one whose shell is an ancestor of this
/// process. Not trusting HERDR_PANE_ID alone: a pane that moved got a new
/// id, and herdr answers a gone id with the focused pane (#287). Without
/// HERDR_PANE_ID this process has no pane (a detached ✕ of the overview):
/// None, where today's asked herdr and took whichever pane is focused.
/// Asked again in one run (wt:new asks three times), the answer found is
/// checked, not searched again.
pub fn current_pane() -> Option<Value> {
    if !is_in() || std::env::var_os("HERDR_PANE_ID").is_none() {
        return None;
    }
    let found = FOUND.lock().ok().and_then(|found| found.clone());
    if found == Some(None) {
        return None;
    }
    let current = call(&["pane", "current", "--current"]).and_then(|result| result.get("pane").cloned());
    let chain = ancestors_of(i64::from(std::process::id()));
    if chain.len() < 2 {
        return current;
    }
    let is_ours = |id: &str| shell_of(id).is_some_and(|shell| chain.contains(&shell));
    let remember = |pane: Option<&Value>| {
        if let Ok(mut found) = FOUND.lock() {
            *found = Some(pane.map(|pane| s(pane, "pane_id").to_string()));
        }
    };
    if let Some(current) = current.filter(|pane| is_ours(s(pane, "pane_id"))) {
        remember(Some(&current));
        return Some(current);
    }
    let panes = all_panes();
    if let Some(Some(id)) = &found
        && let Some(pane) = panes.iter().find(|pane| s(pane, "pane_id") == id)
        && is_ours(id)
    {
        return Some(pane.clone());
    }
    let (ours, is_answer) = first_ours(&panes, &chain);
    // An empty list, or a shell herdr didn't tell, is no "in no pane".
    if ours.is_some() || (is_answer && !panes.is_empty()) {
        remember(ours.as_ref());
    }
    ours
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
    let result = serde_json::from_str::<Value>(&line)
        .ok()?
        .get("result")
        .cloned()
        .filter(|result| !result.is_null())?;
    JOURNAL.with_borrow_mut(|journal| {
        if let Some(journal) = journal {
            journal
                .writes
                .push(vec!["socket".into(), method.into(), params_text(&request)]);
        }
    });
    Some(result)
}

/// The params of a socket request as one word of the journal.
fn params_text(request: &Value) -> String {
    request.get("params").map(Value::to_string).unwrap_or_default()
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
    fn a_failed_pane_list_is_no_empty_one() {
        use std::os::unix::process::ExitStatusExt;
        let output = |code: i32, stdout: &str, stderr: &str| std::process::Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.into(),
            stderr: stderr.into(),
        };
        let panes = output(0, r#"{"result":{"panes":[{"pane_id":"p1"}]}}"#, "");
        assert_eq!(panes_answered(&panes).unwrap(), vec![json!({ "pane_id": "p1" })]);
        assert_eq!(
            panes_answered(&output(0, r#"{"result":{}}"#, "")).unwrap(),
            Vec::<Value>::new()
        );
        assert_eq!(
            panes_answered(&output(1, "", "\nerror: no server on the socket\n")).unwrap_err(),
            "herdr pane list failed: error: no server on the socket"
        );
        assert_eq!(
            panes_answered(&output(0, "garbage", "")).unwrap_err(),
            "herdr pane list answered no JSON"
        );
    }

    #[test]
    fn walks_up_to_init() {
        let chain = ancestors_of(i64::from(std::process::id()));
        assert_eq!(chain.first(), Some(&i64::from(std::process::id())));
        assert!(chain.len() >= 2);
    }
}
