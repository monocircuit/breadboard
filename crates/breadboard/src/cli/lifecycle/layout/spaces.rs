//! The herdr spaces (worktree/spaces.mjs): the main one, where a
//! checkout's space is; and what may close in them. A worktree has none of
//! its own (#409).

use super::super::herdr::{self, did, has_agent, s};
use super::super::repo::Repo;
use super::tabs::{
    CHANGELOG, OVERVIEW, ROOT, View, Want, ensure_space, ensure_tab, keep_order, overall_changelog_command,
    overview_command,
};
use breadboard_core::record::SPACE;
use serde_json::Value;
use std::path::Path;

/// The space a checkout's tabs belong in.
pub fn space_of(repo: &Repo, dir: &Path) -> Option<String> {
    herdr::space_labelled(&repo.space_label(dir)).map(|space| s(&space, "workspace_id").to_string())
}

/// The main space: `root`, `overview` and `changelog`; a tab `develop` of
/// before is renamed `root`. With products (#405) only `root` and
/// `overview`: the changelog is in each product's space, the main space's
/// of before closes. None when herdr does not answer.
pub fn main_space(repo: &Repo) -> Option<String> {
    let main = repo.main.as_path();
    let workspace = ensure_space(
        &SPACE.get(),
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
    if repo.has_products() {
        let changelogs: Vec<&Value> = tabs.iter().filter(|tab| s(tab, "label") == CHANGELOG).collect();
        if !changelogs.is_empty() {
            let panes = herdr::panes_of(&workspace);
            let current = herdr::current_pane();
            for tab in changelogs
                .into_iter()
                .filter(|tab| !is_busy(tab, &panes, current.as_ref()))
            {
                did(&["tab", "close", s(tab, "tab_id")]);
            }
        }
        keep_order(&workspace, &[ROOT, OVERVIEW]);
        return Some(workspace);
    }
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
