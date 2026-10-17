//! A worktree left half-merged (#396): a merge open in it (MERGE_HEAD) for
//! longer than HALF_MERGED_MS with no run of its agent in flight, a run cut
//! off mid-merge or a merge nobody finished. A fact on its node and an
//! automatic `half-merged` that tells its agent to finish or abort it.

use super::{Paint, Span, Tree};
use crate::time::{clock, to_iso};

/// How long a merge may stay open before someone is needed.
pub const HALF_MERGED_MS: i64 = 10 * 60_000;

/// `half-merged since 14:02`: yellow while the merge is young, red once it is old.
pub fn half_merged_fact(tree: &Tree, now: i64) -> Option<Span> {
    let at = tree.merge_head_at?;
    let paint = if now - at >= HALF_MERGED_MS {
        Paint::Red
    } else {
        Paint::Yellow
    };
    Some(Span::new(format!("half-merged since {}", clock(at, now)), paint))
}

/// The automatic `half-merged` of a worktree: its text, when, the key it is
/// sent under, the line its click tells the agent.
#[derive(Debug, Clone, PartialEq)]
pub struct HalfMerged {
    pub key: String,
    pub text: String,
    pub at: String,
    pub line: String,
    pub is_sent: bool,
}

/// None while the merge is young or a run of its agent is in flight (it may
/// be resolving it right now).
pub fn half_merged_action_of(branch: &str, tree: &Tree, is_running: bool, now: i64) -> Option<HalfMerged> {
    let at = tree.merge_head_at.filter(|at| now - at >= HALF_MERGED_MS)?;
    if is_running {
        return None;
    }
    let since = clock(at, now);
    let key = format!("half-merged:{branch}:{at}");
    let is_sent = tree.sent.contains(&key);
    let line = format!(
        "[overview] {branch} is half-merged since {since} (a merge is open: MERGE_HEAD, maybe conflict markers): finish it now (resolve the files, git add, git commit --no-edit, {run} wt:check, push) or abort it (git merge --abort, the worktree is as before) and merge again with {run} wt:merge-base.",
        run = crate::config::run_word()
    );
    Some(HalfMerged {
        text: if is_sent {
            format!("half-merged since {since}: its agent was told")
        } else {
            format!("half-merged since {since}: click to tell its agent to finish or abort it")
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
    use crate::time::parse_iso;

    fn tree(at: Option<i64>) -> Tree {
        Tree {
            merge_head_at: at,
            ..Default::default()
        }
    }

    #[test]
    fn flags_a_merge_open_for_long_with_no_run_in_flight() {
        let now = parse_iso("2026-10-16T12:00:00Z").unwrap();
        let old = now - HALF_MERGED_MS;
        assert_eq!(half_merged_action_of("feature/7-x", &tree(None), false, now), None);
        assert_eq!(
            half_merged_action_of("feature/7-x", &tree(Some(now - 60_000)), false, now),
            None
        );
        assert_eq!(half_merged_action_of("feature/7-x", &tree(Some(old)), true, now), None);
        let found = half_merged_action_of("feature/7-x", &tree(Some(old)), false, now).unwrap();
        let since = clock(old, now);
        assert_eq!(
            found.text,
            format!("half-merged since {since}: click to tell its agent to finish or abort it")
        );
        assert!(found.line.contains("git merge --abort"));
        assert!(found.line.contains("git commit --no-edit"));
        assert!(!found.is_sent);

        let mut told = tree(Some(old));
        told.sent.insert(found.key.clone());
        let again = half_merged_action_of("feature/7-x", &told, false, now).unwrap();
        assert!(again.is_sent);
        assert_eq!(again.text, format!("half-merged since {since}: its agent was told"));
    }

    #[test]
    fn says_since_when_on_the_worktree_red_once_old() {
        let now = parse_iso("2026-10-16T12:00:00Z").unwrap();
        assert_eq!(half_merged_fact(&tree(None), now), None);
        let young = half_merged_fact(&tree(Some(now - 60_000)), now).unwrap();
        assert!(young.text.starts_with("half-merged since "));
        assert_eq!(young.paint, Paint::Yellow);
        let old = half_merged_fact(&tree(Some(now - HALF_MERGED_MS)), now).unwrap();
        assert_eq!(old.paint, Paint::Red);
    }
}
