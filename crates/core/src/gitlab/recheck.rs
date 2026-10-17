//! GitLab's mergeability of an open MR, asked for when it sits unknown
//! (#391): after its target moves, GitLab resets `detailed_merge_status` to
//! `unchecked` and computes it again only when someone asks. The read asks
//! with `with_merge_status_recheck=true` and takes the answer's status, once
//! per MR every two minutes while it stays unchecked; one seen settled is
//! forgotten, so the next move of develop is asked about at once. A conflict
//! GitLab answered stays one while it rechecks the same head (#416): its
//! listing may say `unchecked` again right after, and a row would then wait
//! for a recheck that already found the conflict.

use super::client::Gitlab;
use super::rest::mr_of;
use super::{BranchRemote, Mr};
use std::collections::HashMap;

/// How long a `checking` may last before it is asked about.
const CHECKING_MS: i64 = 60_000;
/// How long after an ask the same MR is asked again.
const AGAIN_MS: i64 = 120_000;

/// Of one MR: since when GitLab says `checking`, when it was asked last,
/// the head GitLab last said it conflicts at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recheck {
    pub checking_since: Option<i64>,
    pub asked_at: Option<i64>,
    pub conflict_at: Option<String>,
}

/// Whether GitLab hasn't settled `mr`'s mergeability.
pub fn is_unsettled(mr: &Mr) -> bool {
    matches!(mr.merge_status.as_deref(), Some("unchecked" | "checking"))
}

fn head_of(mr: &Mr) -> String {
    mr.sha.clone().unwrap_or_default()
}

/// Whether `mr` is asked about now; moves on its entry in `rechecks`. One
/// seen settled loses its times, and its entry unless it conflicts.
pub fn is_due(rechecks: &mut HashMap<i64, Recheck>, mr: &Mr, now: i64) -> bool {
    if !mr.is_open() {
        rechecks.remove(&mr.iid);
        return false;
    }
    if !is_unsettled(mr) {
        if mr.has_conflicts {
            rechecks.insert(
                mr.iid,
                Recheck {
                    conflict_at: Some(head_of(mr)),
                    ..Recheck::default()
                },
            );
        } else {
            rechecks.remove(&mr.iid);
        }
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

/// Takes GitLab's answer to a recheck of `mr`: a settled one says whether it conflicts at its head.
fn take(rechecks: &mut HashMap<i64, Recheck>, mr: &mut Mr, fresh: Mr) {
    mr.merge_status = fresh.merge_status;
    mr.merge_check = fresh.merge_check;
    mr.has_conflicts = fresh.has_conflicts;
    if !is_unsettled(mr) {
        let head = mr.has_conflicts.then(|| head_of(mr));
        rechecks.entry(mr.iid).or_default().conflict_at = head;
    }
}

/// While GitLab rechecks `mr`, its last answer stands when that was a
/// conflict: at the same head (`rechecks`), or GitLab's own
/// `cannot_be_merged_recheck`. The row says the conflict, not the recheck.
pub fn carry(rechecks: &HashMap<i64, Recheck>, mr: &mut Mr) {
    if !is_unsettled(mr) {
        return;
    }
    let head = head_of(mr);
    let had = rechecks.get(&mr.iid).and_then(|recheck| recheck.conflict_at.as_deref()) == Some(head.as_str());
    let is_rechecking_one = mr.merge_check.as_deref() == Some("cannot_be_merged_recheck");
    if had || is_rechecking_one || mr.has_conflicts {
        mr.merge_status = Some("conflict".into());
        mr.merge_check = Some("cannot_be_merged".into());
        mr.has_conflicts = true;
    }
}

/// Asks GitLab to recheck each open MR of `of` that is due, and takes what
/// it answers; one still unsettled keeps a conflict GitLab answered before.
pub async fn recheck(
    gitlab: &Gitlab,
    rechecks: &mut HashMap<i64, Recheck>,
    of: &mut HashMap<String, BranchRemote>,
    now: i64,
) {
    for mr in of.values_mut().filter_map(|remote| remote.mr.as_mut()) {
        if is_due(rechecks, mr, now) {
            let endpoint = format!("merge_requests/{}?with_merge_status_recheck=true", mr.iid);
            if let Ok(answer) = gitlab.get_json(&endpoint).await {
                let (fresh, _) = mr_of(&answer);
                if fresh.iid == mr.iid {
                    take(rechecks, mr, fresh);
                }
            }
        }
        carry(rechecks, mr);
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

    fn at(status: &str, sha: &str) -> Mr {
        Mr {
            sha: Some(sha.into()),
            ..mr(status)
        }
    }
    fn carried(rechecks: &HashMap<i64, Recheck>, mut found: Mr) -> (Option<String>, bool) {
        carry(rechecks, &mut found);
        (found.merge_status, found.has_conflicts)
    }

    #[test]
    fn a_conflict_gitlab_answered_stays_while_it_rechecks_the_same_head() {
        let mut rechecks = HashMap::new();
        let mut asked = at("unchecked", "a");
        assert!(is_due(&mut rechecks, &asked, 0));
        take(
            &mut rechecks,
            &mut asked,
            Mr {
                has_conflicts: true,
                ..at("conflict", "a")
            },
        );
        // The next listing says `unchecked` again: the conflict stands, no ask before two minutes.
        let again = at("unchecked", "a");
        assert!(!is_due(&mut rechecks, &again, 1_000));
        assert_eq!(carried(&rechecks, again), (Some("conflict".into()), true));
        // Pushed since: a new head, the recheck again.
        assert_eq!(
            carried(&rechecks, at("unchecked", "b")),
            (Some("unchecked".into()), false)
        );
        // Answered mergeable: forgotten.
        let mut fixed = at("unchecked", "a");
        take(&mut rechecks, &mut fixed, at("mergeable", "a"));
        assert_eq!(
            carried(&rechecks, at("unchecked", "a")),
            (Some("unchecked".into()), false)
        );
    }

    #[test]
    fn a_settled_conflict_in_the_listing_is_kept_for_its_recheck() {
        let mut rechecks = HashMap::new();
        let found = Mr {
            has_conflicts: true,
            ..at("conflict", "a")
        };
        assert!(!is_due(&mut rechecks, &found, 0));
        // develop moved: GitLab rechecks, the row keeps the conflict and the read asks at once.
        assert!(is_due(&mut rechecks, &at("unchecked", "a"), 1_000));
        assert_eq!(
            carried(&rechecks, at("unchecked", "a")),
            (Some("conflict".into()), true)
        );
    }

    #[test]
    fn gitlabs_own_recheck_of_a_conflict_is_a_conflict() {
        let found = Mr {
            merge_check: Some("cannot_be_merged_recheck".into()),
            ..at("checking", "a")
        };
        assert_eq!(carried(&HashMap::new(), found), (Some("conflict".into()), true));
        assert_eq!(
            carried(&HashMap::new(), at("checking", "a")),
            (Some("checking".into()), false)
        );
    }
}
