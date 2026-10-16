//! The orchestrator's own context (#382): how full its chat is, whether this
//! is a quiet point for a fresh start, and the last lines of its journal.
//! Above `[orchestrator] fresh_above` and quiet, the overview offers the
//! fresh start (or makes it, `auto_fresh`); the title band says where it is.

use super::order::Item;
use crate::herdr::Agent;

/// The kinds of action that ask the user something and wait for the answer.
const QUESTIONS: [&str; 2] = ["decide", "plan"];

/// What the overview knows of the orchestrator's chat.
#[derive(Debug, Clone, PartialEq)]
pub struct Orchestrator {
    /// The share of its context used (0 to 1), None while no conversation is known.
    pub share: Option<f64>,
    /// From which share on it gets a fresh start.
    pub fresh_above: f64,
    /// Whether the overview starts it fresh by itself at a quiet point.
    pub is_auto: bool,
    /// Why now is no quiet point; None: it is one.
    pub hold: Option<String>,
    /// The last lines of its journal, oldest first.
    pub journal: Vec<String>,
}

impl Orchestrator {
    /// Above the threshold: due for a fresh start.
    pub fn is_due(&self) -> bool {
        self.share.is_some_and(|share| share >= self.fresh_above)
    }

    /// Due, and nothing holds it: the fresh start is on offer now.
    pub fn is_ready(&self) -> bool {
        self.is_due() && self.hold.is_none()
    }

    /// The title band's part: `orchestrator 62 % · restart at a quiet point`.
    pub fn band(&self) -> Option<String> {
        let share = self.share?;
        let percent = format!("orchestrator {} %", (share * 100.0).round() as i64);
        Some(match (self.is_due(), &self.hold) {
            (false, _) => percent,
            (true, Some(_)) => format!("{percent} · restart at a quiet point"),
            (true, None) if self.is_auto => format!("{percent} · restarts now"),
            (true, None) => format!("{percent} · fresh start ready"),
        })
    }
}

/// Why now is no quiet point for a fresh start, None when it is one: the
/// chat must be there and idle, no question to the user open.
pub fn hold_of(has_chat: bool, agent: Option<&Agent>, actions: &[Item]) -> Option<String> {
    if !has_chat {
        return Some("its chat is not running".into());
    }
    if agent.is_some_and(|agent| agent.status.as_deref() == Some("working")) {
        return Some("it is working".into());
    }
    if let Some(question) = actions.iter().find(|item| {
        QUESTIONS.contains(&item.kind.as_str()) && item.action.as_ref().is_none_or(|action| action.answer.is_none())
    }) {
        return Some(format!(
            "a question to the user is open ({}: {})",
            question.kind, question.who
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str) -> Item {
        Item {
            kind: kind.into(),
            who: "7-x".into(),
            ..Item::default()
        }
    }

    fn orchestrator(share: f64, hold: Option<&str>) -> Orchestrator {
        Orchestrator {
            share: Some(share),
            fresh_above: 0.5,
            is_auto: false,
            hold: hold.map(String::from),
            journal: Vec::new(),
        }
    }

    #[test]
    fn says_its_context_and_when_it_restarts() {
        assert_eq!(orchestrator(0.32, None).band().as_deref(), Some("orchestrator 32 %"));
        assert_eq!(
            orchestrator(0.62, Some("it is working")).band().as_deref(),
            Some("orchestrator 62 % · restart at a quiet point")
        );
        assert_eq!(
            orchestrator(0.62, None).band().as_deref(),
            Some("orchestrator 62 % · fresh start ready")
        );
        assert!(orchestrator(0.62, None).is_ready());
        assert!(!orchestrator(0.62, Some("x")).is_ready());
        assert!(!orchestrator(0.49, None).is_ready());
        let none = Orchestrator {
            share: None,
            ..orchestrator(0.0, None)
        };
        assert_eq!(none.band(), None);
    }

    #[test]
    fn a_quiet_point_has_an_idle_chat_and_no_open_question() {
        let agent = |status: &str| Agent {
            pane: "p".into(),
            tab: "t".into(),
            status: Some(status.into()),
        };
        let (idle, working) = (agent("idle"), agent("working"));
        assert_eq!(hold_of(true, Some(&idle), &[item("try"), item("merge")]), None);
        assert_eq!(hold_of(true, None, &[]), None);
        assert_eq!(
            hold_of(true, Some(&idle), &[item("decide")]).as_deref(),
            Some("a question to the user is open (decide: 7-x)")
        );
        assert_eq!(hold_of(true, Some(&working), &[]).as_deref(), Some("it is working"));
        assert!(hold_of(false, Some(&idle), &[]).is_some());
    }
}
