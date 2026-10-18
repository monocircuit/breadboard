//! AGENTS of an orchestration (#289, free-agents.mjs): the free agents, their
//! worktree gone, their claude ended and their tab closed (#345): the record
//! keeps each by its conversation's id, a next issue resumes it. Each with
//! its state (#288), its context, where it came from and its conversation's
//! id; two clicks on a row dismiss it.

use super::agent_state::{AgentState, Facts, agent_state_of};
use super::display_names::Names;
use super::{Paint, Span, Tree, counted};
use crate::herdr::Agent;
use crate::record::Worker;
use std::collections::HashMap;

/// A free agent's row in AGENTS (free-agents.mjs `freeRowOf`): its state as an
/// agent without a worktree (`at work`, `asks you`, else `free`), its context,
/// the worktree and MR it came from, how many issues it did.
#[derive(Debug, Clone, PartialEq)]
pub struct FreeRow {
    /// Its conversation's id: `claude --resume <id>`.
    pub id: String,
    pub name: String,
    /// The person's name it is shown by (#292).
    pub display: String,
    pub state: AgentState,
    pub text: String,
    /// Its tab, which its dismiss closes: only one resumed by hand has one (#345).
    pub tab: Option<String>,
}

/// `names`: the orchestration's (display_names.rs `names_of`), so a free
/// agent goes by the name its worktree's node showed.
pub fn free_rows_of(
    free: &[(Worker, crate::fresh::Context)],
    agents: &HashMap<String, Agent>,
    names: &Names,
) -> Vec<FreeRow> {
    free.iter()
        .map(|(worker, context)| {
            let agent = agents.get(&worker.id);
            let state = agent_state_of(
                &Tree::default(),
                &Facts {
                    status: agent.and_then(|agent| agent.status.as_deref()),
                    ..Default::default()
                },
            );
            let mut parts = vec![crate::fresh::k(context.used)];
            if let Some(from) = worker.free.as_ref().and_then(|free| free.from.as_deref()) {
                let mr = worker.free.as_ref().and_then(|free| free.mr);
                // The worktree it is named after says itself on the row's first line: its MR alone.
                match (worker.name.as_deref() == Some(from), mr) {
                    (true, Some(mr)) => parts.push(format!("from !{mr}")),
                    (true, None) => {}
                    (false, Some(mr)) => parts.push(format!("from {from} (!{mr})")),
                    (false, None) => parts.push(format!("from {from}")),
                }
            }
            if !worker.issues.is_empty() {
                parts.push(counted(worker.issues.len() as u64, "issue"));
            }
            FreeRow {
                id: worker.id.clone(),
                name: worker.name.clone().unwrap_or_default(),
                display: names
                    .by_id
                    .get(&worker.id)
                    .cloned()
                    .or_else(|| worker.display_name.clone())
                    .unwrap_or_else(|| super::display_names::pick_name(&worker.id, [])),
                state,
                text: parts.join(" · "),
                tab: agent.map(|agent| agent.tab.clone()),
            }
        })
        .collect()
}

/// The AGENTS box's title: `2 free · no tab, …`.
pub fn free_head(count: usize) -> Span {
    Span::new(
        format!(
            "{count} free · no tab, a next issue resumes one; its id copies `claude --resume`, two clicks dismiss one"
        ),
        Paint::Dim,
    )
}

/// A conversation's id as its column shows it: its first 8 characters (#345).
pub fn short_id(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(at, _)| &id[..at])
}

/// What a click on the id copies.
pub fn resume_command(id: &str) -> String {
    format!("claude --resume {id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle(id: &str, status: &str) -> HashMap<String, Agent> {
        [(
            id.to_string(),
            Agent {
                pane: "p".into(),
                tab: "t1".into(),
                status: Some(status.into()),
            },
        )]
        .into()
    }

    #[test]
    fn its_id_is_short_and_resumes_whole() {
        let id = "0f3c2a91-5b7e-4d0a-9c11-2e8f6a7b4c3d";
        assert_eq!(short_id(id), "0f3c2a91");
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(resume_command(id), format!("claude --resume {id}"));
    }

    #[test]
    fn rows_a_free_agent_with_its_state_context_and_whence() {
        let free = vec![(
            Worker {
                id: "f1".into(),
                name: Some("272-orchestration".into()),
                issues: vec![272],
                free: Some(crate::record::Freed {
                    at: None,
                    from: Some("272-orchestration".into()),
                    mr: Some(289),
                }),
                ..Default::default()
            },
            crate::fresh::Context { used: 36_000, at: None },
        )];
        let names = super::super::display_names::names_of(&[], &[free[0].0.clone()]);
        let rows = free_rows_of(&free, &idle("f1", "idle"), &names);
        assert_eq!(rows[0].state.text, "free");
        assert_eq!(rows[0].display, "Priya Virtanen");
        // Its name is the worktree it came from: not twice on the row.
        assert_eq!(rows[0].text, "36 k · from !289 · 1 issue");
        let mut moved = free.clone();
        moved[0].0.name = Some("270-before".into());
        assert_eq!(
            free_rows_of(&moved, &idle("f1", "idle"), &names)[0].text,
            "36 k · from 272-orchestration (!289) · 1 issue"
        );
        assert_eq!(
            free_rows_of(&free, &idle("f1", "working"), &names)[0].state.text,
            "at work"
        );
    }
}
