//! The conflict state of an open MR in MERGE REQUESTS (#246, #290,
//! resolve.mjs): a button that has its agent merge develop in and resolve the
//! conflicts, shown only while a click can do that. GitLab's `has_conflicts`
//! counts once its check is settled and it has the pushed head; before that
//! the row says it is checking. After a click the button turns into where the
//! order stands (queued, merging develop in, pushed) until GitLab no longer
//! reports conflicts. The clicks are the view's: it keeps a `Resolves`.

use super::Tree;
use crate::gitlab::Mr;
use crate::herdr::Agent;
use std::collections::HashMap;

/// How long a click's "didn't get in" stays next to the button.
const NOTE_MS: i64 = 10_000;
/// How long "conflicts resolved" stays before the row says "no merge conflicts".
const RESOLVED_MS: i64 = 60_000;

/// Whether GitLab's `has_conflicts` of `mr` is about its pushed head and
/// settled: its check isn't running, and it has the commit pushed last.
pub fn is_conflict_settled(mr: &Mr, tree: Option<&Tree>) -> bool {
    let check = mr.merge_check.as_deref().unwrap_or("");
    let status = mr.merge_status.as_deref().unwrap_or("");
    let is_rechecking = ["unchecked", "checking", "cannot_be_merged_recheck"].contains(&check);
    let is_checking = ["unchecked", "checking"].contains(&status);
    let pushed = tree.and_then(|tree| tree.pushed.as_deref());
    let is_old = pushed.is_some() && mr.sha.is_some() && mr.sha.as_deref() != pushed;
    !is_rechecking && !is_checking && !is_old
}

/// Whether GitLab reports settled conflicts for `mr`.
pub fn has_settled_conflicts(mr: &Mr, tree: Option<&Tree>) -> bool {
    mr.has_conflicts && is_conflict_settled(mr, tree)
}

/// The line its agent gets: merge develop in, resolve, check, push.
pub fn resolve_line(mr: &Mr) -> String {
    format!(
        "[overview] !{} has merge conflicts with develop: merge develop in (git fetch origin, git merge origin/develop), resolve the conflicts, run lint, typecheck and test of the packages they touch, commit the merge and push.",
        mr.iid
    )
}

/// Why a line can't reach the agent of `tree`, or None when it may (empty
/// `agents`: outside herdr, nothing is known then).
pub fn no_agent_of(tree: Option<&Tree>, agents: &HashMap<String, Agent>) -> Option<String> {
    let Some(tree) = tree else {
        return Some("no worktree, so no agent: merge develop in yourself".into());
    };
    let Some(claude) = &tree.claude else {
        return Some("no agent in its worktree".into());
    };
    if agents.is_empty() {
        return None;
    }
    match agents.get(&claude.id) {
        None => Some("its agent isn't running".into()),
        Some(agent) if agent.status.as_deref() == Some("blocked") => {
            Some("its agent is at a question: answer it first".into())
        }
        Some(_) => None,
    }
}

/// What a click did: sent or not, when, the commit on origin then.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolveClick {
    pub is_sent: bool,
    pub at: i64,
    pub pushed: Option<String>,
    pub is_started: bool,
    pub resolved_at: Option<i64>,
}

/// Where an open MR's conflicts stand, the start of its second line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// GitLab checks again (after a push): no button.
    Checking,
    /// No conflicts.
    None,
    /// The order went through, GitLab no longer reports them.
    Resolved,
    /// The button, `behind` the commits of develop it lacks, `is_refused` a click didn't get in.
    Button { behind: u64, is_refused: bool },
    /// Pushed, GitLab still has conflicts at that commit: the button again.
    LeftAfterPush,
    /// A grey button: why a click can't reach an agent.
    Grey { why: String },
    /// Where a sent order stands: `queued for its agent (at work)`, `sent to its
    /// agent`, `its agent merges develop in`, `pushed, GitLab checks`; since `at`.
    Sent { text: String, at: i64 },
}

impl Conflict {
    /// Whether a click on its button reaches the agent.
    pub fn can_click(&self) -> bool {
        matches!(self, Conflict::Button { .. } | Conflict::LeftAfterPush)
    }
}

/// The clicks of the resolve buttons, by the MR's iid, over the frames.
#[derive(Debug, Clone, Default)]
pub struct Resolves {
    clicks: HashMap<i64, ResolveClick>,
}

impl Resolves {
    /// The conflict state of an open `mr`; it moves on as it is read.
    pub fn state_of(
        &mut self,
        mr: &Mr,
        tree: Option<&Tree>,
        agents: &HashMap<String, Agent>,
        behind: u64,
        now: i64,
    ) -> Conflict {
        let is_settled = is_conflict_settled(mr, tree);
        if let Some(click) = self.clicks.get_mut(&mr.iid).filter(|click| click.is_sent) {
            let pushed = tree.and_then(|tree| tree.pushed.clone());
            if is_settled && !mr.has_conflicts {
                let resolved_at = *click.resolved_at.get_or_insert(now);
                if now - resolved_at < RESOLVED_MS {
                    return Conflict::Resolved;
                }
                self.clicks.remove(&mr.iid);
            } else if is_settled && click.is_started && pushed.is_some() && pushed != click.pushed {
                self.clicks.remove(&mr.iid);
                return Conflict::LeftAfterPush;
            } else {
                return progress_of(mr, tree, agents, behind, click);
            }
        }
        if !is_settled {
            return Conflict::Checking;
        }
        if !mr.has_conflicts {
            return Conflict::None;
        }
        if let Some(unpushed) = tree.map(|tree| tree.unpushed).filter(|unpushed| *unpushed > 0) {
            return Conflict::Grey {
                why: format!("conflicts as of the pushed commits · {unpushed} not pushed: its agent pushes first"),
            };
        }
        if let Some(why) = no_agent_of(tree, agents) {
            return Conflict::Grey { why };
        }
        let is_refused = self.clicks.get(&mr.iid).is_some_and(|click| now - click.at < NOTE_MS);
        Conflict::Button { behind, is_refused }
    }

    /// A click on the button: whether the order got in to the agent, the commit on origin then.
    pub fn clicked(&mut self, mr: &Mr, tree: Option<&Tree>, is_sent: bool, now: i64) {
        let pushed = tree.and_then(|tree| tree.pushed.clone());
        self.clicks.insert(
            mr.iid,
            ResolveClick {
                is_sent,
                at: now,
                pushed,
                is_started: false,
                resolved_at: None,
            },
        );
    }

    /// The automatic line about the conflicts got in: the row shows where it stands, as after a click.
    pub fn note_told(&mut self, mr: &Mr, tree: Option<&Tree>, now: i64) {
        self.clicked(mr, tree, true, now);
    }

    /// Forgets the clicks of MRs that are no longer open.
    pub fn forget_resolved(&mut self, open: &[i64]) {
        self.clicks.retain(|iid, _| open.contains(iid));
    }
}

/// Whether the agent took the order up: its last prompt is it, after the click.
fn has_started(mr: &Mr, tree: Option<&Tree>, click: &ResolveClick) -> bool {
    if click.is_started {
        return true;
    }
    let Some(claude) = tree.and_then(|tree| tree.claude.as_ref()) else {
        return false;
    };
    let prompt = claude.prompt.as_deref().unwrap_or("");
    let is_line = prompt.contains("has conflicts with develop") || prompt.contains("has merge conflicts with develop");
    is_line && prompt.contains(&format!("!{} ", mr.iid)) && claude.at.is_some_and(|at| at >= click.at)
}

fn progress_of(
    mr: &Mr,
    tree: Option<&Tree>,
    agents: &HashMap<String, Agent>,
    behind: u64,
    click: &mut ResolveClick,
) -> Conflict {
    if has_started(mr, tree, click) {
        click.is_started = true;
    }
    let text = if !click.is_started {
        let id = tree
            .and_then(|tree| tree.claude.as_ref())
            .map(|claude| claude.id.as_str())
            .unwrap_or("");
        let is_working = agents.get(id).and_then(|agent| agent.status.as_deref()) == Some("working");
        if is_working {
            "◐ queued for its agent (at work)"
        } else {
            "◐ sent to its agent"
        }
    } else {
        let is_pushed = tree.is_some_and(|tree| tree.unpushed == 0 && behind == 0 && tree.pushed != click.pushed);
        if is_pushed {
            "◐ pushed, GitLab checks"
        } else {
            "◐ its agent merges develop in"
        }
    };
    Conflict::Sent {
        text: text.into(),
        at: click.at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::handovers::tests::claude_of;

    fn mr(has_conflicts: bool) -> Mr {
        Mr {
            iid: 246,
            state: "opened".into(),
            has_conflicts,
            ..Default::default()
        }
    }
    fn agents(id: &str, status: Option<&str>) -> HashMap<String, Agent> {
        [(
            id.to_string(),
            Agent {
                pane: "p".into(),
                tab: "t".into(),
                status: status.map(String::from),
            },
        )]
        .into()
    }
    fn tree() -> Tree {
        Tree {
            dir: "/w".into(),
            claude: Some(claude_of("c")),
            pushed: Some("a1".into()),
            ..Default::default()
        }
    }
    const NOW: i64 = 1_000_000;

    #[test]
    fn says_no_conflicts_or_shows_the_button() {
        let mut resolves = Resolves::default();
        let working = agents("c", Some("working"));
        assert_eq!(resolves.state_of(&mr(false), None, &working, 0, NOW), Conflict::None);
        let state = resolves.state_of(&mr(true), Some(&tree()), &working, 48, NOW);
        assert_eq!(
            state,
            Conflict::Button {
                behind: 48,
                is_refused: false
            }
        );
        assert!(state.can_click());
    }

    #[test]
    fn shows_no_button_while_gitlab_checks_or_lacks_the_pushed_head() {
        let mut resolves = Resolves::default();
        let working = agents("c", Some("working"));
        for found in [
            Mr {
                merge_check: Some("checking".into()),
                ..mr(true)
            },
            Mr {
                merge_check: Some("cannot_be_merged_recheck".into()),
                ..mr(true)
            },
            Mr {
                merge_status: Some("unchecked".into()),
                ..mr(true)
            },
            Mr {
                sha: Some("old".into()),
                ..mr(true)
            },
        ] {
            assert_eq!(
                resolves.state_of(&found, Some(&tree()), &working, 0, NOW),
                Conflict::Checking
            );
        }
        assert!(is_conflict_settled(
            &Mr {
                sha: Some("a1".into()),
                ..mr(true)
            },
            Some(&tree())
        ));
    }

    #[test]
    fn greys_the_button_with_the_reason() {
        let mut resolves = Resolves::default();
        let grey = |resolves: &mut Resolves, tree: Option<&Tree>, given: &HashMap<String, Agent>| match resolves
            .state_of(&mr(true), tree, given, 0, NOW)
        {
            Conflict::Grey { why } => why,
            other => panic!("{other:?}"),
        };
        let working = agents("c", Some("working"));
        assert!(grey(&mut resolves, None, &working).contains("no worktree, so no agent"));
        assert!(grey(&mut resolves, Some(&tree()), &agents("c", Some("blocked"))).contains("at a question"));
        assert!(grey(&mut resolves, Some(&tree()), &agents("d", None)).contains("isn't running"));
        let unpushed = Tree { unpushed: 9, ..tree() };
        assert_eq!(
            grey(&mut resolves, Some(&unpushed), &working),
            "conflicts as of the pushed commits · 9 not pushed: its agent pushes first"
        );
    }

    #[test]
    fn follows_the_order_from_queued_to_resolved() {
        let mut resolves = Resolves::default();
        let working = agents("c", Some("working"));
        let found = Mr {
            iid: 290,
            sha: Some("a1".into()),
            ..mr(true)
        };
        resolves.note_told(&found, Some(&tree()), NOW);
        let text = |state: Conflict| match state {
            Conflict::Sent { text, .. } => text,
            other => format!("{other:?}"),
        };
        assert_eq!(
            text(resolves.state_of(&found, Some(&tree()), &working, 3, NOW)),
            "◐ queued for its agent (at work)"
        );
        let mut claude = claude_of("c");
        claude.prompt = Some(resolve_line(&found));
        claude.at = Some(NOW + 1000);
        let started = Tree {
            claude: Some(claude),
            ..tree()
        };
        assert_eq!(
            text(resolves.state_of(&found, Some(&started), &working, 3, NOW)),
            "◐ its agent merges develop in"
        );
        let pushed = Tree {
            pushed: Some("b2".into()),
            ..started.clone()
        };
        assert_eq!(
            text(resolves.state_of(&found, Some(&pushed), &working, 0, NOW)),
            "◐ pushed, GitLab checks"
        );
        let clean = Mr {
            has_conflicts: false,
            sha: Some("b2".into()),
            ..found.clone()
        };
        assert_eq!(
            resolves.state_of(&clean, Some(&pushed), &working, 0, NOW),
            Conflict::Resolved
        );
        assert_eq!(
            resolves.state_of(&clean, Some(&pushed), &working, 0, NOW + 120_000),
            Conflict::None
        );
    }

    #[test]
    fn offers_the_button_again_when_conflicts_are_left_after_the_push() {
        let mut resolves = Resolves::default();
        let working = agents("c", Some("working"));
        let found = Mr { iid: 291, ..mr(true) };
        resolves.note_told(&found, Some(&tree()), NOW);
        let mut claude = claude_of("c");
        claude.prompt = Some(resolve_line(&found));
        claude.at = Some(NOW + 1000);
        let started = Tree {
            claude: Some(claude),
            ..tree()
        };
        resolves.state_of(&found, Some(&started), &working, 3, NOW);
        let pushed = Tree {
            pushed: Some("b2".into()),
            ..started
        };
        let state = resolves.state_of(
            &Mr {
                sha: Some("b2".into()),
                ..found
            },
            Some(&pushed),
            &working,
            0,
            NOW,
        );
        assert_eq!(state, Conflict::LeftAfterPush);
        assert!(state.can_click());
    }

    #[test]
    fn names_what_keeps_the_order_from_its_agent() {
        let tree = tree();
        assert!(no_agent_of(None, &HashMap::new()).unwrap().contains("no worktree"));
        assert!(
            no_agent_of(Some(&Tree::default()), &HashMap::new())
                .unwrap()
                .contains("no agent")
        );
        assert!(
            no_agent_of(Some(&tree), &agents("d", None))
                .unwrap()
                .contains("isn't running")
        );
        assert!(
            no_agent_of(Some(&tree), &agents("c", Some("blocked")))
                .unwrap()
                .contains("at a question")
        );
        assert_eq!(no_agent_of(Some(&tree), &agents("c", Some("idle"))), None);
        assert_eq!(no_agent_of(Some(&tree), &HashMap::new()), None);
        let line = resolve_line(&mr(true));
        assert!(
            line.starts_with("[overview] !246 ")
                && line.contains("git merge origin/develop")
                && line.ends_with("push.")
        );
    }
}
