//! The three views printed: an orchestration's, a worktree's, everything.
//! Each reads its local state and its GitLab calls side by side.

use super::clock::band_time;
use super::frame::{Frame, Row, table};
use crate::cli::read::gitlab;
use crate::cli::read::issues::boxes_of;
use breadboard_core::actions::{Action, actions_file, actions_of};
use breadboard_core::git::{self, Worktree, issue_of_branch};
use breadboard_core::json::str_of;
use breadboard_core::record::{ORCHESTRATOR, origin_of};
use breadboard_core::time::now_ms;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::thread;

// ---------------------------------------------------------------------------
// What the views read
// ---------------------------------------------------------------------------

/// A merge request as the views show it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mr {
    pub iid: u64,
    pub title: String,
    pub state: String,
    pub is_draft: bool,
    pub source_branch: String,
    pub merged_at: Option<String>,
    /// The pipeline's status, None when GitLab's answer has no word on it.
    pub pipeline: Option<Option<String>>,
}

impl Mr {
    fn from_value(value: &Value) -> Option<Mr> {
        let pipeline = ["head_pipeline", "pipeline"]
            .iter()
            .find_map(|key| value.get(*key))
            .map(|pipeline| str_of(pipeline, "status").map(String::from));
        Some(Mr {
            iid: value.get("iid")?.as_u64()?,
            title: plain_title(str_of(value, "title").unwrap_or("")),
            state: str_of(value, "state").unwrap_or("").to_string(),
            is_draft: value.get("draft").and_then(Value::as_bool).unwrap_or(false),
            source_branch: str_of(value, "source_branch").unwrap_or("").to_string(),
            merged_at: str_of(value, "merged_at").map(String::from),
            pipeline,
        })
    }

    /// `draft` / `ready` / `merged` / `closed`.
    fn state_word(&self) -> &str {
        match self.state.as_str() {
            "opened" if self.is_draft => "draft",
            "opened" => "ready",
            other => other,
        }
    }

    /// `✓ passed`, `◐ running`, … (boxes.mjs `statusMark`); empty when unknown.
    fn pipeline_mark(&self) -> String {
        match &self.pipeline {
            None => String::new(),
            Some(None) => "no pipeline".into(),
            Some(Some(status)) => match status.as_str() {
                "success" => "✓ passed".into(),
                "failed" => "✗ failed".into(),
                "running" => "◐ running".into(),
                "pending" => "… pending".into(),
                "created" => "… created".into(),
                "canceled" => "⊘ canceled".into(),
                "manual" => "▸ manual".into(),
                other => other.to_string(),
            },
        }
    }
}

/// A title without its `Draft:`: the state says it.
fn plain_title(title: &str) -> String {
    let is_draft = title.get(..6).is_some_and(|start| start.eq_ignore_ascii_case("draft:"));
    if is_draft {
        title[6..].trim_start().to_string()
    } else {
        title.to_string()
    }
}

/// The MRs of GitLab's lists, Err the reason none came.
fn mrs_of(result: Result<Vec<Value>, String>) -> Result<Vec<Mr>, String> {
    result.map(|list| list.iter().filter_map(Mr::from_value).collect())
}

/// A worktree with what git and its files say.
#[derive(Debug, Clone, Default)]
struct Tree {
    dir: PathBuf,
    feature: String,
    branch: Option<String>,
    origin: Option<String>,
    actions: Vec<Action>,
    ahead: u64,
    behind: u64,
    changed: usize,
}

/// Every worktree but the main checkout, read side by side.
fn trees_of(main: &Path, worktrees: &[Worktree]) -> Vec<Tree> {
    thread::scope(|scope| {
        let handles: Vec<_> = worktrees
            .iter()
            .map(|tree| {
                scope.spawn(move || Tree {
                    feature: git::feature_of(main, &tree.dir),
                    branch: tree.branch.clone(),
                    origin: origin_of(&tree.dir).map(|origin| origin.name),
                    actions: actions_of(&actions_file(&tree.dir)),
                    ahead: git::ahead_of_develop(&tree.dir),
                    behind: git::behind_develop(&tree.dir),
                    changed: git::changed_count(&tree.dir),
                    dir: tree.dir.clone(),
                })
            })
            .collect();
        handles.into_iter().filter_map(|handle| handle.join().ok()).collect()
    })
}

/// `1 ahead · 2 behind · 3 changed`, what a branch holds; `clean` for nothing.
fn git_text(tree: &Tree) -> String {
    let parts: Vec<String> = [
        (tree.ahead as usize, "ahead"),
        (tree.behind as usize, "behind"),
        (tree.changed, "changed"),
    ]
    .iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, word)| format!("{count} {word}"))
    .collect();
    if parts.is_empty() {
        "clean".into()
    } else {
        parts.join(" · ")
    }
}

/// The summary of ACTIONS: how many wait for an answer.
fn waiting_summary(count: usize) -> String {
    match count {
        0 => "nothing waits for you".into(),
        1 => "1 waits for you".into(),
        count => format!("{count} wait for you"),
    }
}

/// An action's rows: what it asks, and under it who asks, where it opens and
/// what it offers.
fn action_rows(action: &Action, from: &str) -> Vec<Row> {
    let kind = if action.is_try() { "try" } else { action.kind.as_str() };
    let head = Row::new(0, format!("#{} {kind}  {}", action.id, action.text)).hang(4);
    let mut details: Vec<String> = Vec::new();
    if !from.is_empty() {
        details.push(from.to_string());
    }
    if let Some(url) = &action.url {
        details.push(if action.on.as_deref() == Some("test") {
            format!("{url} on the test instance")
        } else {
            url.clone()
        });
    }
    if let Some(run) = &action.run {
        details.push(format!("$ {run}"));
    }
    if !action.options.is_empty() {
        let options: Vec<String> = action
            .options
            .iter()
            .map(|option| {
                if action.recommend.as_deref() == Some(option) {
                    format!("{option} (Recommended)")
                } else {
                    option.clone()
                }
            })
            .collect();
        details.push(format!("[{}]", options.join(" | ")));
    }
    if let Some(answer) = &action.answer {
        details.push(format!("→ {answer}"));
    }
    let head = if action.answer.is_some() { head } else { head.strong() };
    let mut rows = vec![head];
    if !details.is_empty() {
        rows.push(Row::dim(4, details.join(" · ")).hang(4));
    }
    rows
}

// --worktree <dir>
// ---------------------------------------------------------------------------

pub fn worktree(frame: &mut Frame, main: &Path, dir: &Path) {
    let worktrees = git::worktrees(main);
    // The worktree the directory lies in, the innermost.
    let found = worktrees
        .iter()
        .filter(|tree| git::is_inside(dir, &tree.dir))
        .max_by_key(|tree| tree.dir.as_os_str().len())
        .cloned();
    let Some(found) = found else {
        frame.band(&dir.display().to_string(), &band_time(now_ms()));
        frame.line(Row::new(
            2,
            format!("{} is not a worktree of the repo any more", dir.display()),
        ));
        return;
    };
    let branch = found.branch.clone().unwrap_or_else(|| "(detached)".into());
    let number = issue_of_branch(&branch);
    let (tree, mr, issue) = thread::scope(|scope| {
        let mr = scope.spawn(|| {
            let endpoint = format!(
                "projects/:fullpath/merge_requests?state=all&source_branch={}&per_page=20",
                encode(&branch)
            );
            mrs_of(gitlab::list(main, &endpoint)).map(|mrs| {
                let opened = mrs.iter().find(|mr| mr.state == "opened").cloned();
                opened.or_else(|| mrs.into_iter().max_by_key(|mr| mr.iid))
            })
        });
        let issue =
            scope.spawn(|| number.map(|number| gitlab::get(main, &format!("projects/:fullpath/issues/{number}"))));
        let tree = trees_of(main, std::slice::from_ref(&found)).pop().unwrap_or_default();
        let mr = mr.join().unwrap_or_else(|_| Err("the read broke off".into()));
        let issue = issue.join().unwrap_or(None);
        (tree, mr, issue)
    });
    let is_main = worktrees.first().is_some_and(|first| first.dir == found.dir);
    let name = if is_main {
        found
            .dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        tree.feature.clone()
    };
    let origin = tree
        .origin
        .as_ref()
        .map(|origin| format!("  {ORCHESTRATOR} {origin}"))
        .unwrap_or_default();
    frame.band(&format!("{name}  {branch}{origin}"), &band_time(now_ms()));

    let waiting = tree.actions.iter().filter(|action| action.answer.is_none()).count();
    frame.section("ACTIONS", &waiting_summary(waiting), || {
        if tree.actions.is_empty() {
            return vec![Row::dim(0, "none: an agent asks with pnpm wt:action <kind> <text>")];
        }
        tree.actions.iter().flat_map(|action| action_rows(action, "")).collect()
    });
    frame.section("GIT", "", || {
        let push = git::push_state(&tree.dir, tree.branch.as_deref());
        let mut text = git_text(&tree);
        if push.unpushed > 0 {
            text += &format!(" · {} not pushed", push.unpushed);
        }
        if !push.has_remote_branch {
            text += " · no remote branch";
        }
        vec![Row::new(0, text)]
    });
    let error = mr.as_ref().err().cloned();
    frame.section("MERGE REQUEST", "", || match mr {
        Ok(Some(mr)) => {
            let cells = [
                format!("!{}", mr.iid),
                mr.state_word().to_string(),
                mr.pipeline_mark(),
                mr.title.clone(),
            ];
            let cells: Vec<String> = cells.into_iter().filter(|cell| !cell.is_empty()).collect();
            vec![Row::new(0, cells.join("  ")).hang(4)]
        }
        Ok(None) => vec![Row::dim(
            0,
            "no MR yet: pnpm wt:mr opens the draft from the first commit",
        )],
        Err(_) => vec![],
    });
    frame.section("ISSUE", "", || match (number, issue) {
        (None, _) => vec![Row::dim(0, "the branch names no issue")],
        (Some(number), Some(Ok(issue))) => {
            let boxes = boxes_of(str_of(&issue, "description").unwrap_or(""));
            let ticked = boxes.iter().filter(|is_ticked| **is_ticked).count();
            let state = str_of(&issue, "state").unwrap_or("");
            let title = str_of(&issue, "title").unwrap_or("");
            let (mut lines, hang) = table(&[vec![format!("#{number}"), state.to_string(), title.to_string()]]);
            let mut rows = vec![Row::new(0, lines.remove(0)).hang(hang)];
            if !boxes.is_empty() {
                rows.push(Row::dim(
                    4,
                    format!("{ticked}/{} boxes ticked · pnpm wt:tick {number}", boxes.len()),
                ));
            }
            rows
        }
        (Some(number), Some(Err(reason))) => vec![Row::new(0, format!("#{number}: {reason}")).strong()],
        (Some(number), None) => vec![Row::dim(0, format!("#{number}"))],
    });
    if let Some(error) = error {
        frame.line(Row::new(1, format!("GitLab could not be read: {error}")).strong());
    }
    frame.blank();
    let back = tree
        .origin
        .as_ref()
        .map(|origin| format!("pnpm overview --orchestration {origin}"))
        .unwrap_or_default();
    let footer = if back.is_empty() {
        "pnpm overview (everything)".to_string()
    } else {
        format!("{back} · pnpm overview (everything)")
    };
    frame.line(Row::dim(1, footer));
}

/// A branch as one part of a URL's query: `/` and the rest of the reserved characters escaped.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// everything
// ---------------------------------------------------------------------------

pub fn everything(frame: &mut Frame, main: &Path, here: &Path) {
    let worktrees = git::worktrees(main);
    let (trees, open, issues) = thread::scope(|scope| {
        let open = scope.spawn(|| {
            mrs_of(gitlab::list(
                main,
                "projects/:fullpath/merge_requests?state=opened&per_page=100",
            ))
        });
        let issues = scope.spawn(|| gitlab::list(main, "projects/:fullpath/issues?state=opened&per_page=100"));
        let trees = trees_of(main, &worktrees);
        let open = open.join().unwrap_or_else(|_| Err("the read broke off".into()));
        let issues = issues.join().unwrap_or_else(|_| Err("the read broke off".into()));
        (trees, open, issues)
    });
    let main_branch = trees
        .first()
        .and_then(|tree| tree.branch.clone())
        .unwrap_or_else(|| "(detached)".into());
    let repo = main
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    frame.band(&format!("{repo}  {main_branch}"), &band_time(now_ms()));
    let error = open.as_ref().err().or(issues.as_ref().err()).cloned();
    let open = open.unwrap_or_default();
    let issues = issues.unwrap_or_default();

    frame.section("WORKTREES", &trees.len().to_string(), || {
        let cells: Vec<Vec<String>> = trees
            .iter()
            .enumerate()
            .map(|(index, tree)| {
                let mark = if git::is_inside(here, &tree.dir) { "→" } else { " " };
                let name = if index == 0 {
                    format!("{repo} {main_branch}")
                } else {
                    let origin = tree
                        .origin
                        .as_ref()
                        .map(|origin| format!(" {ORCHESTRATOR} {origin}"))
                        .unwrap_or_default();
                    format!("{}{origin}", tree.feature)
                };
                let mr = tree
                    .branch
                    .as_ref()
                    .and_then(|branch| open.iter().find(|mr| &mr.source_branch == branch));
                let mr = mr
                    .map(|mr| format!("!{} {}", mr.iid, mr.state_word()))
                    .unwrap_or_default();
                vec![mark.to_string(), name, git_text(tree), mr]
            })
            .collect();
        let (lines, hang) = table(&cells);
        lines.into_iter().map(|line| Row::new(0, line).hang(hang)).collect()
    });
    frame.section("MERGE REQUESTS", &format!("{} open", open.len()), || {
        let cells: Vec<Vec<String>> = open
            .iter()
            .map(|mr| {
                vec![
                    format!("!{}", mr.iid),
                    mr.state_word().into(),
                    mr.pipeline_mark(),
                    mr.title.clone(),
                ]
            })
            .collect();
        let (lines, hang) = table(&cells);
        lines.into_iter().map(|line| Row::new(0, line).hang(hang)).collect()
    });
    // Claimed: a worktree's or an open MR's branch names it.
    let claim_of = |number: u64| {
        let tree = trees.iter().skip(1).find_map(|tree| {
            tree.branch
                .clone()
                .filter(|branch| issue_of_branch(branch) == Some(number))
        });
        let branch = tree.or_else(|| {
            open.iter()
                .find(|mr| issue_of_branch(&mr.source_branch) == Some(number))
                .map(|mr| mr.source_branch.clone())
        })?;
        let mr = open
            .iter()
            .find(|mr| mr.source_branch == branch)
            .map(|mr| format!(" !{}", mr.iid))
            .unwrap_or_default();
        Some(format!("← {branch}{mr}"))
    };
    let mut rows: Vec<(Option<String>, u64, String)> = issues
        .iter()
        .filter_map(|issue| {
            let iid = issue.get("iid")?.as_u64()?;
            Some((claim_of(iid), iid, str_of(issue, "title").unwrap_or("").to_string()))
        })
        .collect();
    rows.sort_by_key(|(claim, iid, _)| (claim.is_none(), std::cmp::Reverse(*iid)));
    frame.section("ISSUES", &format!("{} open, claimed first", rows.len()), || {
        let cells: Vec<Vec<String>> = rows
            .iter()
            .map(|(_, iid, title)| vec![format!("#{iid}"), title.clone()])
            .collect();
        let (lines, hang) = table(&cells);
        let mut shown = Vec::new();
        for ((claim, _, _), line) in rows.iter().zip(lines) {
            shown.push(Row::new(0, line).hang(hang));
            shown.push(Row::dim(hang, claim.clone().unwrap_or_else(|| "unclaimed".into())).hang(hang));
        }
        shown
    });
    if let Some(error) = error {
        frame.line(Row::new(1, format!("GitLab could not be read: {error}")).strong());
    }
    frame.blank();
    frame.line(Row::dim(
        1,
        "pnpm stack:up|down|reset --port <p> · pnpm wt:new <n-feature> · pnpm commands",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_an_mr_with_or_without_its_pipeline() {
        let mr = Mr::from_value(
            &json!({"iid": 3, "state": "opened", "draft": true, "head_pipeline": {"status": "success"}}),
        )
        .unwrap();
        assert_eq!((mr.state_word(), mr.pipeline_mark().as_str()), ("draft", "✓ passed"));
        let mr = Mr::from_value(&json!({"iid": 4, "state": "opened", "head_pipeline": null})).unwrap();
        assert_eq!((mr.state_word(), mr.pipeline_mark().as_str()), ("ready", "no pipeline"));
        assert_eq!(Mr::from_value(&json!({"iid": 5})).unwrap().pipeline_mark(), "");
        assert_eq!(Mr::from_value(&json!({"title": "no iid"})), None);
    }

    #[test]
    fn encodes_a_branch_for_a_query() {
        assert_eq!(encode("feature/3-a.b"), "feature%2F3-a.b");
    }
}
