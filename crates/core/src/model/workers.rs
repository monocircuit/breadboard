//! Agents are workers (#217, overview/workers.mjs, worktree/fit.mjs): under UP
//! NEXT, for each queued issue the free agent that knows its code best, else
//! the one with the most room, else a fresh one (#265). The live overview
//! does it by itself: this says what is due (`orders`), the writing side runs
//! it and keeps what runs (`StartRun`).

use super::Entry;
use super::handovers::{Announced, CONTEXT_LIMIT, Known, mrs_text, worker_of};
use super::memory::{THRESHOLD, gb};
use super::order::{Go, Item, Target};
use crate::gitlab::{Mr, Remote};
use crate::herdr::Agent;
use crate::record::Worker;
use crate::setups::Setup;
use crate::time::parse_iso;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// The paths of `paths` a file of `files` is: the same, or `files` has it under a folder.
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

/// An agent free for a next issue: one of a worktree whose MR is done, or a
/// free one without a worktree (#289, `is_free`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FreeWorker {
    pub name: String,
    pub share: f64,
    pub files: Vec<String>,
    pub dir: Option<PathBuf>,
    pub is_merged: bool,
    pub is_free: bool,
    /// The issue of its worktree.
    pub number: Option<i64>,
}

/// The worker suggested for an issue, and the files of the issue it knows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Suggestion {
    pub worker: FreeWorker,
    pub shared: Vec<String>,
}

/// For each issue (`(number, paths)`, in the order they wait), the free worker
/// that shares the most of its paths, the one with the most room on a tie;
/// each worker one issue. One nothing shares goes to the merged worker with
/// the most room left (#265); one only ready waits for an issue it knows.
pub fn suggestions_of(issues: &[(i64, Vec<String>)], workers: &[FreeWorker]) -> HashMap<i64, Suggestion> {
    let mut taken: HashSet<usize> = HashSet::new();
    let mut suggested: HashMap<i64, Suggestion> = HashMap::new();
    let best_of = |candidates: Vec<(usize, Vec<String>)>, taken: &HashSet<usize>| {
        candidates
            .into_iter()
            .filter(|(at, _)| !taken.contains(at))
            .min_by(|(a, a_shared), (b, b_shared)| {
                b_shared
                    .len()
                    .cmp(&a_shared.len())
                    .then(workers[*a].share.total_cmp(&workers[*b].share))
            })
    };
    // Those that share files first: an issue without any takes no worker another one knows.
    for (number, paths) in issues {
        let candidates = workers
            .iter()
            .enumerate()
            .map(|(at, worker)| (at, shared_of(paths, &worker.files)))
            .filter(|(_, shared)| !shared.is_empty())
            .collect();
        if let Some((at, shared)) = best_of(candidates, &taken) {
            taken.insert(at);
            suggested.insert(
                *number,
                Suggestion {
                    worker: workers[at].clone(),
                    shared,
                },
            );
        }
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
        if let Some((at, shared)) = best_of(candidates, &taken) {
            taken.insert(at);
            suggested.insert(
                *number,
                Suggestion {
                    worker: workers[at].clone(),
                    shared,
                },
            );
        }
    }
    suggested
}

/// An agent whose work waits for no one but the human: its MR merged, or ready with its pipeline passed.
pub fn is_done_mr(mr: Option<&Mr>) -> bool {
    mr.is_some_and(|mr| mr.is_merged() || (mr.is_open() && !mr.is_draft && mr.pipeline_status() == Some("success")))
}

/// The free workers: an agent of a worktree herdr sees idle (or done), whose MR
/// is done and whose context is below the limit; then the free agents (#289).
/// `free`: the record's free agents with their share of context.
pub fn free_workers_of(
    issues: &[Entry],
    remote: Option<&Remote>,
    agents: &HashMap<String, Agent>,
    known: &Known,
    free: &[(Worker, f64)],
) -> Vec<FreeWorker> {
    let is_idle = |id: &str| {
        matches!(
            agents.get(id).and_then(|agent| agent.status.as_deref()),
            Some("idle" | "done")
        )
    };
    let mut workers: Vec<FreeWorker> = issues
        .iter()
        .filter_map(|issue| {
            let tree = issue.tree.as_ref()?;
            let claude = tree.claude.as_ref()?;
            let mr = remote.and_then(|remote| remote.of(&issue.branch).mr.as_ref());
            if !is_idle(&claude.id) || !is_done_mr(mr) {
                return None;
            }
            let share = if claude.window > 0 {
                claude.used as f64 / claude.window as f64
            } else {
                0.0
            };
            if share > CONTEXT_LIMIT {
                return None;
            }
            let record = worker_of(known, &claude.id);
            let mut files: Vec<String> = record.map(|worker| worker.files.clone()).unwrap_or_default();
            for file in &tree.branch_files {
                if !files.contains(file) {
                    files.push(file.clone());
                }
            }
            Some(FreeWorker {
                name: record
                    .and_then(|worker| worker.name.clone())
                    .unwrap_or_else(|| tree.name.clone()),
                share,
                files,
                dir: Some(tree.dir.clone()),
                is_merged: mr.is_some_and(Mr::is_merged),
                is_free: false,
                number: issue.number,
            })
        })
        .collect();
    workers.extend(
        free.iter()
            .filter(|(worker, share)| is_idle(&worker.id) && *share <= CONTEXT_LIMIT)
            .map(|(worker, share)| FreeWorker {
                name: worker.name.clone().unwrap_or_default(),
                share: *share,
                files: worker.files.clone(),
                dir: None,
                is_merged: true,
                is_free: true,
                number: None,
            }),
    );
    workers
}

/// Which queued issues go to their suggested agent now: those whose agent is merged.
pub fn hand_out_of(queued: &[i64], suggested: &HashMap<i64, Suggestion>) -> Vec<(i64, FreeWorker)> {
    queued
        .iter()
        .filter_map(|number| {
            let worker = &suggested.get(number)?.worker;
            worker.is_merged.then(|| (*number, worker.clone()))
        })
        .collect()
}

/// Which of the issues no agent is free for start a fresh agent now, memory
/// permitting (#229): `free` the memory free now, `reserved` what the fresh
/// agents still starting take. `(starts, left)`.
pub fn fresh_of(unsuggested: &[i64], free: u64, reserved: u64) -> (Vec<i64>, u64) {
    let mut left = free.saturating_sub(reserved);
    let mut starts = Vec::new();
    for number in unsuggested {
        if left < THRESHOLD {
            break;
        }
        starts.push(*number);
        left -= THRESHOLD;
    }
    (starts, left)
}

/// The MRs of `after` a queued issue still waits for (#272): not merged, or
/// not known yet (`awaited` None before GitLab answered).
pub fn open_after_of(after: &[i64], awaited: Option<&HashMap<i64, String>>) -> Vec<i64> {
    after
        .iter()
        .copied()
        .filter(|iid| awaited.and_then(|awaited| awaited.get(iid)).map(String::as_str) != Some("merged"))
        .collect()
}

/// The queued issues in the order they start: the work order, then when they were queued, then by number.
pub fn queue_order<'a>(queued: Vec<&'a Entry>, work: &[i64]) -> Vec<&'a Entry> {
    let rank = |issue: &Entry| {
        issue
            .number
            .and_then(|number| work.iter().position(|known| *known == number))
            .unwrap_or(usize::MAX)
    };
    let mut queued = queued;
    queued.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| {
                a.queued_at
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.queued_at.as_deref().unwrap_or(""))
            })
            .then_with(|| a.number.unwrap_or(i64::MAX).cmp(&b.number.unwrap_or(i64::MAX)))
    });
    queued
}

/// How long a failed announced handover waits before it is tried again.
pub const RETRY_MS: i64 = 60_000;

/// A start or handover running in the background, or what stopped it (the
/// writing side keeps these): `dir` the worktree of the agent it hands the
/// issue to, held until the agent moved; `is_fresh` for a fresh agent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartRun {
    pub is_running: bool,
    pub error: Option<String>,
    /// When it ended, ms.
    pub ended: i64,
    pub dir: Option<PathBuf>,
    pub is_fresh: bool,
    pub to: Option<String>,
}

/// Whether the announced handover goes now: due, not running, and not tried
/// yet, or failed at least RETRY_MS ago.
pub fn is_hand_over_now(handover: &Announced, run: Option<&StartRun>, now: i64) -> bool {
    handover.is_due && run.is_none_or(|run| !run.is_running && now - run.ended >= RETRY_MS)
}

/// `free, shares a.mjs, 18 %` / `once its MR is merged, 18 %`.
fn worker_text(found: &Suggestion) -> String {
    let mut parts = vec![if found.worker.is_merged {
        "free".to_string()
    } else {
        "once its MR is merged".into()
    }];
    if !found.shared.is_empty() {
        parts.push(format!("shares {}", found.shared.join(", ")));
    }
    parts.push(format!("{} %", (found.worker.share * 100.0).round() as i64));
    parts.join(", ")
}

/// What workerActionsOf finds: the rows of UP NEXT, the worktrees held while
/// their agents move out and why, the suggestions and fresh starts for the
/// queue of WORKTREES, and what the live overview does now (`orders`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkerActions {
    pub actions: Vec<Item>,
    pub holds: HashMap<PathBuf, String>,
    pub suggested: HashMap<i64, Suggestion>,
    pub fresh: HashMap<i64, String>,
    /// HandOver and StartFresh due now, for the one live overview (`should_hand_out`).
    pub orders: Vec<Go>,
}

/// What workerActionsOf knows besides the issues.
#[derive(Debug, Clone, Default)]
pub struct WorkerFacts<'a> {
    /// The one live overview, once GitLab answered: it hands out by itself.
    pub should_hand_out: bool,
    pub announced: &'a [Announced],
    /// Free memory, bytes.
    pub memory: u64,
    /// The starts running now, from any process (#272).
    pub setups: &'a [Setup],
    pub awaited: Option<&'a HashMap<i64, String>>,
    pub work: &'a [i64],
    /// The free agents (#289) with their share of context.
    pub free: &'a [(Worker, f64)],
    /// What this overview started, by issue number.
    pub runs: Option<&'a HashMap<i64, StartRun>>,
    pub now: i64,
}

/// How the text of a start that failed ends: the queued node paints it red.
pub const TRIES_AGAIN: &str = "; a click tries again";

/// The rows of UP NEXT for the workers of the orchestration: per queued issue
/// its suggested agent (`hand over`), else a fresh one (`start`); each
/// announced handover a row of its own (#227).
pub fn worker_actions_of(
    issues: &[Entry],
    remote: Option<&Remote>,
    agents: &HashMap<String, Agent>,
    known: &Known,
    facts: &WorkerFacts,
) -> WorkerActions {
    let none = HashMap::new();
    let runs = facts.runs.unwrap_or(&none);
    let running: Vec<&StartRun> = runs.values().filter(|run| run.is_running).collect();
    // An agent a start moves takes nothing else meanwhile (#272).
    let promised: HashSet<PathBuf> = facts
        .announced
        .iter()
        .filter_map(|handover| handover.agent.as_ref().and_then(|agent| agent.dir.clone()))
        .chain(
            facts
                .setups
                .iter()
                .filter_map(|setup| setup.from.as_ref().map(PathBuf::from)),
        )
        .chain(running.iter().filter_map(|run| run.dir.clone()))
        .collect();
    // A free agent has no worktree: it is promised by its name.
    let promised_to: HashSet<String> = facts
        .announced
        .iter()
        .map(|handover| handover.to.clone())
        .chain(facts.setups.iter().filter_map(|setup| setup.to.clone()))
        .chain(running.iter().filter_map(|run| run.to.clone()))
        .collect();
    let free: Vec<FreeWorker> = free_workers_of(issues, remote, agents, known, facts.free)
        .into_iter()
        .filter(|worker| {
            if worker.is_free {
                !promised_to.contains(&worker.name)
            } else {
                worker.dir.as_ref().is_none_or(|dir| !promised.contains(dir))
            }
        })
        .collect();
    let setting_up: HashSet<i64> = facts.setups.iter().map(|setup| setup.number).collect();
    // Queued again since its start failed (by hand): the old failure is over.
    let run_of = |number: Option<i64>| -> Option<&StartRun> {
        let number = number?;
        let run = runs.get(&number)?;
        let issue = issues.iter().find(|issue| issue.number == Some(number));
        let queued_at = issue.and_then(|issue| issue.queued_at.as_deref()).and_then(parse_iso);
        if !run.is_running && queued_at.is_some_and(|at| at > run.ended) {
            None
        } else {
            Some(run)
        }
    };
    let all = queue_order(
        issues
            .iter()
            .filter(|issue| {
                issue.is_queued
                    && !facts.announced.iter().any(|handover| Some(handover.number) == issue.number)
                    // Set up by another process: its worktree's node says so.
                    && !(issue.number.is_some_and(|number| setting_up.contains(&number)) && run_of(issue.number).is_none())
            })
            .collect(),
        facts.work,
    );
    // One that needs an open MR waits for it, for an agent and a fresh start alike.
    let queued: Vec<&Entry> = all
        .into_iter()
        .filter(|issue| open_after_of(&issue.after, facts.awaited).is_empty())
        .collect();
    let paths_of = |issue: &Entry| {
        remote
            .and_then(|remote| remote.of(&issue.branch).issue.as_ref())
            .map(|found| found.paths.clone())
            .unwrap_or_default()
    };
    let asked: Vec<(i64, Vec<String>)> = queued
        .iter()
        .filter_map(|issue| Some((issue.number?, paths_of(issue))))
        .collect();
    let suggested = suggestions_of(&asked, &free);
    // Not one started already, nor one that failed: a click retries it.
    let waiting: Vec<i64> = queued
        .iter()
        .filter_map(|issue| issue.number)
        .filter(|number| run_of(Some(*number)).is_none())
        .collect();
    let mut orders = Vec::new();
    if facts.should_hand_out {
        for (number, worker) in hand_out_of(&waiting, &suggested) {
            orders.push(Go::HandOver {
                number,
                to: worker.name.clone(),
                dir: worker.dir.clone(),
            });
        }
    }
    // The fresh agents still starting take their memory once their stacks are up.
    let reserved = running.iter().filter(|run| run.is_fresh).count() as u64 * THRESHOLD;
    let unsuggested: Vec<i64> = waiting
        .iter()
        .copied()
        .filter(|number| !suggested.contains_key(number))
        .collect();
    let (starts, left) = fresh_of(&unsuggested, facts.memory, reserved);
    if facts.should_hand_out {
        orders.extend(starts.iter().map(|number| Go::StartFresh(*number)));
    }
    let entry_of = |number: i64| issues.iter().find(|issue| issue.number == Some(number));
    let item = |key: String, who: &str, number: i64, text: String, is_running: bool, at: String, go: Go| Item {
        key,
        kind: if matches!(go, Go::StartFresh(_)) {
            "start".into()
        } else {
            "hand over".into()
        },
        text,
        at,
        who: who.to_string(),
        is_auto: true,
        rank: -1,
        number: Some(number),
        branch: entry_of(number).map(|issue| issue.branch.clone()),
        to: entry_of(number)
            .and_then(|issue| issue.tree.as_ref())
            .map(|tree| Target::Dir(tree.dir.clone())),
        is_running,
        go,
        ..Default::default()
    };
    let mut actions = Vec::new();
    for handover in facts.announced {
        let run = run_of(Some(handover.number));
        let dir = handover.agent.as_ref().and_then(|agent| agent.dir.clone());
        if facts.should_hand_out && is_hand_over_now(handover, run, facts.now) {
            orders.push(Go::HandOver {
                number: handover.number,
                to: handover.to.clone(),
                dir: dir.clone(),
            });
        }
        let is_running = run.is_some_and(|run| run.is_running);
        let text = if is_running {
            format!("#{} → {}: handing over …", handover.number, handover.to)
        } else if !handover.open.is_empty() {
            format!(
                "#{} → {} after {} (announced)",
                handover.number,
                handover.to,
                mrs_text(&handover.open)
            )
        } else {
            let state = match (run.and_then(|run| run.error.as_deref()), &handover.why) {
                (Some(error), _) => format!("failed: {error}; tries again once {} is idle", handover.to),
                (None, Some(why)) => format!("waits: {why}"),
                (None, None) if handover.is_due => "now".into(),
                (None, None) => "once it is idle".into(),
            };
            format!("#{} → {} {state} (announced)", handover.number, handover.to)
        };
        actions.push(item(
            format!("hand over:#{}:{}", handover.number, handover.to),
            &handover.to,
            handover.number,
            text,
            is_running,
            handover.at.clone().unwrap_or_default(),
            Go::HandOver {
                number: handover.number,
                to: handover.to.clone(),
                dir,
            },
        ));
    }
    // What the fresh start of each issue no agent is free for does.
    let mut fresh = HashMap::new();
    for number in queued
        .iter()
        .filter_map(|issue| issue.number)
        .filter(|number| !suggested.contains_key(number))
    {
        let run = run_of(Some(number));
        let text = match run {
            Some(run) if run.is_running => "starting a fresh agent …".to_string(),
            Some(StartRun {
                error: Some(error),
                is_fresh,
                to,
                ..
            }) => format!(
                "{} failed: {error}{TRIES_AGAIN}",
                if *is_fresh {
                    "its fresh agent".to_string()
                } else {
                    format!("its handover to {}", to.as_deref().unwrap_or("?"))
                }
            ),
            _ if starts.contains(&number) => "starts a fresh agent".into(),
            _ => format!(
                "waits for an agent to get free: {} free, below {} for a fresh one; a click starts one on the shared stack",
                gb(left),
                gb(THRESHOLD)
            ),
        };
        fresh.insert(number, text);
    }
    // A failed handover to the suggested agent: its queued node says why until a click tries again.
    for number in suggested.keys() {
        if let Some(StartRun {
            is_running: false,
            error: Some(error),
            ..
        }) = run_of(Some(*number))
        {
            fresh.insert(*number, format!("its handover failed: {error}{TRIES_AGAIN}"));
        }
    }
    for issue in &queued {
        let Some(number) = issue.number else { continue };
        let run = run_of(Some(number));
        // Queued until its agent moved in (#272): its handover's row meanwhile.
        if let Some(run) = run.filter(|run| run.is_running)
            && let Some(to) = &run.to
        {
            actions.push(item(
                format!("hand over:#{number}:{to}"),
                to,
                number,
                format!("#{number} → {to}: handing over …"),
                true,
                String::new(),
                Go::Nothing,
            ));
            continue;
        }
        let Some(found) = suggested.get(&number) else {
            actions.push(item(
                format!("start:#{number}"),
                &issue.feature,
                number,
                format!(
                    "#{number} queued (--queue), no agent free: {}",
                    fresh.get(&number).cloned().unwrap_or_default()
                ),
                run.is_some_and(|run| run.is_running),
                String::new(),
                Go::StartFresh(number),
            ));
            continue;
        };
        let name = &found.worker.name;
        let text = if run.is_some_and(|run| run.is_running) {
            format!("#{number} → {name}: handing over …")
        } else {
            let error = run
                .and_then(|run| run.error.as_deref())
                .map(|error| format!("  {error}"))
                .unwrap_or_default();
            format!("#{number} → {name} ({}){error}", worker_text(found))
        };
        actions.push(item(
            format!("hand over:#{number}:{name}"),
            name,
            number,
            text,
            run.is_some_and(|run| run.is_running),
            String::new(),
            Go::HandOver {
                number,
                to: name.clone(),
                dir: found.worker.dir.clone(),
            },
        ));
    }
    // Why the clean-up of each held worktree waits: only while its agent moves out.
    let mut holds: HashMap<PathBuf, String> = HashMap::new();
    for (number, run) in runs {
        if run.is_running
            && let Some(dir) = &run.dir
        {
            holds.insert(dir.clone(), format!("its agent takes #{number} now"));
        }
    }
    for setup in facts.setups {
        if let Some(from) = &setup.from {
            holds.insert(PathBuf::from(from), format!("its agent takes #{} now", setup.number));
        }
    }
    WorkerActions {
        actions,
        holds,
        suggested,
        fresh,
        orders,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::{BranchRemote, Issue, Pipeline};
    use crate::model::Tree;
    use crate::model::handovers::tests::claude_of;

    const GIB: u64 = 1 << 30;

    fn worker(name: &str, share: f64, files: &[&str], is_merged: bool) -> FreeWorker {
        FreeWorker {
            name: name.into(),
            share,
            files: files.iter().map(|f| f.to_string()).collect(),
            is_merged,
            ..Default::default()
        }
    }
    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn finds_the_files_an_issue_names_and_matches_them_by_their_end() {
        assert_eq!(
            crate::gitlab::paths_of(
                "Read `packages/breadboard/src/overview/render-orchestration.mjs`, overview/boxes.mjs and `tty.mjs`; tty.mjs again, orchestrate.md."
            ),
            [
                "packages/breadboard/src/overview/render-orchestration.mjs",
                "overview/boxes.mjs",
                "tty.mjs",
                "orchestrate.md"
            ]
        );
        let files = paths(&[
            "packages/breadboard/src/overview/boxes.mjs",
            "packages/breadboard/src/lib/tty.mjs",
        ]);
        assert_eq!(
            shared_of(
                &paths(&["overview/boxes.mjs", "tty.mjs", "xtty.mjs", "view.mjs"]),
                &files
            ),
            ["overview/boxes.mjs", "tty.mjs"]
        );
    }

    #[test]
    fn picks_the_agent_sharing_most_files_the_roomier_on_a_tie() {
        let a = worker("a", 0.3, &["src/x.mjs", "src/y.mjs"], false);
        let b = worker("b", 0.1, &["src/x.mjs"], false);
        let c = worker("c", 0.05, &["src/z.mjs"], false);
        let suggested = suggestions_of(&[(1, paths(&["x.mjs", "y.mjs"]))], &[a.clone(), b.clone(), c.clone()]);
        assert_eq!(suggested[&1].worker.name, "a");
        assert_eq!(suggested[&1].shared, ["x.mjs", "y.mjs"]);
        assert_eq!(
            suggestions_of(&[(2, paths(&["x.mjs"]))], &[a.clone(), b.clone()])[&2]
                .worker
                .name,
            "b"
        );
        let suggested = suggestions_of(
            &[
                (1, paths(&["x.mjs"])),
                (2, paths(&["x.mjs"])),
                (3, paths(&["x.mjs"])),
                (4, paths(&["w.mjs"])),
            ],
            &[a, b],
        );
        assert_eq!(suggested[&1].worker.name, "b");
        assert_eq!(suggested[&2].worker.name, "a");
        assert!(!suggested.contains_key(&3) && !suggested.contains_key(&4));
        let full = worker("full", 0.4, &[], true);
        let roomy = worker("roomy", 0.1, &[], true);
        let knows = FreeWorker { is_merged: true, ..c };
        let suggested = suggestions_of(
            &[
                (1, paths(&["new.mjs"])),
                (2, paths(&["z.mjs"])),
                (3, vec![]),
                (4, vec![]),
            ],
            &[full, knows, roomy],
        );
        assert_eq!(suggested[&2].worker.name, "c");
        assert_eq!(suggested[&1].worker.name, "roomy");
        assert!(suggested[&1].shared.is_empty());
        assert_eq!(suggested[&3].worker.name, "full");
        assert!(!suggested.contains_key(&4));
    }

    #[test]
    fn starts_fresh_agents_while_they_fit_and_hands_out_to_merged_ones() {
        assert_eq!(fresh_of(&[1, 2, 3], 4 * GIB, 0).0, [1, 2]);
        assert!(fresh_of(&[1], 4 * GIB, 3 * GIB).0.is_empty());
        let mut suggested = HashMap::new();
        for (number, name, is_merged) in [
            (1, "a", true),
            (2, "r", false),
            (3, "b", true),
            (4, "c", true),
            (5, "d", true),
            (6, "e", true),
        ] {
            suggested.insert(
                number,
                Suggestion {
                    worker: worker(name, 0.0, &[], is_merged),
                    shared: vec![],
                },
            );
        }
        let out: Vec<i64> = hand_out_of(&[1, 2, 3, 4, 5, 6, 7], &suggested)
            .into_iter()
            .map(|(number, _)| number)
            .collect();
        assert_eq!(out, [1, 3, 4, 5, 6]);
    }

    #[test]
    fn counts_an_mr_gitlab_didnt_answer_for_as_open_and_orders_the_queue() {
        assert_eq!(open_after_of(&[282, 283], None), [282, 283]);
        let merged: HashMap<i64, String> = [(282, "merged".to_string())].into();
        assert_eq!(open_after_of(&[282, 283], Some(&merged)), [283]);
        let item = |number: i64, at: &str| Entry {
            number: Some(number),
            queued_at: Some(at.into()),
            ..Default::default()
        };
        let items = [item(9, "b"), item(8, "a"), item(7, "c"), item(6, "c")];
        let ordered: Vec<i64> = queue_order(items.iter().collect(), &[7])
            .iter()
            .map(|issue| issue.number.unwrap())
            .collect();
        assert_eq!(ordered, [7, 8, 9, 6]);
    }

    fn idle(id: &str, status: &str) -> HashMap<String, Agent> {
        [(
            id.to_string(),
            Agent {
                pane: "p".into(),
                tab: "t1".into(),
                status: Some(status.into()),
            },
        )]
        .into()
    }
    fn with_claude(id: &str, used: u64) -> crate::conversations::Claude {
        crate::conversations::Claude {
            used,
            window: 200_000,
            ..claude_of(id)
        }
    }
    fn merged_tree(number: i64, feature: &str, id: &str, used: u64) -> Entry {
        Entry {
            number: Some(number),
            feature: feature.into(),
            branch: format!("feature/{feature}"),
            tree: Some(Tree {
                dir: format!("/nonexistent/platform-{feature}").into(),
                name: feature.into(),
                claude: Some(with_claude(id, used)),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn queued(number: i64, feature: &str) -> Entry {
        Entry {
            number: Some(number),
            feature: feature.into(),
            branch: format!("feature/{feature}"),
            is_queued: true,
            queued_at: Some(format!("2026-10-01T09:0{}:00Z", number % 10)),
            ..Default::default()
        }
    }
    fn remote_of(parts: Vec<(&str, Option<Mr>, Vec<&str>)>) -> Remote {
        Remote {
            of: parts
                .into_iter()
                .map(|(branch, mr, files)| {
                    let issue = Issue {
                        paths: paths(&files),
                        ..Default::default()
                    };
                    (
                        format!("feature/{branch}"),
                        BranchRemote {
                            mr,
                            issue: Some(issue),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }
    }
    fn merged() -> Option<Mr> {
        Some(Mr {
            state: "merged".into(),
            ..Default::default()
        })
    }

    #[test]
    fn offers_a_queued_issue_nothing_fits_as_a_start_one_that_fits_as_a_hand_over() {
        let issues = [
            merged_tree(3, "3-done", "c1", 20_000),
            queued(8, "8-fits"),
            queued(9, "9-new"),
        ];
        let remote = remote_of(vec![
            ("3-done", merged(), vec![]),
            ("8-fits", None, vec!["src/a.mjs"]),
            ("9-new", None, vec!["src/z.mjs"]),
        ]);
        let known = [Worker {
            id: "c1".into(),
            name: Some("3-done".into()),
            files: vec!["src/a.mjs".into()],
            ..Default::default()
        }];
        let found = worker_actions_of(
            &issues,
            Some(&remote),
            &idle("c1", "idle"),
            &known,
            &WorkerFacts::default(),
        );
        let kinds: Vec<(String, String)> = found
            .actions
            .iter()
            .map(|item| (item.kind.clone(), item.who.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("hand over".to_string(), "3-done".to_string()),
                ("start".to_string(), "9-new".to_string())
            ]
        );
        assert!(found.holds.is_empty());
        assert_eq!(found.suggested[&8].worker.name, "3-done");
    }

    #[test]
    fn hands_a_queued_issue_nothing_shares_to_the_free_agent() {
        let issues = [merged_tree(30, "30-done", "c1", 36_000), queued(31, "31-new")];
        let remote = remote_of(vec![("30-done", merged(), vec![]), ("31-new", None, vec![])]);
        let facts = WorkerFacts {
            should_hand_out: true,
            memory: 0,
            ..Default::default()
        };
        let found = worker_actions_of(&issues, Some(&remote), &idle("c1", "idle"), &[], &facts);
        assert_eq!(
            found.orders,
            [Go::HandOver {
                number: 31,
                to: "30-done".into(),
                dir: Some("/nonexistent/platform-30-done".into())
            }]
        );
        assert_eq!(found.actions[0].text, "#31 → 30-done (free, 18 %)");
        assert!(found.holds.is_empty());
    }

    #[test]
    fn starts_a_fresh_agent_while_memory_allows() {
        let issues = [queued(32, "32-a"), queued(33, "33-b")];
        let remote = remote_of(vec![("32-a", None, vec![]), ("33-b", None, vec![])]);
        let facts = WorkerFacts {
            should_hand_out: true,
            memory: 2 * GIB,
            ..Default::default()
        };
        let found = worker_actions_of(&issues, Some(&remote), &HashMap::new(), &[], &facts);
        assert_eq!(found.orders, [Go::StartFresh(32)]);
        assert_eq!(found.fresh[&32], "starts a fresh agent");
        assert!(found.fresh[&33].starts_with("waits for an agent to get free: "));
        assert_eq!(
            found.actions.iter().map(|item| item.kind.as_str()).collect::<Vec<_>>(),
            ["start", "start"]
        );
    }

    #[test]
    fn gives_the_free_agent_one_issue_none_while_its_handover_runs() {
        let issues = [
            merged_tree(268, "268-tokens", "c268", 40_000),
            queued(194, "194-q"),
            queued(267, "267-q"),
            queued(269, "269-q"),
        ];
        let remote = remote_of(vec![("268-tokens", merged(), vec![])]);
        let facts = WorkerFacts {
            should_hand_out: true,
            ..Default::default()
        };
        let found = worker_actions_of(&issues, Some(&remote), &idle("c268", "idle"), &[], &facts);
        assert_eq!(found.orders.len(), 1);
        assert!(matches!(&found.orders[0], Go::HandOver { number: 194, to, .. } if to == "268-tokens"));
        let setup = Setup {
            number: 194,
            dir: Some("/nonexistent/platform-194-q".into()),
            to: Some("268-tokens".into()),
            from: Some("/nonexistent/platform-268-tokens".into()),
            pid: 1,
            at: String::new(),
        };
        let facts = WorkerFacts {
            should_hand_out: true,
            setups: std::slice::from_ref(&setup),
            ..Default::default()
        };
        let found = worker_actions_of(&issues, Some(&remote), &idle("c268", "idle"), &[], &facts);
        assert!(found.orders.is_empty());
        assert!(found.suggested.is_empty());
        assert_eq!(
            found
                .actions
                .iter()
                .map(|item| (item.kind.as_str(), item.number.unwrap()))
                .collect::<Vec<_>>(),
            [("start", 267), ("start", 269)]
        );
        assert_eq!(
            found.holds[&PathBuf::from("/nonexistent/platform-268-tokens")],
            "its agent takes #194 now"
        );
    }

    #[test]
    fn starts_nothing_that_waits_for_an_open_mr() {
        let after = |number: i64, feature: &str| Entry {
            after: vec![282],
            ..queued(number, feature)
        };
        let issues = [
            merged_tree(268, "268-tokens", "c268", 40_000),
            after(267, "267-q"),
            after(269, "269-q"),
            queued(194, "194-q"),
        ];
        let remote = remote_of(vec![("268-tokens", merged(), vec![])]);
        let facts = WorkerFacts {
            should_hand_out: true,
            memory: 4 * GIB,
            work: &[194, 267],
            ..Default::default()
        };
        let found = worker_actions_of(&issues, Some(&remote), &idle("c268", "idle"), &[], &facts);
        assert!(matches!(&found.orders[..], [Go::HandOver { number: 194, .. }]));
        assert_eq!(found.suggested.keys().copied().collect::<Vec<_>>(), [194]);
        let awaited: HashMap<i64, String> = [(282, "merged".to_string())].into();
        let facts = WorkerFacts {
            awaited: Some(&awaited),
            ..facts
        };
        let found = worker_actions_of(&issues[..3], Some(&remote), &idle("c268", "idle"), &[], &facts);
        assert!(matches!(
            &found.orders[..],
            [Go::HandOver { number: 267, .. }, Go::StartFresh(269)]
        ));
    }

    #[test]
    fn takes_an_idle_agent_whose_mr_is_done_and_below_the_limit() {
        let issue = |mr: Option<Mr>, used: u64| {
            let entry = Entry {
                branch: "feature/5-x".into(),
                ..merged_tree(5, "5-x", "c1", used)
            };
            let remote = Remote {
                of: [(
                    "feature/5-x".to_string(),
                    BranchRemote {
                        mr,
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            };
            (entry, remote)
        };
        let known = [Worker {
            id: "c1".into(),
            name: Some("3-first".into()),
            files: vec!["src/a.mjs".into()],
            ..Default::default()
        }];
        let (entry, remote) = issue(merged(), 40_000);
        let workers = free_workers_of(&[entry], Some(&remote), &idle("c1", "idle"), &known, &[]);
        assert_eq!(
            (workers[0].name.as_str(), workers[0].share, workers[0].files.clone()),
            ("3-first", 0.2, vec!["src/a.mjs".to_string()])
        );
        let ready = Some(Mr {
            state: "opened".into(),
            pipeline: Some(Pipeline {
                status: "success".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        let (entry, remote) = issue(ready.clone(), 40_000);
        assert_eq!(
            free_workers_of(&[entry], Some(&remote), &idle("c1", "done"), &[], &[]).len(),
            1
        );
        let (entry, remote) = issue(merged(), 40_000);
        assert!(free_workers_of(&[entry], Some(&remote), &idle("c1", "working"), &[], &[]).is_empty());
        let draft = ready.map(|mr| Mr { is_draft: true, ..mr });
        let (entry, remote) = issue(draft, 40_000);
        assert!(free_workers_of(&[entry], Some(&remote), &idle("c1", "idle"), &[], &[]).is_empty());
        let (entry, remote) = issue(merged(), 150_000);
        assert!(free_workers_of(&[entry], Some(&remote), &idle("c1", "idle"), &[], &[]).is_empty());
    }
}
