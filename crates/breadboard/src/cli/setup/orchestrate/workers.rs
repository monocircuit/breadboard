//! Agents are workers (#217, workers.mjs, fit.mjs, memory.mjs, history.mjs):
//! the next issue goes to an idle agent that knows its files, else to the one
//! with the most room, else to a fresh one on a stack the memory allows.

use crate::cli::lifecycle::handover::{self, CONTEXT_LIMIT, Worker, refusal_of, with_start};
use crate::cli::setup::gitlab;
use crate::cli::setup::repo;
use breadboard_core::conversations::context_share_of;
use breadboard_core::gitlab::paths_of;
use breadboard_core::model::workers::shared_of;
use breadboard_core::record::{self, Record};
use breadboard_core::time::{now_ms, to_iso};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

/// The agents of `name`: one per worktree of it with a conversation bound,
/// then the free ones (the lifecycle group's, one with the handover).
pub fn workers_of(name: &str) -> Vec<Worker> {
    handover::workers_of(&repo::lifecycle(), name)
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

/// A number as JSON.stringify writes it: `0`, not `0.0`.
pub fn js_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// The record learns that the agent `id` started on `number` (history.mjs
/// `recordStart`): `fields` first, its entry over them, its history with the
/// start (the handover's `with_start`).
pub fn record_start(name: &str, id: &str, number: i64, how: &str, context: Option<f64>, fields: Map<String, Value>) {
    let main = repo::main_dir();
    let entry = record::record_of(&main, name)
        .and_then(|found| found.raw.get("workers")?.get(id).cloned())
        .unwrap_or(Value::Null);
    let start = json!({ "number": number, "how": how, "at": to_iso(now_ms()), "context": context.map(js_number) });
    let mut merged = fields;
    merged.extend(with_start(&entry, start));
    let _ = record::write_worker(&main, name, id, merged);
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
    let candidates: Vec<(Worker, Option<f64>)> = workers_of(name)
        .into_iter()
        .map(|worker| {
            let share = context_share_of(&worker.id);
            (worker, share)
        })
        .filter(|(worker, share)| {
            let pane = handover::pane_of_conversation(&worker.id).ok().flatten();
            refusal_of(pane.as_ref(), worker.is_free, *share, CONTEXT_LIMIT).is_none()
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
                breadboard_core::git::branch_files(&worker.dir)
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

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
/// An orchestration's task stack (#351): no Postgres of its own.
const STACK_MEMORY: f64 = 0.5 * GIB;
const AGENT_MEMORY: f64 = 0.5 * GIB;

/// The memory free for a new process: core's `MemAvailable` of Linux, else what sysconf says.
pub fn free_memory() -> f64 {
    breadboard_core::model::memory::free_memory()
        .map(|bytes| bytes as f64)
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
            "{} free, below {} (task stack ~{} + agent ~{}): on the shared stack",
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
            1.5 * GIB,
        );
        assert!(matches!(
            &starts[0],
            Start::Fresh {
                is_shared: false,
                why: None
            }
        ));
        assert!(
            matches!(&starts[1], Start::Fresh { is_shared: true, why: Some(why) } if why == "0.5 GB free, below 1.0 GB (task stack ~0.5 GB + agent ~0.5 GB): on the shared stack")
        );
    }
}
