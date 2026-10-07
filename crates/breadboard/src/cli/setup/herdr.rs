//! herdr for the setup commands (lib/herdr.mjs, tabs.mjs, spaces.mjs,
//! layout.mjs): the CLI as JSON results, the spaces and tabs a new worktree
//! or orchestration gets, the stack tabs. Outside herdr, or with herdr
//! unhappy, every call answers nothing and nothing fails: the worktree is
//! the point, the space is comfort.

use super::proc::{self, quote};
use super::repo::{self, STUDIO_PORT, VITA};
use breadboard_core::json::str_of;
use breadboard_core::record::{self, ORCHESTRATOR, Origin, SLOTS, SPACE};
use serde_json::Value;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, SystemTime};

pub const OVERVIEW: &str = "overview";
pub const CHANGELOG: &str = "changelog";
pub const CHANGES: &str = "changes";
pub const ROOT: &str = "root";
pub const EDITOR: &str = "nvim";
pub const ORCHESTRATOR_TAB: &str = "orchestrator";
const DEV: &str = "dev";
const STUDIO: &str = "studio";
/// Whose sidebar tokens they are: the plugin's id.
pub const TOKEN_SOURCE: &str = "monocircuit.platform";
const BRANCH_TOKEN: &str = "wt_branch";

pub fn is_in_herdr() -> bool {
    std::env::var("HERDR_ENV").is_ok_and(|value| value == "1")
}

/// `herdr <args>` → its `result`, None when herdr is missing, old or unhappy.
pub fn herdr(args: &[&str]) -> Option<Value> {
    let output = Command::new("herdr")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    value.get("result").filter(|result| !result.is_null()).cloned()
}

/// `herdr <args>` for its effect: true when herdr did it.
pub fn herdr_did(args: &[&str]) -> bool {
    Command::new("herdr")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn list(result: Option<Value>, key: &str) -> Vec<Value> {
    result
        .and_then(|result| result.get(key).and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

pub fn spaces() -> Vec<Value> {
    list(herdr(&["workspace", "list"]), "workspaces")
}

pub fn tabs_of(workspace: &str) -> Vec<Value> {
    list(herdr(&["tab", "list", "--workspace", workspace]), "tabs")
}

pub fn panes_of(workspace: &str) -> Vec<Value> {
    list(herdr(&["pane", "list", "--workspace", workspace]), "panes")
}

pub fn all_panes() -> Vec<Value> {
    list(herdr(&["pane", "list"]), "panes")
}

/// A string field of a herdr answer, "" without it.
pub fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    str_of(value, key).unwrap_or_default()
}

/// The session a pane's agent runs, `agent_session.value`.
pub fn session_of(pane: &Value) -> Option<&str> {
    pane.get("agent_session").and_then(|session| str_of(session, "value"))
}

/// The pids from `pid` up to init, from /proc; only `pid` without it.
pub fn ancestors_of(pid: u32) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut id = pid;
    while id > 1 && chain.len() < 64 {
        chain.push(id);
        let Ok(stat) = fs::read_to_string(format!("/proc/{id}/stat")) else {
            break;
        };
        let Some(after) = stat.rfind(')').map(|at| &stat[at + 2..]) else {
            break;
        };
        match after.split(' ').nth(1).and_then(|ppid| ppid.parse().ok()) {
            Some(parent) => id = parent,
            None => break,
        }
    }
    chain
}

fn shell_of(pane: &str) -> Option<u64> {
    herdr(&["pane", "process-info", "--pane", pane])?
        .get("process_info")?
        .get("shell_pid")?
        .as_u64()
}

/// The pane running this: the one whose shell is an ancestor of this
/// process, herdr's answer alone only without /proc (#287). herdr's
/// `current` is asked first; the other panes only when it names one whose
/// shell herdr knows and that is not ours (a gone pane id answers with the
/// focused pane). When herdr can't say the shell of a pane it names, it
/// can't for the others either: one call instead of one per pane, the
/// same answer (none).
pub fn current_pane() -> Option<Value> {
    let current = herdr(&["pane", "current", "--current"]).and_then(|result| result.get("pane").cloned());
    let ancestors = ancestors_of(std::process::id());
    if ancestors.len() < 2 {
        return current;
    }
    let chain: Vec<u64> = ancestors.iter().map(|pid| u64::from(*pid)).collect();
    if let Some(current) = current {
        match shell_of(field(&current, "pane_id")) {
            Some(shell) if chain.contains(&shell) => return Some(current),
            Some(_) => {}
            None => return None,
        }
    }
    // One process-info a pane: asked 16 at a time, the first of herdr's
    // order that is ours wins (a run outside every pane asks them all: 94
    // panes took 3.3 s one after another).
    let panes = all_panes();
    for batch in panes.chunks(16) {
        let found = thread::scope(|scope| {
            let asked: Vec<_> = batch
                .iter()
                .map(|pane| scope.spawn(|| shell_of(field(pane, "pane_id"))))
                .collect();
            asked
                .into_iter()
                .map(|handle| handle.join().ok().flatten())
                .zip(batch)
                .find(|(shell, _)| shell.is_some_and(|shell| chain.contains(&shell)))
                .map(|(_, pane)| pane.clone())
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}
const LOCK_STALE: Duration = Duration::from_secs(60);

/// `run` as one change of the layout at a time across every process of the
/// user (lock.mjs `exclusive`): the directory lock Node uses too. A call
/// inside a locked one just runs; outside herdr there is nothing to lock.
pub fn exclusive<T>(run: impl FnOnce() -> T) -> T {
    if DEPTH.get() > 0 || !is_in_herdr() {
        return run();
    }
    let lock = breadboard_core::lock::layout_lock();
    let mut waited = Duration::ZERO;
    while fs::create_dir(&lock).is_err() {
        let age = fs::metadata(&lock)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .unwrap_or_default();
        if age > LOCK_STALE || waited > LOCK_STALE {
            let _ = fs::remove_dir_all(&lock);
        }
        thread::sleep(Duration::from_millis(100));
        waited += Duration::from_millis(100);
    }
    DEPTH.set(DEPTH.get() + 1);
    let result = run();
    DEPTH.set(DEPTH.get() - 1);
    let _ = fs::remove_dir_all(&lock);
    result
}

// ---------------------------------------------------------------------------
// Tabs (tabs.mjs)
// ---------------------------------------------------------------------------

/// What the overview tab of a space runs: this binary, of everything, a worktree or an orchestration.
pub fn overview_command(dir: Option<&Path>, orchestration: Option<&str>) -> String {
    let binary = quote(&proc::entry().to_string_lossy());
    match (orchestration, dir) {
        (Some(name), _) => format!("{binary} overview --orchestration {name}"),
        (None, Some(dir)) => format!("{binary} overview --worktree {}", quote(&dir.to_string_lossy())),
        (None, None) => format!("{binary} overview --herdr"),
    }
}

/// What the tab `changelog` of the main space runs.
pub fn overall_changelog_tab_command() -> String {
    format!("{} changelog --watch", quote(&proc::entry().to_string_lossy()))
}

/// What the tab `changelog` of an orchestration's space runs.
pub fn changelog_tab_command(name: &str) -> String {
    format!(
        "{} wt:orchestrate {name} --changelog --watch",
        quote(&proc::entry().to_string_lossy())
    )
}

/// A space by its label, created with the tab `first` (and its command) if missing.
pub fn ensure_space(label: &str, cwd: &Path, first: &str, command: Option<&str>) -> Option<String> {
    if let Some(found) = spaces().into_iter().find(|space| field(space, "label") == label) {
        return Some(field(&found, "workspace_id").to_string());
    }
    let cwd = cwd.to_string_lossy();
    let created = herdr(&["workspace", "create", "--label", label, "--cwd", &cwd, "--no-focus"])?;
    let workspace = created.get("workspace")?;
    if let Some(tab) = created.get("tab") {
        herdr(&["tab", "rename", field(tab, "tab_id"), first]);
    }
    if let (Some(command), Some(pane)) = (command, created.get("root_pane")) {
        herdr(&["pane", "run", field(pane, "pane_id"), command]);
    }
    Some(field(workspace, "workspace_id").to_string())
}

fn is_linked(space: &Value) -> bool {
    space
        .get("worktree")
        .and_then(|tree| tree.get("is_linked_worktree"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The space of a worktree in the group of the main space `main`: `worktree
/// open` links it; one standing alone is taken in; an older herdr gets a space of its own.
pub fn ensure_worktree_space(label: &str, dir: &Path, main: Option<&str>, first: &str) -> Option<String> {
    let found = spaces().into_iter().find(|space| field(space, "label") == label);
    let Some(main) = main.filter(|_| !found.as_ref().is_some_and(is_linked)) else {
        return found
            .map(|space| field(&space, "workspace_id").to_string())
            .or_else(|| ensure_space(label, dir, first, None));
    };
    let path = dir.to_string_lossy();
    // herdr takes the main space for the worktree's while its first tab stands there (herdrdev/herdr#4293).
    let listed = list(herdr(&["worktree", "list", "--workspace", main]), "worktrees");
    let is_main_taken = listed
        .iter()
        .any(|tree| field(tree, "path") == path && field(tree, "open_workspace_id") == main);
    let opened = if is_main_taken {
        None
    } else {
        herdr(&[
            "worktree",
            "open",
            "--workspace",
            main,
            "--path",
            &path,
            "--label",
            label,
            "--no-focus",
        ])
    };
    let Some(opened) = opened.filter(|opened| opened.get("workspace").is_some()) else {
        return found
            .map(|space| field(&space, "workspace_id").to_string())
            .or_else(|| ensure_space(label, dir, first, None));
    };
    let workspace = opened
        .get("workspace")
        .map(|space| field(space, "workspace_id").to_string())
        .unwrap_or_default();
    if opened.get("already_open").and_then(Value::as_bool).unwrap_or(false) {
        return Some(workspace);
    }
    if let Some(tab) = opened.get("tab") {
        herdr(&["tab", "rename", field(tab, "tab_id"), first]);
    }
    if let Some(found) = found {
        move_into(field(&found, "workspace_id"), &workspace);
        drop_duplicates(&workspace, first);
    }
    Some(workspace)
}

/// Every tab of the space `from` into `to`, in order and with its label; `from` closes once empty.
pub fn move_into(from: &str, to: &str) {
    let panes = panes_of(from);
    for tab in tabs_of(from) {
        let own: Vec<&Value> = panes
            .iter()
            .filter(|pane| field(pane, "tab_id") == field(&tab, "tab_id"))
            .collect();
        let Some(head) = own.first() else { continue };
        let moved = herdr(&[
            "pane",
            "move",
            field(head, "pane_id"),
            "--workspace",
            to,
            "--new-tab",
            "--no-focus",
        ]);
        let Some(created) = created_tab(&moved) else { continue };
        herdr(&["tab", "rename", &created, field(&tab, "label")]);
        for pane in &own[1..] {
            herdr(&[
                "pane",
                "move",
                field(pane, "pane_id"),
                "--tab",
                &created,
                "--split",
                "right",
                "--no-focus",
            ]);
        }
    }
    if spaces().iter().any(|space| field(space, "workspace_id") == from) && tabs_of(from).is_empty() {
        herdr(&["workspace", "close", from]);
    }
}

/// The tab a `pane move --new-tab` created, its id.
pub fn created_tab(moved: &Option<Value>) -> Option<String> {
    moved
        .as_ref()?
        .get("move_result")?
        .get("created_tab")
        .map(|tab| field(tab, "tab_id").to_string())
}

/// The branch of a worktree as the sidebar token `$wt_branch`, only on a change (herdrdev/herdr#2952).
pub fn mark_branch(workspace: &str, dir: &Path) {
    let Some(branch) = repo::worktrees()
        .into_iter()
        .find(|tree| tree.dir == dir)
        .and_then(|tree| tree.branch)
    else {
        return;
    };
    let Some(found) = spaces()
        .into_iter()
        .find(|space| field(space, "workspace_id") == workspace)
    else {
        return;
    };
    if found.get("tokens").and_then(|tokens| str_of(tokens, BRANCH_TOKEN)) == Some(branch.as_str()) {
        return;
    }
    let token = format!("{BRANCH_TOKEN}={branch}");
    herdr(&[
        "workspace",
        "report-metadata",
        workspace,
        "--source",
        TOKEN_SOURCE,
        "--token",
        &token,
    ]);
}

/// A pane whose shell has the terminal to itself: nothing runs in it.
pub fn is_idle(pane: &str) -> bool {
    let Some(info) =
        herdr(&["pane", "process-info", "--pane", pane]).and_then(|result| result.get("process_info").cloned())
    else {
        return false;
    };
    let shell = info.get("shell_pid").and_then(Value::as_u64);
    shell.is_some_and(|shell| {
        shell > 0 && info.get("foreground_process_group_id").and_then(Value::as_u64) == Some(shell)
    })
}

fn pane_count(tab: &Value) -> u64 {
    tab.get("pane_count").and_then(Value::as_u64).unwrap_or(0)
}

/// The single pane of the tab `label` when nothing runs in it.
pub fn idle_pane_of(workspace: &str, label: &str, tabs: &[Value]) -> Option<String> {
    let tab = tabs.iter().find(|tab| field(tab, "label") == label)?;
    if pane_count(tab) != 1 {
        return None;
    }
    let pane = panes_of(workspace)
        .into_iter()
        .find(|pane| field(pane, "tab_id") == field(tab, "tab_id"))?;
    let id = field(&pane, "pane_id").to_string();
    is_idle(&id).then_some(id)
}

/// What `ensure_tab` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The tab was there and its command runs (or it has none).
    Kept,
    /// Created, or its idle shell given its command again: the command runs.
    Started,
    /// Created, but herdr gave it no pane: its command didn't start.
    Empty,
}

/// A tab by its label, created unfocused with `command` in it if missing,
/// `command` run again when its shell idles.
pub fn ensure_tab(workspace: &str, label: &str, cwd: &Path, command: Option<&str>, tabs: &[Value]) -> Tab {
    if tabs.iter().any(|tab| field(tab, "label") == label) {
        let Some(command) = command else { return Tab::Kept };
        let is_run =
            idle_pane_of(workspace, label, tabs).is_some_and(|idle| herdr(&["pane", "run", &idle, command]).is_some());
        return if is_run { Tab::Started } else { Tab::Kept };
    }
    let cwd = cwd.to_string_lossy();
    let created = herdr(&[
        "tab",
        "create",
        "--workspace",
        workspace,
        "--cwd",
        &cwd,
        "--label",
        label,
        "--no-focus",
    ]);
    let Some(created) = created else { return Tab::Kept };
    let Some(command) = command else { return Tab::Started };
    let is_run = created
        .get("root_pane")
        .is_some_and(|pane| herdr(&["pane", "run", field(pane, "pane_id"), command]).is_some());
    if is_run { Tab::Started } else { Tab::Empty }
}

/// The tabs `order` names at the left end of their space, in that order
/// (herdr can't reorder: a tab out of place moves into a new one at the end).
pub fn keep_order(workspace: &str, order: &[&str]) {
    let tabs = tabs_of(workspace);
    let wanted: Vec<&str> = order
        .iter()
        .copied()
        .filter(|label| tabs.iter().any(|tab| field(tab, "label") == *label))
        .collect();
    let mut fixed = 0;
    while fixed < wanted.len() && tabs.get(fixed).is_some_and(|tab| field(tab, "label") == wanted[fixed]) {
        fixed += 1;
    }
    if fixed == wanted.len() {
        return;
    }
    let rest = &tabs[fixed.min(tabs.len())..];
    let named: Vec<&Value> = rest
        .iter()
        .filter(|tab| wanted.contains(&field(tab, "label")))
        .collect();
    let others: Vec<&Value> = rest
        .iter()
        .filter(|tab| !wanted.contains(&field(tab, "label")))
        .collect();
    let is_in_order = named
        .iter()
        .enumerate()
        .all(|(i, tab)| wanted.get(fixed + i) == Some(&field(tab, "label")));
    let moving: Vec<&Value> = if is_in_order {
        let last = named.last().map(|tab| field(tab, "tab_id"));
        let end = rest
            .iter()
            .position(|tab| Some(field(tab, "tab_id")) == last)
            .unwrap_or(0);
        others
            .into_iter()
            .filter(|tab| rest.iter().position(|known| known == *tab).unwrap_or(usize::MAX) < end)
            .collect()
    } else {
        let mut moving: Vec<&Value> = wanted[fixed..]
            .iter()
            .filter_map(|label| named.iter().find(|tab| field(tab, "label") == *label).copied())
            .collect();
        moving.extend(others);
        moving
    };
    let before = spaces();
    let panes = panes_of(workspace);
    let mut renamed: Vec<(String, String)> = Vec::new();
    for tab in moving {
        if pane_count(tab) != 1 {
            break;
        }
        let Some(pane) = panes.iter().find(|pane| field(pane, "tab_id") == field(tab, "tab_id")) else {
            break;
        };
        let moved = herdr(&[
            "pane",
            "move",
            field(pane, "pane_id"),
            "--workspace",
            workspace,
            "--new-tab",
        ]);
        let Some(created) = created_tab(&moved) else { break };
        herdr(&["tab", "rename", &created, field(tab, "label")]);
        renamed.push((field(tab, "tab_id").to_string(), created));
    }
    if renamed.is_empty() {
        return;
    }
    let now = |tab: &str| {
        renamed
            .iter()
            .find(|(old, _)| old == tab)
            .map(|(_, new)| new.clone())
            .unwrap_or(tab.to_string())
    };
    let own = before
        .iter()
        .find(|space| field(space, "workspace_id") == workspace)
        .map(|space| field(space, "active_tab_id"));
    let focused = before
        .iter()
        .find(|space| space.get("focused").and_then(Value::as_bool).unwrap_or(false))
        .map(|space| field(space, "active_tab_id"));
    if let Some(own) = own.filter(|tab| !tab.is_empty()) {
        herdr(&["tab", "focus", &now(own)]);
    }
    if let Some(focused) = focused.filter(|tab| !tab.is_empty()) {
        herdr(&["tab", "focus", &now(focused)]);
    }
}

/// One tab `label` in the space: spare duplicates (one pane, no agent, idle) close.
pub fn drop_duplicates(workspace: &str, label: &str) {
    let tabs: Vec<Value> = tabs_of(workspace)
        .into_iter()
        .filter(|tab| field(tab, "label") == label)
        .collect();
    if tabs.len() < 2 {
        return;
    }
    let panes = panes_of(workspace);
    let spare: Vec<&Value> = tabs
        .iter()
        .filter(|tab| {
            let own: Vec<&Value> = panes
                .iter()
                .filter(|pane| field(pane, "tab_id") == field(tab, "tab_id"))
                .collect();
            own.len() == 1 && !has_agent(own[0]) && is_idle(field(own[0], "pane_id"))
        })
        .collect();
    let closing = if spare.len() == tabs.len() {
        &spare[1..]
    } else {
        &spare[..]
    };
    for tab in closing {
        herdr(&["tab", "close", field(tab, "tab_id")]);
    }
}

/// Whether an agent runs in a pane.
pub fn has_agent(pane: &Value) -> bool {
    pane.get("agent")
        .is_some_and(|agent| !agent.is_null() && agent != "" && agent != false)
}

/// A tab that must not be closed from a script: it runs this, it has several panes, an agent is in it.
pub fn is_busy(tab: &Value, panes: &[Value], current: Option<&Value>) -> bool {
    current.is_some_and(|current| field(current, "tab_id") == field(tab, "tab_id"))
        || pane_count(tab) > 1
        || panes
            .iter()
            .any(|pane| field(pane, "tab_id") == field(tab, "tab_id") && has_agent(pane))
}

// ---------------------------------------------------------------------------
// Spaces (spaces.mjs)
// ---------------------------------------------------------------------------

/// The main space: `root`, `overview`, `changelog`; `root` comes back whenever it is missing.
pub fn main_space() -> Option<String> {
    let main = repo::main_dir();
    let workspace = ensure_space(SPACE, &main, ROOT, None)?;
    let mut tabs = tabs_of(&workspace);
    if let Some(old) = tabs.iter().find(|tab| field(tab, "label") == "develop")
        && !tabs.iter().any(|tab| field(tab, "label") == ROOT)
    {
        herdr(&["tab", "rename", field(old, "tab_id"), ROOT]);
        tabs = tabs_of(&workspace);
    }
    ensure_tab(&workspace, OVERVIEW, &main, Some(&overview_command(None, None)), &tabs);
    ensure_tab(&workspace, ROOT, &main, None, &tabs);
    ensure_tab(
        &workspace,
        CHANGELOG,
        &main,
        Some(&overall_changelog_tab_command()),
        &tabs,
    );
    keep_order(&workspace, &[ROOT, OVERVIEW, CHANGELOG]);
    Some(workspace)
}

/// Whether nvim is on the PATH: without it no nvim tab.
pub fn can_edit() -> bool {
    static HAS_NVIM: OnceLock<bool> = OnceLock::new();
    *HAS_NVIM.get_or_init(|| {
        Command::new("nvim")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// The tab `nvim` in the worktree, listening on its socket, with the IDE integration unless it is off.
pub fn ensure_editor(workspace: &str, dir: &Path, tabs: &[Value]) {
    if !can_edit() {
        return;
    }
    let main = repo::main_dir();
    let ide = super::nvim::ide_for(dir, &main);
    let command = super::nvim::editor_command(dir, None, &[super::nvim::changes_lua(&main)], ide.as_deref());
    ensure_tab(workspace, EDITOR, dir, Some(&command), tabs);
}

/// The space of a worktree: `root`, its `overview`, the conversation's tab (unless an agent brings its own), `nvim`.
pub fn worktree_space(dir: &Path, has_work_tab: bool) -> Option<String> {
    let feature = repo::feature_of(dir);
    let main = space_of(&repo::main_dir());
    let workspace = ensure_worktree_space(&feature, dir, main.as_deref(), ROOT)?;
    mark_branch(&workspace, dir);
    let tabs = tabs_of(&workspace);
    ensure_tab(
        &workspace,
        OVERVIEW,
        dir,
        Some(&overview_command(Some(dir), None)),
        &tabs,
    );
    if has_work_tab {
        ensure_tab(&workspace, &feature, dir, None, &tabs);
    }
    ensure_tab(&workspace, ROOT, dir, None, &tabs);
    if has_work_tab {
        ensure_editor(&workspace, dir, &tabs);
    }
    drop_duplicates(&workspace, &feature);
    keep_order(&workspace, &[ROOT, OVERVIEW, &feature, EDITOR]);
    Some(workspace)
}

/// The label of a checkout's space: the main space for the main checkout.
pub fn space_label(dir: &Path) -> String {
    if dir == repo::main_dir() {
        SPACE.to_string()
    } else {
        repo::feature_of(dir)
    }
}

/// The space a checkout's tabs belong in.
pub fn space_of(dir: &Path) -> Option<String> {
    let label = space_label(dir);
    spaces()
        .into_iter()
        .find(|space| field(space, "label") == label)
        .map(|space| field(&space, "workspace_id").to_string())
}

/// `docker compose … <args>` in the vita app of `dir`, quiet: its output.
pub fn compose_quiet(dir: &Path, args: &[&str]) -> String {
    let mut all = vec!["compose", "--env-file", ".env.local", "-f", "docker-compose.local.yml"];
    all.extend_from_slice(args);
    proc::sh("docker", &all, Some(&dir.join(VITA))).out
}

/// While the stack of `dir` runs its space shows it (a tab per container,
/// `dev`, `studio`); when it is down those tabs close unless busy. What changed, `+postgres` / `-dev`.
pub fn sync_stack_tabs(dir: &Path, workspace: Option<String>) -> Vec<String> {
    exclusive(|| {
        let Some(workspace) = workspace.or_else(|| space_of(dir)) else {
            return Vec::new();
        };
        if !repo::env_file_of(dir).exists() {
            return Vec::new();
        }
        let main = repo::main_dir();
        let offset = if dir == main { 0 } else { repo::offset_of(dir) };
        if dir != main && offset == 0 {
            return Vec::new();
        }
        let vita = dir.join(VITA);
        let services = proc::lines(&compose_quiet(dir, &["config", "--services"]));
        let is_up = !compose_quiet(dir, &["ps", "--status", "running", "-q"])
            .trim()
            .is_empty();
        let tabs = tabs_of(&workspace);
        let mut changed = Vec::new();
        if is_up {
            let logs = "docker compose --env-file .env.local -f docker-compose.local.yml logs -f --tail 200";
            let mut wanted: Vec<(String, String)> = services
                .iter()
                .map(|service| (service.clone(), format!("{logs} {service}")))
                .collect();
            if repo::is_port_free(3000 + u32::from(offset)) {
                wanted.push((DEV.into(), "pnpm dev".into()));
            }
            let studio = u32::from(STUDIO_PORT) + u32::from(offset);
            if repo::is_port_free(studio) {
                wanted.push((STUDIO.into(), format!("pnpm pg:studio --port {studio}")));
            }
            let mut empty = Vec::new();
            for (label, command) in wanted {
                match ensure_tab(&workspace, &label, &vita, Some(&command), &tabs) {
                    Tab::Started => changed.push(format!("+{label}")),
                    Tab::Empty => empty.push(label),
                    Tab::Kept => {}
                }
            }
            // A tab herdr gave no pane runs nothing: said, not counted as running.
            if !empty.is_empty() {
                let dot = if changed.is_empty() { "" } else { "· " };
                changed.push(format!(
                    "{dot}{} opened empty: herdr gave no pane, nothing runs there",
                    empty.join(" ")
                ));
            }
            return changed;
        }
        let is_ours = |tab: &Value| {
            let label = field(tab, "label");
            services.iter().any(|service| service == label) || label == DEV || label == STUDIO
        };
        // The pane running this is looked for only when there is a tab to close (a read per pane).
        if !tabs.iter().any(is_ours) {
            return changed;
        }
        let panes = panes_of(&workspace);
        let current = current_pane();
        for tab in &tabs {
            let label = field(tab, "label");
            if !is_ours(tab) || is_busy(tab, &panes, current.as_ref()) {
                continue;
            }
            if herdr(&["tab", "close", field(tab, "tab_id")]).is_some() {
                changed.push(format!("-{label}"));
            }
        }
        changed
    })
}

// ---------------------------------------------------------------------------
// Orchestrations' marks (origins.mjs)
// ---------------------------------------------------------------------------

/// The colours of the slots (herdr config names, fallbacks).
pub const SLOT_COLOURS: [(&str, &str); 3] = [("blue", "#89B4FA"), ("teal", "#94E2D5"), ("mauve", "#CBA6F7")];

/// The sidebar token of a slot: `orch1`.
pub fn origin_token(slot: u32) -> String {
    format!("orch{}", slot + 1)
}

/// The orchestration of a space's label.
pub fn orchestration_of(label: &str) -> Option<String> {
    record::orchestration_of_label(label, &record::titles(&repo::main_dir()))
}

/// What the spaces of an orchestration's worktrees show: `⇶ <title>`.
pub fn origin_mark(name: &str) -> String {
    format!("{ORCHESTRATOR} {}", record::title_of(&repo::main_dir(), name))
}

/// Every worktree an orchestration started, with its origin.
pub fn orchestrated() -> Vec<(PathBuf, Origin)> {
    repo::worktrees()
        .into_iter()
        .skip(1)
        .filter_map(|tree| record::origin_of(&tree.dir).map(|origin| (tree.dir, origin)))
        .collect()
}

fn slot_of_tokens(tokens: Option<&Value>) -> Option<u32> {
    (0..SLOTS).find(|slot| {
        tokens
            .and_then(|tokens| tokens.get(origin_token(*slot)))
            .is_some_and(|token| !token.is_null() && token != "")
    })
}

/// The colour slot of an orchestration: its worktrees', else its space's, else the least used.
pub fn slot_of(name: &str) -> u32 {
    let trees = orchestrated();
    if let Some((_, origin)) = trees.iter().find(|(_, origin)| origin.name == name) {
        return origin.slot;
    }
    let marked: Vec<(String, Value)> = if is_in_herdr() {
        spaces()
            .into_iter()
            .filter_map(|space| orchestration_of(field(&space, "label")).map(|of| (of, space)))
            .collect()
    } else {
        Vec::new()
    };
    if let Some(slot) = marked
        .iter()
        .find(|(of, _)| of == name)
        .and_then(|(_, space)| slot_of_tokens(space.get("tokens")))
    {
        return slot;
    }
    let mut uses = [0u32; SLOTS as usize];
    for (_, origin) in &trees {
        uses[origin.slot as usize % SLOTS as usize] += 1;
    }
    for (_, space) in &marked {
        if let Some(slot) = slot_of_tokens(space.get("tokens")) {
            uses[slot as usize] += 1;
        }
    }
    let least = uses.iter().min().copied().unwrap_or(0);
    uses.iter().position(|count| *count == least).unwrap_or(0) as u32
}

/// The origin token of a space as `origin` wants it; nothing sent when it shows that already.
pub fn mark_space(space: &Value, origin: Option<&Origin>) {
    let mut args: Vec<String> = Vec::new();
    for slot in 0..SLOTS {
        let token = origin_token(slot);
        let wanted = origin
            .filter(|origin| origin.slot == slot)
            .map(|origin| origin_mark(&origin.name));
        let shown = space
            .get("tokens")
            .and_then(|tokens| str_of(tokens, &token))
            .filter(|shown| !shown.is_empty());
        match (&wanted, shown) {
            (Some(wanted), shown) if shown != Some(wanted.as_str()) => {
                args.extend(["--token".into(), format!("{token}={wanted}")]);
            }
            (None, Some(_)) => args.extend(["--clear-token".into(), token]),
            _ => {}
        }
    }
    if args.is_empty() {
        return;
    }
    let mut all = vec![
        "workspace",
        "report-metadata",
        field(space, "workspace_id"),
        "--source",
        TOKEN_SOURCE,
    ];
    all.extend(args.iter().map(String::as_str));
    herdr(&all);
}

/// Every space shows the orchestration it belongs to.
pub fn mark_origins() {
    let by_feature: Vec<(String, Origin)> = orchestrated()
        .into_iter()
        .map(|(dir, origin)| (repo::feature_of(&dir), origin))
        .collect();
    for space in spaces() {
        let label = field(&space, "label");
        let origin = match orchestration_of(label) {
            Some(name) => Some(Origin {
                slot: slot_of(&name),
                name,
            }),
            None => by_feature
                .iter()
                .find(|(feature, _)| feature == label)
                .map(|(_, origin)| origin.clone()),
        };
        mark_space(&space, origin.as_ref());
    }
}

/// The orchestration whose space holds the pane running this (#245).
pub fn orchestration_here() -> Option<String> {
    if !is_in_herdr() || std::env::var_os("HERDR_PANE_ID").is_none() {
        return None;
    }
    let pane = current_pane()?;
    let label = spaces()
        .into_iter()
        .find(|space| field(space, "workspace_id") == field(&pane, "workspace_id"))
        .map(|space| field(&space, "label").to_string())?;
    orchestration_of(&label)
}

/// Whether wt:new takes the tab running it into the new space: an agent's, not an orchestrator's.
pub fn moves_tab(is_agent: bool, origin: Option<&Origin>, stays: Option<&str>) -> bool {
    is_agent && origin.is_none() && stays.is_none()
}

/// After wt:new: the worktree's space. A human gets it with a tab in front,
/// an agent takes its own tab in; an orchestration's is marked, unfocused.
pub fn open_space(feature: &str, dir: &Path, origin: Option<&Origin>, stays: Option<&str>) -> Option<String> {
    exclusive(|| {
        if !is_in_herdr() {
            return None;
        }
        main_space()?;
        if !moves_tab(proc::is_agent(), origin, stays) {
            let workspace = worktree_space(dir, true)?;
            sync_stack_tabs(dir, Some(workspace.clone()));
            if let Some(origin) = origin {
                if let Some(found) = spaces()
                    .into_iter()
                    .find(|space| field(space, "workspace_id") == workspace)
                {
                    mark_space(&found, Some(origin));
                }
                return Some(format!(
                    "opened the space {feature}, marked {}",
                    origin_mark(&origin.name)
                ));
            }
            if let Some(stays) = stays {
                let label = record::orchestrator_label(&repo::main_dir(), stays);
                return Some(format!("opened the space {feature}, this tab stays in {label}"));
            }
            if let Some(tab) = tabs_of(&workspace)
                .into_iter()
                .find(|tab| field(tab, "label") == feature)
            {
                herdr(&["tab", "focus", field(&tab, "tab_id")]);
            }
            return Some(format!("opened the space {feature}"));
        }
        let workspace = worktree_space(dir, false)?;
        let pane = current_pane()?;
        let moved = herdr(&[
            "pane",
            "move",
            field(&pane, "pane_id"),
            "--workspace",
            &workspace,
            "--new-tab",
        ]);
        let created = created_tab(&moved)?;
        herdr(&["tab", "rename", &created, feature]);
        drop_duplicates(&workspace, feature);
        ensure_editor(&workspace, dir, &tabs_of(&workspace));
        keep_order(&workspace, &[ROOT, OVERVIEW, feature, EDITOR]);
        sync_stack_tabs(dir, Some(workspace.clone()));
        let had_root = space_of(&repo::main_dir())
            .map(|main| tabs_of(&main).iter().any(|tab| field(tab, "label") == ROOT))
            .unwrap_or(false);
        main_space();
        Some(format!(
            "this tab moved into the space {feature}{}",
            if had_root {
                String::new()
            } else {
                format!(", a new tab {ROOT} waits in {SPACE}")
            }
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_up_to_init() {
        let chain = ancestors_of(std::process::id());
        assert_eq!(chain.first(), Some(&std::process::id()));
        assert!(chain.len() >= 2, "{chain:?}");
    }

    #[test]
    fn moves_only_an_agents_free_tab() {
        let origin = Origin {
            name: "x".into(),
            slot: 0,
        };
        assert!(moves_tab(true, None, None));
        assert!(!moves_tab(false, None, None));
        assert!(!moves_tab(true, Some(&origin), None));
        assert!(!moves_tab(true, None, Some("x")));
    }

    #[test]
    fn names_the_slot_tokens() {
        assert_eq!(origin_token(0), "orch1");
        let tokens = serde_json::json!({ "orch2": "⇶ x" });
        assert_eq!(slot_of_tokens(Some(&tokens)), Some(1));
        assert_eq!(slot_of_tokens(None), None);
    }
}
