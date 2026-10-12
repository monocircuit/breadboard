//! A merged worktree goes by itself (#227, clean-up.mjs): the live overview
//! of its orchestration runs wt:remove once nothing is left in it and its
//! agent is idle; the agent goes free first (#289) and the removal waits
//! until it left. What stops it is named on its node.

use super::{Tree, counted};
use crate::conversations::left_text;
use crate::gitlab::Mr;

/// Where the clean-up of a merged worktree stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanUp {
    /// Changes or commits not in develop: wt:remove would refuse.
    Kept(String),
    /// Something still needs it: an agent in it, a try on the test instance, a handover.
    Waits(String),
    /// Nothing stops it.
    Due,
}

/// cleanUpOf: None for a worktree whose MR isn't merged. `status` herdr's
/// state of its agent, `hold` why a handover holds the agent.
pub fn clean_up_of(tree: Option<&Tree>, mr: Option<&Mr>, status: Option<&str>, hold: Option<&str>) -> Option<CleanUp> {
    let tree = tree?;
    if !mr.is_some_and(Mr::is_merged) {
        return None;
    }
    let mut kept = Vec::new();
    if tree.changed > 0 {
        kept.push(format!("{} changed", tree.changed));
    }
    if tree.own_commits > 0 {
        kept.push(format!(
            "{} not in {}",
            counted(tree.own_commits as u64, "commit"),
            tree.base.local
        ));
    }
    if !kept.is_empty() {
        return Some(CleanUp::Kept(kept.join(", ")));
    }
    if !tree.left.is_empty() {
        return Some(CleanUp::Waits(format!("{} still stands in it", left_text(&tree.left))));
    }
    match status {
        Some("working") => return Some(CleanUp::Waits("its agent is working".into())),
        Some("blocked") => return Some(CleanUp::Waits("its agent asks you".into())),
        _ => {}
    }
    if tree
        .actions
        .iter()
        .any(|action| action.on.as_deref() == Some("test") && action.answer.is_none())
    {
        return Some(CleanUp::Waits("its try on the test instance is open".into()));
    }
    if let Some(hold) = hold {
        return Some(CleanUp::Waits(hold.into()));
    }
    Some(CleanUp::Due)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Action;
    use crate::conversations::{Running, left_in};
    use std::path::{Path, PathBuf};

    fn merged() -> Mr {
        Mr {
            state: "merged".into(),
            ..Default::default()
        }
    }
    fn tree() -> Tree {
        Tree {
            dir: "/w".into(),
            ..Default::default()
        }
    }

    #[test]
    fn is_due_for_a_merged_worktree_with_nothing_left() {
        assert_eq!(
            clean_up_of(Some(&tree()), Some(&merged()), Some("idle"), None),
            Some(CleanUp::Due)
        );
        assert_eq!(
            clean_up_of(Some(&tree()), Some(&merged()), None, None),
            Some(CleanUp::Due)
        );
        let open = Mr {
            state: "opened".into(),
            ..Default::default()
        };
        assert_eq!(clean_up_of(Some(&tree()), Some(&open), None, None), None);
        assert_eq!(clean_up_of(None, Some(&merged()), None, None), None);
    }

    #[test]
    fn keeps_what_wt_remove_would_refuse() {
        let busy = Tree {
            changed: 2,
            own_commits: 3,
            ..tree()
        };
        assert_eq!(
            clean_up_of(Some(&busy), Some(&merged()), None, None),
            Some(CleanUp::Kept("2 changed, 3 commits not in develop".into()))
        );
    }

    #[test]
    fn waits_for_a_try_on_test_its_agent_and_a_handover() {
        let on_test = Action {
            id: 1,
            kind: "try".into(),
            on: Some("test".into()),
            url: Some("/atlas".into()),
            ..Default::default()
        };
        let trying = Tree {
            actions: vec![on_test.clone()],
            ..tree()
        };
        assert_eq!(
            clean_up_of(Some(&trying), Some(&merged()), None, None),
            Some(CleanUp::Waits("its try on the test instance is open".into()))
        );
        let answered = Tree {
            actions: vec![Action {
                answer: Some("looks good".into()),
                ..on_test
            }],
            ..tree()
        };
        assert_eq!(
            clean_up_of(Some(&answered), Some(&merged()), None, None),
            Some(CleanUp::Due)
        );
        assert_eq!(
            clean_up_of(Some(&tree()), Some(&merged()), Some("working"), None),
            Some(CleanUp::Waits("its agent is working".into()))
        );
        assert_eq!(
            clean_up_of(Some(&tree()), Some(&merged()), Some("blocked"), None),
            Some(CleanUp::Waits("its agent asks you".into()))
        );
        assert_eq!(
            clean_up_of(
                Some(&tree()),
                Some(&merged()),
                Some("idle"),
                Some("its agent takes #226 after !232")
            ),
            Some(CleanUp::Waits("its agent takes #226 after !232".into()))
        );
    }

    #[test]
    fn waits_while_a_handed_over_agent_still_stands_in_it() {
        let agent = Running {
            pid: 7,
            id: "c242".into(),
            name: Some("242-x".into()),
            cwd: PathBuf::from("/w/242"),
        };
        let old = |cwd: &str| Tree {
            dir: "/w/242".into(),
            left: left_in(
                Path::new("/w/242"),
                &[Running {
                    cwd: PathBuf::from(cwd),
                    ..agent.clone()
                }],
                None,
            ),
            ..Default::default()
        };
        assert_eq!(
            clean_up_of(Some(&old("/w/242")), Some(&merged()), None, None),
            Some(CleanUp::Waits("242-x still stands in it".into()))
        );
        assert!(matches!(
            clean_up_of(Some(&old("/w/242/packages/x")), Some(&merged()), None, None),
            Some(CleanUp::Waits(_))
        ));
        assert_eq!(
            clean_up_of(Some(&old("/w/246")), Some(&merged()), None, None),
            Some(CleanUp::Due)
        );
        assert!(
            left_in(
                Path::new("/w/242"),
                &[Running {
                    cwd: PathBuf::from("/w/2420"),
                    ..agent.clone()
                }],
                None
            )
            .is_empty()
        );
        assert!(left_in(Path::new("/w/242"), std::slice::from_ref(&agent), Some("c242")).is_empty());
    }
}
