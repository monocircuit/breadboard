//! AGENTS of an orchestration (#289, free-agents.mjs): the free agents, their
//! worktree gone, waiting in its space for the next issue. Each with its state
//! (#288), its context and where it came from; two clicks on a row dismiss it.

use super::agent_state::{AgentState, Facts, agent_state_of};
use super::{Paint, Span, Tree, counted};
use crate::herdr::Agent;
use crate::record::Worker;
use std::collections::HashMap;

/// A free agent's row in AGENTS (free-agents.mjs `freeRowOf`): its state as an
/// agent without a worktree (`at work`, `asks you`, else `free`), its context,
/// the worktree and MR it came from, how many issues it did.
#[derive(Debug, Clone, PartialEq)]
pub struct FreeRow {
    pub id: String,
    pub name: String,
    pub state: AgentState,
    pub text: String,
    /// Its tab, which its dismiss closes.
    pub tab: Option<String>,
}

pub fn free_rows_of(free: &[(Worker, f64)], agents: &HashMap<String, Agent>) -> Vec<FreeRow> {
    free.iter()
        .map(|(worker, share)| {
            let agent = agents.get(&worker.id);
            let state = agent_state_of(
                &Tree::default(),
                &Facts {
                    status: agent.and_then(|agent| agent.status.as_deref()),
                    ..Default::default()
                },
            );
            let mut parts = vec![format!("{} %", (share * 100.0).round() as i64)];
            if let Some(from) = worker.free.as_ref().and_then(|free| free.from.as_deref()) {
                let mr = worker
                    .free
                    .as_ref()
                    .and_then(|free| free.mr)
                    .map(|mr| format!(" (!{mr})"))
                    .unwrap_or_default();
                parts.push(format!("from {from}{mr}"));
            }
            if !worker.issues.is_empty() {
                parts.push(counted(worker.issues.len() as u64, "issue"));
            }
            FreeRow {
                id: worker.id.clone(),
                name: worker.name.clone().unwrap_or_default(),
                state,
                text: parts.join(" · "),
                tab: agent.map(|agent| agent.tab.clone()),
            }
        })
        .collect()
}

/// The AGENTS box's title: `2 free · wait in their tab …`.
pub fn free_head(count: usize) -> Span {
    Span::new(
        format!("{count} free · wait in their tab for the next issue; two clicks dismiss one"),
        Paint::Dim,
    )
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
            0.18,
        )];
        let rows = free_rows_of(&free, &idle("f1", "idle"));
        assert_eq!(rows[0].state.text, "free");
        assert_eq!(rows[0].text, "18 % · from 272-orchestration (!289) · 1 issue");
        assert_eq!(free_rows_of(&free, &idle("f1", "working"))[0].state.text, "at work");
    }
}
