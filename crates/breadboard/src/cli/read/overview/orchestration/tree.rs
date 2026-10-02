//! WORKTREES of an orchestration (tree.mjs): its worktrees as a folder tree
//! under the orchestration, each numbered, with where it stands in its life
//! (at work, waits for the pipeline, waits for you, merged and about to go)
//! in the colour of whom it waits for, its facts and title below; the queue,
//! then the ones done and removed. Its head counts them and names the free
//! memory.

use super::View;
use super::local::{Check, Item, Tree};
use super::merge::{RUNNING, checks_of, is_change_open};
use super::remote::Mr;
use super::workers::{Announced, Workers, agent_name_of, next_text, queued_text, status_of};
use crate::cli::read::overview::agents::{Claude, question_of, tokens};
use crate::cli::read::overview::clock::node_time;
use crate::cli::read::overview::paint::{bold, cyan, dim, green, link, pad, red, visible, wrap, yellow};
use crate::cli::read::overview::theme::{paint_tone, pink};
use breadboard_core::record::{ORCHESTRATOR, Worker};
use breadboard_core::time::{ago, parse_iso};
use std::collections::HashSet;

/// Past this many, "done and removed" is folded into its one line.
const FOLD_OVER: usize = 3;
/// What a worktree's own stack and an agent take: below it a fresh agent gets the shared stack (memory.mjs).
pub const THRESHOLD: f64 = 1.5 * 1024.0 * 1024.0 * 1024.0;
/// How long unpushed commits may sit before someone is needed (change.mjs).
const UNPUSHED_MS: i64 = 10 * 60_000;

/// Where a worktree stands: `key` what, `text` painted, `tone` whom it waits for.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub key: &'static str,
    pub text: String,
    pub tone: Option<&'static str>,
}

fn state(key: &'static str, text: String, tone: Option<&'static str>) -> State {
    State { key, text, tone }
}

/// `waits for !213, !217 · 5 merged` (wait.mjs `waitsText`).
pub fn waits_text(mrs: &[i64], merged: &HashSet<i64>) -> String {
    let open: Vec<String> = mrs
        .iter()
        .filter(|iid| !merged.contains(iid))
        .map(|iid| format!("!{iid}"))
        .collect();
    if open.is_empty() {
        return "waits: all merged, wakes up".into();
    }
    let done = mrs.len() - open.len();
    let done = if done > 0 {
        format!(" · {done} merged")
    } else {
        String::new()
    };
    format!("waits for {}{done}", open.join(", "))
}

/// What the agent of a worktree waits for, first match wins
/// (agent-state.mjs `agentStateOf`): `status` herdr's state of it, `handover`
/// the next issue announced for it.
pub fn agent_state_of(
    tree: &Tree,
    mr: Option<&Mr>,
    merged: &HashSet<i64>,
    checks: &[String],
    status: Option<&str>,
    handover: Option<&Announced>,
) -> State {
    if status == Some("working") {
        return state("working", "at work".into(), Some("work"));
    }
    let asks = tree
        .claude
        .as_ref()
        .and_then(|claude| claude.answer.as_deref())
        .and_then(question_of);
    if status == Some("blocked") || asks.is_some() {
        return state("asks", "asks you".into(), Some("you"));
    }
    let status = mr.and_then(Mr::status);
    let is_open = mr.is_some_and(Mr::is_open);
    let sha = mr
        .and_then(|mr| mr.pipeline.as_ref())
        .and_then(|pipeline| pipeline.sha.as_ref());
    let is_behind = matches!((sha, &tree.pushed), (Some(sha), Some(pushed)) if sha != pushed);
    if is_open && (status.is_some_and(|status| RUNNING.contains(&status)) || is_behind) {
        return state("pipeline", "waits for the pipeline".into(), Some("gitlab"));
    }
    if is_open && status == Some("failed") {
        return state("failed", "pipeline failed".into(), Some("you"));
    }
    let mut kinds: Vec<String> = Vec::new();
    for action in tree.actions.iter().filter(|action| action.answer.is_none()) {
        let kind = if action.is_try() {
            format!("try #{}", action.id)
        } else {
            action.kind.clone()
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    let is_green = is_open && status == Some("success") && !is_change_open(Some(tree), mr);
    if is_green && checks.is_empty() && !kinds.iter().any(|kind| kind == "merge") {
        kinds.push("merge".into());
    }
    if !kinds.is_empty() {
        return state("waits", format!("waits for you: {}", kinds.join(", ")), Some("you"));
    }
    if is_green {
        return state("checks", format!("checks open: {}", checks.join(", ")), Some("you"));
    }
    if let Some(mrs) = &tree.wait {
        return state("waitsForMr", waits_text(mrs, merged), Some("gitlab"));
    }
    if let Some(handover) = handover {
        return state("next", next_text(handover), Some("gitlab"));
    }
    if is_change_open(Some(tree), mr) {
        return state("change", "change requested".into(), Some("you"));
    }
    if tree.unpushed > 0 {
        let plural = if tree.unpushed == 1 { "" } else { "s" };
        return state(
            "unpushed",
            format!("not pushed: {} commit{plural}", tree.unpushed),
            Some("you"),
        );
    }
    state("free", "free".into(), Some("free"))
}

/// The handover announced for the agent of `tree`.
fn handover_of<'a>(view: &View, workers: &'a Workers, tree: &Tree) -> Option<&'a Announced> {
    workers
        .announced
        .iter()
        .find(|handover| handover.tree.is_some_and(|at| view.trees[at].dir == tree.dir))
}

/// herdr's state of the agent of a worktree.
fn tree_status<'a>(view: &'a View, tree: &Tree) -> Option<&'a str> {
    tree.claude.as_ref().and_then(|claude| status_of(view, &claude.id))
}

/// Where a worktree stands in its life (tree.mjs `lifecycleOf`): merged
/// first, what is left then is the clean-up; else what its agent waits for.
fn lifecycle_of(view: &View, workers: &Workers, at: usize, mr: Option<&Mr>, merged: &HashSet<i64>) -> State {
    let item = &view.items[at];
    let Some(tree) = item.tree.map(|at| &view.trees[at]) else {
        return if item.entry.is_queued {
            state("queued", cyan(&queued_text(view, workers, at)), None)
        } else if let Some(why) = &item.entry.why {
            state("skipped", yellow(why), None)
        } else {
            state("removed", dim("removed"), None)
        };
    };
    let status = tree_status(view, tree);
    if tree.claude.is_none()
        && let Some(setup) = view
            .setups
            .iter()
            .find(|setup| setup.dir.as_deref() == Some(tree.dir.as_path()))
    {
        return state(
            "setup",
            cyan(&format!(
                "being set up for {} (wt:new) …, leave it",
                setup.to.as_deref().unwrap_or("a fresh agent")
            )),
            None,
        );
    }
    if mr.is_some_and(Mr::is_merged) {
        let mut kept = Vec::new();
        if tree.changed > 0 {
            kept.push(format!("{} changed", tree.changed));
        }
        if tree.own_commits > 0 {
            let plural = if tree.own_commits == 1 { "" } else { "s" };
            kept.push(format!("{} commit{plural} not in develop", tree.own_commits));
        }
        if !kept.is_empty() {
            return state("kept", yellow(&format!("kept: {}", kept.join(", "))), None);
        }
        let held = if !tree.left.is_empty() {
            Some(format!("{} still stands in it", tree.left.join(", ")))
        } else if status == Some("working") {
            Some("its agent is working".into())
        } else if status == Some("blocked") {
            Some("its agent asks you".into())
        } else if tree
            .actions
            .iter()
            .any(|action| action.on.as_deref() == Some("test") && action.answer.is_none())
        {
            Some("its try on the test instance is open".into())
        } else {
            workers.holds.get(&tree.dir).cloned()
        };
        if let Some(why) = held {
            return state("held", yellow(&format!("waits: {why}")), None);
        }
        return state("removable", cyan("merged: will be removed"), None);
    }
    let boxes = view
        .found(item)
        .and_then(|found| found.issue.as_ref())
        .map(|issue| issue.boxes.as_slice());
    let checks = checks_of(boxes.unwrap_or_default(), Some(tree));
    let handover = handover_of(view, workers, tree);
    let agent = agent_state_of(tree, mr, merged, &checks, status, handover);
    // No agent and nothing to do: nobody stands in it.
    if agent.key == "free" && tree.claude.is_none() && status.is_none() {
        return state("none", dim("no agent"), None);
    }
    State {
        text: paint_tone(view.theme, agent.tone, &agent.text),
        ..agent
    }
}

/// An agent: how full its context is and when it last answered (tree.mjs `agentFact`).
fn agent_fact(claude: &Claude, now: i64) -> String {
    let Some(at) = claude.at else {
        return dim("no answer yet");
    };
    let share = claude.share();
    let percent = format!("{}%", (share * 100.0).round() as i64);
    let tone = if share >= 0.8 {
        red(&percent)
    } else if share >= 0.5 {
        yellow(&percent)
    } else {
        green(&percent)
    };
    format!(
        "{tone} {}",
        dim(&format!("of {} · {}", tokens(claude.window), ago(at, now)))
    )
}

/// How long the name before a handover is shown (workers.mjs `WAS_FOR`).
const WAS_FOR: i64 = 24 * 60 * 60 * 1000;

/// The agent of a worktree by its name when that isn't the worktree's (a
/// handover, #251), its name before for a day (tree.mjs `agentNameFact`).
fn agent_name_fact(view: &View, tree: &Tree) -> String {
    let Some(worker) = tree
        .claude
        .as_ref()
        .and_then(|claude| view.record.workers.iter().find(|worker| worker.id == claude.id))
    else {
        return String::new();
    };
    let was = worker.was.as_ref().filter(|_| {
        worker
            .renamed
            .as_deref()
            .and_then(parse_iso)
            .is_some_and(|renamed| view.now - renamed < WAS_FOR)
    });
    let name = agent_name_of(view, tree);
    [
        (worker.name.is_some() && name != tree.name).then(|| format!("agent {}", bold(&name))),
        was.map(|was| dim(&format!("was {was}"))),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

/// `✓ tested 1a2b3c4d` / `untested: 2 commits` / … (check.mjs `checkFact`).
pub fn check_fact(check: &Check) -> String {
    match check {
        Check::Tested { sha } => green(&format!("✓ tested {}", sha.get(..8).unwrap_or(sha))),
        Check::Checking => cyan("checking…"),
        Check::Failed { what, is_test } => red(&format!(
            "✗ {} failed: {}",
            if *is_test { "tests" } else { "check" },
            if what.is_empty() { "see pnpm wt:check" } else { what }
        )),
        Check::Untested { count: 0 } => yellow("untested"),
        Check::Untested { count } => yellow(&format!(
            "untested: {count} commit{}",
            if *count == 1 { "" } else { "s" }
        )),
        Check::None => String::new(),
    }
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`.
pub fn ordinal(n: usize) -> String {
    let tens = n % 100;
    if (11..=13).contains(&tens) {
        return format!("{n}th");
    }
    let suffix = match n % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// An agent's issues in order: its `history`, else its `issues`.
fn history_of(worker: &Worker) -> Vec<(i64, Option<f64>)> {
    match &worker.history {
        Some(history) => history.iter().map(|start| (start.number, start.context)).collect(),
        None => worker.issues.iter().map(|number| (*number, None)).collect(),
    }
}

fn percent(share: f64) -> String {
    format!("{}%", (share * 100.0).round() as i64)
}

/// Which issue of its agent this is (history.mjs `historyText`): `agent:
/// 1st issue`, `agent: 2nd issue (#297 → #305 at 13%)`; "" when unknown.
pub fn history_text(workers: &[Worker], number: Option<i64>) -> String {
    let Some(number) = number else { return String::new() };
    let found = workers.iter().find_map(|worker| {
        let history = history_of(worker);
        let index = history.iter().position(|(each, _)| *each == number)?;
        Some((history, index))
    });
    let Some((history, index)) = found else {
        return String::new();
    };
    let place = format!("agent: {} issue", ordinal(index + 1));
    if history.len() == 1 {
        return place;
    }
    let chain: Vec<String> = history
        .iter()
        .enumerate()
        .map(|(at, (number, context))| match context {
            Some(context) if at > 0 => format!("#{number} at {}", percent(*context)),
            _ => format!("#{number}"),
        })
        .collect();
    format!("{place} ({})", chain.join(" → "))
}

/// How reuse works in the orchestration (history.mjs `reuseOf`, `reuseText`):
/// `agents 14 · issues 22 · reused 5, started at 34% · handovers failed 2`.
pub fn reuse_text(items: &[Item], workers: &[Worker]) -> String {
    let worked: Vec<&Item> = items
        .iter()
        .filter(|item| !item.entry.is_queued && item.entry.why.is_none())
        .collect();
    let histories: Vec<Vec<(i64, Option<f64>)>> = workers
        .iter()
        .map(history_of)
        .filter(|history| !history.is_empty())
        .collect();
    let covered: HashSet<i64> = histories.iter().flatten().map(|(number, _)| *number).collect();
    let later: Vec<&(i64, Option<f64>)> = histories.iter().flat_map(|history| history.iter().skip(1)).collect();
    let contexts: Vec<f64> = later.iter().filter_map(|(_, context)| *context).collect();
    let agents = histories.len()
        + worked
            .iter()
            .filter(|item| !item.number().is_some_and(|number| covered.contains(&number)))
            .count();
    let context = if contexts.is_empty() {
        String::new()
    } else {
        format!(
            ", started at {}",
            percent(contexts.iter().sum::<f64>() / contexts.len() as f64)
        )
    };
    let failed: usize = workers.iter().map(|worker| worker.failed.len()).sum();
    format!(
        "agents {agents} · issues {} · reused {}{context} · handovers failed {failed}",
        worked.len(),
        later.len()
    )
}

/// The number of each issue, 1 up (numbering.mjs `numbersOf`): the work
/// order first, then the open MRs in the merge order, then the ones without
/// an MR by number, then the merged and removed by their merge time.
fn numbers_of(view: &View, work: &[i64], merge: &[i64]) -> Vec<usize> {
    let rank = |order: &[i64], item: &Item| {
        item.number()
            .and_then(|number| order.iter().position(|each| *each == number))
            .unwrap_or(usize::MAX)
    };
    let merged_at = |item: &Item| -> Option<String> {
        item.entry
            .merged_at
            .clone()
            .or_else(|| {
                view.mr(item)
                    .filter(|mr| mr.is_merged())
                    .map(|mr| mr.merged_at.clone().unwrap_or_default())
            })
            .or_else(|| item.entry.removed_at.clone())
    };
    let by_number = |at: &usize| view.items[*at].number().unwrap_or(i64::MAX);
    let all: Vec<usize> = (0..view.items.len()).collect();
    let mut in_work: Vec<usize> = all
        .iter()
        .copied()
        .filter(|at| rank(work, &view.items[*at]) != usize::MAX)
        .collect();
    in_work.sort_by_key(|at| rank(work, &view.items[*at]));
    let rest: Vec<usize> = all.into_iter().filter(|at| !in_work.contains(at)).collect();
    let (mut done, live): (Vec<usize>, Vec<usize>) =
        rest.into_iter().partition(|at| merged_at(&view.items[*at]).is_some());
    let mut in_merge: Vec<usize> = live
        .iter()
        .copied()
        .filter(|at| rank(merge, &view.items[*at]) != usize::MAX)
        .collect();
    in_merge.sort_by_key(|at| rank(merge, &view.items[*at]));
    let mut others: Vec<usize> = live
        .into_iter()
        .filter(|at| rank(merge, &view.items[*at]) == usize::MAX)
        .collect();
    others.sort_by_key(by_number);
    done.sort_by(|a, b| {
        merged_at(&view.items[*a])
            .cmp(&merged_at(&view.items[*b]))
            .then(by_number(a).cmp(&by_number(b)))
    });
    let mut numbers = vec![0; view.items.len()];
    for (place, at) in in_work
        .into_iter()
        .chain(in_merge)
        .chain(others)
        .chain(done)
        .enumerate()
    {
        numbers[at] = place + 1;
    }
    numbers
}

/// A node of the tree.
struct Node {
    item: usize,
    state: State,
    number: usize,
    name: String,
    facts: String,
    title: String,
}

/// The facts line of a node (tree.mjs `factsOf`): issue, MR and pipeline,
/// work, whether its HEAD is tested, which issue of its agent this is, a
/// wait its state doesn't name, a requested change, what isn't pushed.
fn facts_of(
    view: &View,
    workers: &Workers,
    item: &Item,
    mr: Option<&Mr>,
    state: &State,
    merged: &HashSet<i64>,
) -> String {
    let tree = item.tree.map(|at| &view.trees[at]);
    let found = view.found(item).and_then(|found| found.issue.as_ref());
    let mut facts: Vec<String> = Vec::new();
    if let Some(number) = item.number() {
        let issue_state = match found {
            Some(found) if found.state == "closed" => cyan("closed"),
            Some(_) => "open".into(),
            None => dim(if view.remote.is_some() { "?" } else { "…" }),
        };
        facts.push(format!(
            "{} {issue_state}",
            link(found.and_then(|found| found.url.as_deref()), &format!("#{number}"))
        ));
    }
    let merged_at = item
        .entry
        .merged_at
        .clone()
        .or_else(|| mr.and_then(|mr| mr.merged_at.clone()));
    let mr_fact = match mr {
        None => merged_at
            .as_deref()
            .and_then(parse_iso)
            .map(|at| dim(&format!("merged {}", node_time(at, view.now))))
            .unwrap_or_default(),
        Some(mr) => {
            let state = match mr.state.as_str() {
                "merged" => cyan(" merged"),
                "closed" => dim(" closed"),
                _ if mr.is_draft => dim(" draft"),
                _ => green(" ready"),
            };
            let pipeline = if mr.is_open() {
                format!(" {}", pipeline_mark(mr))
            } else {
                String::new()
            };
            let when = match merged_at.as_deref().and_then(parse_iso) {
                Some(at) if mr.is_merged() => format!(" {}", dim(&node_time(at, view.now))),
                _ => String::new(),
            };
            format!(
                "{}{state}{pipeline}{when}",
                link(mr.url.as_deref(), &format!("!{}", mr.iid))
            )
        }
    };
    if tree.is_some() && mr_fact.is_empty() {
        // Without GitLab's answer nothing says there is none: unknown, as the issue's `…`.
        facts.push(dim(if view.remote.is_some() { "no MR yet" } else { "MR …" }));
    } else {
        facts.push(mr_fact);
    }
    if let Some(tree) = tree {
        let work: Vec<String> = [
            (tree.ahead > 0).then(|| format!("{} ahead", tree.ahead)),
            (tree.changed > 0).then(|| yellow(&format!("{} changed", tree.changed))),
        ]
        .into_iter()
        .flatten()
        .collect();
        facts.push(work.join(" · "));
        facts.push(check_fact(&tree.check));
        facts.push(agent_name_fact(view, tree));
    }
    facts.push(dim(&history_text(&view.record.workers, item.number())));
    if let Some(tree) = tree {
        if let Some(claude) = &tree.claude {
            facts.push(agent_fact(claude, view.now));
        }
        if let Some(mrs) = &tree.wait
            && state.key != "waitsForMr"
        {
            facts.push(yellow(&waits_text(mrs, merged)));
        }
        if is_change_open(Some(tree), mr)
            && let Some(change) = &tree.change
        {
            facts.push(yellow(&format!("change requested: {}", change.text)));
        }
        if tree.unpushed > 0 {
            let text = format!(
                "{} commit{} not pushed{}",
                tree.unpushed,
                if tree.unpushed == 1 { "" } else { "s" },
                if tree.has_remote_branch {
                    ""
                } else {
                    " (no remote branch)"
                }
            );
            let is_long = tree.unpushed_at.is_some_and(|at| view.now - at >= UNPUSHED_MS);
            facts.push(if is_long { red(&text) } else { yellow(&text) });
        }
        if let Some(handover) = handover_of(view, workers, tree)
            && state.key != "next"
        {
            facts.push(cyan(&next_text(handover)));
        }
    }
    facts.retain(|fact| visible(fact) > 0);
    facts.join(" · ")
}

/// A pipeline as a mark, linked to it (boxes.mjs `pipelineMark`).
pub fn pipeline_mark(mr: &Mr) -> String {
    let Some(pipeline) = &mr.pipeline else {
        return dim("no pipeline");
    };
    let mark = match pipeline.status.as_str() {
        "success" => green("✓ passed"),
        "failed" => red("✗ failed"),
        "running" => yellow("◐ running"),
        "pending" => yellow("… pending"),
        "created" => yellow("… created"),
        "canceled" => dim("⊘ canceled"),
        "skipped" => dim("skipped"),
        "manual" => dim("▸ manual"),
        other => dim(other),
    };
    link(pipeline.url.as_deref(), &mark)
}

/// The lines of a node `width` wide (tree.mjs `nodeLines`): the branch glyph,
/// place and name with its state at the right end, then its facts and title
/// under the name, `stem` in front.
fn node_lines(node: &Node, count: usize, branch: &str, stem: &str, width: usize) -> Vec<String> {
    let digits = count.to_string().len();
    let place = dim(&format!("{:>digits$}. ", node.number));
    let head = format!("{}{place}", dim(branch));
    let room = width.saturating_sub(visible(&head));
    let indent = format!(
        "{}{}",
        dim(stem),
        " ".repeat(visible(&head).saturating_sub(visible(stem)))
    );
    let below = width.saturating_sub(visible(&indent)).max(1);
    let is_beside = visible(&node.name) + 2 + visible(&node.state.text) <= room;
    let mut lines = Vec::new();
    if is_beside {
        lines.push(format!(
            "{head}{}{}",
            pad(&node.name, room - visible(&node.state.text)),
            node.state.text
        ));
    } else {
        lines.push(format!("{head}{}", node.name));
        for line in wrap(&node.state.text, below) {
            lines.push(format!(
                "{indent}{}{line}",
                " ".repeat(below.saturating_sub(visible(&line)))
            ));
        }
    }
    for text in [&node.facts, &node.title] {
        if visible(text) > 0 {
            lines.extend(wrap(text, below).into_iter().map(|line| format!("{indent}{line}")));
        }
    }
    lines
}

/// `9.3 GB`.
pub fn gb(bytes: f64) -> String {
    format!("{:.1} GB", bytes / (1024.0 * 1024.0 * 1024.0))
}

/// The memory free for a new process: Linux's `MemAvailable`.
pub fn free_memory() -> Option<f64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|line| line.starts_with("MemAvailable:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024.0)
}

/// WORKTREES: the title and the lines inside its box, `width` wide.
pub fn worktrees(view: &View, workers: &Workers, width: usize) -> (String, Vec<String>) {
    let merged = view.merged_iids();
    let work = view
        .record
        .plan
        .as_ref()
        .map(|plan| plan.work.clone())
        .unwrap_or_default();
    let numbers = numbers_of(view, &work, &view.open_order());
    let nodes: Vec<Node> = view
        .items
        .iter()
        .enumerate()
        .map(|(at, item)| {
            let mr = view.mr(item);
            let lifecycle = lifecycle_of(view, workers, at, mr, &merged);
            let state = if lifecycle.key == "removed" && item.entry.removed_at.is_some() {
                let was_merged = item.entry.merged_at.is_some() || mr.is_some_and(Mr::is_merged);
                let when = item
                    .entry
                    .removed_at
                    .as_deref()
                    .and_then(parse_iso)
                    .map(|at| node_time(at, view.now))
                    .unwrap_or_default();
                State {
                    text: dim(&format!("{}removed {when}", if was_merged { "merged: " } else { "" })),
                    ..lifecycle
                }
            } else {
                lifecycle
            };
            let found = view.found(item).and_then(|found| found.issue.as_ref());
            let title = found
                .map(|found| found.title.clone())
                .or_else(|| item.entry.title.clone())
                .or_else(|| item.entry.branch.clone())
                .unwrap_or_default();
            Node {
                item: at,
                facts: facts_of(view, workers, item, mr, &state, &merged),
                state,
                number: numbers[at],
                name: pink(
                    view.theme,
                    &if item.tree.is_some() {
                        bold(&item.feature)
                    } else {
                        item.feature.clone()
                    },
                ),
                title: dim(&title),
            }
        })
        .collect();
    let (mut live, mut queued, mut removed) = (Vec::new(), Vec::new(), Vec::new());
    for node in nodes {
        match node.state.key {
            "removed" => removed.push(node),
            "queued" => queued.push(node),
            _ => live.push(node),
        }
    }
    live.sort_by_key(|node| node.number);
    let rank = |node: &Node| {
        view.items[node.item]
            .number()
            .and_then(|number| work.iter().position(|each| *each == number))
            .unwrap_or(usize::MAX)
    };
    queued.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(
                view.items[a.item]
                    .entry
                    .queued_at
                    .cmp(&view.items[b.item].entry.queued_at),
            )
            .then(a.number.cmp(&b.number))
    });
    removed.sort_by(|a, b| {
        view.items[b.item]
            .entry
            .removed_at
            .cmp(&view.items[a.item].entry.removed_at)
    });

    let count = view.items.len();
    let mut lines = vec![
        format!(
            "{}  {}",
            bold(&format!("{ORCHESTRATOR} {}", view.name)),
            dim(&format!(".claude/orchestrations/{}.json", view.name))
        ),
        dim(&format!(
            "{}  {}",
            if count > 0 { "│" } else { " " },
            reuse_text(&view.items, &view.record.workers)
        )),
    ];
    let is_last_live = |at: usize| at + 1 == live.len() && queued.is_empty() && removed.is_empty();
    for (at, node) in live.iter().enumerate() {
        let (branch, stem) = if is_last_live(at) {
            ("└─ ", "   ")
        } else {
            ("├─ ", "│  ")
        };
        lines.extend(node_lines(node, count, branch, stem, width));
    }
    if !queued.is_empty() {
        let is_last = removed.is_empty();
        lines.push(dim(&format!(
            "{}▾ queued ({}) · in the order they start",
            if is_last { "└─ " } else { "├─ " },
            queued.len()
        )));
        let level = if is_last { "   " } else { "│  " };
        for (at, node) in queued.iter().enumerate() {
            let is_end = at + 1 == queued.len();
            let branch = format!("{level}{}", if is_end { "└─ " } else { "├─ " });
            let stem = format!("{level}{}", if is_end { "   " } else { "│  " });
            lines.extend(node_lines(node, count, &branch, &stem, width));
        }
    }
    if !removed.is_empty() {
        let is_open = removed.len() <= FOLD_OVER;
        lines.push(dim(&format!(
            "└─ {} done and removed ({})",
            if is_open { "▾" } else { "▸" },
            removed.len()
        )));
        if is_open {
            for (at, node) in removed.iter().enumerate() {
                let is_end = at + 1 == removed.len();
                let branch = format!("   {}", if is_end { "└─ " } else { "├─ " });
                let stem = format!("   {}", if is_end { "   " } else { "│  " });
                lines.extend(node_lines(node, count, &branch, &stem, width));
            }
        }
    }

    let tone_count = |tone: &str| live.iter().filter(|node| node.state.tone == Some(tone)).count();
    let counts: Vec<String> = [
        (tone_count("work"), "at work"),
        (tone_count("gitlab"), "wait for GitLab"),
        (tone_count("you"), "wait for you"),
        (tone_count("free"), "free"),
        (
            live.iter().filter(|node| node.state.key == "removable").count(),
            "will be removed",
        ),
        (queued.len(), "queued"),
        (removed.len(), "removed"),
    ]
    .iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, what)| format!("{count} {what}"))
    .collect();
    let memory = free_memory().map(|free| {
        if free < THRESHOLD {
            red(&format!("{} free: fresh agents on the shared stack", gb(free)))
        } else {
            dim(&format!("{} free (shared stack below {})", gb(free), gb(THRESHOLD)))
        }
    });
    let head: Vec<String> = [(!counts.is_empty()).then(|| dim(&counts.join(" · "))), memory]
        .into_iter()
        .flatten()
        .collect();
    (format!("{} {}", bold("WORKTREES"), head.join(&dim(" · "))), lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::record::Start;

    fn worker(issues: &[(i64, Option<f64>)]) -> Worker {
        Worker {
            history: Some(
                issues
                    .iter()
                    .map(|(number, context)| Start {
                        number: *number,
                        how: "fresh".into(),
                        at: None,
                        context: *context,
                    })
                    .collect(),
            ),
            ..Worker::default()
        }
    }

    #[test]
    fn names_which_issue_of_its_agent_this_is() {
        let workers = vec![worker(&[(297, Some(0.0)), (305, Some(0.13))]), worker(&[(303, None)])];
        assert_eq!(
            history_text(&workers, Some(305)),
            "agent: 2nd issue (#297 → #305 at 13%)"
        );
        assert_eq!(history_text(&workers, Some(303)), "agent: 1st issue");
        assert_eq!(history_text(&workers, Some(1)), "");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(22), "22nd");
    }

    #[test]
    fn counts_agents_issues_and_reuse() {
        let workers = vec![worker(&[(297, Some(0.0)), (305, Some(0.13))]), worker(&[(303, None)])];
        let item = |number: i64| Item {
            entry: breadboard_core::record::IssueEntry {
                number: Some(number),
                ..Default::default()
            },
            ..Item::default()
        };
        let items = vec![item(297), item(305), item(303), item(242)];
        assert_eq!(
            reuse_text(&items, &workers),
            "agents 3 · issues 4 · reused 1, started at 13% · handovers failed 0"
        );
    }

    #[test]
    fn a_wait_names_the_mrs_still_open() {
        let merged: HashSet<i64> = [309, 310].into_iter().collect();
        assert_eq!(waits_text(&[309, 310, 303], &merged), "waits for !303 · 2 merged");
        assert_eq!(waits_text(&[309], &merged), "waits: all merged, wakes up");
    }
}
