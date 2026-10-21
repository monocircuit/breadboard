//! An orchestration's epic branch (#350): `epic/<n>-<name>` off develop,
//! where its tasks branch off and merge into by themselves; only the epic
//! goes to develop by the user's two clicks. No new one is made: the
//! product's rolling batch (#411, `batch.rs`) replaced it, and an epic still
//! open lives on until it ships as a batch.

use crate::gitlab::{Issue, Mr};
use crate::json::str_of;
use crate::model::order::Item;
use serde_json::Value;

/// What every epic branch starts with.
pub const PREFIX: &str = "epic/";

/// Whether `branch` is an epic branch, the target of an automatic merge.
pub fn is_epic(branch: &str) -> bool {
    branch.starts_with(PREFIX) && branch.len() > PREFIX.len()
}

/// The epic branch the orchestration's record keeps, None for one that
/// works on develop (made before #350).
pub fn epic_of(record: &Value) -> Option<String> {
    record
        .get("epic")
        .and_then(|epic| str_of(epic, "branch"))
        .filter(|branch| is_epic(branch))
        .map(String::from)
}

/// Whether a task's MR is in its epic or batch but not yet in develop
/// (#350, #411): merged into one, its issue still open (the MR into develop
/// closes it).
pub fn is_in_epic(mr: &Mr, issue: Option<&Issue>) -> bool {
    let target = mr.target();
    mr.is_merged()
        && (is_epic(target) || crate::batch::is_batch(target))
        && issue.is_none_or(|issue| issue.state != "closed")
}

/// What a task [`is_in_epic`] is said to be: `in batch`, or `in epic` for an epic of before.
pub fn in_word(mr: &Mr) -> &'static str {
    if crate::batch::is_batch(mr.target()) {
        "in batch"
    } else {
        "in epic"
    }
}

/// The merge the overview runs by itself now (#350, #411): the first in
/// ACTIONS whose turn offers it (no blockers) and whose MR goes into a
/// target `is_auto` takes (a batch, an epic: `batch::is_auto_target`), its
/// MR's head not tried already (`tried`: a failed one waits for a push).
/// Into develop or main it stays the user's two clicks.
pub fn auto_merge_of(
    items: &[Item],
    target_of: impl Fn(&str) -> Option<String>,
    is_auto: impl Fn(&str) -> bool,
    tried: impl Fn(i64) -> bool,
) -> Option<&Item> {
    items.iter().find(|item| {
        let (Some(offer), Some(branch)) = (item.merge.as_ref(), item.branch.as_deref()) else {
            return false;
        };
        offer.blockers.is_empty()
            && !item.is_running
            && target_of(branch).is_some_and(|target| is_auto(&target))
            && !tried(offer.iid)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::order::MergeOffer;
    use serde_json::json;

    fn merge(iid: i64, branch: &str, blockers: &[&str]) -> Item {
        Item {
            kind: "merge".into(),
            branch: Some(branch.into()),
            merge: Some(MergeOffer {
                iid,
                blockers: blockers.iter().map(|blocker| blocker.to_string()).collect(),
                ..MergeOffer::default()
            }),
            ..Item::default()
        }
    }

    #[test]
    fn merges_by_itself_only_an_offered_mr_into_an_epic() {
        let target = |branch: &str| Some(if branch.contains("dev") { "develop" } else { "epic/15-b" }.to_string());
        let items = [
            merge(1, "feature/1-dev", &[]),
            merge(2, "feature/2-x", &["pipeline running"]),
            merge(3, "feature/3-x", &[]),
            merge(4, "feature/4-x", &[]),
        ];
        let never = |_: i64| false;
        assert_eq!(
            auto_merge_of(&items, target, is_epic, never)
                .and_then(|item| item.merge.as_ref())
                .map(|offer| offer.iid),
            Some(3)
        );
        let tried = |iid: i64| iid == 3;
        assert_eq!(
            auto_merge_of(&items, target, is_epic, tried)
                .and_then(|item| item.merge.as_ref())
                .map(|offer| offer.iid),
            Some(4)
        );
        assert!(
            auto_merge_of(&items[..2], target, is_epic, never).is_none(),
            "into develop: the user's clicks"
        );
        assert!(
            auto_merge_of(&items, |_: &str| None, is_epic, never).is_none(),
            "no MR read: nothing"
        );
    }

    #[test]
    fn a_task_merged_into_its_epic_is_in_epic_until_its_issue_closes() {
        let mr = |state: &str, target: &str| Mr {
            state: state.into(),
            target: target.into(),
            ..Mr::default()
        };
        let issue = |state: &str| Issue {
            state: state.into(),
            ..Issue::default()
        };
        assert!(is_in_epic(&mr("merged", "epic/15-b"), None));
        assert!(is_in_epic(&mr("merged", "epic/15-b"), Some(&issue("opened"))));
        assert!(
            !is_in_epic(&mr("merged", "epic/15-b"), Some(&issue("closed"))),
            "the milestone MR landed"
        );
        assert!(!is_in_epic(&mr("merged", "develop"), None));
        assert!(is_in_epic(&mr("merged", "batch/breadboard-1"), None));
        assert!(!is_in_epic(&mr("opened", "epic/15-b"), None));
    }

    #[test]
    fn tells_an_epic_branch() {
        assert!(is_epic("epic/15-breadboard"));
        assert!(!is_epic("epic/") && !is_epic("develop") && !is_epic("feature/epic/1"));
    }

    #[test]
    fn reads_the_epic_of_a_record() {
        assert_eq!(
            epic_of(&json!({ "epic": { "branch": "epic/15-breadboard", "number": 15 } })),
            Some("epic/15-breadboard".into())
        );
        assert_eq!(epic_of(&json!({ "issues": [] })), None);
    }
}
