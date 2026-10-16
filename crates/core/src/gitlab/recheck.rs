//! GitLab's mergeability of an open MR, asked for when it sits unknown
//! (#391): after its target moves, GitLab resets `detailed_merge_status` to
//! `unchecked` and computes it again only when someone asks. The read asks
//! with `with_merge_status_recheck=true` and takes the answer's status, once
//! per MR every two minutes while it stays unchecked; one seen settled is
//! forgotten, so the next move of develop is asked about at once.

use super::client::Gitlab;
use super::rest::mr_of;
use super::{BranchRemote, Mr};
use std::collections::HashMap;

/// How long a `checking` may last before it is asked about.
const CHECKING_MS: i64 = 60_000;
/// How long after an ask the same MR is asked again.
const AGAIN_MS: i64 = 120_000;

/// Of one MR: since when GitLab says `checking`, when it was asked last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Recheck {
    pub checking_since: Option<i64>,
    pub asked_at: Option<i64>,
}

/// Whether GitLab hasn't settled `mr`'s mergeability.
pub fn is_unsettled(mr: &Mr) -> bool {
    matches!(mr.merge_status.as_deref(), Some("unchecked" | "checking"))
}

/// Whether `mr` is asked about now; moves on its entry in `rechecks`, drops it once settled.
pub fn is_due(rechecks: &mut HashMap<i64, Recheck>, mr: &Mr, now: i64) -> bool {
    if !mr.is_open() || !is_unsettled(mr) {
        rechecks.remove(&mr.iid);
        return false;
    }
    let recheck = rechecks.entry(mr.iid).or_default();
    let is_stuck = match mr.merge_status.as_deref() {
        Some("checking") => now - *recheck.checking_since.get_or_insert(now) >= CHECKING_MS,
        _ => {
            recheck.checking_since = None;
            true
        }
    };
    let is_time = recheck.asked_at.is_none_or(|at| now - at >= AGAIN_MS);
    if is_stuck && is_time {
        recheck.asked_at = Some(now);
    }
    is_stuck && is_time
}

/// Asks GitLab to recheck each open MR of `of` that is due, and takes what it answers.
pub async fn recheck(
    gitlab: &Gitlab,
    rechecks: &mut HashMap<i64, Recheck>,
    of: &mut HashMap<String, BranchRemote>,
    now: i64,
) {
    for mr in of.values_mut().filter_map(|remote| remote.mr.as_mut()) {
        if !is_due(rechecks, mr, now) {
            continue;
        }
        let endpoint = format!("merge_requests/{}?with_merge_status_recheck=true", mr.iid);
        let Ok(answer) = gitlab.get_json(&endpoint).await else {
            continue;
        };
        let (fresh, _) = mr_of(&answer);
        if fresh.iid != mr.iid {
            continue;
        }
        mr.merge_status = fresh.merge_status;
        mr.merge_check = fresh.merge_check;
        mr.has_conflicts = fresh.has_conflicts;
        if !is_unsettled(mr) {
            rechecks.remove(&mr.iid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mr(status: &str) -> Mr {
        Mr {
            iid: 7,
            state: "opened".into(),
            merge_status: Some(status.into()),
            ..Mr::default()
        }
    }

    #[test]
    fn asks_about_unchecked_at_once_then_every_two_minutes() {
        let mut rechecks = HashMap::new();
        assert!(is_due(&mut rechecks, &mr("unchecked"), 0));
        assert!(!is_due(&mut rechecks, &mr("unchecked"), AGAIN_MS - 1));
        assert!(is_due(&mut rechecks, &mr("unchecked"), AGAIN_MS));
    }

    #[test]
    fn asks_about_checking_after_a_minute() {
        let mut rechecks = HashMap::new();
        assert!(!is_due(&mut rechecks, &mr("checking"), 0));
        assert!(!is_due(&mut rechecks, &mr("checking"), CHECKING_MS - 1));
        assert!(is_due(&mut rechecks, &mr("checking"), CHECKING_MS));
    }

    #[test]
    fn a_settled_mr_is_forgotten_so_the_next_move_is_asked_at_once() {
        let mut rechecks = HashMap::new();
        assert!(is_due(&mut rechecks, &mr("unchecked"), 0));
        assert!(!is_due(&mut rechecks, &mr("mergeable"), 1_000));
        assert!(rechecks.is_empty());
        assert!(is_due(&mut rechecks, &mr("unchecked"), 2_000));
    }

    #[test]
    fn leaves_out_settled_and_closed_mrs() {
        let mut rechecks = HashMap::new();
        assert!(!is_due(&mut rechecks, &mr("mergeable"), 0));
        assert!(!is_due(&mut rechecks, &mr("conflict"), 0));
        let merged = Mr {
            state: "merged".into(),
            ..mr("unchecked")
        };
        assert!(!is_due(&mut rechecks, &merged, 0));
    }
}
