//! The space `⇶ <title>` of an orchestration (orchestrator.mjs,
//! orchestrator-chat.mjs): on its own detached checkout, so herdr groups it
//! under the main space; `root`, `overview`, `orchestrator`, `changelog`,
//! `changes`; the tab running wt:orchestrate moves in as `orchestrator`.

use super::workers::checkout_of;
use crate::cli::setup::conversations;
use crate::cli::setup::herdr::{
    self, CHANGELOG, CHANGES, ORCHESTRATOR_TAB, OVERVIEW, ROOT, can_edit, ensure_space, ensure_tab,
    ensure_worktree_space, field, has_agent, herdr, is_idle, keep_order, space_of, spaces, tabs_of,
};
use crate::cli::setup::nvim;
use crate::cli::setup::proc::git;
use crate::cli::setup::repo;
use breadboard_core::record::{self, ORCHESTRATOR, Origin, is_session_id};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

/// The checkouts of every orchestration.
pub fn orchestrator_checkouts() -> Vec<PathBuf> {
    breadboard_core::git::orchestrator_checkouts(&repo::main_dir())
}

/// The checkout of `name`, made on first use; None when git refuses.
fn orchestrator_checkout(name: &str) -> Option<PathBuf> {
    let dir = checkout_of(name);
    if orchestrator_checkouts().contains(&dir) {
        return Some(dir);
    }
    fs::create_dir_all(dir.parent()?).ok()?;
    let text = dir.to_string_lossy().into_owned();
    git(
        &["worktree", "add", "--quiet", "--detach", &text, "origin/develop"],
        &repo::main_dir(),
    )
    .is_ok
    .then_some(dir)
}

/// The checkout goes with its space; one with changes stays, and says so.
pub fn remove_orchestrator_checkout(name: &str) -> Option<String> {
    let dir = checkout_of(name);
    if !orchestrator_checkouts().contains(&dir) {
        return None;
    }
    let text = dir.to_string_lossy().into_owned();
    if git(&["worktree", "remove", &text], &repo::main_dir()).is_ok {
        return None;
    }
    let label = record::orchestrator_label(&repo::main_dir(), name);
    Some(format!("kept the checkout of {label}, it has changes: {text}"))
}

const ORDER: [&str; 5] = [ROOT, OVERVIEW, ORCHESTRATOR_TAB, CHANGELOG, CHANGES];

/// The space `⇶ <title>` with its tabs; renamed after a new title, an older one handing its tabs over.
pub fn orchestrator_space(name: &str) -> Option<String> {
    let main = repo::main_dir();
    let label = record::orchestrator_label(&main, name);
    let all = spaces();
    if !all.iter().any(|space| field(space, "label") == label)
        && let Some(retitled) = all.iter().find(|space| {
            field(space, "label").starts_with(&format!("{ORCHESTRATOR} "))
                && herdr::orchestration_of(field(space, "label")).as_deref() == Some(name)
        })
    {
        herdr(&["workspace", "rename", field(retitled, "workspace_id"), &label]);
    }
    let workspace = match orchestrator_checkout(name) {
        Some(checkout) => ensure_worktree_space(&label, &checkout, space_of(&main).as_deref(), ROOT),
        None => ensure_space(&label, &main, ROOT, None),
    }?;
    for old in spaces() {
        if field(&old, "workspace_id") == workspace
            || herdr::orchestration_of(field(&old, "label")).as_deref() != Some(name)
        {
            continue;
        }
        if let Some(overview) = tabs_of(field(&old, "workspace_id"))
            .into_iter()
            .find(|tab| field(tab, "label") == OVERVIEW)
        {
            herdr(&["tab", "close", field(&overview, "tab_id")]);
        }
        herdr::move_into(field(&old, "workspace_id"), &workspace);
        herdr::drop_duplicates(&workspace, ROOT);
    }
    let tabs = tabs_of(&workspace);
    ensure_tab(
        &workspace,
        OVERVIEW,
        &main,
        Some(&herdr::overview_command(None, Some(name))),
        &tabs,
    );
    ensure_tab(&workspace, ROOT, &main, None, &tabs);
    ensure_tab(
        &workspace,
        CHANGELOG,
        &main,
        Some(&herdr::changelog_tab_command(name)),
        &tabs,
    );
    if can_edit() {
        let command = nvim::editor_command(&main, Some(name), &nvim::changes_commands(&main, name), None);
        ensure_tab(&workspace, CHANGES, &main, Some(&command), &tabs);
    }
    keep_order(&workspace, &ORDER);
    Some(workspace)
}

/// The conversation a pane runs, `(sessionId, cwd)`.
fn chat_of_pane(pane: &Value) -> Option<(String, String)> {
    if pane.get("agent").and_then(Value::as_str) != Some("claude") {
        return None;
    }
    let id = herdr::session_of(pane).filter(|id| is_session_id(id))?.to_string();
    let cwd = conversations::running_claudes()
        .into_iter()
        .find(|(_, running, _, _)| *running == id)
        .map(|(_, _, _, cwd)| cwd)
        .filter(|cwd| !cwd.is_empty())
        .or_else(|| Some(field(pane, "foreground_cwd").to_string()).filter(|cwd| !cwd.is_empty()))
        .or_else(|| Some(field(pane, "cwd").to_string()).filter(|cwd| !cwd.is_empty()))
        .unwrap_or_else(|| repo::main_dir().to_string_lossy().into_owned());
    Some((id, cwd))
}

/// The chat into the record, only on a change.
fn record_chat(name: &str, chat: Option<(String, String)>) {
    let Some((id, cwd)) = chat else { return };
    let main = repo::main_dir();
    let known = record::record_of(&main, name).and_then(|found| found.chat);
    if known.as_ref().is_some_and(|known| {
        known.session_id == id && known.cwd.clone().unwrap_or_else(|| main.to_string_lossy().into_owned()) == cwd
    }) {
        return;
    }
    let fields = json!({ "orchestrator": { "sessionId": id, "cwd": cwd } });
    let _ = record::update_record(&main, name, super::issue::object(fields));
}

/// Whether `⇶ <name>` has an orchestrator already: a tab `orchestrator` with more than a spare shell, or its one claude.
fn has_chat(tabs: &[Value], panes: &[Value]) -> bool {
    let Some(tab) = tabs.iter().find(|tab| field(tab, "label") == ORCHESTRATOR_TAB) else {
        let free: Vec<&str> = tabs
            .iter()
            .filter(|tab| field(tab, "label").starts_with("free "))
            .map(|tab| field(tab, "tab_id"))
            .collect();
        let claudes = panes.iter().filter(|pane| {
            pane.get("agent").and_then(Value::as_str) == Some("claude")
                && herdr::session_of(pane).is_some()
                && !free.contains(&field(pane, "tab_id"))
        });
        return claudes.count() == 1;
    };
    let own: Vec<&Value> = panes
        .iter()
        .filter(|pane| field(pane, "tab_id") == field(tab, "tab_id"))
        .collect();
    let is_spare = own.len() == 1 && !has_agent(own[0]) && is_idle(field(own[0], "pane_id"));
    !is_spare
}

/// Why the tab running wt:orchestrate stays where it is, or None when it moves in (#287).
pub fn stay_reason(name: &str, here: Option<&str>, is_worktree: bool, has_live: bool) -> Option<String> {
    if here == Some(name) {
        return None;
    }
    if let Some(here) = here {
        return Some(format!("it is the orchestrator of {here}"));
    }
    if is_worktree {
        return Some("it is a worktree's".into());
    }
    if has_live {
        let label = record::orchestrator_label(&repo::main_dir(), name);
        return Some(format!("{label} has its {ORCHESTRATOR_TAB}"));
    }
    None
}

/// The space of `name`, marked, with the tab running this moved in as `orchestrator` when it is nobody's.
pub fn open_orchestrator(name: &str) -> Option<String> {
    herdr::exclusive(|| {
        if !herdr::is_in_herdr() {
            return None;
        }
        herdr::main_space()?;
        let main = repo::main_dir();
        let label = record::orchestrator_label(&main, name);
        let workspace = orchestrator_space(name)?;
        let pane = if std::env::var_os("HERDR_PANE_ID").is_some() {
            herdr::current_pane()
        } else {
            None
        };
        let from = pane.as_ref().and_then(|pane| {
            spaces()
                .into_iter()
                .find(|space| field(space, "workspace_id") == field(pane, "workspace_id"))
                .map(|space| field(&space, "label").to_string())
        });
        let why = pane.as_ref().and_then(|_| {
            let here = from.as_deref().and_then(herdr::orchestration_of);
            let is_worktree = repo::worktrees()
                .iter()
                .skip(1)
                .any(|tree| Some(repo::feature_of(&tree.dir)) == from);
            let has_live = has_chat(&tabs_of(&workspace), &herdr::panes_of(&workspace));
            stay_reason(name, here.as_deref(), is_worktree, has_live)
        });
        let mut is_moved = false;
        if let Some(pane) = &pane
            && why.is_none()
            && field(pane, "workspace_id") != workspace
        {
            let moved = herdr(&[
                "pane",
                "move",
                field(pane, "pane_id"),
                "--workspace",
                &workspace,
                "--new-tab",
            ]);
            if let Some(created) = herdr::created_tab(&moved) {
                herdr(&["tab", "rename", &created, ORCHESTRATOR_TAB]);
                is_moved = true;
                herdr::drop_duplicates(&workspace, ORCHESTRATOR_TAB);
            }
        }
        let is_own = is_moved
            || pane.as_ref().is_some_and(|pane| {
                field(pane, "workspace_id") == workspace
                    && tabs_of(&workspace)
                        .iter()
                        .find(|tab| field(tab, "tab_id") == field(pane, "tab_id"))
                        .is_some_and(|tab| field(tab, "label") == ORCHESTRATOR_TAB)
            });
        if is_own && let Some(pane) = &pane {
            record_chat(name, chat_of_pane(pane));
        }
        keep_order(&workspace, &ORDER);
        herdr::main_space();
        if let Some(found) = spaces()
            .into_iter()
            .find(|space| field(space, "workspace_id") == workspace)
        {
            herdr::mark_space(
                &found,
                Some(&Origin {
                    name: name.to_string(),
                    slot: herdr::slot_of(name),
                }),
            );
        }
        let note = if is_moved {
            format!(", this tab moved in as {ORCHESTRATOR_TAB}")
        } else {
            why.map(|why| format!(", this tab stays where it is: {why}"))
                .unwrap_or_default()
        };
        Some(format!("space {label}{note}"))
    })
}

/// The space `⇶ <name>`, its entry, or None.
pub fn space_of_orchestration(name: &str) -> Option<Value> {
    if !herdr::is_in_herdr() {
        return None;
    }
    spaces()
        .into_iter()
        .find(|space| herdr::orchestration_of(field(space, "label")).as_deref() == Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_only_a_tab_that_is_nobodys() {
        assert_eq!(stay_reason("b", Some("b"), false, true), None);
        assert_eq!(
            stay_reason("b", Some("a"), false, false).as_deref(),
            Some("it is the orchestrator of a")
        );
        assert_eq!(
            stay_reason("b", None, true, false).as_deref(),
            Some("it is a worktree's")
        );
        assert_eq!(stay_reason("b", None, false, false), None);
    }
}
