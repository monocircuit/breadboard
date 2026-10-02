//! The order of the spaces in the main space's group (order.mjs, #255): the
//! worktrees of no orchestration first, then per orchestration its space
//! `⇶ <title>` with its worktrees' spaces right below it, in the work order
//! of its plan. herdr's CLI cannot move a space, its socket API can
//! (`workspace.move_block`); the main space's overview keeps the order on
//! every read.

use super::{feature_of, id_of, is_linked, label_of, listed, orchestrated, text};
use breadboard_core::herdr::is_in_herdr;
use breadboard_core::record::{self, Record, SPACE, orchestration_of_label, record_of};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// How long herdr's socket gets to answer.
const ANSWER: Duration = Duration::from_secs(2);

/// The features of an orchestration's worktrees in its work order: the
/// issues of `plan.work` first, then the rest as they were started.
fn work_order_of(record: Option<&Record>) -> Vec<String> {
    let Some(record) = record else { return Vec::new() };
    let planned = record
        .plan
        .iter()
        .flat_map(|plan| plan.work.iter())
        .filter_map(|number| record.issues.iter().find(|issue| issue.number == Some(*number)));
    let mut order: Vec<String> = Vec::new();
    for feature in planned
        .chain(record.issues.iter())
        .filter_map(|issue| issue.feature.clone())
    {
        if !order.contains(&feature) {
            order.push(feature);
        }
    }
    order
}

/// The move that brings the main space's group in `list` (herdr's
/// `workspace list`, in sidebar order) into its order, the params of
/// `workspace.move_block`; None when it stands so already (order.mjs `groupMove`).
fn group_move(
    list: &[Value],
    origin_of: impl Fn(&str) -> Option<String>,
    order_of: impl Fn(&str) -> Vec<String>,
    orchestration_of: impl Fn(&str) -> Option<String>,
) -> Option<Value> {
    let main_at = list.iter().position(|space| label_of(space) == SPACE)?;
    let repo = text(&list[main_at], "/worktree/repo_key")?;
    let group: Vec<usize> = (0..list.len())
        .filter(|at| is_linked(&list[*at]) && text(&list[*at], "/worktree/repo_key") == Some(repo))
        .collect();
    let label = |at: &usize| label_of(&list[*at]);
    let orchestrations: Vec<usize> = group
        .iter()
        .copied()
        .filter(|at| orchestration_of(label(at)).is_some())
        .collect();
    let open: Vec<String> = orchestrations
        .iter()
        .filter_map(|at| orchestration_of(label(at)))
        .collect();
    let worktrees: Vec<usize> = group
        .iter()
        .copied()
        .filter(|at| orchestration_of(label(at)).is_none())
        .collect();
    let mut wanted: Vec<usize> = worktrees
        .iter()
        .copied()
        .filter(|at| !origin_of(label(at)).is_some_and(|name| open.contains(&name)))
        .collect();
    for space in &orchestrations {
        let Some(name) = orchestration_of(label(space)) else {
            continue;
        };
        let order = order_of(&name);
        let rank = |at: &usize| {
            order
                .iter()
                .position(|feature| feature == label(at))
                .unwrap_or(order.len())
        };
        let mut own: Vec<usize> = worktrees
            .iter()
            .copied()
            .filter(|at| origin_of(label(at)).as_deref() == Some(name.as_str()))
            .collect();
        own.sort_by_key(rank);
        wanted.push(*space);
        wanted.extend(own);
    }
    if wanted == group {
        return None;
    }
    // Behind the main space and where the group starts, before the first space outside it.
    let from = main_at.max(group[0]);
    let before = (from..list.len())
        .find(|at| *at != main_at && !wanted.contains(at))
        .map(|at| id_of(&list[at]).to_string());
    Some(json!({
        "workspace_ids": wanted.iter().map(|at| id_of(&list[*at])).collect::<Vec<_>>(),
        "before_workspace_id": before,
    }))
}

/// A method of herdr's socket API → its result; None outside herdr, on an error or after ANSWER.
fn herdr_api(method: &str, params: Value) -> Option<Value> {
    let socket = std::env::var_os("HERDR_SOCKET_PATH").filter(|_| is_in_herdr())?;
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(ANSWER)).ok()?;
    stream.set_write_timeout(Some(ANSWER)).ok()?;
    let request = json!({ "id": "breadboard", "method": method, "params": params });
    writeln!(stream, "{request}").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let answer: Value = serde_json::from_str(&line).ok()?;
    answer.get("result").filter(|result| !result.is_null()).cloned()
}

/// The main space's group into its order when it is out of it; a line for what moved.
pub fn sort_spaces(main: &Path) -> Option<String> {
    if !is_in_herdr() {
        return None;
    }
    let list = listed();
    let origins: HashMap<String, String> = orchestrated(main)
        .into_iter()
        .map(|(dir, origin)| (feature_of(main, &dir), origin.name))
        .collect();
    let titles = record::titles(main);
    let params = group_move(
        &list,
        |label| origins.get(label).cloned(),
        |name| work_order_of(record_of(main, name).as_ref()),
        |label| orchestration_of_label(label, &titles),
    )?;
    herdr_api("workspace.move_block", params).map(|_| format!("spaces of {SPACE} sorted by orchestration"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(id: &str, label: &str, is_linked: bool) -> Value {
        json!({
            "workspace_id": id,
            "label": label,
            "worktree": { "is_linked_worktree": is_linked, "repo_key": "/r/.git" },
        })
    }

    fn move_of(list: &[Value]) -> Option<Value> {
        let origin = |label: &str| (label.starts_with("30")).then(|| "bb".to_string());
        let order = |_: &str| vec!["303-b".to_string(), "302-a".to_string()];
        let of = |label: &str| label.strip_prefix("⇶ ").map(String::from);
        group_move(list, origin, order, of)
    }

    #[test]
    fn sorts_an_orchestration_below_the_plain_worktrees_in_its_work_order() {
        let list = [
            space("wD", SPACE, false),
            space("w1", "302-a", true),
            space("w2", "⇶ bb", true),
            space("w3", "plain", true),
            space("w4", "303-b", true),
            json!({ "workspace_id": "wX", "label": "other" }),
        ];
        assert_eq!(
            move_of(&list),
            Some(json!({ "workspace_ids": ["w3", "w2", "w4", "w1"], "before_workspace_id": "wX" }))
        );
        let sorted = [
            space("wD", SPACE, false),
            space("w3", "plain", true),
            space("w2", "⇶ bb", true),
            space("w4", "303-b", true),
            space("w1", "302-a", true),
        ];
        assert_eq!(move_of(&sorted), None);
    }

    #[test]
    fn worktrees_of_an_orchestration_without_its_space_stay_plain() {
        let list = [
            space("wD", SPACE, false),
            space("w1", "302-a", true),
            space("w3", "plain", true),
        ];
        assert_eq!(move_of(&list), None);
    }
}
