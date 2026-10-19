//! A requested change and work that isn't pushed (#228, worktree/change.mjs,
//! overview/change.mjs): facts on the worktree's node and its MR row; breadboard
//! pushes the commits itself (#450, core's `push`), no row asks for it.

use super::{Paint, Span, Tree};
use crate::gitlab::Mr;
use crate::push::Stage;

/// How long commits may sit unpushed before someone is needed.
pub const UNPUSHED_MS: i64 = 10 * 60_000;

/// Whether the change requested for a worktree's MR still blocks it: until a
/// commit after the request is pushed and the pipeline of that commit passed.
pub fn is_change_open(tree: Option<&Tree>, mr: Option<&Mr>) -> bool {
    let Some(tree) = tree else { return false };
    let Some(change) = &tree.change else { return false };
    if tree.pushed.is_none() || tree.pushed == change.head || tree.unpushed > 0 {
        return true;
    }
    let Some(pipeline) = mr.and_then(|mr| mr.pipeline.as_ref()) else {
        return true;
    };
    if pipeline.status != "success" {
        return true;
    }
    pipeline.sha.is_some() && pipeline.sha != tree.pushed
}

/// Whether the unpushed commits of a worktree are old enough to need someone.
pub fn is_unpushed_long(tree: &Tree, now: i64) -> bool {
    tree.unpushed > 0 && tree.unpushed_at.is_some_and(|at| now - at >= UNPUSHED_MS)
}

/// `change requested: <what>` while the change blocks the MR.
pub fn change_fact(tree: &Tree, mr: Option<&Mr>) -> Option<Span> {
    let change = tree.change.as_ref().filter(|_| is_change_open(Some(tree), mr))?;
    Some(Span::new(format!("change requested: {}", change.text), Paint::Yellow))
}

/// `2 commits + a merge of develop not pushed` (#450): what its upstream lacks.
pub fn unpushed_text(tree: &Tree) -> String {
    format!(
        "{} not pushed",
        crate::push::commits_text(tree.unpushed, tree.unpushed_own, tree.merged_base.as_deref())
    )
}

/// `2 commits not pushed`, `(no remote branch)` when origin has none, and
/// what breadboard's push is at (#450): `· wt:check running, then push`;
/// red once a person is needed, or once they sit there long with nothing at them.
pub fn unpushed_fact(tree: &Tree, now: i64) -> Option<Span> {
    if tree.unpushed == 0 {
        return None;
    }
    let remote = if tree.has_remote_branch {
        ""
    } else {
        " (no remote branch)"
    };
    let (at, paint) = match tree
        .pushing
        .as_ref()
        .map(|pushing| (pushing.stage, pushing.why.as_deref()))
    {
        Some((Stage::Checking, _)) => (" · wt:check running, then push".to_string(), Paint::Yellow),
        Some((Stage::Told, why)) => (
            format!(
                " · wt:check failed{}: its agent fixes it",
                why.map(|why| format!(" ({why})")).unwrap_or_default()
            ),
            Paint::Yellow,
        ),
        Some((Stage::Dirty, _)) => (" · uncommitted changes: its agent commits them".into(), Paint::Yellow),
        Some((Stage::Asked, why)) => (format!(" · {}: you look", why.unwrap_or("stuck")), Paint::Red),
        None if is_unpushed_long(tree, now) => (String::new(), Paint::Red),
        None => (String::new(), Paint::Yellow),
    };
    Some(Span::new(format!("{}{remote}{at}", unpushed_text(tree)), paint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Change;
    use crate::gitlab::Pipeline;
    use crate::time::parse_iso;

    fn tree(pushed: Option<&str>, unpushed: u64) -> Tree {
        Tree {
            change: Some(Change {
                text: "padding".into(),
                at: Some("2026-09-29T10:00:00Z".into()),
                mr: Some(7),
                head: Some("a".into()),
            }),
            pushed: pushed.map(String::from),
            unpushed,
            ..Default::default()
        }
    }
    fn mr(status: &str, sha: &str) -> Mr {
        Mr {
            pipeline: Some(Pipeline {
                status: status.into(),
                sha: Some(sha.into()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_change_is_closed_without_one() {
        assert!(!is_change_open(Some(&Tree::default()), Some(&mr("success", "b"))));
        assert!(!is_change_open(None, Some(&mr("success", "b"))));
    }

    #[test]
    fn a_change_blocks_until_a_later_commit_is_pushed_and_passed() {
        assert!(is_change_open(Some(&tree(Some("a"), 0)), Some(&mr("success", "a"))));
        assert!(is_change_open(Some(&tree(Some("b"), 1)), Some(&mr("success", "b"))));
        assert!(is_change_open(Some(&tree(None, 0)), Some(&mr("success", "b"))));
        assert!(is_change_open(Some(&tree(Some("b"), 0)), Some(&mr("running", "b"))));
        assert!(is_change_open(Some(&tree(Some("b"), 0)), Some(&mr("success", "a"))));
        assert!(!is_change_open(Some(&tree(Some("b"), 0)), Some(&mr("success", "b"))));
    }

    #[test]
    fn unpushed_commits_are_long_past_ten_minutes() {
        let now = parse_iso("2026-09-29T12:00:00Z").unwrap();
        let at = |ms: i64| Tree {
            unpushed: 2,
            unpushed_at: Some(now - ms),
            ..Default::default()
        };
        assert!(is_unpushed_long(&at(UNPUSHED_MS), now));
        assert!(!is_unpushed_long(&at(UNPUSHED_MS - 1), now));
        assert!(!is_unpushed_long(
            &Tree {
                unpushed: 0,
                ..at(UNPUSHED_MS)
            },
            now
        ));
        let fact = unpushed_fact(&at(UNPUSHED_MS), now).unwrap();
        assert_eq!(
            (fact.text.as_str(), fact.paint),
            ("2 commits not pushed (no remote branch)", Paint::Red)
        );
    }

    #[test]
    fn says_what_breadboards_push_is_at() {
        let now = parse_iso("2026-09-29T12:00:00Z").unwrap();
        let at = |stage: Stage, why: Option<&str>| Tree {
            unpushed: 77,
            unpushed_own: 2,
            merged_base: Some("develop".into()),
            unpushed_at: Some(now - 3 * UNPUSHED_MS),
            has_remote_branch: true,
            pushing: Some(crate::push::Pushing {
                head: "b".into(),
                stage,
                at: now,
                reds: 1,
                why: why.map(String::from),
            }),
            ..Default::default()
        };
        let text = |tree: &Tree| unpushed_fact(tree, now).map(|span| (span.text, span.paint)).unwrap();
        assert_eq!(
            text(&at(Stage::Checking, None)),
            (
                "2 commits + a merge of develop not pushed · wt:check running, then push".into(),
                Paint::Yellow
            )
        );
        assert_eq!(
            text(&at(Stage::Told, Some("vita-web test"))).0,
            "2 commits + a merge of develop not pushed · wt:check failed (vita-web test): its agent fixes it"
        );
        assert_eq!(
            text(&at(Stage::Asked, Some("push refused"))),
            (
                "2 commits + a merge of develop not pushed · push refused: you look".into(),
                Paint::Red
            )
        );
    }
}
