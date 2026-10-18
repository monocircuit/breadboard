//! The announced handovers of an orchestration (#227, overview/handovers.mjs):
//! whose worktree they are on, what they still wait for, whether they are
//! due. workers.rs does a due one, to the agent of a worktree or a free one.

use super::Entry;
use crate::herdr::Agent;
use crate::record::{Handover, Worker};
use std::collections::HashMap;
use std::path::PathBuf;

/// The record's workers, in its order; by their conversation's id with `worker_of`.
pub type Known = [Worker];

/// The worker of a conversation's id.
pub fn worker_of<'a>(known: &'a Known, id: &str) -> Option<&'a Worker> {
    known.iter().find(|worker| worker.id == id)
}

/// The name an agent of the orchestration goes by: the record's (#217), else its worktree's.
pub fn agent_name_of(id: Option<&str>, feature: &str, known: &Known) -> String {
    id.and_then(|id| worker_of(known, id))
        .and_then(|worker| worker.name.clone())
        .unwrap_or_else(|| feature.to_string())
}

/// Why an agent can't take a next issue, or None (worktree/workers.mjs
/// `refusalOf`): no pane runs it, it works or waits for an answer, or it
/// starts fresh (`fresh`, why: `Fresh::why`, #445). `pane`: herdr's state
/// of its pane, None without one.
pub fn refusal_of(pane: Option<Option<&str>>, fresh: Option<String>) -> Option<String> {
    let Some(status) = pane else {
        return Some("no claude runs it (herdr knows no pane with its conversation)".into());
    };
    let status = status.unwrap_or("unknown");
    match status {
        "working" => return Some("it is working".into()),
        "blocked" => return Some("it waits for an answer".into()),
        "limit" => return Some("its run stopped at the usage limit".into()),
        "idle" | "done" => {}
        other => return Some(format!("herdr can't tell what it does ({other})")),
    }
    fresh
}

/// Where an announced handover's agent is: a worktree's (`dir`) or a free one (#289, no `dir`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAt {
    pub dir: Option<PathBuf>,
    pub name: String,
    pub id: String,
    pub is_free: bool,
}

/// An announced handover with where it stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Announced {
    pub number: i64,
    pub to: String,
    pub after: Vec<i64>,
    pub at: Option<String>,
    /// Its agent, None when no worktree of the orchestration has one of that name.
    pub agent: Option<AgentAt>,
    /// The awaited MRs not merged yet (one GitLab didn't answer for counts as open).
    pub open: Vec<i64>,
    /// All merged and the agent idle.
    pub is_due: bool,
    /// Why not, once the MRs are merged (#244).
    pub why: Option<String>,
}

/// The free agents of the record: `(id, worker)`.
pub type FreeAgent<'a> = (&'a str, &'a Worker);

/// Each announced handover with where it stands. `awaited`: iid → state from
/// GitLab, None before it answered: then nothing is due.
pub fn announced_of(
    handovers: &[Handover],
    issues: &[Entry],
    known: &Known,
    agents: &HashMap<String, Agent>,
    awaited: Option<&HashMap<i64, String>>,
    free: &[Worker],
) -> Vec<Announced> {
    handovers
        .iter()
        .map(|handover| {
            let in_tree = issues.iter().filter_map(|issue| issue.tree.as_ref()).find_map(|tree| {
                let claude = tree.claude.as_ref()?;
                let name = agent_name_of(Some(&claude.id), &tree.name, known);
                let was = worker_of(known, &claude.id).and_then(|worker| worker.was.as_deref());
                // Announced under its name before a handover renamed it (#251): the same agent.
                (name == handover.to || was == Some(handover.to.as_str())).then(|| AgentAt {
                    dir: Some(tree.dir.clone()),
                    name,
                    id: claude.id.clone(),
                    is_free: false,
                })
            });
            let agent = in_tree.or_else(|| {
                free.iter()
                    .find(|worker| {
                        worker.name.as_deref() == Some(handover.to.as_str())
                            || worker.was.as_deref() == Some(handover.to.as_str())
                    })
                    .map(|worker| AgentAt {
                        dir: None,
                        name: worker.name.clone().unwrap_or_default(),
                        id: worker.id.clone(),
                        is_free: true,
                    })
            });
            let open: Vec<i64> = handover
                .after
                .iter()
                .copied()
                .filter(|iid| awaited.and_then(|awaited| awaited.get(iid)).map(String::as_str) != Some("merged"))
                .collect();
            let why = match &agent {
                Some(agent) => {
                    let found = agents.get(&agent.id);
                    refusal_of(found.map(|agent| agent.status.as_deref()), None)
                }
                None => Some(format!("no worktree of the orchestration has an agent {}", handover.to)),
            };
            let is_merged = awaited.is_some() && open.is_empty();
            Announced {
                number: handover.number,
                to: handover.to.clone(),
                after: handover.after.clone(),
                at: handover.at.clone(),
                agent,
                is_due: is_merged && why.is_none(),
                why: if is_merged { why } else { None },
                open,
            }
        })
        .collect()
}

/// `!232`, `!232, !233`.
pub fn mrs_text(iids: &[i64]) -> String {
    iids.iter().map(|iid| format!("!{iid}")).collect::<Vec<_>>().join(", ")
}

/// What the node of the agent's worktree says: `→ next #226 after !232`, `→ next #226 now`.
pub fn next_text(number: i64, open: &[i64]) -> String {
    if open.is_empty() {
        format!("→ next #{number} now")
    } else {
        format!("→ next #{number} after {}", mrs_text(open))
    }
}

/// Why the clean-up of the agent's worktree waits: `its agent takes #226 after !232`.
pub fn takes_text(number: i64, open: &[i64]) -> String {
    if open.is_empty() {
        format!("its agent takes #{number} now")
    } else {
        format!("its agent takes #{number} after {}", mrs_text(open))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::conversations::Claude;
    use crate::model::Tree;

    pub fn claude_of(id: &str) -> Claude {
        Claude {
            id: id.into(),
            prompt: None,
            prompt_at: None,
            answer: None,
            answer_at: None,
            transcript: PathBuf::new(),
            cwd: PathBuf::new(),
            model: None,
            at: None,
            used: 0,
            window: 200_000,
        }
    }

    pub fn issues() -> Vec<Entry> {
        let tree = Tree {
            dir: "/w/224".into(),
            name: "224-x".into(),
            claude: Some(claude_of("c224")),
            ..Default::default()
        };
        vec![
            Entry {
                number: Some(224),
                feature: "224-x".into(),
                branch: "feature/224-x".into(),
                tree: Some(tree),
                ..Default::default()
            },
            Entry {
                number: Some(226),
                feature: "226-y".into(),
                branch: "feature/226-y".into(),
                is_queued: true,
                ..Default::default()
            },
        ]
    }
    pub fn handover() -> Handover {
        Handover {
            number: 226,
            to: "224-x".into(),
            after: vec![232],
            at: Some(String::new()),
        }
    }
    pub fn agents(status: Option<&str>) -> HashMap<String, Agent> {
        status
            .map(|status| {
                [(
                    "c224".to_string(),
                    Agent {
                        pane: "p".into(),
                        tab: "t".into(),
                        status: Some(status.into()),
                    },
                )]
                .into()
            })
            .unwrap_or_default()
    }
    fn awaited(state: &str) -> HashMap<i64, String> {
        [(232, state.to_string())].into()
    }

    #[test]
    fn finds_the_agents_worktree_and_what_it_waits_for() {
        let [waiting] = &announced_of(
            &[handover()],
            &issues(),
            &[],
            &agents(Some("idle")),
            Some(&awaited("opened")),
            &[],
        )[..] else {
            panic!()
        };
        assert_eq!(waiting.agent.as_ref().unwrap().dir, Some(PathBuf::from("/w/224")));
        assert_eq!(waiting.open, [232]);
        assert!(!waiting.is_due);
        assert_eq!(next_text(waiting.number, &waiting.open), "→ next #226 after !232");
        assert_eq!(
            takes_text(waiting.number, &waiting.open),
            "its agent takes #226 after !232"
        );
    }

    #[test]
    fn is_due_once_the_mrs_are_merged_and_the_agent_is_idle() {
        let due = |status: &str| {
            announced_of(
                &[handover()],
                &issues(),
                &[],
                &agents(Some(status)),
                Some(&awaited("merged")),
                &[],
            )[0]
            .is_due
        };
        assert!(due("idle"));
        assert!(due("done"));
        assert!(!due("working"));
        assert!(!announced_of(&[handover()], &issues(), &[], &agents(Some("idle")), None, &[])[0].is_due);
    }

    #[test]
    fn says_why_it_cant_go_once_the_mrs_are_merged() {
        let why = |status: Option<&str>, to: &str| {
            let handover = Handover {
                to: to.into(),
                ..handover()
            };
            announced_of(
                &[handover],
                &issues(),
                &[],
                &agents(status),
                Some(&awaited("merged")),
                &[],
            )[0]
            .why
            .clone()
        };
        assert_eq!(why(Some("idle"), "224-x"), None);
        assert_eq!(why(Some("blocked"), "224-x").as_deref(), Some("it waits for an answer"));
        assert_eq!(why(Some("working"), "224-x").as_deref(), Some("it is working"));
        assert!(why(None, "224-x").unwrap().starts_with("no claude runs it"));
        assert_eq!(
            why(Some("idle"), "gone").as_deref(),
            Some("no worktree of the orchestration has an agent gone")
        );
        let open = announced_of(
            &[handover()],
            &issues(),
            &[],
            &agents(Some("working")),
            Some(&awaited("opened")),
            &[],
        );
        assert_eq!(open[0].why, None);
    }

    #[test]
    fn knows_the_agent_by_the_records_name_and_its_name_before() {
        let known = [Worker {
            id: "c224".into(),
            name: Some("first-name".into()),
            ..Default::default()
        }];
        let handover = Handover {
            to: "first-name".into(),
            ..handover()
        };
        let found = announced_of(&[handover], &issues(), &known, &HashMap::new(), None, &[]);
        assert_eq!(found[0].agent.as_ref().unwrap().dir, Some(PathBuf::from("/w/224")));
        let known = [Worker {
            id: "c224".into(),
            name: Some("224-x".into()),
            was: Some("old-name".into()),
            ..Default::default()
        }];
        let handover = Handover {
            to: "old-name".into(),
            ..super::tests::handover()
        };
        let found = announced_of(&[handover], &issues(), &known, &HashMap::new(), None, &[]);
        assert_eq!(found[0].agent.as_ref().unwrap().name, "224-x");
    }

    #[test]
    fn finds_a_free_agent_without_a_worktree() {
        let free = vec![Worker {
            id: "f1".into(),
            name: Some("free-one".into()),
            ..Default::default()
        }];
        let handover = Handover {
            to: "free-one".into(),
            ..handover()
        };
        let found = announced_of(&[handover], &issues(), &[], &HashMap::new(), None, &free);
        assert_eq!(
            found[0].agent,
            Some(AgentAt {
                dir: None,
                name: "free-one".into(),
                id: "f1".into(),
                is_free: true
            })
        );
    }

    #[test]
    fn refuses_a_busy_agent_or_one_out_of_context() {
        let full = Some("its context holds 162 k tokens, above 150 k".to_string());
        assert_eq!(refusal_of(Some(Some("idle")), full.clone()), full);
        assert_eq!(refusal_of(Some(Some("idle")), None), None);
        assert_eq!(
            refusal_of(Some(Some("working")), full).as_deref(),
            Some("it is working")
        );
        assert_eq!(
            refusal_of(Some(None), None).as_deref(),
            Some("herdr can't tell what it does (unknown)")
        );
    }
}
