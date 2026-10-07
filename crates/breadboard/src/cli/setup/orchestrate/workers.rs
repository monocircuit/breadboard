//! Agents are workers (#217, workers.mjs, fit.mjs, memory.mjs, history.mjs):
//! the next issue goes to an idle agent that knows its files, else to the one
//! with the most room, else to a fresh one on a stack the memory allows.

use crate::cli::setup::conversations::{bound_session, context_share_of};
use crate::cli::setup::gitlab;
use crate::cli::setup::herdr;
use crate::cli::setup::repo;
use breadboard_core::json::{int_of, str_of};
use breadboard_core::record::{self, Record};
use breadboard_core::time::{now_ms, to_iso};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

/// The share of its context above which an agent gets no next issue.
pub const CONTEXT_LIMIT: f64 = 0.5;

/// An agent of the orchestration: bound to one of its worktrees, or free in its checkout.
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub dir: PathBuf,
    pub id: String,
    pub name: String,
    pub is_free: bool,
}

/// The checkout of an orchestration (its space stands on it).
pub fn checkout_of(name: &str) -> PathBuf {
    record::records_dir(&repo::main_dir()).join(name)
}

/// The agents of `name`: one per worktree of it with a conversation bound, then the free ones.
pub fn workers_of(name: &str) -> Vec<WorkerInfo> {
    let main = repo::main_dir();
    let found = record::record_of(&main, name);
    let known = |id: &str| found.as_ref().and_then(|found| found.worker(id).cloned());
    let bound: Vec<WorkerInfo> = repo::worktrees()
        .into_iter()
        .skip(1)
        .filter(|tree| record::origin_of(&tree.dir).is_some_and(|origin| origin.name == name))
        .filter_map(|tree| {
            let id = bound_session(&tree.dir)?;
            let feature = repo::feature_of(&tree.dir);
            let worker = known(&id);
            Some(WorkerInfo {
                name: worker
                    .as_ref()
                    .and_then(|worker| worker.name.clone())
                    .unwrap_or_else(|| feature.clone()),
                dir: tree.dir,
                id,
                is_free: false,
            })
        })
        .collect();
    let free: Vec<WorkerInfo> = found
        .as_ref()
        .map(|found| {
            found
                .free_agents()
                .into_iter()
                .filter(|worker| !bound.iter().any(|known| known.id == worker.id))
                .map(|worker| WorkerInfo {
                    dir: checkout_of(name),
                    id: worker.id.clone(),
                    name: worker.name.clone().unwrap_or_default(),
                    is_free: true,
                })
                .collect()
        })
        .unwrap_or_default();
    bound.into_iter().chain(free).collect()
}

/// The pane herdr runs the conversation `id` in.
pub fn pane_of_conversation(id: &str) -> Option<Value> {
    herdr::all_panes()
        .into_iter()
        .find(|pane| herdr::session_of(pane) == Some(id))
}

/// Why an agent can't take a next issue, or None.
pub fn refusal_of(pane: Option<&Value>, share: Option<f64>, limit: f64) -> Option<String> {
    let Some(pane) = pane else {
        return Some("no claude runs it (herdr knows no pane with its conversation)".into());
    };
    let status = str_of(pane, "agent_status").unwrap_or("unknown");
    match status {
        "working" => return Some("it is working".into()),
        "blocked" => return Some("it waits for an answer".into()),
        "idle" | "done" => {}
        other => return Some(format!("herdr can't tell what it does ({other})")),
    }
    share.filter(|share| *share > limit).map(|share| {
        format!(
            "its context is at {} %, above the limit of {} %",
            (share * 100.0).round() as i64,
            (limit * 100.0).round() as i64
        )
    })
}

/// The files a worktree's branch touched (workers.mjs `filesOf`).
pub fn files_of(dir: &std::path::Path) -> Vec<String> {
    breadboard_core::git::branch_files(dir)
}

fn unique(list: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::new();
    for item in list {
        if !seen.contains(&item) {
            seen.push(item);
        }
    }
    seen
}

fn history_of(entry: Option<&Value>) -> Vec<Value> {
    if let Some(history) = entry.and_then(|entry| entry.get("history")).and_then(Value::as_array) {
        return history.clone();
    }
    entry
        .and_then(|entry| entry.get("issues"))
        .and_then(Value::as_array)
        .map(|issues| {
            issues
                .iter()
                .enumerate()
                .map(|(i, number)| json!({ "number": number, "how": if i == 0 { "fresh" } else { "handover" }, "at": null, "context": null }))
                .collect()
        })
        .unwrap_or_default()
}

fn worker_entry(name: &str, id: &str) -> Option<Value> {
    record::record_of(&repo::main_dir(), name)?
        .raw
        .get("workers")?
        .get(id)
        .cloned()
}

/// A number as JSON.stringify writes it: `0`, not `0.0`.
pub fn js_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// The record learns that the agent `id` started on `number` (history.mjs `recordStart`).
pub fn record_start(name: &str, id: &str, number: i64, how: &str, context: Option<f64>, fields: Map<String, Value>) {
    let entry = worker_entry(name, id);
    let history: Vec<Value> = history_of(entry.as_ref())
        .into_iter()
        .filter(|start| int_of(start, "number") != Some(number))
        .collect();
    let mut issues: Vec<Value> = history
        .iter()
        .filter_map(|start| start.get("number").cloned())
        .collect();
    issues.push(json!(number));
    let mut all = history;
    all.push(json!({ "number": number, "how": how, "at": to_iso(now_ms()), "context": context.map(js_number) }));
    let mut merged = fields;
    if let Some(Value::Object(entry)) = &entry {
        for (key, value) in entry {
            merged.insert(key.clone(), value.clone());
        }
    }
    merged.insert("history".into(), Value::Array(all));
    merged.insert("issues".into(), Value::Array(issues));
    let _ = record::write_worker(&repo::main_dir(), name, id, merged);
}

// ---------------------------------------------------------------------------
// Who gets which issue (fit.mjs, memory.mjs)
// ---------------------------------------------------------------------------

/// An idle agent free for a next issue now.
#[derive(Debug, Clone)]
pub struct Idle {
    pub name: String,
    pub share: f64,
    pub files: Vec<String>,
    pub is_merged: bool,
}

/// The agents of `name` free for a next issue now: idle, context below the limit, MR merged (or free).
pub fn idle_workers_now(name: &str, found: Option<&Record>) -> Vec<Idle> {
    let branches: Vec<(PathBuf, Option<String>)> = repo::worktrees()
        .into_iter()
        .map(|tree| (tree.dir, tree.branch))
        .collect();
    let candidates: Vec<(WorkerInfo, Option<f64>)> = workers_of(name)
        .into_iter()
        .map(|worker| {
            let share = context_share_of(&worker.id);
            (worker, share)
        })
        .filter(|(worker, share)| {
            refusal_of(pane_of_conversation(&worker.id).as_ref(), *share, CONTEXT_LIMIT).is_none()
        })
        .collect();
    let merged = gitlab::in_turns(&candidates, 12, |(worker, _)| {
        if worker.is_free {
            return true;
        }
        let branch = branches
            .iter()
            .find(|(dir, _)| *dir == worker.dir)
            .and_then(|(_, branch)| branch.clone())
            .unwrap_or_default();
        gitlab::list(&format!(
            "projects/:fullpath/merge_requests?state=merged&source_branch={}",
            gitlab::encode(&branch)
        ))
        .is_ok_and(|mrs| !mrs.is_empty())
    });
    candidates
        .into_iter()
        .zip(merged)
        .filter(|(_, is_merged)| *is_merged)
        .map(|((worker, share), _)| {
            let known = found
                .and_then(|found| found.worker(&worker.id))
                .map(|known| known.files.clone())
                .unwrap_or_default();
            let own = if worker.is_free {
                Vec::new()
            } else {
                files_of(&worker.dir)
            };
            Idle {
                name: worker.name,
                share: share.unwrap_or(0.0),
                files: unique(known.into_iter().chain(own)),
                is_merged: true,
            }
        })
        .collect()
}

static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:[\w@.-]+/)*[\w@-][\w@.-]*\.(?:mjs|cjs|js|jsx|ts|tsx|json|md|css|scss|sql|toml|ya?ml)\b").unwrap()
});

/// The file paths a description names, each once.
pub fn paths_of(text: &str) -> Vec<String> {
    unique(PATH.find_iter(text).map(|found| found.as_str().to_string()))
}

/// The paths of `paths` a file of `files` is.
pub fn shared_of(paths: &[String], files: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|name| {
            files
                .iter()
                .any(|file| file == *name || file.ends_with(&format!("/{name}")))
        })
        .cloned()
        .collect()
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const STACK_MEMORY: f64 = GIB;
const AGENT_MEMORY: f64 = 0.5 * GIB;

/// The memory free for a new process: `MemAvailable` of Linux.
pub fn free_memory() -> f64 {
    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
        .map(|kb| kb * 1024.0)
        .unwrap_or_else(|| {
            // SAFETY: sysconf only reads.
            let pages = unsafe { libc::sysconf(libc::_SC_AVPHYS_PAGES) } as f64;
            let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as f64;
            pages * size
        })
}

fn gb(bytes: f64) -> String {
    format!("{:.1} GB", bytes / GIB)
}

/// The stack of the next fresh agent: `(is_shared, why)`.
pub fn stack_for(is_shared: bool, free: f64, reserved: f64) -> (bool, Option<String>) {
    if is_shared {
        return (true, None);
    }
    let left = free - reserved;
    let threshold = STACK_MEMORY + AGENT_MEMORY;
    if left < threshold {
        let why = format!(
            "{} free, below {} (own stack ~{} + agent ~{}): on the shared stack",
            gb(left.max(0.0)),
            gb(threshold),
            gb(STACK_MEMORY),
            gb(AGENT_MEMORY)
        );
        return (true, Some(why));
    }
    (false, None)
}

/// How an issue starts.
#[derive(Debug, Clone)]
pub enum Start {
    Worker { worker: Idle, shared: Vec<String> },
    Fresh { is_shared: bool, why: Option<String> },
}

/// For each issue (`(number, description)`, in order): an idle agent that
/// fits, else a fresh one, the memory of the fresh ones before it counted.
pub fn starts_of(items: &[(String, String)], idle: &[Idle], is_shared: bool, free: f64) -> Vec<Start> {
    let mut taken: Vec<usize> = Vec::new();
    let mut suggested: Vec<Option<(usize, Vec<String>)>> = vec![None; items.len()];
    let pick = |candidates: Vec<(usize, Vec<String>)>, taken: &mut Vec<usize>| -> Option<(usize, Vec<String>)> {
        let mut free: Vec<(usize, Vec<String>)> = candidates
            .into_iter()
            .filter(|(worker, _)| !taken.contains(worker))
            .collect();
        free.sort_by(|a, b| {
            b.1.len().cmp(&a.1.len()).then(
                idle[a.0]
                    .share
                    .partial_cmp(&idle[b.0].share)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        let best = free.into_iter().next()?;
        taken.push(best.0);
        Some(best)
    };
    for (at, (_, description)) in items.iter().enumerate() {
        let paths = paths_of(description);
        let candidates = idle
            .iter()
            .enumerate()
            .map(|(worker, idle)| (worker, shared_of(&paths, &idle.files)))
            .filter(|(_, shared)| !shared.is_empty())
            .collect();
        suggested[at] = pick(candidates, &mut taken);
    }
    for slot in suggested.iter_mut().filter(|slot| slot.is_none()) {
        let candidates = idle
            .iter()
            .enumerate()
            .filter(|(_, idle)| idle.is_merged)
            .map(|(worker, _)| (worker, Vec::new()))
            .collect();
        *slot = pick(candidates, &mut taken);
    }
    let mut reserved = 0.0;
    suggested
        .into_iter()
        .map(|found| match found {
            Some((worker, shared)) => Start::Worker {
                worker: idle[worker].clone(),
                shared,
            },
            None => {
                let (is_shared, why) = stack_for(is_shared, free, reserved);
                reserved += AGENT_MEMORY + if is_shared { 0.0 } else { STACK_MEMORY };
                Start::Fresh { is_shared, why }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle(name: &str, share: f64, files: &[&str]) -> Idle {
        Idle {
            name: name.into(),
            share,
            files: files.iter().map(|f| f.to_string()).collect(),
            is_merged: true,
        }
    }

    #[test]
    fn hands_an_issue_to_the_agent_that_knows_its_files() {
        let workers = [idle("a", 0.1, &["x/y.ts"]), idle("b", 0.3, &["src/z.mjs"])];
        let items = [
            ("1".to_string(), "touches z.mjs".to_string()),
            ("2".to_string(), "nothing".to_string()),
        ];
        let starts = starts_of(&items, &workers, false, 64.0 * GIB);
        assert!(matches!(&starts[0], Start::Worker { worker, shared } if worker.name == "b" && shared == &["z.mjs"]));
        assert!(matches!(&starts[1], Start::Worker { worker, shared } if worker.name == "a" && shared.is_empty()));
    }

    #[test]
    fn starts_fresh_ones_on_the_shared_stack_when_memory_is_short() {
        let starts = starts_of(
            &[("1".into(), String::new()), ("2".into(), String::new())],
            &[],
            false,
            2.0 * GIB,
        );
        assert!(matches!(
            &starts[0],
            Start::Fresh {
                is_shared: false,
                why: None
            }
        ));
        assert!(
            matches!(&starts[1], Start::Fresh { is_shared: true, why: Some(why) } if why == "0.5 GB free, below 1.5 GB (own stack ~1.0 GB + agent ~0.5 GB): on the shared stack")
        );
    }

    #[test]
    fn refuses_a_busy_or_full_agent() {
        let pane = json!({ "agent_status": "working" });
        assert_eq!(refusal_of(Some(&pane), None, 0.5).as_deref(), Some("it is working"));
        let idle = json!({ "agent_status": "idle" });
        assert_eq!(
            refusal_of(Some(&idle), Some(0.61), 0.5).as_deref(),
            Some("its context is at 61 %, above the limit of 50 %")
        );
        assert_eq!(refusal_of(Some(&idle), Some(0.2), 0.5), None);
        assert!(refusal_of(None, None, 0.5).unwrap().starts_with("no claude runs it"));
    }

    #[test]
    fn finds_the_paths_a_text_names() {
        assert_eq!(paths_of("see src/a.ts and a.ts, src/a.ts"), vec!["src/a.ts", "a.ts"]);
    }
}
