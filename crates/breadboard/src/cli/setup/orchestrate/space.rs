//! The space `⇶ <title>` of an orchestration (orchestrator.mjs,
//! orchestrator-chat.mjs): on its own detached checkout, so herdr groups it
//! under the main space; `root`, `overview`, `orchestrator`, `changelog`,
//! `changes`; the tab running wt:orchestrate moves in as `orchestrator`.

use crate::cli::lifecycle::layout::orchestrations::{self, ChatState, ORDER, chat_state_of};
use crate::cli::lifecycle::layout::tabs::{ORCHESTRATOR_TAB, drop_duplicates, keep_order};
use crate::cli::setup::herdr::{self, field, herdr, spaces, tabs_of};
use crate::cli::setup::repo;
use breadboard_core::record::{self, Origin};
use serde_json::Value;
use std::path::PathBuf;

/// The checkouts of every orchestration.
pub fn orchestrator_checkouts() -> Vec<PathBuf> {
    breadboard_core::git::orchestrator_checkouts(&repo::main_dir())
}

/// The checkout goes with its space; one with changes stays, and says so (the lifecycle group's).
pub fn remove_orchestrator_checkout(name: &str) -> Option<String> {
    orchestrations::remove_orchestrator_checkout(&repo::lifecycle(), name)
}

/// The space `⇶ <title>` with its tabs; renamed after a new title, an older
/// one handing its tabs over (the lifecycle group's, one with `wt:herdr`).
pub fn orchestrator_space(name: &str) -> Option<String> {
    orchestrations::orchestrator_space(&repo::lifecycle(), name)
}

/// Whether `⇶ <name>` has an orchestrator already: a tab `orchestrator` with
/// more than a spare shell, or its one claude (orchestrator-chat.mjs `chatStateOf`).
fn has_chat(tabs: &[Value], panes: &[Value]) -> bool {
    matches!(
        chat_state_of(tabs, panes, &[], None, herdr::is_idle),
        ChatState::Alive { .. } | ChatState::Busy
    )
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
        let label = record::space_label_of(&repo::main_dir(), name);
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
        let lifecycle = repo::lifecycle();
        crate::cli::lifecycle::layout::spaces::main_space(&lifecycle)?;
        let label = lifecycle.space_label_of(name);
        let product = record::product_of(&lifecycle.main, name);
        let workspace = orchestrator_space(name)?;
        let pane = herdr::current_pane();
        let from = pane.as_ref().and_then(|pane| {
            spaces()
                .into_iter()
                .find(|space| field(space, "workspace_id") == field(pane, "workspace_id"))
                .map(|space| field(&space, "label").to_string())
        });
        let why = pane.as_ref().and_then(|_| {
            // In a product's space (#405) the tab is the orchestrator of each of its orchestrations.
            let here = from.as_deref().and_then(|label| match &product {
                Some(product) if label == product => Some(name.to_string()),
                _ if lifecycle.is_product(label) => Some(label.to_string()),
                _ => herdr::orchestration_of(label),
            });
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
            if let Some(created) =
                moved.and_then(|moved| herdr::id_at(&moved, &["move_result", "created_tab", "tab_id"]))
            {
                herdr(&["tab", "rename", &created, ORCHESTRATOR_TAB]);
                is_moved = true;
                drop_duplicates(&workspace, ORCHESTRATOR_TAB);
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
            orchestrations::record_chat(&lifecycle, name, orchestrations::chat_of_pane(&lifecycle, pane));
        }
        keep_order(&workspace, &ORDER);
        crate::cli::lifecycle::layout::spaces::main_space(&lifecycle);
        // A product's space holds several: its label says which space it is, no orchestration's mark.
        if product.is_none()
            && let Some(found) = spaces()
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

/// The space `⇶ <name>`, its entry, or None; of an orchestration of a
/// product (#405) the product's, only while it is the product's last.
pub fn space_of_orchestration(name: &str) -> Option<Value> {
    if !herdr::is_in_herdr() {
        return None;
    }
    let main = repo::main_dir();
    if let Some(product) = record::product_of(&main, name) {
        if record::orchestrations_of_product(&main, &product)
            .iter()
            .any(|other| other != name)
        {
            return None;
        }
        return spaces().into_iter().find(|space| field(space, "label") == product);
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
