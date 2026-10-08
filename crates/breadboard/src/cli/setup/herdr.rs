//! herdr for the setup commands: what only they do (the space a new
//! worktree opens with, whether the tab running it moves along, the
//! orchestration it runs in). The layout itself (spaces, tabs, their order,
//! the stack tabs, the orchestrations' marks) is the lifecycle group's, one
//! implementation for `wt:new`, `stack:*`, `wt:herdr` and the overview,
//! called here with the repo of the working directory. Outside herdr, or
//! with herdr unhappy, every call answers nothing and nothing fails: the
//! worktree is the point, the space is comfort.

use super::proc;
use super::repo;
use crate::cli::lifecycle::herdr as lifecycle_herdr;
use crate::cli::lifecycle::layout::{self, orchestrations, spaces, tabs};
use breadboard_core::json::str_of;
use breadboard_core::record::{self, ORCHESTRATOR, Origin, SPACE};
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;

pub use crate::cli::lifecycle::herdr::{all_panes, current_pane, id_at, is_idle, panes_of, spaces, tabs_of};
pub use crate::cli::lifecycle::layout::exclusive;
pub use crate::cli::lifecycle::layout::tabs::idle_pane_of;

/// The pane running this, asked once a run. wt:new asks three times (the
/// orchestration it runs in, the worktree an agent leaves, the tab it
/// moves) and nothing moves the pane in between; a pane not found is a
/// scan of every pane's shell, one herdr call each (critic, round 7: 190
/// calls and 7 s before `▸ git fetch` when herdr told no shell).
pub fn pane_here() -> Option<Value> {
    static HERE: OnceLock<Option<Value>> = OnceLock::new();
    HERE.get_or_init(current_pane).clone()
}

pub fn is_in_herdr() -> bool {
    lifecycle_herdr::is_in()
}

/// `herdr <args>` → its `result`, None when herdr is missing, old or unhappy.
pub fn herdr(args: &[&str]) -> Option<Value> {
    lifecycle_herdr::call(args)
}

/// `herdr <args>` for its effect: true when herdr did it.
pub fn herdr_did(args: &[&str]) -> bool {
    lifecycle_herdr::did(args)
}

/// A string field of a herdr answer, "" without it.
pub fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    lifecycle_herdr::s(value, key)
}

/// The session a pane's agent runs, `agent_session.value`.
pub fn session_of(pane: &Value) -> Option<&str> {
    pane.get("agent_session").and_then(|session| str_of(session, "value"))
}

/// The space a checkout's tabs belong in.
pub fn space_of(dir: &Path) -> Option<String> {
    spaces::space_of(&repo::lifecycle(), dir)
}

/// The label of a checkout's space: the main space for the main checkout.
pub fn space_label(dir: &Path) -> String {
    repo::lifecycle().space_label(dir)
}

/// The space of a worktree with its tabs (`root`, `overview` unless an
/// orchestration's, the conversation's tab, `nvim`), under the layout's lock.
pub fn worktree_space(dir: &Path, has_work_tab: bool) -> Option<String> {
    exclusive(|| spaces::worktree_space(&repo::lifecycle(), dir, has_work_tab, false))
}

/// While the stack of `dir` runs its space shows it (a tab per container,
/// `dev`, `studio`); when it is down those tabs close unless busy. What
/// changed, `+postgres` / `-dev`; a tab herdr gave no pane is said as such.
pub fn sync_stack_tabs(dir: &Path, workspace: Option<String>) -> Vec<String> {
    if !repo::env_file_of(dir).exists() {
        return Vec::new();
    }
    exclusive(|| {
        let repo = repo::lifecycle();
        let workspace = workspace.or_else(|| spaces::space_of(&repo, dir));
        layout::sync_stack_tabs(&repo, dir, workspace.as_deref())
    })
}

// ---------------------------------------------------------------------------
// Orchestrations' marks (origins.mjs): the lifecycle group's
// ---------------------------------------------------------------------------

/// The colours of the slots (herdr config names, fallbacks).
pub const SLOT_COLOURS: [(&str, &str); 3] = [("blue", "#89B4FA"), ("teal", "#94E2D5"), ("mauve", "#CBA6F7")];

/// The sidebar token of a slot: `orch1`.
pub fn origin_token(slot: u32) -> String {
    format!("orch{}", slot + 1)
}

/// The orchestration of a space's label.
pub fn orchestration_of(label: &str) -> Option<String> {
    repo::lifecycle().orchestration_of(label)
}

/// The colour slot of an orchestration: its worktrees', else its space's, else the least used.
pub fn slot_of(name: &str) -> u32 {
    let repo = repo::lifecycle();
    orchestrations::slot_of(&repo, name, &repo.orchestrated())
}

/// The origin tokens of a space as `origin` wants them.
pub fn mark_space(space: &Value, origin: Option<&Origin>) {
    orchestrations::mark_space(&repo::lifecycle(), space, origin);
}

/// Every space shows the orchestration it belongs to.
pub fn mark_origins() {
    orchestrations::mark_origins(&repo::lifecycle());
}

/// The orchestration whose space holds the pane running this (#245).
pub fn orchestration_here() -> Option<String> {
    let pane = pane_here()?;
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
        let repo = repo::lifecycle();
        spaces::main_space(&repo)?;
        if !moves_tab(proc::is_agent(), origin, stays) {
            let workspace = spaces::worktree_space(&repo, dir, true, false)?;
            layout::sync_stack_tabs(&repo, dir, Some(&workspace));
            if let Some(origin) = origin {
                if let Some(found) = spaces()
                    .into_iter()
                    .find(|space| field(space, "workspace_id") == workspace)
                {
                    mark_space(&found, Some(origin));
                }
                return Some(format!(
                    "opened the space {feature}, marked {ORCHESTRATOR} {}",
                    record::title_of(&repo.main, &origin.name)
                ));
            }
            if let Some(stays) = stays {
                let label = repo.orchestrator_label(stays);
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
        let workspace = spaces::worktree_space(&repo, dir, false, false)?;
        let pane = pane_here()?;
        let moved = herdr(&[
            "pane",
            "move",
            field(&pane, "pane_id"),
            "--workspace",
            &workspace,
            "--new-tab",
        ]);
        let created =
            moved.and_then(|moved| lifecycle_herdr::id_at(&moved, &["move_result", "created_tab", "tab_id"]))?;
        herdr(&["tab", "rename", &created, feature]);
        tabs::drop_duplicates(&workspace, feature);
        spaces::ensure_editor(&repo, &workspace, dir, &tabs_of(&workspace));
        tabs::keep_order(
            &workspace,
            &spaces::tab_order(record::is_orchestrated(&repo.main, dir), feature),
        );
        layout::sync_stack_tabs(&repo, dir, Some(&workspace));
        let had_root = spaces::space_of(&repo, &repo.main)
            .map(|main| tabs_of(&main).iter().any(|tab| field(tab, "label") == tabs::ROOT))
            .unwrap_or(false);
        spaces::main_space(&repo);
        Some(format!(
            "this tab moved into the space {feature}{}",
            if had_root {
                String::new()
            } else {
                format!(", a new tab {} waits in {SPACE}", tabs::ROOT)
            }
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(origin_token(2), "orch3");
    }
}
