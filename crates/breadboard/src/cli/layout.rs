//! herdr's layout (`wt:herdr`; layout.mjs, spaces.mjs, tabs.mjs, prune.mjs
//! `pruneSpaces`, origins.mjs `markOrigins`, order.mjs `sortSpaces`):
//!
//! ```text
//! Monocircuit/Platform   root · overview (of everything, --herdr) · changelog · stack tabs
//! ├─ <feature>           root · overview (this worktree) · <feature> (its conversation) · nvim · stack tabs
//! └─ ⇶ <title>           root · overview · orchestrator · changelog · changes
//! ```
//!
//! Built from `git worktree list`, idempotent: the way back after a herdr
//! restart or tabs closed by hand. The main space's overview (`--herdr`)
//! keeps it in step: it prunes and sorts on every read, lays out anew when a
//! worktree comes or goes, and follows a stack up or down with its tabs.
//! The herdr writes are exactly Node's (rule 6); outside herdr nothing runs.
//! The commands a new tab runs are this binary's (`<exe> overview --herdr`),
//! the names of the root package.json (rule 5).

mod order;

pub use order::sort_spaces;

use super::remove::{prune_orchestrators, release_space};
use breadboard_core::conversations::{bound_session, claude_dir, running_claudes, transcript_of};
use breadboard_core::git::{self, feature_of, git, orchestrations_dir, orchestrator_checkouts, worktrees};
use breadboard_core::herdr::{Pane, Tab, herdr, is_idle, is_in_herdr, panes_in, panes_of, tabs_of};
use breadboard_core::local::{ORCHESTRATOR_TAB, env_file_of, is_port_taken, offset_of, read_env};
use breadboard_core::lock::{layout_lock, with_dir_lock};
use breadboard_core::model::develop::VITA;
use breadboard_core::record::{
    self, ORCHESTRATOR, Origin, SLOTS, SPACE, orchestration_of_label, orchestrator_label, origin_of, record_of,
    title_of,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

pub const ROOT: &str = "root";
pub const OVERVIEW: &str = "overview";
pub const CHANGELOG: &str = "changelog";
/// The tab of an orchestration's space with nvim on what its MRs changed.
pub const CHANGES: &str = "changes";
/// The tab of a worktree's space with nvim open in the worktree.
pub const EDITOR: &str = "nvim";
const DEV: &str = "dev";
const STUDIO: &str = "studio";
/// drizzle studio's default port; a worktree's moves by its offset.
const STUDIO_PORT: i64 = 4983;
/// Whose sidebar tokens they are: the plugin's id.
const TOKEN_SOURCE: &str = "monocircuit.platform";
/// herdr's own `branch` stays empty for a linked worktree (herdrdev/herdr#2952).
const BRANCH_TOKEN: &str = "wt_branch";
/// A layout's lock left by a killed process counts as gone after this long.
const LOCK_STALE: Duration = Duration::from_secs(60);
/// A transcript written to within this time may belong to a running conversation: not moved.
const LIVE: Duration = Duration::from_secs(10 * 60);
/// claudecode.nvim, pinned (ide.mjs `PLUGIN`).
const PLUGIN_URL: &str = "https://github.com/coder/claudecode.nvim";
const PLUGIN_TAG: &str = "v0.3.0";
const PLUGIN_COMMIT: &str = "2e6ea6f2a63cdf4fd3c05e6a054151d46848d319";

// ---------------------------------------------------------------------------
// herdr, as lib/herdr.mjs answers it
// ---------------------------------------------------------------------------

/// `herdr <args>` → its result, None when herdr refused (Node's `herdr`).
fn call(args: &[&str]) -> Option<Value> {
    herdr(args)
}

/// `herdr <args>` for its effect: whether herdr did it, some answer without a result (Node's `herdrDid`).
fn did(args: &[&str]) -> bool {
    Command::new("herdr")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `workspace list` as herdr answers it: with tokens, worktree and focus.
fn listed() -> Vec<Value> {
    call(&["workspace", "list"])
        .and_then(|result| result.get("workspaces").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

fn id_of(space: &Value) -> &str {
    text(space, "/workspace_id").unwrap_or_default()
}

fn label_of(space: &Value) -> &str {
    text(space, "/label").unwrap_or_default()
}

fn is_linked(space: &Value) -> bool {
    space
        .pointer("/worktree/is_linked_worktree")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The id of the space labelled `label`.
fn space_id(label: &str) -> Option<String> {
    listed()
        .iter()
        .find(|space| label_of(space) == label)
        .map(|space| id_of(space).to_string())
}

/// The label of a checkout's space: the main space for the main checkout.
fn space_label(main: &Path, dir: &Path) -> String {
    if dir == main {
        SPACE.to_string()
    } else {
        feature_of(main, dir)
    }
}

/// A word for sh, bash, zsh and fish alike (proc.mjs `quote`).
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The pids from this process up to init, from Linux's /proc (herdr.mjs `ancestorsOf`).
fn ancestors() -> Vec<u32> {
    let mut chain = Vec::new();
    let mut id = std::process::id();
    while id > 1 && chain.len() < 64 {
        chain.push(id);
        match fs::read_to_string(format!("/proc/{id}/stat"))
            .ok()
            .and_then(|stat| parent_in(&stat))
        {
            Some(parent) => id = parent,
            None => break,
        }
    }
    chain
}

/// The parent's pid in `/proc/<pid>/stat`: `<pid> (<comm>) <state> <ppid> …`, a name may hold `)`.
fn parent_in(stat: &str) -> Option<u32> {
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    rest.split(' ').nth(1)?.parse().ok()
}

/// The shell of a pane, its pid.
fn shell_of(pane: &str) -> Option<i64> {
    call(&["pane", "process-info", "--pane", pane])?
        .pointer("/process_info/shell_pid")
        .and_then(Value::as_i64)
}

/// The pane running this (herdr.mjs `currentPane`): herdr's answer when its
/// shell is an ancestor of this process, else the first pane whose is.
fn current_pane() -> Option<Pane> {
    let current =
        call(&["pane", "current", "--current"]).and_then(|result| result.get("pane").and_then(Pane::from_value));
    let chain = ancestors();
    if chain.len() < 2 {
        return current;
    }
    let is_ours = |pane: &Pane| shell_of(&pane.id).is_some_and(|pid| chain.iter().any(|id| i64::from(*id) == pid));
    if let Some(current) = current.filter(|pane| is_ours(pane)) {
        return Some(current);
    }
    panes_in(call(&["pane", "list"])).into_iter().find(|pane| is_ours(pane))
}

/// A tab no script closes: it runs this, holds several panes, or an agent (conversations.mjs `isBusy`).
fn is_busy(tab: &Tab, panes: &[Pane], current: Option<&Pane>) -> bool {
    current.is_some_and(|pane| pane.tab == tab.id)
        || tab.pane_count > 1
        || panes.iter().any(|pane| pane.tab == tab.id && pane.agent.is_some())
}

/// What a new tab runs, made only when it is needed (the editor's may clone a plugin).
type Run<'a> = Option<&'a dyn Fn() -> String>;

/// The single pane of the tab `label` when its shell idles (tabs.mjs `idlePaneOf`).
fn idle_pane_of(workspace: &str, label: &str, tabs: &[Tab]) -> Option<String> {
    let tab = tabs.iter().find(|tab| tab.label == label)?;
    if tab.pane_count != 1 {
        return None;
    }
    let pane = panes_of(workspace).into_iter().find(|pane| pane.tab == tab.id)?;
    is_idle(&pane.id).then_some(pane.id)
}

/// A tab by its label, made unfocused with `command` running in it when it
/// is missing, `command` run again when its shell idles (tabs.mjs `ensureTab`).
fn ensure_tab(workspace: &str, label: &str, cwd: &Path, command: Run, tabs: &[Tab]) -> bool {
    if tabs.iter().any(|tab| tab.label == label) {
        let Some(command) = command else { return false };
        return idle_pane_of(workspace, label, tabs)
            .is_some_and(|pane| call(&["pane", "run", &pane, &command()]).is_some());
    }
    let cwd = cwd.to_string_lossy();
    let args = [
        "tab",
        "create",
        "--workspace",
        workspace,
        "--cwd",
        &cwd,
        "--label",
        label,
        "--no-focus",
    ];
    let Some(created) = call(&args) else { return false };
    if let Some(command) = command
        && let Some(pane) = text(&created, "/root_pane/pane_id")
    {
        call(&["pane", "run", pane, &command()]);
    }
    true
}

/// A space by its label, made with the tab `first` (and its command) when missing (tabs.mjs `ensureSpace`).
fn ensure_space(label: &str, cwd: &Path, first: &str, command: Option<&str>) -> Option<String> {
    if let Some(found) = space_id(label) {
        return Some(found);
    }
    let cwd = cwd.to_string_lossy();
    let created = call(&["workspace", "create", "--label", label, "--cwd", &cwd, "--no-focus"])?;
    let workspace = text(&created, "/workspace/workspace_id")?.to_string();
    if let Some(tab) = text(&created, "/tab/tab_id") {
        call(&["tab", "rename", tab, first]);
    }
    if let (Some(command), Some(pane)) = (command, text(&created, "/root_pane/pane_id")) {
        call(&["pane", "run", pane, command]);
    }
    Some(workspace)
}

/// The space of a worktree in the group of the main space `main` (tabs.mjs
/// `ensureWorktreeSpace`): herdr links it only when `worktree open` opens it;
/// one standing alone is taken into the group with what runs in it.
fn ensure_worktree_space(label: &str, dir: &Path, main: Option<&str>, first: &str) -> Option<String> {
    let found = listed().into_iter().find(|space| label_of(space) == label);
    let found_id = found.as_ref().map(|space| id_of(space).to_string());
    let Some(main) = main.filter(|_| !found.as_ref().is_some_and(is_linked)) else {
        return found_id.or_else(|| ensure_space(label, dir, first, None));
    };
    let path = dir.to_string_lossy();
    // herdr takes the main space for the worktree's while its first tab stands in it (herdrdev/herdr#4293).
    let is_main_taken = call(&["worktree", "list", "--workspace", main])
        .and_then(|result| result.get("worktrees").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .any(|tree| text(tree, "/path") == Some(&path) && text(tree, "/open_workspace_id") == Some(main));
    let opened = (!is_main_taken)
        .then(|| {
            call(&[
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
        })
        .flatten();
    let Some(workspace) = opened
        .as_ref()
        .and_then(|opened| text(opened, "/workspace/workspace_id"))
        .map(String::from)
    else {
        return found_id.or_else(|| ensure_space(label, dir, first, None));
    };
    let opened = opened.unwrap_or_default();
    if opened.get("already_open").and_then(Value::as_bool) == Some(true) {
        return Some(workspace);
    }
    if let Some(tab) = text(&opened, "/tab/tab_id") {
        call(&["tab", "rename", tab, first]);
    }
    if let Some(from) = found_id {
        move_into(&from, &workspace);
        drop_duplicates(&workspace, first);
    }
    Some(workspace)
}

/// Every tab of `from` into `to`, in order and labelled, what runs keeps
/// running; `from` closes once empty (tabs.mjs `moveInto`).
fn move_into(from: &str, to: &str) {
    let panes = panes_of(from);
    for tab in tabs_of(from) {
        let own: Vec<&Pane> = panes.iter().filter(|pane| pane.tab == tab.id).collect();
        let Some((head, rest)) = own.split_first() else {
            continue;
        };
        let moved = call(&["pane", "move", &head.id, "--workspace", to, "--new-tab", "--no-focus"]);
        let Some(created) = moved
            .as_ref()
            .and_then(|moved| text(moved, "/move_result/created_tab/tab_id"))
        else {
            continue;
        };
        call(&["tab", "rename", created, &tab.label]);
        for pane in rest {
            call(&[
                "pane",
                "move",
                &pane.id,
                "--tab",
                created,
                "--split",
                "right",
                "--no-focus",
            ]);
        }
    }
    if listed().iter().any(|space| id_of(space) == from) && tabs_of(from).is_empty() {
        call(&["workspace", "close", from]);
    }
}

/// One tab `label` in the space: spare ones (one pane, no agent, an idle
/// shell) close; when all are spare the first stays (tabs.mjs `dropDuplicates`).
fn drop_duplicates(workspace: &str, label: &str) {
    let tabs: Vec<Tab> = tabs_of(workspace)
        .into_iter()
        .filter(|tab| tab.label == label)
        .collect();
    if tabs.len() < 2 {
        return;
    }
    let panes = panes_of(workspace);
    let spare: Vec<&Tab> = tabs
        .iter()
        .filter(|tab| {
            let own: Vec<&Pane> = panes.iter().filter(|pane| pane.tab == tab.id).collect();
            own.len() == 1 && own[0].agent.is_none() && is_idle(&own[0].id)
        })
        .collect();
    let skip = usize::from(spare.len() == tabs.len());
    for tab in spare.iter().skip(skip) {
        call(&["tab", "close", &tab.id]);
    }
}

/// Which of `labels` (a space's tabs in order) move to the end so `order`
/// stands at the left (tabs.mjs `keepOrder`): indices, in the order they move.
fn moving_of(labels: &[&str], order: &[&str]) -> Vec<usize> {
    let wanted: Vec<&str> = order.iter().copied().filter(|label| labels.contains(label)).collect();
    let mut fixed = 0;
    while fixed < wanted.len() && labels.get(fixed) == Some(&wanted[fixed]) {
        fixed += 1;
    }
    if fixed == wanted.len() {
        return Vec::new();
    }
    let rest: Vec<usize> = (fixed..labels.len()).collect();
    let named: Vec<usize> = rest
        .iter()
        .copied()
        .filter(|at| wanted.contains(&labels[*at]))
        .collect();
    let others: Vec<usize> = rest
        .iter()
        .copied()
        .filter(|at| !wanted.contains(&labels[*at]))
        .collect();
    let is_in_order = named
        .iter()
        .enumerate()
        .all(|(i, at)| wanted.get(fixed + i) == Some(&labels[*at]));
    if is_in_order {
        let last = named.last().copied();
        return others
            .into_iter()
            .filter(|at| last.is_some_and(|last| *at < last))
            .collect();
    }
    wanted[fixed..]
        .iter()
        .filter_map(|label| named.iter().copied().find(|at| labels[*at] == *label))
        .chain(others)
        .collect()
}

/// The tabs `order` names at the left of their space, the others behind
/// (tabs.mjs `keepOrder`): herdr can't reorder, a tab out of place moves its
/// pane into a new tab at the end; focus goes back to where it was.
fn keep_order(workspace: &str, order: &[&str]) {
    let tabs = tabs_of(workspace);
    let labels: Vec<&str> = tabs.iter().map(|tab| tab.label.as_str()).collect();
    let moving = moving_of(&labels, order);
    if moving.is_empty() {
        return;
    }
    let before = listed();
    let panes = panes_of(workspace);
    let mut renamed: HashMap<String, String> = HashMap::new();
    for at in moving {
        let tab = &tabs[at];
        if tab.pane_count != 1 {
            break;
        }
        let Some(pane) = panes.iter().find(|pane| pane.tab == tab.id) else {
            break;
        };
        let moved = call(&["pane", "move", &pane.id, "--workspace", workspace, "--new-tab"]);
        let Some(created) = moved
            .as_ref()
            .and_then(|moved| text(moved, "/move_result/created_tab/tab_id"))
            .map(String::from)
        else {
            break;
        };
        call(&["tab", "rename", &created, &tab.label]);
        renamed.insert(tab.id.clone(), created);
    }
    if renamed.is_empty() {
        return;
    }
    let now = |tab: &str| renamed.get(tab).cloned().unwrap_or_else(|| tab.to_string());
    let own = before
        .iter()
        .find(|space| id_of(space) == workspace)
        .and_then(|space| text(space, "/active_tab_id"));
    let focused = before
        .iter()
        .find(|space| space.get("focused").and_then(Value::as_bool) == Some(true))
        .and_then(|space| text(space, "/active_tab_id"));
    for tab in [own, focused].into_iter().flatten() {
        call(&["tab", "focus", &now(tab)]);
    }
}

// ---------------------------------------------------------------------------
// What the tabs run
// ---------------------------------------------------------------------------

/// This binary, for the commands of the tabs it builds.
fn exe() -> String {
    std::env::current_exe()
        .map(|exe| exe.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "breadboard".into())
}

/// The live overview a tab runs: of everything with `--herdr`, of a worktree, of an orchestration.
fn overview_command(dir: Option<&Path>, orchestration: Option<&str>) -> String {
    let what = match (orchestration, dir) {
        (Some(name), _) => format!("--orchestration {name}"),
        (None, Some(dir)) => format!("--worktree {}", quote(&dir.to_string_lossy())),
        (None, None) => "--herdr".into(),
    };
    format!("{} overview {what}", quote(&exe()))
}

/// Whether nvim runs at all here (spaces.mjs `canEdit`), asked once.
fn can_edit() -> bool {
    static HAS_NVIM: OnceLock<bool> = OnceLock::new();
    *HAS_NVIM.get_or_init(|| {
        Command::new("nvim")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// The shell line of an nvim tab in `dir` on the socket of `name` (nvim.mjs `editorCommand`).
fn editor_command(main: &Path, dir: &Path, name: Option<&str>, commands: &[String], ide: Option<&Path>) -> String {
    let socket = quote(&match name {
        Some(name) => format!(".claude/nvim-{name}.sock"),
        None => ".claude/nvim.sock".into(),
    });
    let json = |path: &Path| serde_json::to_string(&path.to_string_lossy()).unwrap_or_default();
    let ide = ide.map(|plugin| {
        format!(
            "lua dofile({})({})",
            json(&main.join("packages/breadboard/nvim/ide.lua")),
            json(plugin)
        )
    });
    let start: String = commands
        .iter()
        .chain(ide.as_ref())
        .map(|command| format!(" -c {}", quote(command)))
        .collect();
    format!(
        "cd {}; nvim --server {socket} --remote-expr 1 >/dev/null 2>&1 || rm -f {socket}; nvim --listen {socket}{start}",
        quote(&dir.to_string_lossy())
    )
}

/// Whether `git config breadboard.ide` leaves Claude Code's IDE plugin on for `dir` (ide.mjs `isIdeOn`).
fn is_ide_on(dir: &Path) -> bool {
    let value = git(dir, &["config", "breadboard.ide"]).stdout;
    !["false", "off", "no", "0"].contains(&value.trim().to_lowercase().as_str())
}

/// claudecode.nvim under the main checkout, cloned first when missing; None
/// when the clone fails or the tag moved (ide.mjs `ensurePlugin`).
fn ensure_plugin(main: &Path) -> Option<PathBuf> {
    let dir = main.join(".claude/nvim").join(format!("claudecode.nvim-{PLUGIN_TAG}"));
    if dir.join("lua/claudecode").exists() {
        return Some(dir);
    }
    fs::create_dir_all(dir.parent()?).ok()?;
    let partial = PathBuf::from(format!("{}.partial", dir.display()));
    let _ = fs::remove_dir_all(&partial);
    let partial_text = partial.to_string_lossy();
    let is_cloned = git(
        main,
        &[
            "-c",
            "advice.detachedHead=false",
            "clone",
            "--quiet",
            "--depth",
            "1",
            "--branch",
            PLUGIN_TAG,
            PLUGIN_URL,
            &partial_text,
        ],
    )
    .is_ok;
    if !is_cloned || git(&partial, &["rev-parse", "HEAD"]).stdout.trim() != PLUGIN_COMMIT {
        let _ = fs::remove_dir_all(&partial);
        return None;
    }
    fs::rename(&partial, &dir).ok()?;
    Some(dir)
}

/// The tab `nvim` of a worktree's space, nvim on its changes (spaces.mjs `ensureEditor`).
fn ensure_editor(main: &Path, workspace: &str, dir: &Path, tabs: &[Tab]) {
    if !can_edit() {
        return;
    }
    let command = || {
        let ide = is_ide_on(dir).then(|| ensure_plugin(main)).flatten();
        let changes = format!(
            "luafile {}",
            main.join("packages/breadboard/nvim/changes.lua").display()
        );
        editor_command(main, dir, None, &[changes], ide.as_deref())
    };
    ensure_tab(workspace, EDITOR, dir, Some(&command), tabs);
}

/// ` --ide` for a claude in a checkout with the IDE plugin on (ide.mjs `ideFlag`).
fn ide_flag(dir: &Path) -> &'static str {
    if is_ide_on(dir) { " --ide" } else { "" }
}

/// Whether a claude runs the conversation already, anywhere on this machine.
fn is_conversation_running(id: &str) -> bool {
    running_claudes().iter().any(|claude| claude.id == id)
}

// ---------------------------------------------------------------------------
// The spaces
// ---------------------------------------------------------------------------

/// The main space: `root`, `overview` and `changelog`; `root` comes back
/// whenever it is missing, an older `develop` is renamed (spaces.mjs `mainSpace`).
fn main_space(main: &Path) -> Option<String> {
    let workspace = ensure_space(SPACE, main, ROOT, None)?;
    let mut tabs = tabs_of(&workspace);
    if let Some(old) = tabs.iter().find(|tab| tab.label == "develop")
        && !tabs.iter().any(|tab| tab.label == ROOT)
    {
        call(&["tab", "rename", &old.id, ROOT]);
        tabs = tabs_of(&workspace);
    }
    let overview = || overview_command(None, None);
    ensure_tab(&workspace, OVERVIEW, main, Some(&overview), &tabs);
    ensure_tab(&workspace, ROOT, main, None, &tabs);
    let changelog = || format!("{} changelog --watch", quote(&exe()));
    ensure_tab(&workspace, CHANGELOG, main, Some(&changelog), &tabs);
    keep_order(&workspace, &[ROOT, OVERVIEW, CHANGELOG]);
    Some(workspace)
}

/// The branch of a worktree as the sidebar token `$wt_branch`, on a change (tabs.mjs `markBranch`).
fn mark_branch(main: &Path, workspace: &str, dir: &Path) {
    let branch = worktrees(main)
        .into_iter()
        .find(|tree| tree.dir == dir)
        .and_then(|tree| tree.branch);
    let Some(branch) = branch else { return };
    let Some(found) = listed().into_iter().find(|space| id_of(space) == workspace) else {
        return;
    };
    if text(&found, &format!("/tokens/{BRANCH_TOKEN}")) == Some(branch.as_str()) {
        return;
    }
    let token = format!("{BRANCH_TOKEN}={branch}");
    call(&[
        "workspace",
        "report-metadata",
        workspace,
        "--source",
        TOKEN_SOURCE,
        "--token",
        &token,
    ]);
}

/// The space of a worktree: `root`, its `overview`, its conversation's tab
/// and `nvim`, in that order (spaces.mjs `worktreeSpace`).
fn worktree_space(main: &Path, dir: &Path, has_work_tab: bool, should_resume: bool) -> Option<String> {
    let feature = feature_of(main, dir);
    let workspace = ensure_worktree_space(&feature, dir, space_id(SPACE).as_deref(), ROOT)?;
    mark_branch(main, &workspace, dir);
    let tabs = tabs_of(&workspace);
    let overview = || overview_command(Some(dir), None);
    ensure_tab(&workspace, OVERVIEW, dir, Some(&overview), &tabs);
    if has_work_tab {
        ensure_tab(&workspace, &feature, dir, None, &tabs);
    }
    ensure_tab(&workspace, ROOT, dir, None, &tabs);
    if has_work_tab {
        ensure_editor(main, &workspace, dir, &tabs);
    }
    drop_duplicates(&workspace, &feature);
    if should_resume {
        resume_session(main, dir, &workspace);
    }
    keep_order(&workspace, &[ROOT, OVERVIEW, &feature, EDITOR]);
    Some(workspace)
}

/// After a restart the conversation bound to a worktree's space comes back in
/// its idle tab (conversations.mjs `resumeSession`).
fn resume_session(main: &Path, dir: &Path, workspace: &str) -> bool {
    let Some(id) = bound_session(dir) else { return false };
    if panes_of(workspace).iter().any(|pane| pane.agent.is_some()) || is_conversation_running(&id) {
        return false;
    }
    let Some(idle) = idle_pane_of(workspace, &feature_of(main, dir), &tabs_of(workspace)) else {
        return false;
    };
    let cwd = session_cwd(&id, dir);
    let cd = if cwd == dir {
        String::new()
    } else {
        format!("cd {} && ", quote(&cwd.to_string_lossy()))
    };
    let command = format!("{cd}claude --resume {id}{}", ide_flag(dir));
    call(&["pane", "run", &idle, &command]).is_some()
}

/// Where a conversation last worked, from its transcript; else `dir`.
fn session_cwd(id: &str, dir: &Path) -> PathBuf {
    let last = transcript_of(id)
        .and_then(|file| fs::read_to_string(file).ok())
        .and_then(|text| {
            text.rmatch_indices("\"cwd\":\"").next().and_then(|(at, key)| {
                let rest = &text[at + key.len()..];
                rest.find('"').map(|end| rest[..end].to_string())
            })
        });
    last.map(PathBuf::from)
        .filter(|last| last.exists())
        .unwrap_or_else(|| dir.to_path_buf())
}

/// The checkout an orchestration's space stands on, made on first use (orchestrator.mjs `orchestratorCheckout`).
fn orchestrator_checkout(main: &Path, name: &str) -> Option<PathBuf> {
    let dir = orchestrations_dir(main).join(name);
    if orchestrator_checkouts(main).contains(&dir) {
        return Some(dir);
    }
    fs::create_dir_all(dir.parent()?).ok()?;
    let path = dir.to_string_lossy();
    git(
        main,
        &["worktree", "add", "--quiet", "--detach", &path, "origin/develop"],
    )
    .is_ok
    .then_some(dir)
}

/// The space `⇶ <title>` (orchestrator.mjs `orchestratorSpace`): `root`,
/// its `overview`, the orchestrator's tab, `changelog` and `changes`.
fn orchestrator_space(main: &Path, name: &str, should_resume: bool) -> Option<String> {
    let label = orchestrator_label(main, name);
    let titles = record::titles(main);
    let all = listed();
    if !all.iter().any(|space| label_of(space) == label)
        && let Some(retitled) = all.iter().find(|space| {
            label_of(space).starts_with(&format!("{ORCHESTRATOR} "))
                && orchestration_of_label(label_of(space), &titles).as_deref() == Some(name)
        })
    {
        call(&["workspace", "rename", id_of(retitled), &label]);
    }
    let workspace = match orchestrator_checkout(main, name) {
        Some(checkout) => ensure_worktree_space(&label, &checkout, space_id(SPACE).as_deref(), ROOT),
        None => ensure_space(&label, main, ROOT, None),
    }?;
    // An older space of the orchestration hands its tabs over; its overview starts anew here.
    for old in listed() {
        let old_id = id_of(&old);
        if old_id == workspace || orchestration_of_label(label_of(&old), &titles).as_deref() != Some(name) {
            continue;
        }
        if let Some(overview) = tabs_of(old_id).into_iter().find(|tab| tab.label == OVERVIEW) {
            call(&["tab", "close", &overview.id]);
        }
        move_into(old_id, &workspace);
        drop_duplicates(&workspace, ROOT);
    }
    let tabs = tabs_of(&workspace);
    let overview = || overview_command(None, Some(name));
    ensure_tab(&workspace, OVERVIEW, main, Some(&overview), &tabs);
    ensure_tab(&workspace, ROOT, main, None, &tabs);
    let changelog = || format!("{} wt:orchestrate {name} --changelog --watch", quote(&exe()));
    ensure_tab(&workspace, CHANGELOG, main, Some(&changelog), &tabs);
    if can_edit() {
        let changes = || {
            let lua = main.join("packages/breadboard/nvim/orchestration.lua");
            let commands = [
                format!("luafile {}", lua.display()),
                format!("OrchestrationChanges {name}"),
            ];
            editor_command(main, main, Some(name), &commands, None)
        };
        ensure_tab(&workspace, CHANGES, main, Some(&changes), &tabs);
    }
    keep_order(&workspace, &[ROOT, OVERVIEW, ORCHESTRATOR_TAB, CHANGELOG, CHANGES]);
    if should_resume {
        resume_free(main, name, &workspace);
    }
    Some(workspace)
}

/// After a restart the free agents of an orchestration run again in their
/// tabs `free <agent>`, in its checkout (free.mjs `resumeFree`).
fn resume_free(main: &Path, name: &str, workspace: &str) {
    let checkout = orchestrations_dir(main).join(name);
    if !checkout.exists() {
        return;
    }
    let tabs = tabs_of(workspace);
    let Some(record) = record_of(main, name) else { return };
    for worker in record.free_agents() {
        if is_conversation_running(&worker.id) {
            continue;
        }
        let label = format!("free {}", worker.name.clone().unwrap_or_default());
        let command = || {
            format!(
                "cd {} && claude --resume {}",
                quote(&checkout.to_string_lossy()),
                worker.id
            )
        };
        ensure_tab(workspace, &label, &checkout, Some(&command), &tabs);
    }
}

/// After a restart the orchestrator's recorded conversation comes back in its
/// idle tab; a lone claude in the space becomes the orchestrator
/// (orchestrator-chat.mjs `restoreChat` without a click). What it did.
fn restore_chat(main: &Path, name: &str) -> Option<String> {
    let workspace = space_id(&orchestrator_label(main, name))?;
    let chat = record_of(main, name).and_then(|record| record.chat);
    let tabs = tabs_of(&workspace);
    let panes = panes_of(&workspace);
    let all = panes_in(call(&["pane", "list"]));
    let session = chat.as_ref().map(|chat| chat.session_id.as_str());
    if session.is_some_and(|id| all.iter().any(|pane| pane.session.as_deref() == Some(id))) {
        return None;
    }
    let Some(tab) = tabs.iter().find(|tab| tab.label == ORCHESTRATOR_TAB) else {
        let free: Vec<&str> = tabs
            .iter()
            .filter(|tab| tab.label.starts_with("free "))
            .map(|tab| tab.id.as_str())
            .collect();
        let claudes: Vec<&Pane> = panes
            .iter()
            .filter(|pane| {
                pane.agent.as_deref() == Some("claude") && pane.session.is_some() && !free.contains(&pane.tab.as_str())
            })
            .collect();
        if let [lone] = claudes.as_slice() {
            call(&["tab", "rename", &lone.tab, ORCHESTRATOR_TAB]);
            let cwd = running_claudes()
                .into_iter()
                .find(|claude| Some(&claude.id) == lone.session.as_ref())
                .map(|claude| claude.cwd)
                .or_else(|| lone.cwd.clone().map(PathBuf::from))
                .unwrap_or_else(|| main.to_path_buf());
            let fields = serde_json::json!({ "orchestrator": { "sessionId": lone.session, "cwd": cwd } });
            let _ = record::update_record(main, name, fields.as_object().cloned().unwrap_or_default());
        }
        return None;
    };
    let own: Vec<&Pane> = panes.iter().filter(|pane| pane.tab == tab.id).collect();
    let chat = chat?;
    if own.iter().any(|pane| pane.agent.is_some()) || own.len() != 1 || !is_idle(&own[0].id) {
        return None;
    }
    if is_conversation_running(&chat.session_id) {
        return Some(format!(
            "the conversation {} runs elsewhere, outside herdr's panes",
            chat.session_id
        ));
    }
    let cwd = chat
        .cwd
        .clone()
        .filter(|cwd| !cwd.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| main.to_path_buf());
    let command = format!(
        "cd {} && claude --resume {}{}",
        quote(&cwd.to_string_lossy()),
        chat.session_id,
        ide_flag(&cwd)
    );
    if !did(&["pane", "run", &own[0].id, &command]) {
        return None;
    }
    Some(format!("resumed {} in its tab", chat.session_id))
}

// ---------------------------------------------------------------------------
// Stack tabs
// ---------------------------------------------------------------------------

/// `docker compose` of a checkout's stack, its trimmed output; empty when it fails.
fn compose(dir: &Path, args: &[&str]) -> String {
    Command::new("docker")
        .args(["compose", "--env-file", ".env.local", "-f", "docker-compose.local.yml"])
        .args(args)
        .current_dir(dir.join(VITA))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default()
}

/// While the stack of `dir` runs its space shows it: a tab per container
/// following its logs, `dev` and `studio` when their port is free; when it
/// is down they close unless busy (layout.mjs `syncStackTabs`). `+postgres`, `-dev`.
fn stack_tabs(main: &Path, dir: &Path, workspace: Option<&str>) -> Vec<String> {
    let Some(workspace) = workspace else { return Vec::new() };
    let env_file = env_file_of(dir);
    if !env_file.exists() {
        return Vec::new();
    }
    let offset = if dir == main {
        0
    } else {
        offset_of(read_env(&env_file).as_ref())
    };
    if dir != main && offset == 0 {
        return Vec::new();
    }
    let vita = dir.join(VITA);
    let services: Vec<String> = compose(dir, &["config", "--services"])
        .lines()
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();
    let is_up = !compose(dir, &["ps", "--status", "running", "-q"]).is_empty();
    let tabs = tabs_of(workspace);
    let mut changed = Vec::new();
    if is_up {
        let logs = "docker compose --env-file .env.local -f docker-compose.local.yml logs -f --tail 200";
        let mut wanted: Vec<(String, String)> = services
            .iter()
            .map(|service| (service.clone(), format!("{logs} {service}")))
            .collect();
        if !is_port_taken(3000 + offset) {
            wanted.push((DEV.into(), "pnpm dev".into()));
        }
        if !is_port_taken(STUDIO_PORT + offset) {
            wanted.push((STUDIO.into(), format!("pnpm pg:studio --port {}", STUDIO_PORT + offset)));
        }
        for (label, command) in wanted {
            let run = || command.clone();
            if ensure_tab(workspace, &label, &vita, Some(&run), &tabs) {
                changed.push(format!("+{label}"));
            }
        }
        return changed;
    }
    let panes = panes_of(workspace);
    let current = current_pane();
    for tab in &tabs {
        let is_ours = services.contains(&tab.label) || tab.label == DEV || tab.label == STUDIO;
        if !is_ours || is_busy(tab, &panes, current.as_ref()) {
            continue;
        }
        if call(&["tab", "close", &tab.id]).is_some() {
            changed.push(format!("-{}", tab.label));
        }
    }
    changed
}

/// `run` as one change of the layout at a time, across every process of the
/// user (lock.mjs `exclusive`): two at once would both make a missing tab.
fn exclusive<T>(run: impl FnOnce() -> T) -> T {
    let mut run = Some(run);
    let mut go = || run.take().map(|run| run());
    match with_dir_lock(&layout_lock().with_extension(""), LOCK_STALE, &mut go) {
        Ok(Some(out)) => out,
        // The lock could not be made ($TMPDIR gone): the layout is comfort, it runs anyway.
        _ => go().expect("the lock fails before it runs"),
    }
}

/// The stack tabs of one checkout, after its stack came up or went down (the `--herdr` loop).
pub fn sync_stack_tabs(main: &Path, dir: &Path) -> Vec<String> {
    if !is_in_herdr() {
        return Vec::new();
    }
    exclusive(|| stack_tabs(main, dir, space_id(&space_label(main, dir)).as_deref()))
}

// ---------------------------------------------------------------------------
// Marks, pruning, conversations
// ---------------------------------------------------------------------------

/// Every worktree an orchestration started, with its origin.
fn orchestrated(main: &Path) -> Vec<(PathBuf, Origin)> {
    worktrees(main)
        .into_iter()
        .skip(1)
        .filter_map(|tree| origin_of(&tree.dir).map(|origin| (tree.dir, origin)))
        .collect()
}

fn origin_token(slot: u32) -> String {
    format!("orch{}", slot + 1)
}

/// The slot a space's tokens show.
fn slot_of_tokens(space: &Value) -> Option<u32> {
    (0..SLOTS).find(|slot| {
        space
            .pointer(&format!("/tokens/{}", origin_token(*slot)))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    })
}

/// The colour slot of an orchestration: the one its worktrees or its space
/// carry, else the least used (origins.mjs `slotOf`).
fn slot_of(main: &Path, name: &str, titles: &[(String, String)]) -> u32 {
    let trees = orchestrated(main);
    if let Some((_, origin)) = trees.iter().find(|(_, origin)| origin.name == name) {
        return origin.slot;
    }
    let marked: Vec<Value> = listed()
        .into_iter()
        .filter(|space| orchestration_of_label(label_of(space), titles).is_some())
        .collect();
    if let Some(slot) = marked
        .iter()
        .find(|space| orchestration_of_label(label_of(space), titles).as_deref() == Some(name))
        .and_then(slot_of_tokens)
    {
        return slot;
    }
    let mut uses = vec![0u32; SLOTS as usize];
    for (_, origin) in &trees {
        uses[origin.slot as usize] += 1;
    }
    for slot in marked.iter().filter_map(slot_of_tokens) {
        uses[slot as usize] += 1;
    }
    let least = uses.iter().copied().min().unwrap_or(0);
    uses.iter().position(|uses| *uses == least).unwrap_or(0) as u32
}

/// A space's origin tokens as `origin` wants them: its slot's set, the others cleared (origins.mjs `markSpace`).
fn mark_space(main: &Path, space: &Value, origin: Option<&Origin>) {
    let mut args: Vec<String> = Vec::new();
    for slot in 0..SLOTS {
        let token = origin_token(slot);
        let wanted = origin
            .filter(|origin| origin.slot == slot)
            .map(|origin| format!("{ORCHESTRATOR} {}", title_of(main, &origin.name)));
        let shown = space
            .pointer(&format!("/tokens/{token}"))
            .and_then(Value::as_str)
            .filter(|shown| !shown.is_empty());
        match wanted {
            Some(wanted) if shown != Some(wanted.as_str()) => {
                args.extend(["--token".into(), format!("{token}={wanted}")]);
            }
            None if shown.is_some() => args.extend(["--clear-token".into(), token]),
            _ => {}
        }
    }
    if args.is_empty() {
        return;
    }
    let mut all = vec!["workspace", "report-metadata", id_of(space), "--source", TOKEN_SOURCE];
    all.extend(args.iter().map(String::as_str));
    call(&all);
}

/// Every space shows the orchestration it belongs to (origins.mjs `markOrigins`).
fn mark_origins(main: &Path) {
    let titles = record::titles(main);
    let by_feature: HashMap<String, Origin> = orchestrated(main)
        .into_iter()
        .map(|(dir, origin)| (feature_of(main, &dir), origin))
        .collect();
    for space in listed() {
        let origin = match orchestration_of_label(label_of(&space), &titles) {
            Some(name) => Some(Origin {
                slot: slot_of(main, &name, &titles),
                name,
            }),
            None => by_feature.get(label_of(&space)).cloned(),
        };
        mark_space(main, &space, origin.as_ref());
    }
}

/// Whether every pane of a space stands in `dir` (a removed worktree's directory).
fn is_orphan(panes: &[Pane], dir: &str) -> bool {
    !panes.is_empty()
        && panes.iter().all(|pane| {
            pane.cwd.as_deref().is_some_and(|cwd| {
                cwd == dir || cwd.starts_with(&format!("{dir}/")) || cwd.starts_with(&format!("{dir} "))
            })
        })
}

/// The spaces of worktrees that are gone close, however they went: only a
/// space whose label names no worktree and whose every pane stands in that
/// deleted directory, a tab an agent works in stays (prune.mjs `pruneSpaces`);
/// then the orchestrations' spaces without worktrees. What closed.
pub fn prune_spaces(main: &Path) -> Vec<String> {
    if !is_in_herdr() {
        return Vec::new();
    }
    let live: HashSet<String> = worktrees(main)
        .iter()
        .map(|tree| space_label(main, &tree.dir))
        .collect();
    let mut closed = Vec::new();
    for space in listed() {
        let label = label_of(&space);
        if live.contains(label) {
            continue;
        }
        let dir = git::dir_of(main, label);
        if dir.exists() || !is_orphan(&panes_of(id_of(&space)), &dir.to_string_lossy()) {
            continue;
        }
        closed.extend(release_space(label, false));
    }
    closed.extend(prune_orchestrators(main));
    closed
}

/// The directory claude keeps a directory's transcripts in: every other character a `-`.
fn project_of(dir: &Path) -> PathBuf {
    let name: String = dir
        .to_string_lossy()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    claude_dir().join("projects").join(name)
}

/// Everything of `from` into `to`, folders merged, a file `to` has already
/// as `<name>.from-<suffix>`; true when `from` ended empty and is gone.
fn merge_into(from: &Path, to: &Path, suffix: &str) -> std::io::Result<bool> {
    for entry in fs::read_dir(from)?.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        match fs::symlink_metadata(&target) {
            Err(_) => fs::rename(&source, &target)?,
            Ok(existing) if entry.file_type()?.is_dir() && existing.is_dir() => {
                merge_into(&source, &target, suffix)?;
            }
            Ok(_) => {
                let aside = PathBuf::from(format!("{}.from-{suffix}", target.display()));
                if !aside.exists() {
                    fs::rename(&source, aside)?;
                }
            }
        }
    }
    if fs::read_dir(from)?.next().is_some() {
        return Ok(false);
    }
    fs::remove_dir(from)?;
    Ok(true)
}

/// Every conversation of the repo kept in the main checkout's project: a
/// worktree's project is a link to it, an older directory merged in once
/// none of its transcripts was written lately (conversations.mjs `shareConversations`).
fn share_conversations(main: &Path) -> Vec<String> {
    let link_projects = || -> std::io::Result<Vec<String>> {
        let home = project_of(main);
        let projects = home.parent().map(Path::to_path_buf).unwrap_or_default();
        fs::create_dir_all(&home)?;
        let home_name = home.file_name().map(|name| name.to_os_string()).unwrap_or_default();
        let link = |project: &Path| -> std::io::Result<()> {
            if fs::symlink_metadata(project).is_err() {
                std::os::unix::fs::symlink(&home_name, project)?;
            }
            Ok(())
        };
        for tree in worktrees(main).iter().skip(1) {
            link(&project_of(&tree.dir))?;
        }
        let prefix = format!("{}-", home_name.to_string_lossy());
        let mut merged = Vec::new();
        for entry in fs::read_dir(&projects)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type()?.is_dir() || !name.starts_with(&prefix) {
                continue;
            }
            let project = entry.path();
            let is_live = fs::read_dir(&project)?.flatten().any(|file| {
                file.file_name().to_string_lossy().ends_with(".jsonl")
                    && file
                        .metadata()
                        .and_then(|meta| meta.modified())
                        .is_ok_and(|at| SystemTime::now().duration_since(at).unwrap_or_default() < LIVE)
            });
            let suffix = &name[prefix.len()..];
            if is_live || !merge_into(&project, &home, suffix)? {
                continue;
            }
            link(&project)?;
            merged.push(suffix.to_string());
        }
        Ok(merged)
    };
    // Comfort like the spaces: a claude directory it can't arrange never fails the layout.
    link_projects().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The layout
// ---------------------------------------------------------------------------

/// The whole layout from `git worktree list` (layout.mjs `layout`): the main
/// space, the orchestrations' spaces, a space per worktree, the stack tabs;
/// tabs of an older layout in the main space move into their worktree's
/// space or close when their directory is gone. `should_resume` (after a
/// restart): conversations come back in their tabs. The main space's id and
/// a line per change; None when herdr does not answer.
pub fn layout(main: &Path, should_resume: bool) -> Option<(String, Vec<String>)> {
    if !is_in_herdr() {
        return None;
    }
    exclusive(|| build(main, should_resume))
}

fn build(main: &Path, should_resume: bool) -> Option<(String, Vec<String>)> {
    let main_ws = main_space(main)?;
    let mut report = Vec::new();
    let trees: Vec<PathBuf> = worktrees(main).into_iter().skip(1).map(|tree| tree.dir).collect();
    report.extend(prune_spaces(main));
    let merged = share_conversations(main);
    if !merged.is_empty() {
        report.push(format!(
            "conversations of {} moved to the project of {}",
            merged.join(", "),
            main.file_name().unwrap_or_default().to_string_lossy()
        ));
    }

    let panes = panes_of(&main_ws);
    let current = current_pane();
    for tab in tabs_of(&main_ws) {
        if [OVERVIEW, ROOT, CHANGELOG].contains(&tab.label.as_str()) {
            continue;
        }
        let own: Vec<&Pane> = panes.iter().filter(|pane| pane.tab == tab.id).collect();
        let tree = trees.iter().find(|dir| {
            let feature = feature_of(main, dir);
            tab.label == feature || tab.label.starts_with(&format!("{feature} :"))
        });
        if let Some(dir) = tree {
            let feature = feature_of(main, dir);
            if let Some(workspace) = worktree_space(main, dir, false, false) {
                for pane in &own {
                    let moved = call(&["pane", "move", &pane.id, "--workspace", &workspace, "--new-tab"]);
                    if let Some(created) = moved
                        .as_ref()
                        .and_then(|moved| text(moved, "/move_result/created_tab/tab_id"))
                    {
                        call(&["tab", "rename", created, &feature]);
                    }
                }
            }
            report.push(format!("{} moved into the space {feature}", tab.label));
            continue;
        }
        let is_gone = own
            .iter()
            .all(|pane| pane.cwd.as_deref().is_some_and(|cwd| cwd.ends_with(" (deleted)")));
        if is_gone && !is_busy(&tab, &panes, current.as_ref()) && call(&["tab", "close", &tab.id]).is_some() {
            report.push(format!("{} closed, its directory is gone", tab.label));
        }
    }

    // The orchestrations' spaces, in the group of the main space above their worktrees'.
    let titles = record::titles(main);
    let mut orchestrations: Vec<String> = Vec::new();
    for space in listed() {
        if let Some(name) = orchestration_of_label(label_of(&space), &titles)
            && !orchestrations.contains(&name)
        {
            orchestrations.push(name);
        }
    }
    for name in &orchestrations {
        orchestrator_space(main, name, should_resume);
        if should_resume && let Some(resumed) = restore_chat(main, name) {
            report.push(format!("{}: {resumed}", orchestrator_label(main, name)));
        }
    }

    let known = listed();
    for dir in &trees {
        let feature = feature_of(main, dir);
        let before = known.iter().find(|space| label_of(space) == feature);
        let workspace = worktree_space(main, dir, true, should_resume);
        if before.is_none() && workspace.is_some() {
            report.push(format!("space {feature} opened"));
        }
        if let (Some(before), Some(workspace)) = (before, workspace.as_deref())
            && !is_linked(before)
            && listed()
                .iter()
                .find(|space| id_of(space) == workspace)
                .is_some_and(is_linked)
        {
            report.push(format!("space {feature} moved into its group"));
        }
        let changed = stack_tabs(main, dir, workspace.as_deref());
        if !changed.is_empty() {
            report.push(format!("{feature}: {}", changed.join(" ")));
        }
    }
    let changed = stack_tabs(main, main, Some(&main_ws));
    if !changed.is_empty() {
        report.push(format!("{SPACE}: {}", changed.join(" ")));
    }
    // Last, when every space is there: herdr forgets the marks on a restart.
    mark_origins(main);
    report.extend(sort_spaces(main));
    Some((main_ws, report))
}

/// `wt:herdr [--no-focus]` (commands/herdr.mjs): the layout, with the
/// conversations resumed, and the main space in front unless `--no-focus`.
pub fn main(args: &[String]) -> std::process::ExitCode {
    let fail = |message: &str| {
        eprintln!("\n✗ {message}");
        std::process::ExitCode::from(1)
    };
    if !is_in_herdr() {
        return fail("this runs inside herdr (HERDR_ENV is not set)");
    }
    let here = std::env::current_dir().unwrap_or_default();
    let Some(main) = git::main_dir(&here) else {
        return fail("not in a checkout of the repo");
    };
    let Some((main_ws, report)) = layout(&main, true) else {
        return fail("herdr did not answer, is its server running? (herdr status)");
    };
    for line in report {
        println!("  {line}");
    }
    if !args.iter().any(|arg| arg == "--no-focus") {
        call(&["workspace", "focus", &main_ws]);
    }
    println!("\n✓ {SPACE} and a space per worktree");
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_parent_past_a_name_with_parentheses() {
        assert_eq!(parent_in("42 (a (b) c) S 7 42 42"), Some(7));
        assert_eq!(parent_in("garbage"), None);
    }

    #[test]
    fn keeps_the_named_tabs_at_the_left() {
        let order = [ROOT, OVERVIEW, "303-x", EDITOR];
        assert!(moving_of(&[ROOT, OVERVIEW, "303-x", EDITOR, "dev"], &order).is_empty());
        // In order behind a stranger: only the stranger before the last named moves.
        assert_eq!(moving_of(&[ROOT, "dev", OVERVIEW, "303-x"], &order), [1]);
        // Out of order: the named ones in order, then the others.
        assert_eq!(moving_of(&[OVERVIEW, ROOT, "dev"], &order), [1, 0, 2]);
        assert_eq!(moving_of(&["dev", EDITOR, ROOT], &order), [2, 1, 0]);
    }

    #[test]
    fn a_space_is_an_orphan_only_when_every_pane_stands_in_the_directory() {
        let pane = |cwd: Option<&str>| Pane {
            id: "p".into(),
            cwd: cwd.map(String::from),
            ..Default::default()
        };
        let dir = "/r/platform-299-x";
        assert!(is_orphan(
            &[pane(Some(dir)), pane(Some("/r/platform-299-x/src (deleted)"))],
            dir
        ));
        assert!(is_orphan(&[pane(Some("/r/platform-299-x (deleted)"))], dir));
        assert!(!is_orphan(&[pane(Some(dir)), pane(Some("/r/platform"))], dir));
        assert!(!is_orphan(&[pane(Some("/r/platform-299-xy"))], dir));
        assert!(!is_orphan(&[pane(None)], dir));
        assert!(!is_orphan(&[], dir));
    }

    #[test]
    fn names_the_project_as_claude_does() {
        let project = project_of(Path::new("/r/my platform.x"));
        assert!(project.ends_with("projects/-r-my-platform-x"));
    }

    #[test]
    fn writes_the_editor_line_for_every_shell() {
        let main = Path::new("/r/platform");
        let line = editor_command(main, Path::new("/r/it's"), None, &["luafile /c.lua".into()], None);
        assert_eq!(
            line,
            "cd '/r/it'\\''s'; nvim --server '.claude/nvim.sock' --remote-expr 1 >/dev/null 2>&1 || rm -f '.claude/nvim.sock'; nvim --listen '.claude/nvim.sock' -c 'luafile /c.lua'"
        );
        let own = editor_command(main, main, Some("bb"), &[], Some(Path::new("/p")));
        assert!(own.contains("'.claude/nvim-bb.sock'"));
        assert!(own.ends_with(r#" -c 'lua dofile("/r/platform/packages/breadboard/nvim/ide.lua")("/p")'"#));
    }

    #[test]
    fn merges_a_project_without_losing_a_file() {
        let root = std::env::temp_dir().join(format!("bb-merge-{}", std::process::id()));
        let (from, to) = (root.join("from"), root.join("to"));
        fs::create_dir_all(from.join("memory")).unwrap();
        fs::create_dir_all(to.join("memory")).unwrap();
        fs::write(from.join("a.jsonl"), "a").unwrap();
        fs::write(from.join("memory/m.md"), "new").unwrap();
        fs::write(to.join("memory/m.md"), "old").unwrap();
        assert!(merge_into(&from, &to, "303").unwrap());
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(to.join("a.jsonl")).unwrap(), "a");
        assert_eq!(fs::read_to_string(to.join("memory/m.md")).unwrap(), "old");
        assert_eq!(fs::read_to_string(to.join("memory/m.md.from-303")).unwrap(), "new");
        fs::remove_dir_all(root).unwrap();
    }
}
