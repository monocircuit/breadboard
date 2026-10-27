//! What a change set knows of a branch without asking GitLab (#484): the
//! changes pane read every branch of its record each minute, 180 for
//! `breadboard`, though a merged MR's range never changes. A merged branch
//! comes from the overview's entries (`branches`, `mrs`): what one of them
//! learned, the other doesn't ask. An open one from the list of the open
//! MRs, one request for all; one with only closed MRs is asked once an hour.

use super::cache::Cache;
use super::read::Final;
use super::rest::mr_of;
use serde_json::{Value, json};

/// How long a branch with only closed MRs isn't asked again: a new MR on it
/// shows in the open list meanwhile.
pub const CLOSED_FOR_MS: i64 = 60 * 60 * 1000;

fn state(value: &Value) -> &str {
    value.get("state").and_then(Value::as_str).unwrap_or_default()
}

/// The merged MR of `branch` the cache holds, in REST's shape (the fields a
/// change set reads); None while it is open, closed, unknown or lacks its merge commit.
pub fn merged_of(cache: &Cache, branch: &str) -> Option<Value> {
    let iid = cache.raw("branches", branch)?.as_i64()?;
    let known = cache.get::<Final>("mrs", &iid.to_string())?;
    let merge_sha = known.merge_sha.filter(|_| known.mr.state == "merged")?;
    let mr = known.mr;
    Some(json!({
        "iid": mr.iid, "web_url": mr.url, "title": mr.title, "state": mr.state,
        "merged_at": mr.merged_at, "merge_commit_sha": merge_sha, "sha": mr.sha,
        "source_branch": branch, "target_branch": mr.target,
    }))
}

/// The MRs of `branch` known without asking it: its open ones from `opened`
/// (the project's open MRs, None when not read in full) with a merged one the
/// cache holds, else that merged one, else none while it was closed lately.
/// None: it is asked.
pub fn known_of(cache: &mut Cache, branch: &str, opened: Option<&[Value]>, now: i64) -> Option<Vec<Value>> {
    if let Some(opened) = opened {
        let mine: Vec<Value> = opened
            .iter()
            .filter(|mr| mr.get("source_branch").and_then(Value::as_str) == Some(branch))
            .cloned()
            .collect();
        if !mine.is_empty() {
            // Open again: once merged, it is asked, not taken for closed.
            if cache.raw("closedBranches", branch).is_some() {
                cache.set("closedBranches", branch, &Value::Null);
            }
            return Some(mine.into_iter().chain(merged_of(cache, branch)).collect());
        }
    }
    if let Some(merged) = merged_of(cache, branch) {
        return Some(vec![merged]);
    }
    let closed_at = cache.raw("closedBranches", branch).and_then(Value::as_i64)?;
    (opened.is_some() && now - closed_at < CLOSED_FOR_MS).then(Vec::new)
}

/// Keeps what REST listed for `branch` once it settled: one merged MR and
/// none open, or only closed ones (until `CLOSED_FOR_MS`). Another final MR
/// the overview holds for it stays.
pub fn keep_settled(cache: &mut Cache, branch: &str, listed: &[Value], now: i64) {
    if listed.iter().any(|value| state(value) == "opened") {
        return;
    }
    let merged: Vec<&Value> = listed.iter().filter(|value| state(value) == "merged").collect();
    if merged.is_empty() && !listed.is_empty() {
        cache.set("closedBranches", branch, &now);
        return;
    }
    let [value] = merged[..] else { return };
    let (mut mr, merge_sha) = mr_of(value);
    if merge_sha.is_none() {
        return;
    }
    if let Some(known) = cache.raw("branches", branch).and_then(Value::as_i64)
        && known != mr.iid
    {
        return;
    }
    mr.branch = branch.to_string();
    let iid = mr.iid;
    cache.set("mrs", &iid.to_string(), &Final { mr, merge_sha });
    cache.set("branches", branch, &iid);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn listed(iid: i64, state: &str) -> Value {
        json!({
            "iid": iid, "state": state, "title": "Draft: fix it", "web_url": format!("https://x/{iid}"),
            "merged_at": "2026-10-01T00:00:00Z", "merge_commit_sha": "abc", "sha": "def",
            "source_branch": "fix/1-a", "target_branch": "develop",
        })
    }

    fn on(branch: &str, mut mr: Value) -> Value {
        mr["source_branch"] = json!(branch);
        mr
    }

    #[test]
    fn a_merged_branch_is_known_and_an_open_or_unknown_one_is_not() {
        let mut cache = Cache::open(None);
        keep_settled(&mut cache, "fix/1-a", &[listed(4, "closed"), listed(5, "merged")], NOW);
        let known = known_of(&mut cache, "fix/1-a", None, NOW).expect("kept");
        assert_eq!(known[0]["iid"], json!(5));
        assert_eq!(known[0]["merge_commit_sha"], json!("abc"));
        assert_eq!(known[0]["title"], json!("fix it"));
        keep_settled(&mut cache, "fix/2-b", &[listed(6, "merged"), listed(7, "opened")], NOW);
        keep_settled(&mut cache, "fix/3-c", &[listed(8, "merged"), listed(9, "merged")], NOW);
        keep_settled(&mut cache, "fix/4-d", &[], NOW);
        for branch in ["fix/2-b", "fix/3-c", "fix/4-d", "fix/5-e"] {
            assert!(known_of(&mut cache, branch, Some(&[]), NOW).is_none(), "{branch}");
        }
    }

    #[test]
    fn an_open_branch_comes_from_the_open_list() {
        let mut cache = Cache::open(None);
        let opened = [on("fix/2-b", listed(7, "opened")), on("fix/3-c", listed(8, "opened"))];
        let known = known_of(&mut cache, "fix/2-b", Some(&opened), NOW).expect("open");
        assert_eq!(known.len(), 1);
        assert_eq!(known[0]["iid"], json!(7));
        assert!(known_of(&mut cache, "fix/2-b", None, NOW).is_none());
    }

    #[test]
    fn a_closed_branch_is_asked_again_after_a_while_or_once_it_was_open() {
        let mut cache = Cache::open(None);
        keep_settled(&mut cache, "fix/1-a", &[listed(4, "closed")], NOW);
        assert_eq!(known_of(&mut cache, "fix/1-a", Some(&[]), NOW + 1000), Some(vec![]));
        // The open list not read in full: it might have a new MR.
        assert!(known_of(&mut cache, "fix/1-a", None, NOW + 1000).is_none());
        assert!(known_of(&mut cache, "fix/1-a", Some(&[]), NOW + CLOSED_FOR_MS).is_none());
        assert!(known_of(&mut cache, "fix/1-a", Some(&[listed(9, "opened")]), NOW).is_some());
        assert!(known_of(&mut cache, "fix/1-a", Some(&[]), NOW + 1000).is_none());
    }

    #[test]
    fn leaves_the_overview_s_final_mr_of_a_branch() {
        let mut cache = Cache::open(None);
        cache.set("branches", "fix/1-a", &4);
        cache.set(
            "mrs",
            "4",
            &Final {
                mr: mr_of(&listed(4, "closed")).0,
                merge_sha: None,
            },
        );
        keep_settled(&mut cache, "fix/1-a", &[listed(5, "merged")], NOW);
        assert_eq!(cache.raw("branches", "fix/1-a"), Some(&json!(4)));
        assert!(known_of(&mut cache, "fix/1-a", Some(&[]), NOW).is_none());
    }

    #[test]
    fn a_merged_mr_without_its_merge_commit_is_asked_again() {
        let mut cache = Cache::open(None);
        cache.set("branches", "fix/1-a", &5);
        let mr = mr_of(&listed(5, "merged")).0;
        cache.set("mrs", "5", &Final { mr, merge_sha: None });
        assert!(known_of(&mut cache, "fix/1-a", None, NOW).is_none());
        keep_settled(&mut cache, "fix/1-a", &[listed(5, "merged")], NOW);
        assert_eq!(
            known_of(&mut cache, "fix/1-a", None, NOW).expect("kept")[0]["merge_commit_sha"],
            json!("abc")
        );
    }
}
