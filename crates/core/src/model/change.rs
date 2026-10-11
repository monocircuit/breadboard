//! A requested change and work that isn't pushed (#228, worktree/change.mjs,
//! overview/change.mjs): facts on the worktree's node and its MR row, and an
//! automatic `push` once commits sit unpushed for long.

use super::{Paint, Span, Tree, counted};
use crate::gitlab::Mr;
use crate::time::to_iso;

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

/// `2 commits not pushed`, red once they sit there long; `(no remote branch)` when origin has none.
pub fn unpushed_fact(tree: &Tree, now: i64) -> Option<Span> {
    if tree.unpushed == 0 {
        return None;
    }
    let text = format!(
        "{} not pushed{}",
        counted(tree.unpushed, "commit"),
        if tree.has_remote_branch {
            ""
        } else {
            " (no remote branch)"
        }
    );
    Some(Span::new(
        text,
        if is_unpushed_long(tree, now) {
            Paint::Red
        } else {
            Paint::Yellow
        },
    ))
}

/// How long, as `25 min` or `3 h`.
fn since(at: i64, now: i64) -> String {
    let minutes = ((now - at) as f64 / 60_000.0).round() as i64;
    if minutes < 120 {
        format!("{minutes} min")
    } else {
        format!("{} h", (minutes as f64 / 60.0).round() as i64)
    }
}

/// The automatic `push` of a worktree holding commits origin doesn't have for
/// longer than UNPUSHED_MS: its text, when, the key it is sent under, the
/// line its click tells the agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Unpushed {
    pub key: String,
    pub text: String,
    pub at: String,
    pub line: String,
    pub is_sent: bool,
}

/// unpushedActionOf: None while the commits are young. When `mr` is merged
/// the push guard refuses the branch: the commits need an issue of their own.
pub fn unpushed_action_of(branch: &str, tree: &Tree, mr: Option<&Mr>, now: i64) -> Option<Unpushed> {
    if !is_unpushed_long(tree, now) {
        return None;
    }
    let at = tree.unpushed_at.unwrap_or(now);
    let key = format!(
        "unpushed:{branch}:{}:{}",
        tree.unpushed,
        tree.pushed.as_deref().unwrap_or("none")
    );
    let fact = format!(
        "{} for {}",
        unpushed_fact(tree, now).map(|span| span.text).unwrap_or_default(),
        since(at, now)
    );
    let is_sent = tree.sent.contains(&key);
    let commits = format!("{} of {branch}", counted(tree.unpushed, "commit"));
    let line = match mr.filter(|mr| mr.is_merged()) {
        Some(mr) => format!(
            "[overview] {commits} came after !{} was merged and are not pushed: they get their own issue, branch and draft MR now (glab issue create, pnpm wt:new, cherry-pick them, push, pnpm wt:mr), waiting with pnpm wt:wait if they must.",
            mr.iid
        ),
        None => format!(
            "[overview] {commits} {} not pushed: push now (git push{}), a draft MR from the first commit (pnpm wt:mr). Waiting for another MR comes after the push.",
            if tree.unpushed == 1 { "is" } else { "are" },
            if tree.has_remote_branch { "" } else { " -u origin HEAD" }
        ),
    };
    Some(Unpushed {
        text: if is_sent {
            format!("{fact}: its agent was told")
        } else {
            format!("{fact}: click to tell its agent")
        },
        at: to_iso(at),
        key,
        line,
        is_sent,
    })
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
        let action = unpushed_action_of("feature/9-x", &at(25 * 60_000), None, now).unwrap();
        assert_eq!(
            action.text,
            "2 commits not pushed (no remote branch) for 25 min: click to tell its agent"
        );
        assert!(action.line.contains("push now (git push -u origin HEAD)"));
        assert!(unpushed_action_of("feature/9-x", &at(60_000), None, now).is_none());
    }
}
