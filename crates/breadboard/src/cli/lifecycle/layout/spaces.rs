//! The herdr spaces (worktree/spaces.mjs): the main one, one per worktree,
//! where a checkout's space is; and what may close in them.

use super::super::herdr::{self, did, has_agent, s};
use super::super::repo::Repo;
use super::nvim::{can_edit, editor_command, ide_for};
use super::tabs::{
    CHANGELOG, EDITOR, OVERVIEW, ROOT, View, Want, drop_duplicates, ensure_space, ensure_tab, ensure_worktree_space,
    keep_order, keep_order_except, mark_branch, overall_changelog_command, overview_command,
};
use breadboard_core::record::{self, SPACE};
use breadboard_core::runs;
use serde_json::Value;
use std::path::Path;

/// The space a checkout's tabs belong in.
pub fn space_of(repo: &Repo, dir: &Path) -> Option<String> {
    herdr::space_labelled(&repo.space_label(dir)).map(|space| s(&space, "workspace_id").to_string())
}

/// The main space: `root`, `overview` and `changelog`; a tab `develop` of
/// before is renamed `root`. None when herdr does not answer.
pub fn main_space(repo: &Repo) -> Option<String> {
    let main = repo.main.as_path();
    let workspace = ensure_space(
        SPACE,
        main,
        &Want {
            label: ROOT,
            cwd: main,
            command: None,
        },
    )?;
    let mut tabs = herdr::tabs_of(&workspace);
    let old = tabs
        .iter()
        .find(|tab| s(tab, "label") == "develop")
        .map(|tab| s(tab, "tab_id").to_string());
    if let Some(old) = old.filter(|_| !tabs.iter().any(|tab| s(tab, "label") == ROOT)) {
        herdr::call(&["tab", "rename", &old, ROOT]);
        tabs = herdr::tabs_of(&workspace);
    }
    ensure_tab(
        &workspace,
        &Want {
            label: OVERVIEW,
            cwd: main,
            command: Some(overview_command(View::Herdr)),
        },
        &tabs,
    );
    ensure_tab(
        &workspace,
        &Want {
            label: ROOT,
            cwd: main,
            command: None,
        },
        &tabs,
    );
    ensure_tab(
        &workspace,
        &Want {
            label: CHANGELOG,
            cwd: main,
            command: Some(overall_changelog_command()),
        },
        &tabs,
    );
    keep_order(&workspace, &[ROOT, OVERVIEW, CHANGELOG]);
    Some(workspace)
}

/// The tab `nvim` of a worktree, on the files it changes; nothing without nvim.
pub fn ensure_editor(repo: &Repo, workspace: &str, dir: &Path, tabs: &[Value]) {
    if !can_edit() {
        return;
    }
    let ide = ide_for(dir, &repo.main);
    let command = editor_command(dir, None, None, ide.as_deref());
    ensure_tab(
        workspace,
        &Want {
            label: EDITOR,
            cwd: dir,
            command: Some(command),
        },
        tabs,
    );
}

/// The space of a worktree: `root`, its `overview` (none for an
/// orchestration's, #332), the tab of its claude conversation (unless an
/// agent brings its own) and `nvim`, in that order.
/// No conversation is resumed there (#361): a task agent is a session, not
/// a tab, and a click opens it in one.
pub fn worktree_space(repo: &Repo, dir: &Path, has_work_tab: bool) -> Option<String> {
    let feature = repo.feature_of(dir);
    // A headless agent (#359) is a session, not a tab: its space keeps `nvim`
    // but gets no conversation's tab.
    let is_headless = has_work_tab && runs::is_headless(&repo.main, dir);
    let (has_editor, has_work_tab) = (has_work_tab, has_work_tab && !is_headless);
    let main = space_of(repo, &repo.main);
    let workspace = ensure_worktree_space(
        &feature,
        dir,
        main.as_deref(),
        &Want {
            label: ROOT,
            cwd: dir,
            command: None,
        },
    )?;
    mark_branch(repo, &workspace, dir);
    let tabs = herdr::tabs_of(&workspace);
    // An orchestration's worktree shows in its orchestration's overview: no tab of its own (#332).
    let is_orchestrated = record::is_orchestrated(&repo.main, dir);
    let mut closed = Vec::new();
    if is_orchestrated {
        closed = close_overview(&workspace, &tabs);
    } else {
        ensure_tab(
            &workspace,
            &Want {
                label: OVERVIEW,
                cwd: dir,
                command: Some(overview_command(View::Worktree(dir))),
            },
            &tabs,
        );
    }
    if has_work_tab {
        ensure_tab(
            &workspace,
            &Want {
                label: &feature,
                cwd: dir,
                command: None,
            },
            &tabs,
        );
    }
    ensure_tab(
        &workspace,
        &Want {
            label: ROOT,
            cwd: dir,
            command: None,
        },
        &tabs,
    );
    // Behind the conversation's tab: an agent that brings its own opens it after the move.
    if has_editor {
        ensure_editor(repo, &workspace, dir, &tabs);
    }
    drop_duplicates(&workspace, &feature);
    // A tab closed here is out of the order, whatever a slow herdr still lists.
    keep_order_except(&workspace, &tab_order(is_orchestrated, &feature), &closed);
    Some(workspace)
}

/// The tabs of a worktree's space in their order; an orchestrated one has no `overview` (#332).
pub fn tab_order(is_orchestrated: bool, feature: &str) -> Vec<&str> {
    if is_orchestrated {
        vec![ROOT, feature, EDITOR]
    } else {
        vec![ROOT, OVERVIEW, feature, EDITOR]
    }
}

/// The `overview` tab an orchestrated worktree's space had before #332 closes,
/// unless something else runs there (a second pane, an agent, this command).
/// The ids of the tabs herdr closed.
fn close_overview(workspace: &str, tabs: &[Value]) -> Vec<String> {
    let olds: Vec<&Value> = tabs.iter().filter(|tab| s(tab, "label") == OVERVIEW).collect();
    if olds.is_empty() {
        return Vec::new();
    }
    let panes = herdr::panes_of(workspace);
    let current = herdr::current_pane();
    olds.into_iter()
        .filter(|tab| !is_busy(tab, &panes, current.as_ref()))
        .filter(|tab| did(&["tab", "close", s(tab, "tab_id")]))
        .map(|tab| s(tab, "tab_id").to_string())
        .collect()
}

/// A tab that must not be closed from a script: it runs this command, holds
/// more than one pane, or an agent is in it.
pub fn is_busy(tab: &Value, panes: &[Value], current: Option<&Value>) -> bool {
    current.is_some_and(|current| s(current, "tab_id") == s(tab, "tab_id"))
        || tab.get("pane_count").and_then(Value::as_i64).unwrap_or(0) > 1
        || panes
            .iter()
            .any(|pane| s(pane, "tab_id") == s(tab, "tab_id") && has_agent(pane))
}

/// What an agent in each of herdr's states holds its tab for; `idle` and `done` hold nothing.
fn agent_busy(status: &str) -> Option<&'static str> {
    match status {
        "working" => Some("an agent works there"),
        "blocked" => Some("an agent waits for an answer there"),
        "unknown" => Some("herdr can't tell whether an agent works there"),
        _ => None,
    }
}

/// Why a tab has to stay once its worktree is gone, or None: it runs this
/// command, has several panes, or its agent works or waits (#142).
pub fn busy_reason(tab: &Value, panes: &[Value], current: Option<&Value>) -> Option<&'static str> {
    if current.is_some_and(|current| s(current, "tab_id") == s(tab, "tab_id")) {
        return Some("it runs this command");
    }
    if tab.get("pane_count").and_then(Value::as_i64).unwrap_or(0) > 1 {
        return Some("it has several panes");
    }
    let status_of = |pane: &Value| {
        pane.get("agent_status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    };
    panes
        .iter()
        .filter(|pane| s(pane, "tab_id") == s(tab, "tab_id") && has_agent(pane))
        .find_map(|pane| agent_busy(&status_of(pane)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_orchestrated_space_has_no_overview_tab() {
        assert_eq!(tab_order(true, "332-x"), [ROOT, "332-x", EDITOR]);
        assert_eq!(tab_order(false, "332-x"), [ROOT, OVERVIEW, "332-x", EDITOR]);
    }

    #[test]
    fn a_tab_stays_for_its_agent_at_work() {
        let tab = json!({ "tab_id": "t1", "pane_count": 1 });
        let working = [json!({ "tab_id": "t1", "agent": "claude", "agent_status": "working" })];
        assert_eq!(busy_reason(&tab, &working, None), Some("an agent works there"));
        let silent = [json!({ "tab_id": "t1", "agent": "claude" })];
        assert_eq!(
            busy_reason(&tab, &silent, None),
            Some("herdr can't tell whether an agent works there")
        );
        let done = [json!({ "tab_id": "t1", "agent": "claude", "agent_status": "done" })];
        assert_eq!(busy_reason(&tab, &done, None), None);
        assert_eq!(
            busy_reason(&tab, &done, Some(&json!({ "tab_id": "t1" }))),
            Some("it runs this command")
        );
        assert_eq!(
            busy_reason(&json!({ "tab_id": "t2", "pane_count": 2 }), &[], None),
            Some("it has several panes")
        );
        assert!(is_busy(&tab, &done, None));
        assert!(!is_busy(&json!({ "tab_id": "t3", "pane_count": 1 }), &done, None));
    }
}
