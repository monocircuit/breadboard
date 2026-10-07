//! The tabs of a herdr space (worktree/tabs.mjs): create, find idle ones,
//! keep them in order, drop duplicates; and the spaces made from them.

use super::super::herdr::{self, call, did, has_agent, id_at, s};
use super::super::proc::{exe, quote, text};
use super::super::repo::Repo;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// A shell in the checkout of its space, the leftmost tab of every space.
pub const ROOT: &str = "root";
/// The live overview, in the main space and in each worktree's.
pub const OVERVIEW: &str = "overview";
/// The changelog, live, in the main space and an orchestration's.
pub const CHANGELOG: &str = "changelog";
/// nvim on what an orchestration's MRs changed.
pub const CHANGES: &str = "changes";
/// nvim in a worktree, after the conversation's tab.
pub const EDITOR: &str = "nvim";
/// The orchestrating session's tab in its space.
pub const ORCHESTRATOR_TAB: &str = "orchestrator";
/// Whose sidebar tokens they are, for herdr: the plugin's id.
pub const TOKEN_SOURCE: &str = "monocircuit.platform";
/// The branch of a worktree as a sidebar token (herdrdev/herdr#2952).
pub const BRANCH_TOKEN: &str = "wt_branch";

/// A tab as `ensure_tab` wants it: its label, where it starts and what it runs.
pub struct Want<'a> {
    pub label: &'a str,
    pub cwd: &'a Path,
    pub command: Option<String>,
}

/// What the live overview of a tab shows.
pub enum View<'a> {
    /// Everything, keeping the spaces in step (`--herdr`).
    Herdr,
    Worktree(&'a Path),
    Orchestration(&'a str),
}

/// The command of a tab `overview`: this binary, where today's ran
/// `node '<src>/overview.mjs'`, quoted the same way.
pub fn overview_command(view: View) -> String {
    let what = match view {
        View::Herdr => "--herdr".to_string(),
        View::Worktree(dir) => format!("--worktree {}", quote(&text(dir))),
        View::Orchestration(name) => format!("--orchestration {name}"),
    };
    format!("{} overview {what}", quote(&exe()))
}

/// What the tab `changelog` of the main space runs: the repo's changelog, live.
pub fn overall_changelog_command() -> String {
    format!("{} changelog --watch", quote(&exe()))
}

/// The first tab of a new space.
fn first_tab(created: &Value, first: &Want) {
    if let Some(tab) = id_at(created, &["tab", "tab_id"]) {
        call(&["tab", "rename", &tab, first.label]);
    }
    if let (Some(command), Some(pane)) = (&first.command, id_at(created, &["root_pane", "pane_id"])) {
        did(&["pane", "run", &pane, command]);
    }
}

/// A space by its label, created with the tab `first` if missing.
pub fn ensure_space(label: &str, cwd: &Path, first: &Want) -> Option<String> {
    if let Some(found) = herdr::space_labelled(label) {
        return Some(s(&found, "workspace_id").to_string());
    }
    let created = call(&[
        "workspace",
        "create",
        "--label",
        label,
        "--cwd",
        &text(cwd),
        "--no-focus",
    ])?;
    let workspace = id_at(&created, &["workspace", "workspace_id"])?;
    first_tab(&created, first);
    Some(workspace)
}

fn is_linked(space: &Value) -> bool {
    space
        .get("worktree")
        .and_then(|tree| tree.get("is_linked_worktree"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The space of a worktree, in the group of the main space `main`: herdr
/// links it only when `worktree open` opens it. A space of it that stands
/// alone is taken into the group; an older herdr gets a space of its own.
pub fn ensure_worktree_space(label: &str, dir: &Path, main: Option<&str>, first: &Want) -> Option<String> {
    let found = herdr::space_labelled(label);
    let fallback = |found: &Option<Value>| match found {
        Some(found) => Some(s(found, "workspace_id").to_string()),
        None => ensure_space(label, dir, first),
    };
    let Some(main) = main.filter(|_| !found.as_ref().is_some_and(is_linked)) else {
        return fallback(&found);
    };
    // herdr takes the main space for the worktree's while its first tab stands in the worktree (herdrdev/herdr#4293).
    let listed = call(&["worktree", "list", "--workspace", main])
        .and_then(|result| result.get("worktrees")?.as_array().cloned())
        .unwrap_or_default();
    let dir_text = text(dir);
    let is_main_taken = listed
        .iter()
        .any(|tree| s(tree, "path") == dir_text && s(tree, "open_workspace_id") == main);
    let opened = if is_main_taken {
        None
    } else {
        call(&[
            "worktree",
            "open",
            "--workspace",
            main,
            "--path",
            &dir_text,
            "--label",
            label,
            "--no-focus",
        ])
    };
    let Some((opened, workspace)) =
        opened.and_then(|opened| id_at(&opened, &["workspace", "workspace_id"]).map(|id| (opened, id)))
    else {
        return fallback(&found);
    };
    if opened.get("already_open").and_then(Value::as_bool).unwrap_or(false) {
        return Some(workspace);
    }
    first_tab(&opened, first);
    if let Some(found) = found {
        move_into(s(&found, "workspace_id"), &workspace);
        drop_duplicates(&workspace, first.label);
    }
    Some(workspace)
}

/// Every tab of the space `from` into `to`, in order and with its label,
/// what runs in it keeps running; `from` closes once it is empty.
pub fn move_into(from: &str, to: &str) {
    let panes = herdr::panes_of(from);
    for tab in herdr::tabs_of(from) {
        let own: Vec<&Value> = panes
            .iter()
            .filter(|pane| s(pane, "tab_id") == s(&tab, "tab_id"))
            .collect();
        let Some((head, rest)) = own.split_first() else {
            continue;
        };
        let moved = call(&[
            "pane",
            "move",
            s(head, "pane_id"),
            "--workspace",
            to,
            "--new-tab",
            "--no-focus",
        ]);
        let Some(created) = moved.and_then(|moved| id_at(&moved, &["move_result", "created_tab", "tab_id"])) else {
            continue;
        };
        call(&["tab", "rename", &created, s(&tab, "label")]);
        for pane in rest {
            call(&[
                "pane",
                "move",
                s(pane, "pane_id"),
                "--tab",
                &created,
                "--split",
                "right",
                "--no-focus",
            ]);
        }
    }
    if herdr::spaces().iter().any(|space| s(space, "workspace_id") == from) && herdr::tabs_of(from).is_empty() {
        call(&["workspace", "close", from]);
    }
}

/// The branch of a worktree as the sidebar token `$wt_branch`, only on a change.
pub fn mark_branch(repo: &Repo, workspace: &str, dir: &Path) {
    let Some(branch) = repo
        .worktrees()
        .into_iter()
        .find(|tree| tree.dir == dir)
        .and_then(|tree| tree.branch)
    else {
        return;
    };
    let Some(found) = herdr::spaces()
        .into_iter()
        .find(|space| s(space, "workspace_id") == workspace)
    else {
        return;
    };
    let shown = found
        .get("tokens")
        .and_then(|tokens| tokens.get(BRANCH_TOKEN))
        .and_then(Value::as_str);
    if shown == Some(branch.as_str()) {
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

/// Whether a space hangs in the main space's group.
pub fn is_grouped(workspace: &str) -> bool {
    herdr::spaces()
        .iter()
        .any(|space| s(space, "workspace_id") == workspace && is_linked(space))
}

/// The single pane of the tab `label` when nothing runs in it, else None.
pub fn idle_pane_of(workspace: &str, label: &str, tabs: &[Value]) -> Option<String> {
    let tab = tabs.iter().find(|tab| s(tab, "label") == label)?;
    if tab.get("pane_count").and_then(Value::as_i64) != Some(1) {
        return None;
    }
    let pane = herdr::panes_of(workspace)
        .into_iter()
        .find(|pane| s(pane, "tab_id") == s(tab, "tab_id"))?;
    let id = s(&pane, "pane_id").to_string();
    herdr::is_idle(&id).then_some(id)
}

/// A tab by its label, created unfocused with its command running if
/// missing, the command run again if the tab is there but its shell idles.
/// Whether either happened.
pub fn ensure_tab(workspace: &str, want: &Want, tabs: &[Value]) -> bool {
    open_tab(workspace, want, tabs) != Opened::Kept
}

/// What [`open_tab`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// The tab was there and its command runs (or it has none), or herdr made none.
    Kept,
    /// Created, or its idle shell given its command again: the command runs.
    Started,
    /// Created, but herdr gave it no pane or took no command: nothing runs there.
    Empty,
}

/// [`ensure_tab`], saying whether the command runs: a tab herdr created
/// without a pane is no running stack (rewrite-log 2026-10-02 00:45).
pub fn open_tab(workspace: &str, want: &Want, tabs: &[Value]) -> Opened {
    if tabs.iter().any(|tab| s(tab, "label") == want.label) {
        let Some(command) = &want.command else {
            return Opened::Kept;
        };
        let is_run =
            idle_pane_of(workspace, want.label, tabs).is_some_and(|idle| did(&["pane", "run", &idle, command]));
        return if is_run { Opened::Started } else { Opened::Kept };
    }
    let cwd = text(want.cwd);
    let Some(created) = call(&[
        "tab",
        "create",
        "--workspace",
        workspace,
        "--cwd",
        &cwd,
        "--label",
        want.label,
        "--no-focus",
    ]) else {
        return Opened::Kept;
    };
    let Some(command) = &want.command else {
        return Opened::Started;
    };
    let is_run = id_at(&created, &["root_pane", "pane_id"]).is_some_and(|pane| did(&["pane", "run", &pane, command]));
    if is_run { Opened::Started } else { Opened::Empty }
}

/// Which tabs `keep_order` moves to the end, by index into `labels` (the
/// space's tabs in order): the named ones out of place and the others in
/// between; when the named ones after the first out of place are in order,
/// only the others in between.
pub fn order_moves(labels: &[&str], order: &[&str]) -> Vec<usize> {
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
        .filter(|&at| wanted.contains(&labels[at]))
        .collect();
    let others: Vec<usize> = rest
        .iter()
        .copied()
        .filter(|&at| !wanted.contains(&labels[at]))
        .collect();
    let is_in_order = named
        .iter()
        .enumerate()
        .all(|(i, &at)| wanted.get(fixed + i) == Some(&labels[at]));
    if is_in_order {
        let last = named.last().copied().unwrap_or(0);
        return others.into_iter().filter(|&at| at < last).collect();
    }
    let mut moving: Vec<usize> = wanted[fixed..]
        .iter()
        .filter_map(|label| named.iter().copied().find(|&at| labels[at] == *label))
        .collect();
    moving.extend(others);
    moving
}

/// The tabs `order` names at the left end of their space, in that order,
/// the others behind them. herdr can't reorder tabs: a tab out of place
/// moves its pane into a new tab at the end, what runs there keeps running.
/// Focus goes back to where it was. It stops at a tab split into panes.
pub fn keep_order(workspace: &str, order: &[&str]) {
    let tabs = herdr::tabs_of(workspace);
    let labels: Vec<&str> = tabs.iter().map(|tab| s(tab, "label")).collect();
    let moving = order_moves(&labels, order);
    if moving.is_empty() {
        return;
    }
    let before = herdr::spaces();
    let panes = herdr::panes_of(workspace);
    let mut renamed: HashMap<String, String> = HashMap::new();
    for at in moving {
        let tab = &tabs[at];
        if tab.get("pane_count").and_then(Value::as_i64) != Some(1) {
            break;
        }
        let Some(pane) = panes.iter().find(|pane| s(pane, "tab_id") == s(tab, "tab_id")) else {
            break;
        };
        let moved = call(&[
            "pane",
            "move",
            s(pane, "pane_id"),
            "--workspace",
            workspace,
            "--new-tab",
        ]);
        let Some(created) = moved.and_then(|moved| id_at(&moved, &["move_result", "created_tab", "tab_id"])) else {
            break;
        };
        call(&["tab", "rename", &created, s(tab, "label")]);
        renamed.insert(s(tab, "tab_id").to_string(), created);
    }
    if renamed.is_empty() {
        return;
    }
    let now = |tab: &str| renamed.get(tab).cloned().unwrap_or_else(|| tab.to_string());
    let own = before
        .iter()
        .find(|space| s(space, "workspace_id") == workspace)
        .map(|space| s(space, "active_tab_id"));
    let focused = before
        .iter()
        .find(|space| space.get("focused").and_then(Value::as_bool) == Some(true));
    for tab in [own, focused.map(|space| s(space, "active_tab_id"))]
        .into_iter()
        .flatten()
        .filter(|tab| !tab.is_empty())
    {
        call(&["tab", "focus", &now(tab)]);
    }
}

/// One tab `label` in the space: the spare duplicates close (one pane, no
/// agent, its shell idle); when all are spare, the first stays.
pub fn drop_duplicates(workspace: &str, label: &str) {
    let tabs: Vec<Value> = herdr::tabs_of(workspace)
        .into_iter()
        .filter(|tab| s(tab, "label") == label)
        .collect();
    if tabs.len() < 2 {
        return;
    }
    let panes = herdr::panes_of(workspace);
    let spare: Vec<&Value> = tabs
        .iter()
        .filter(|tab| {
            let own: Vec<&Value> = panes
                .iter()
                .filter(|pane| s(pane, "tab_id") == s(tab, "tab_id"))
                .collect();
            own.len() == 1 && !has_agent(own[0]) && herdr::is_idle(s(own[0], "pane_id"))
        })
        .collect();
    let skip = usize::from(spare.len() == tabs.len());
    let closing: Vec<String> = spare
        .iter()
        .skip(skip)
        .map(|tab| s(tab, "tab_id").to_string())
        .collect();
    for tab in closing {
        call(&["tab", "close", &tab]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_moves_when_in_order() {
        assert!(
            order_moves(
                &["root", "overview", "x", "nvim", "dev"],
                &["root", "overview", "x", "nvim"]
            )
            .is_empty()
        );
        assert!(order_moves(&["root", "dev"], &["root", "overview", "x"]).is_empty());
    }

    #[test]
    fn only_the_others_in_between_move_when_the_named_are_in_order() {
        // root · dev · overview · x: dev stands in the way.
        assert_eq!(
            order_moves(&["root", "dev", "overview", "x", "studio"], &["root", "overview", "x"]),
            vec![1]
        );
    }

    #[test]
    fn the_named_out_of_order_move_then_the_others() {
        // overview before root: overview, x… all go behind in the wanted order.
        assert_eq!(
            order_moves(&["overview", "root", "dev"], &["root", "overview"]),
            vec![1, 0, 2]
        );
        assert_eq!(
            order_moves(&["root", "x", "overview", "dev"], &["root", "overview", "x"]),
            vec![2, 1, 3]
        );
    }

    #[test]
    fn an_overview_runs_this_binary() {
        let command = overview_command(View::Worktree(Path::new("/r/p-1 x")));
        assert!(command.ends_with(" overview --worktree '/r/p-1 x'"), "{command}");
        assert!(overview_command(View::Orchestration("o")).ends_with(" overview --orchestration o"));
        assert!(overall_changelog_command().ends_with(" changelog --watch"));
    }
}
