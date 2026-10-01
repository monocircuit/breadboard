//! The agents as the workers of an orchestration, as the printed view sees
//! them (workers.mjs, handovers.mjs, setups.mjs, fit.mjs): the handovers the
//! orchestrator announced and where they stand, the starts set up right now
//! by any process, and for each queued issue the free agent that knows its
//! files, else the one with the most room, else a fresh one. The rows of UP
//! NEXT and what the queue's nodes say. A print starts nothing: what the
//! live overview would do by itself is only named.

use super::View;
use super::local::Tree;
use super::remote::Mr;
use super::tree::{THRESHOLD, gb};
use breadboard_core::json::{self, int_of, str_of};
use breadboard_core::lock::is_alive;
use breadboard_core::record::{Worker, records_dir};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Above this share of its context an agent takes no next issue (workers.mjs `CONTEXT_LIMIT`).
pub const CONTEXT_LIMIT: f64 = 0.5;

/// A start running now (setups.mjs): wt:new makes `dir` for the agent `to`
/// (None: a fresh one), which leaves `from`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Setup {
    pub number: i64,
    pub dir: Option<PathBuf>,
    pub to: Option<String>,
    pub from: Option<PathBuf>,
}

/// The setups of the orchestration `name` whose process still runs. Read
/// only: one whose process is gone is left for the live overview to drop.
pub fn setups_of(main: &Path, name: &str) -> Vec<Setup> {
    let Ok(entries) = std::fs::read_dir(records_dir(main).join(format!("{name}.setups"))) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| json::read(&entry.path()))
        .filter(|setup| int_of(setup, "pid").is_some_and(is_alive))
        .filter_map(|setup| {
            Some(Setup {
                number: int_of(&setup, "number")?,
                dir: str_of(&setup, "dir").map(PathBuf::from),
                to: str_of(&setup, "to").map(String::from),
                from: str_of(&setup, "from").map(PathBuf::from),
            })
        })
        .collect()
}

/// The record's agent of a conversation.
fn worker_of<'a>(view: &'a View, id: &str) -> Option<&'a Worker> {
    view.record.workers.iter().find(|worker| worker.id == id)
}

/// The name the agent of a worktree goes by: the record's (#217), else the worktree's.
pub fn agent_name_of(view: &View, tree: &Tree) -> String {
    tree.claude
        .as_ref()
        .and_then(|claude| worker_of(view, &claude.id)?.name.clone())
        .unwrap_or_else(|| tree.name.clone())
}

/// herdr's state of the agent of a conversation, None when herdr runs none.
pub fn status_of<'a>(view: &'a View, id: &str) -> Option<&'a str> {
    view.agents.get(id).map(|agent| agent.status.as_str())
}

/// Why an agent can't take a next issue now (workers.mjs `refusalOf`), None when it can.
fn refusal_of(status: Option<&str>) -> Option<String> {
    match status {
        None => Some("no claude runs it (herdr knows no pane with its conversation)".into()),
        Some("working") => Some("it is working".into()),
        Some("blocked") => Some("it waits for an answer".into()),
        Some("idle" | "done") => None,
        Some(other) => Some(format!(
            "herdr can't tell what it does ({})",
            if other.is_empty() { "unknown" } else { other }
        )),
    }
}

/// An announced handover (#227) and where it stands (handovers.mjs `announcedOf`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Announced {
    pub number: i64,
    pub to: String,
    pub at: Option<String>,
    /// The worktree its agent is in, an index into the view's trees.
    pub tree: Option<usize>,
    /// Or the free agent (#289) it goes to, its id.
    pub free: Option<String>,
    /// The awaited MRs not merged yet.
    pub open: Vec<i64>,
    /// All merged and the agent idle: the live overview hands it over now.
    pub is_due: bool,
    /// Why not, once the MRs are merged.
    pub why: Option<String>,
}

/// The MRs of `after` not known merged (one GitLab didn't answer for counts as open).
fn open_after_of(view: &View, after: &[i64]) -> Vec<i64> {
    after
        .iter()
        .copied()
        .filter(|iid| {
            !view
                .remote
                .as_ref()
                .is_some_and(|remote| remote.awaited.get(iid).is_some_and(|state| state == "merged"))
        })
        .collect()
}

/// Each announced handover of the record with where it stands.
pub fn announced_of(view: &View) -> Vec<Announced> {
    view.record
        .handovers
        .iter()
        .map(|handover| {
            let tree = view.trees.iter().position(|tree| {
                let Some(claude) = &tree.claude else { return false };
                // Announced under its name before a handover renamed it (#251): the same agent.
                let was = worker_of(view, &claude.id).and_then(|worker| worker.was.clone());
                agent_name_of(view, tree) == handover.to || was.as_deref() == Some(handover.to.as_str())
            });
            let free = tree.is_none().then(|| {
                view.record.free_agents().into_iter().find(|agent| {
                    agent.name.as_deref() == Some(handover.to.as_str())
                        || agent.was.as_deref() == Some(handover.to.as_str())
                })
            });
            let free = free.flatten().map(|agent| agent.id.clone());
            let id = tree
                .and_then(|at| view.trees[at].claude.as_ref().map(|claude| claude.id.clone()))
                .or_else(|| free.clone());
            let open = open_after_of(view, &handover.after);
            let why = match &id {
                Some(id) => refusal_of(status_of(view, id)),
                None => Some(format!("no worktree of the orchestration has an agent {}", handover.to)),
            };
            let is_answered = view.remote.is_some();
            Announced {
                number: handover.number,
                to: handover.to.clone(),
                at: handover.at.clone(),
                tree,
                free,
                is_due: is_answered && open.is_empty() && why.is_none(),
                why: if is_answered && open.is_empty() { why } else { None },
                open,
            }
        })
        .collect()
}

/// `!232, !233`.
fn mrs_text(iids: &[i64]) -> String {
    iids.iter().map(|iid| format!("!{iid}")).collect::<Vec<_>>().join(", ")
}

/// What the node of the agent's worktree says (handovers.mjs `nextText`): `→ next #226 after !232`.
pub fn next_text(handover: &Announced) -> String {
    if handover.open.is_empty() {
        format!("→ next #{} now", handover.number)
    } else {
        format!("→ next #{} after {}", handover.number, mrs_text(&handover.open))
    }
}

/// A queued issue's agent from an announced handover (tree.mjs `takenText`): `→ 242-x after !256`.
pub fn taken_text(handover: &Announced) -> String {
    if handover.open.is_empty() {
        format!("→ {} now", handover.to)
    } else {
        format!("→ {} after {}", handover.to, mrs_text(&handover.open))
    }
}

/// An agent free for a next issue (workers.mjs `freeWorkersOf`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FreeWorker {
    pub name: String,
    pub share: f64,
    pub files: Vec<String>,
    /// Its worktree, None for a free agent (#289).
    pub dir: Option<PathBuf>,
    /// Its MR merged: it takes a queued issue by itself.
    pub is_merged: bool,
}

/// Its MR merged, or ready with its pipeline passed: its work waits for no one but you.
fn is_done_mr(mr: Option<&Mr>) -> bool {
    mr.is_some_and(|mr| mr.is_merged() || (mr.is_open() && !mr.is_draft && mr.status() == Some("success")))
}

/// The free workers: an agent of a worktree herdr sees idle or done whose MR
/// is done and whose context is below the limit; then the free agents
/// (#289) herdr sees idle, below the limit.
fn free_workers_of(view: &View) -> Vec<FreeWorker> {
    let is_free = |status: Option<&str>| matches!(status, Some("idle" | "done"));
    let mut workers: Vec<FreeWorker> = view
        .items
        .iter()
        .filter_map(|item| {
            let tree = view.tree(item)?;
            let claude = tree.claude.as_ref()?;
            let mr = view.mr(item);
            if !is_free(status_of(view, &claude.id)) || !is_done_mr(mr) || claude.share() > CONTEXT_LIMIT {
                return None;
            }
            let known = worker_of(view, &claude.id);
            let mut files: Vec<String> = known.map(|worker| worker.files.clone()).unwrap_or_default();
            for file in &tree.files {
                if !files.contains(file) {
                    files.push(file.clone());
                }
            }
            Some(FreeWorker {
                name: known
                    .and_then(|worker| worker.name.clone())
                    .unwrap_or_else(|| tree.name.clone()),
                share: claude.share(),
                files,
                dir: Some(tree.dir.clone()),
                is_merged: mr.is_some_and(Mr::is_merged),
            })
        })
        .collect();
    for agent in view.record.free_agents() {
        let share = view.free_shares.get(&agent.id).copied().unwrap_or(0.0);
        if !is_free(status_of(view, &agent.id)) || share > CONTEXT_LIMIT {
            continue;
        }
        workers.push(FreeWorker {
            name: agent.name.clone().unwrap_or_else(|| agent.id.clone()),
            share,
            files: agent.files.clone(),
            dir: None,
            is_merged: true,
        });
    }
    workers
}

/// The paths of `paths` a file of `files` is: the same, or under a folder (fit.mjs `sharedOf`).
fn shared_of(paths: &[String], files: &[String]) -> Vec<String> {
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

/// The agent suggested for a queued issue and the files they share.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggested {
    pub worker: FreeWorker,
    pub shared: Vec<String>,
}

/// For each queued issue (`(number, paths)`, in the order they wait) the
/// free worker that shares most of its paths, the one with the most room on
/// a tie; an issue nothing shares goes to the merged one with the most room
/// (fit.mjs `suggestionsOf`). Each worker gets one issue.
fn suggestions_of(issues: &[(i64, Vec<String>)], workers: &[FreeWorker]) -> HashMap<i64, Suggested> {
    let mut taken: HashSet<usize> = HashSet::new();
    let mut suggested: HashMap<i64, Suggested> = HashMap::new();
    let mut assign = |number: i64, candidates: Vec<(usize, Vec<String>)>, suggested: &mut HashMap<i64, Suggested>| {
        let best =
            candidates
                .into_iter()
                .filter(|(at, _)| !taken.contains(at))
                .min_by(|(a, a_shared), (b, b_shared)| {
                    b_shared
                        .len()
                        .cmp(&a_shared.len())
                        .then(workers[*a].share.total_cmp(&workers[*b].share))
                });
        if let Some((at, shared)) = best {
            taken.insert(at);
            suggested.insert(
                number,
                Suggested {
                    worker: workers[at].clone(),
                    shared,
                },
            );
        }
    };
    for (number, paths) in issues {
        let candidates = workers
            .iter()
            .enumerate()
            .map(|(at, worker)| (at, shared_of(paths, &worker.files)))
            .filter(|(_, shared)| !shared.is_empty())
            .collect();
        assign(*number, candidates, &mut suggested);
    }
    for (number, _) in issues {
        if suggested.contains_key(number) {
            continue;
        }
        let candidates = workers
            .iter()
            .enumerate()
            .filter(|(_, worker)| worker.is_merged)
            .map(|(at, _)| (at, Vec::new()))
            .collect();
        assign(*number, candidates, &mut suggested);
    }
    suggested
}

/// A row of UP NEXT for the workers: a handover or a fresh start.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub kind: &'static str,
    pub who: String,
    /// Its issue, an index into the view's items.
    pub item: Option<usize>,
    pub at: String,
    pub text: String,
}

/// What the workers part of the view needs.
#[derive(Debug, Clone, Default)]
pub struct Workers {
    pub rows: Vec<Row>,
    pub announced: Vec<Announced>,
    /// Queued issue → its suggested agent.
    pub suggested: HashMap<i64, Suggested>,
    /// Queued issue no agent is free for → what its fresh start does.
    pub fresh: HashMap<i64, String>,
    /// Worktree → why its clean-up waits: its agent moves to another issue now.
    pub holds: HashMap<PathBuf, String>,
}

/// `free, shares a.mjs, 18 %` / `once its MR is merged, 18 %`: why a suggested agent, and when.
fn worker_text(suggested: &Suggested) -> String {
    let mut parts = vec![if suggested.worker.is_merged {
        "free".to_string()
    } else {
        "once its MR is merged".to_string()
    }];
    if !suggested.shared.is_empty() {
        parts.push(format!("shares {}", suggested.shared.join(", ")));
    }
    parts.push(format!("{} %", (suggested.worker.share * 100.0).round() as i64));
    parts.join(", ")
}

/// The workers of the orchestration (workers.mjs `workerActionsOf`, the
/// print's half: nothing is handed out or started). `memory`: the bytes free now.
pub fn workers_of(view: &View, memory: f64) -> Workers {
    let announced = announced_of(view);
    // An agent a start moves takes nothing else meanwhile (#272).
    let promised: HashSet<&Path> = announced
        .iter()
        .filter_map(|handover| handover.tree.map(|at| view.trees[at].dir.as_path()))
        .chain(view.setups.iter().filter_map(|setup| setup.from.as_deref()))
        .collect();
    let promised_to: HashSet<&str> = announced
        .iter()
        .map(|handover| handover.to.as_str())
        .chain(view.setups.iter().filter_map(|setup| setup.to.as_deref()))
        .collect();
    let free: Vec<FreeWorker> = free_workers_of(view)
        .into_iter()
        .filter(|worker| match &worker.dir {
            Some(dir) => !promised.contains(dir.as_path()),
            None => !promised_to.contains(worker.name.as_str()),
        })
        .collect();
    let setting_up: HashSet<i64> = view.setups.iter().map(|setup| setup.number).collect();
    let work = view
        .record
        .plan
        .as_ref()
        .map(|plan| plan.work.clone())
        .unwrap_or_default();
    let mut queue: Vec<(usize, i64)> = view
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.entry.is_queued)
        .filter_map(|(at, item)| Some((at, item.number()?)))
        .filter(|(_, number)| !announced.iter().any(|handover| handover.number == *number))
        .filter(|(_, number)| !setting_up.contains(number))
        .collect();
    let rank = |number: i64| work.iter().position(|each| *each == number).unwrap_or(usize::MAX);
    queue.sort_by(|(a_at, a), (b_at, b)| {
        rank(*a)
            .cmp(&rank(*b))
            .then(
                view.items[*a_at]
                    .entry
                    .queued_at
                    .clone()
                    .unwrap_or_default()
                    .cmp(&view.items[*b_at].entry.queued_at.clone().unwrap_or_default()),
            )
            .then(a.cmp(b))
    });
    // One that needs an open MR waits for it, for an agent and a fresh start alike.
    let queued: Vec<(usize, i64)> = queue
        .into_iter()
        .filter(|(at, _)| open_after_of(view, &view.items[*at].entry.after).is_empty())
        .collect();
    let paths: Vec<(i64, Vec<String>)> = queued
        .iter()
        .map(|(at, number)| {
            let paths = view
                .found(&view.items[*at])
                .and_then(|found| found.issue.as_ref())
                .map(|issue| issue.paths.clone())
                .unwrap_or_default();
            (*number, paths)
        })
        .collect();
    let suggested = suggestions_of(&paths, &free);
    // Fresh agents while the memory allows (#229): its own stack fits next to it.
    let mut left = memory;
    let mut starts: Vec<i64> = Vec::new();
    for (_, number) in queued.iter().filter(|(_, number)| !suggested.contains_key(number)) {
        if left < THRESHOLD {
            break;
        }
        starts.push(*number);
        left -= THRESHOLD;
    }
    let mut rows: Vec<Row> = Vec::new();
    for handover in &announced {
        let state = if !handover.open.is_empty() {
            format!("after {}", mrs_text(&handover.open))
        } else if let Some(why) = &handover.why {
            format!("waits: {why}")
        } else if handover.is_due {
            "now".into()
        } else {
            "once it is idle".into()
        };
        rows.push(Row {
            kind: "hand over",
            who: handover.to.clone(),
            item: view
                .items
                .iter()
                .position(|item| item.number() == Some(handover.number)),
            at: handover.at.clone().unwrap_or_default(),
            text: format!("#{} → {} {state} (announced)", handover.number, handover.to),
        });
    }
    let mut fresh: HashMap<i64, String> = HashMap::new();
    for (_, number) in &queued {
        if suggested.contains_key(number) {
            continue;
        }
        let text = if starts.contains(number) {
            "starts a fresh agent".to_string()
        } else {
            format!(
                "waits for an agent to get free: {} free, below {} for a fresh one; a click starts one on the shared stack",
                gb(left.max(0.0)),
                gb(THRESHOLD)
            )
        };
        fresh.insert(*number, text);
    }
    for (at, number) in &queued {
        let item = &view.items[*at];
        match suggested.get(number) {
            None => rows.push(Row {
                kind: "start",
                who: item.feature.clone(),
                item: Some(*at),
                at: String::new(),
                text: format!(
                    "#{number} queued (--queue), no agent free: {}",
                    fresh.get(number).cloned().unwrap_or_default()
                ),
            }),
            Some(found) => rows.push(Row {
                kind: "hand over",
                who: found.worker.name.clone(),
                item: Some(*at),
                at: String::new(),
                text: format!("#{number} → {} ({})", found.worker.name, worker_text(found)),
            }),
        }
    }
    let holds = view
        .setups
        .iter()
        .filter_map(|setup| Some((setup.from.clone()?, format!("its agent takes #{} now", setup.number))))
        .collect();
    Workers {
        rows,
        announced,
        suggested,
        fresh,
        holds,
    }
}

/// What a queued issue waits for (tree.mjs `queuedText`): the agent the
/// orchestrator announced, the MRs it starts after, why it is back, the free
/// agent UP NEXT suggests, else what its fresh start does.
pub fn queued_text(view: &View, workers: &Workers, at: usize) -> String {
    let item = &view.items[at];
    let number = item.number().unwrap_or_default();
    if let Some(handover) = workers.announced.iter().find(|handover| handover.number == number) {
        return taken_text(handover);
    }
    let merged = view.merged_iids();
    let open: Vec<i64> = item
        .entry
        .after
        .iter()
        .copied()
        .filter(|iid| !merged.contains(iid))
        .collect();
    if !open.is_empty() {
        return format!("waits for {} to be merged", mrs_text(&open));
    }
    if let Some(requeued) = &item.entry.requeued
        && !workers.suggested.contains_key(&number)
    {
        return format!(
            "back in the queue: {requeued}; {}",
            workers
                .fresh
                .get(&number)
                .cloned()
                .unwrap_or_else(|| "waits for an agent".into())
        );
    }
    if let Some(found) = workers.suggested.get(&number) {
        return if found.worker.is_merged {
            format!(
                "→ {} (free, {} %)",
                found.worker.name,
                (found.worker.share * 100.0).round() as i64
            )
        } else {
            format!("→ {} once its MR is merged", found.worker.name)
        };
    }
    workers
        .fresh
        .get(&number)
        .cloned()
        .unwrap_or_else(|| "start: waits for an agent".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(name: &str, share: f64, files: &[&str], is_merged: bool) -> FreeWorker {
        FreeWorker {
            name: name.into(),
            share,
            files: files.iter().map(|file| file.to_string()).collect(),
            dir: None,
            is_merged,
        }
    }

    #[test]
    fn an_issue_goes_to_the_agent_that_knows_its_files_else_the_one_with_room() {
        let workers = vec![
            worker("a", 0.1, &["src/x.mjs"], true),
            worker("b", 0.3, &["src/overview/tree.mjs"], false),
            worker("c", 0.05, &[], true),
        ];
        let issues = vec![
            (1, vec!["tree.mjs".to_string()]),
            (2, vec!["nothing.md".to_string()]),
            (3, Vec::new()),
            (4, Vec::new()),
        ];
        let suggested = suggestions_of(&issues, &workers);
        assert_eq!(suggested[&1].worker.name, "b");
        assert_eq!(suggested[&1].shared, vec!["tree.mjs"]);
        assert_eq!(suggested[&2].worker.name, "c");
        assert_eq!(suggested[&3].worker.name, "a");
        assert!(!suggested.contains_key(&4));
        assert_eq!(
            worker_text(&suggested[&1]),
            "once its MR is merged, shares tree.mjs, 30 %"
        );
        assert_eq!(worker_text(&suggested[&2]), "free, 5 %");
    }

    #[test]
    fn says_why_an_agent_takes_no_next_issue() {
        assert_eq!(refusal_of(Some("idle")), None);
        assert_eq!(refusal_of(Some("done")), None);
        assert_eq!(refusal_of(Some("working")).as_deref(), Some("it is working"));
        assert_eq!(
            refusal_of(Some("unknown")).as_deref(),
            Some("herdr can't tell what it does (unknown)")
        );
        assert!(refusal_of(None).unwrap().starts_with("no claude runs it"));
    }

    #[test]
    fn names_a_handover_after_its_mrs_or_now() {
        let mut handover = Announced {
            number: 226,
            to: "224-x".into(),
            open: vec![232, 233],
            ..Announced::default()
        };
        assert_eq!(next_text(&handover), "→ next #226 after !232, !233");
        assert_eq!(taken_text(&handover), "→ 224-x after !232, !233");
        handover.open.clear();
        assert_eq!(next_text(&handover), "→ next #226 now");
        assert_eq!(taken_text(&handover), "→ 224-x now");
    }
}
