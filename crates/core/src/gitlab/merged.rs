//! A branch's merged MR from the cache, for a reader outside the overview
//! (#484): the changes pane read every branch of its record each minute,
//! 180 for `breadboard`, though a merged MR's range never changes. The
//! entries are the overview's (`branches`, `mrs`): what one of them learned,
//! the other doesn't ask.

use super::cache::Cache;
use super::read::Final;
use super::rest::mr_of;
use serde_json::{Value, json};

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

/// Keeps what REST listed for `branch` once it settled: no open MR, one
/// merged. Another final MR the cache holds for it stays.
pub fn keep_merged(cache: &mut Cache, branch: &str, listed: &[Value]) {
    let state = |value: &Value| value.get("state").and_then(Value::as_str).unwrap_or_default().to_string();
    if listed.iter().any(|value| state(value) == "opened") {
        return;
    }
    let [value] = listed
        .iter()
        .filter(|value| state(value) == "merged")
        .collect::<Vec<_>>()[..]
    else {
        return;
    };
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

    fn listed(iid: i64, state: &str) -> Value {
        json!({
            "iid": iid, "state": state, "title": "Draft: fix it", "web_url": format!("https://x/{iid}"),
            "merged_at": "2026-10-01T00:00:00Z", "merge_commit_sha": "abc", "sha": "def",
            "source_branch": "fix/1-a", "target_branch": "develop",
        })
    }

    #[test]
    fn a_merged_branch_is_known_and_an_open_one_is_not() {
        let mut cache = Cache::open(None);
        keep_merged(&mut cache, "fix/1-a", &[listed(4, "closed"), listed(5, "merged")]);
        let known = merged_of(&cache, "fix/1-a").expect("kept");
        assert_eq!(known["iid"], json!(5));
        assert_eq!(known["merge_commit_sha"], json!("abc"));
        assert_eq!(known["title"], json!("fix it"));
        keep_merged(&mut cache, "fix/2-b", &[listed(6, "merged"), listed(7, "opened")]);
        keep_merged(&mut cache, "fix/3-c", &[listed(8, "merged"), listed(9, "merged")]);
        keep_merged(&mut cache, "fix/4-d", &[listed(10, "closed")]);
        for branch in ["fix/2-b", "fix/3-c", "fix/4-d", "fix/5-e"] {
            assert!(merged_of(&cache, branch).is_none(), "{branch}");
        }
    }

    #[test]
    fn leaves_the_overview_s_final_mr_of_a_branch() {
        let mut cache = Cache::open(None);
        cache.set("branches", "fix/1-a", &4);
        cache.set("mrs", "4", &Final { mr: mr_of(&listed(4, "closed")).0, merge_sha: None });
        keep_merged(&mut cache, "fix/1-a", &[listed(5, "merged")]);
        assert_eq!(cache.raw("branches", "fix/1-a"), Some(&json!(4)));
        assert!(merged_of(&cache, "fix/1-a").is_none());
    }

    #[test]
    fn a_merged_mr_without_its_merge_commit_is_asked_again() {
        let mut cache = Cache::open(None);
        cache.set("branches", "fix/1-a", &5);
        let mr = mr_of(&listed(5, "merged")).0;
        cache.set("mrs", "5", &Final { mr, merge_sha: None });
        assert!(merged_of(&cache, "fix/1-a").is_none());
        keep_merged(&mut cache, "fix/1-a", &[listed(5, "merged")]);
        assert_eq!(merged_of(&cache, "fix/1-a").expect("kept")["merge_commit_sha"], json!("abc"));
    }
}
