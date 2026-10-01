//! What each agent of an orchestration worked on (#264, worktree/history.mjs),
//! for the statistics of reuse: does handing an idle agent the next issue pay
//! off, how often does it fail. An agent of before has only `issues`: its
//! history is read from that.

use super::Entry;
use crate::record::{Start, Worker};

/// The issues of a worker in order: its `history`, else, for one recorded
/// before it, its `issues`, the first fresh and the others handed over.
pub fn history_of(worker: &Worker) -> Vec<Start> {
    if let Some(history) = &worker.history {
        return history.clone();
    }
    worker
        .issues
        .iter()
        .enumerate()
        .map(|(i, number)| Start {
            number: *number,
            how: if i == 0 { "fresh" } else { "handover" }.into(),
            at: None,
            context: None,
        })
        .collect()
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`, `22nd`.
pub fn ordinal(n: usize) -> String {
    let tens = n % 100;
    if (11..=13).contains(&tens) {
        return format!("{n}th");
    }
    let suffix = match n % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn percent(share: f64) -> String {
    format!("{}%", (share * 100.0).round() as i64)
}

/// Which issue of its agent this is, for the node of the issue `number`:
/// `agent: 1st issue` for a fresh one, `agent: 3rd issue (#235 → #240 at 31% →
/// #252 at 38%)` for a reused one. `has_claude`: its worktree runs an agent;
/// one the record knows nothing of yet started fresh. "" when unknown.
pub fn history_text(known: &[Worker], number: Option<i64>, has_claude: bool) -> String {
    let found = number.and_then(|number| {
        known.iter().find_map(|worker| {
            let history = history_of(worker);
            let index = history.iter().position(|start| start.number == number)?;
            Some((history, index))
        })
    });
    let Some((history, index)) = found else {
        return if has_claude {
            "agent: 1st issue".into()
        } else {
            String::new()
        };
    };
    let place = format!("agent: {} issue", ordinal(index + 1));
    if history.len() == 1 {
        return place;
    }
    let chain: Vec<String> = history
        .iter()
        .enumerate()
        .map(|(i, start)| match start.context.filter(|_| i > 0) {
            Some(context) => format!("#{} at {}", start.number, percent(context)),
            None => format!("#{}", start.number),
        })
        .collect();
    format!("{place} ({})", chain.join(" → "))
}

/// How reuse works in an orchestration (reuseOf).
#[derive(Debug, Clone, PartialEq)]
pub struct Reuse {
    /// How many agents worked on its issues.
    pub agents: usize,
    /// How many issues went to an agent.
    pub issues: usize,
    /// How many of them went to one that had worked on another before.
    pub reused: usize,
    /// The share of its context such an agent started on, on average.
    pub context: Option<f64>,
    /// The failed handovers.
    pub failed: usize,
}

pub fn reuse_of(issues: &[Entry], known: &[Worker]) -> Reuse {
    let worked: Vec<&Entry> = issues
        .iter()
        .filter(|issue| !issue.is_queued && issue.why.is_none())
        .collect();
    let histories: Vec<Vec<Start>> = known
        .iter()
        .map(history_of)
        .filter(|history| !history.is_empty())
        .collect();
    let covered: Vec<i64> = histories.iter().flatten().map(|start| start.number).collect();
    let later: Vec<&Start> = histories.iter().flat_map(|history| history.iter().skip(1)).collect();
    let contexts: Vec<f64> = later.iter().filter_map(|start| start.context).collect();
    Reuse {
        agents: histories.len()
            + worked
                .iter()
                .filter(|issue| issue.number.is_none_or(|number| !covered.contains(&number)))
                .count(),
        issues: worked.len(),
        reused: later.len(),
        context: (!contexts.is_empty()).then(|| contexts.iter().sum::<f64>() / contexts.len() as f64),
        failed: known.iter().map(|worker| worker.failed.len()).sum(),
    }
}

/// `agents 14 · issues 22 · reused 5, started at 34% · handovers failed 2`.
pub fn reuse_text(reuse: &Reuse) -> String {
    let context = reuse
        .context
        .map(|context| format!(", started at {}", percent(context)))
        .unwrap_or_default();
    format!(
        "agents {} · issues {} · reused {}{context} · handovers failed {}",
        reuse.agents, reuse.issues, reuse.reused, reuse.failed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Failure;

    fn start(number: i64, how: &str, context: f64) -> Start {
        Start {
            number,
            how: how.into(),
            at: Some("t".into()),
            context: Some(context),
        }
    }
    fn worker(id: &str, history: Vec<Start>) -> Worker {
        Worker {
            id: id.into(),
            history: Some(history),
            ..Default::default()
        }
    }

    #[test]
    fn says_which_issue_of_its_agent_it_is() {
        let known = [
            worker(
                "a",
                vec![
                    start(235, "fresh", 0.0),
                    start(240, "handover", 0.31),
                    start(252, "announced", 0.38),
                ],
            ),
            worker("b", vec![start(236, "fresh", 0.0)]),
        ];
        assert_eq!(
            history_text(&known, Some(252), true),
            "agent: 3rd issue (#235 → #240 at 31% → #252 at 38%)"
        );
        assert!(history_text(&known, Some(235), false).starts_with("agent: 1st issue ("));
        assert_eq!(history_text(&known, Some(236), true), "agent: 1st issue");
        assert_eq!(history_text(&[], Some(7), true), "agent: 1st issue");
        assert_eq!(history_text(&[], Some(7), false), "");
        let old = [Worker {
            id: "a".into(),
            issues: vec![1, 2],
            ..Default::default()
        }];
        assert_eq!(history_text(&old, Some(2), false), "agent: 2nd issue (#1 → #2)");
    }

    #[test]
    fn names_the_place() {
        let places: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 101].into_iter().map(ordinal).collect();
        assert_eq!(
            places,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "101st"
            ]
        );
    }

    #[test]
    fn counts_agents_issues_reused_ones_and_failed_handovers() {
        let issue = |number: i64| Entry {
            number: Some(number),
            ..Default::default()
        };
        let issues = vec![
            issue(1),
            issue(2),
            issue(3),
            issue(4),
            Entry {
                is_queued: true,
                ..issue(5)
            },
            Entry {
                why: Some("failed: no worktree".into()),
                ..issue(6)
            },
        ];
        let failure = |number: i64| Failure {
            number,
            reason: "x".into(),
            at: Some("t".into()),
            times: 1,
        };
        let known = [
            Worker {
                failed: vec![failure(3)],
                ..worker(
                    "a",
                    vec![
                        start(1, "fresh", 0.0),
                        start(2, "handover", 0.2),
                        start(3, "announced", 0.4),
                    ],
                )
            },
            Worker {
                failed: vec![failure(5)],
                ..worker("b", vec![])
            },
        ];
        let reuse = reuse_of(&issues, &known);
        assert_eq!((reuse.agents, reuse.issues, reuse.reused, reuse.failed), (2, 4, 2, 2));
        assert!((reuse.context.unwrap() - 0.3).abs() < 1e-9);
        assert_eq!(
            reuse_text(&reuse),
            "agents 2 · issues 4 · reused 2, started at 30% · handovers failed 2"
        );
        let old = [Worker {
            id: "a".into(),
            name: Some("x".into()),
            issues: vec![1, 2],
            ..Default::default()
        }];
        assert_eq!(
            reuse_of(&[issue(1), issue(2), issue(3)], &old),
            Reuse {
                agents: 2,
                issues: 3,
                reused: 1,
                context: None,
                failed: 0
            }
        );
    }
}
