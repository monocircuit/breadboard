//! herdr for the setup commands: what only they do (the space a new
//! worktree opens with, whether the tab running it moves along, the
//! orchestration it runs in). The layout itself (spaces, tabs, their order,
//! the orchestrations' marks) is the lifecycle group's, one
//! implementation for `wt:new`, `stack:*`, `wt:herdr` and the overview,
//! called here with the repo of the working directory. Outside herdr, or
//! with herdr unhappy, every call answers nothing and nothing fails: the
//! worktree is the point, the space is comfort.

use super::repo;
use crate::cli::lifecycle::herdr as lifecycle_herdr;
use crate::cli::lifecycle::layout::orchestrations;
use breadboard_core::json::str_of;
use breadboard_core::record::Origin;
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;

pub use crate::cli::lifecycle::herdr::{all_panes, current_pane, id_at, is_idle, panes_of, spaces, tabs_of};
pub use crate::cli::lifecycle::layout::exclusive;

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

/// A string field of a herdr answer, "" without it.
pub fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    lifecycle_herdr::s(value, key)
}

/// The session a pane's agent runs, `agent_session.value`.
pub fn session_of(pane: &Value) -> Option<&str> {
    pane.get("agent_session").and_then(|session| str_of(session, "value"))
}

/// The label of a checkout's space: the main space for the main checkout.
pub fn space_label(dir: &Path) -> String {
    repo::lifecycle().space_label(dir)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_slot_tokens() {
        assert_eq!(origin_token(0), "orch1");
        assert_eq!(origin_token(2), "orch3");
    }
}
