//! WORKTREES of an orchestration (tree.mjs): its worktrees as a folder tree
//! under the orchestration, each with where it stands in its life, from what
//! the overview reads anyway: at work, waiting for you, ready, merged and
//! about to go, gone. Every node has its number; its name is in the pink of
//! herdr's sidebar (#227). A node reads like a folder listing: name and state,
//! its facts and title in the lines below (#223).

use super::agent_state::{AgentState, Facts, NextIssue, agent_state_of};
use super::auto_actions::Removal;
use super::change::{change_fact, unpushed_fact};
use super::clean_up::{CleanUp, clean_up_of};
use super::display_names::{Names, counts_of, names_of};
use super::handovers::{Announced, Known, mrs_text, next_text, worker_of};
use super::history::{history_of, reuse_of, reuse_text};
use super::memory::{THRESHOLD, gb};
use super::merge::checks_of;
use super::numbering::numbers_of;
use super::order::Go;
use super::workers::Suggestion;
use super::{Entry, Fact, Paint, Span, Tone, Tree, text_of};
use crate::check::{CheckState, failures_text};
use crate::conversations::Claude;
use crate::gitlab::{Issue, Mr, Pipeline, Remote};
use crate::herdr::Agent;
use crate::setups::Setup;
use crate::time::{ago, clock, parse_iso};
use crate::wait::waits_text;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Past this many, "done and removed" starts folded into its one line.
pub const FOLD_OVER: usize = 3;
/// How long WORKTREES names an agent's name before its last handover (`was <old>`).
pub const WAS_FOR: i64 = 24 * 60 * 60 * 1000;

/// Where a worktree stands in its life: the key of the case, its words in
/// their paint, and whom its agent waits for (#288) when that is what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lifecycle {
    pub key: &'static str,
    pub text: String,
    pub paint: Paint,
    pub tone: Option<Tone>,
}

fn lifecycle(key: &'static str, text: impl Into<String>, paint: Paint) -> Lifecycle {
    Lifecycle {
        key,
        text: text.into(),
        paint,
        tone: None,
    }
}

/// What lifecycleOf knows besides the issue.
#[derive(Debug, Clone, Default)]
pub struct LifeFacts<'a> {
    pub mr: Option<&'a Mr>,
    pub status: Option<&'a str>,
    /// Why a handover holds the agent.
    pub hold: Option<&'a str>,
    /// The automatic wt:remove of its worktree.
    pub removal: Option<&'a Removal>,
    pub merged: Option<&'a HashSet<i64>>,
    pub setup: Option<&'a Setup>,
    pub handover: Option<&'a NextIssue>,
    pub checks: &'a [String],
}

/// lifecycleOf: merged comes first, what is left then is the clean-up; else
/// what its agent waits for (agent_state.rs), `no agent` without one; gone,
/// skipped, queued without a worktree; `setup` while a start sets it up.
pub fn lifecycle_of(issue: &Entry, facts: &LifeFacts) -> Lifecycle {
    let Some(tree) = &issue.tree else {
        if issue.is_queued {
            return lifecycle("queued", "queued: waits for an agent", Paint::Cyan);
        }
        return match &issue.why {
            Some(why) => lifecycle("skipped", why.clone(), Paint::Yellow),
            None => lifecycle("removed", "removed", Paint::Dim),
        };
    };
    if tree.claude.is_none()
        && let Some(setup) = facts
            .setup
            .filter(|setup| setup.dir.as_deref().map(PathBuf::from).as_ref() == Some(&tree.dir))
    {
        let to = setup.to.as_deref().unwrap_or("a fresh agent");
        return lifecycle(
            "setup",
            format!("being set up for {to} (wt:new) …, leave it"),
            Paint::Cyan,
        );
    }
    match clean_up_of(Some(tree), facts.mr, facts.status, facts.hold) {
        Some(CleanUp::Kept(why)) => return lifecycle("kept", format!("kept: {why}"), Paint::Yellow),
        Some(CleanUp::Waits(why)) => return lifecycle("held", format!("waits: {why}"), Paint::Yellow),
        Some(CleanUp::Due) => {
            return match facts.removal {
                Some(Removal { error: Some(error), .. }) => {
                    lifecycle("removable", format!("merged: {error}"), Paint::Red)
                }
                Some(Removal { is_running: true, .. }) => lifecycle("removable", "merged: removing …", Paint::Cyan),
                _ => lifecycle("removable", "merged: will be removed", Paint::Cyan),
            };
        }
        None => {}
    }
    let AgentState { key, text, tone } = agent_state_of(
        tree,
        &Facts {
            mr: facts.mr,
            status: facts.status,
            merged: facts.merged,
            handover: facts.handover,
            checks: facts.checks,
        },
    );
    if key == "free" && tree.claude.is_none() && facts.status.is_none() {
        return lifecycle("none", "no agent", Paint::Dim);
    }
    Lifecycle {
        key,
        text,
        paint: Paint::Wait(tone),
        tone: Some(tone),
    }
}

/// A pipeline (or its status alone) as a mark, linked to the pipeline when its url is known.
pub fn pipeline_mark(pipeline: Option<&Pipeline>) -> Span {
    let status = pipeline
        .map(|pipeline| pipeline.status.as_str())
        .filter(|status| !status.is_empty());
    let (text, paint) = match status {
        Some("success") => ("✓ passed".to_string(), Paint::Green),
        Some("failed") => ("✗ failed".into(), Paint::Red),
        Some("running") => ("◐ running".into(), Paint::Yellow),
        Some("pending") => ("… pending".into(), Paint::Yellow),
        Some("created") => ("… created".into(), Paint::Yellow),
        Some("canceled") => ("⊘ canceled".into(), Paint::Dim),
        Some("skipped") => ("skipped".into(), Paint::Dim),
        Some("manual") => ("▸ manual".into(), Paint::Dim),
        Some(other) => (other.to_string(), Paint::Dim),
        None => ("no pipeline".into(), Paint::Dim),
    };
    Span {
        text,
        paint,
        url: pipeline.and_then(|pipeline| pipeline.web_url.clone()),
    }
}

/// Where an MR stands: ` merged`, ` closed`, ` draft`, ` ready` (render-worktree.mjs `mrState`).
pub fn mr_state(mr: &Mr) -> Span {
    match mr.state.as_str() {
        "merged" => Span::new(" merged", Paint::Cyan),
        "closed" => Span::new(" closed", Paint::Dim),
        _ if mr.is_draft => Span::new(" draft", Paint::Dim),
        _ => Span::new(" ready", Paint::Green),
    }
}

/// `200k`, `1M`: a number of tokens.
pub fn tokens(count: u64) -> String {
    if count >= 1_000_000 {
        let millions = (count as f64 / 100_000.0).round() / 10.0;
        if millions.fract() == 0.0 {
            format!("{}M", millions as u64)
        } else {
            format!("{millions}M")
        }
    } else {
        format!("{}k", (count as f64 / 1000.0).round() as u64)
    }
}

/// `✓ tested 1a2b3c4d` / `untested: 2 commits` / `checking…` / `✗ tests failed: …` (check.mjs `checkFact`).
pub fn check_fact(checked: &CheckState) -> Option<Span> {
    Some(match checked {
        CheckState::Tested { sha, .. } => Span::new(format!("✓ tested {}", sha.get(..8).unwrap_or(sha)), Paint::Green),
        CheckState::Checking { .. } => Span::new("checking…", Paint::Cyan),
        CheckState::Failed { failures, .. } => {
            let is_test = failures.iter().any(|failure| failure.task == "test");
            let what = failures_text(failures);
            let what = if what.is_empty() {
                "see pnpm wt:check".to_string()
            } else {
                what
            };
            Span::new(
                format!("✗ {} failed: {what}", if is_test { "tests" } else { "check" }),
                Paint::Red,
            )
        }
        CheckState::Untested { count } => Span::new(
            if *count > 0 {
                format!("untested: {}", super::counted(*count, "commit"))
            } else {
                "untested".into()
            },
            Paint::Yellow,
        ),
        CheckState::None => return None,
    })
}

/// How full an agent's context is and when it last answered.
pub fn agent_fact(claude: &Claude, now: i64) -> Fact {
    let Some(at) = claude.at else {
        return vec![Span::dim("no answer yet")];
    };
    let share = claude.share();
    let paint = if share >= 0.8 {
        Paint::Red
    } else if share >= 0.5 {
        Paint::Yellow
    } else {
        Paint::Green
    };
    vec![
        Span::new(format!("{}%", (share * 100.0).round() as i64), paint),
        Span::dim(format!(" of {} · {}", tokens(claude.window), ago(at, now))),
    ]
}

/// The name an agent had before its last handover, for a day.
pub fn was_of(known: &Known, id: &str, now: i64) -> Option<String> {
    let worker = worker_of(known, id)?;
    let renamed = worker.renamed.as_deref().and_then(parse_iso)?;
    worker.was.clone().filter(|_| now - renamed < WAS_FOR)
}

/// A node of WORKTREES.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Its place in `issues`.
    pub index: usize,
    pub number: Option<i64>,
    pub state: Lifecycle,
    /// Its number in front of its name, 1 up.
    pub place: usize,
    /// Its feature, bold while its worktree is there.
    pub name: String,
    pub has_tree: bool,
    /// Who works in it (#292), on the line under its name.
    pub agent: Option<AgentLine>,
    /// Its facts, parted by ` · `.
    pub facts: Vec<Fact>,
    pub title: String,
    /// A click: its space, else its issue's page.
    pub go: Go,
}

/// The agent of a node (#292, tree.mjs `agentLine`): `● Mira Albrecht · 3
/// issues · 2 MRs merged`, the dot in the colour of whom it waits for, else
/// of herdr's state.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentLine {
    pub name: String,
    pub dot: Paint,
    /// The issues it worked on, this one included.
    pub issues: usize,
    /// How many of their MRs are merged.
    pub merged: usize,
}

impl AgentLine {
    /// `· 3 issues · 2 MRs merged`.
    pub fn counts(&self) -> String {
        format!(
            "· {} · {} merged",
            super::counted(self.issues as u64, "issue"),
            super::counted(self.merged as u64, "MR")
        )
    }
}

/// What treeOf knows besides the issues.
#[derive(Debug, Clone, Default)]
pub struct TreeFacts<'a> {
    pub agents: Option<&'a HashMap<String, Agent>>,
    pub work: &'a [i64],
    /// The open MRs' issues in the computed merge order.
    pub merge: &'a [i64],
    pub announced: &'a [Announced],
    /// dir → why the clean-up waits for a handover.
    pub holds: Option<&'a HashMap<PathBuf, String>>,
    pub suggested: Option<&'a HashMap<i64, Suggestion>>,
    /// number → what the fresh start of a queued issue does.
    pub fresh: Option<&'a HashMap<i64, String>>,
    pub removals: Option<&'a HashMap<PathBuf, Removal>>,
    pub has_remote: bool,
    pub known: &'a Known,
    pub merged: Option<&'a HashSet<i64>>,
    pub setups: &'a [Setup],
    pub now: i64,
}

/// The tree of the issues: `live`, the worktrees there and the issues that
/// never got one; `queued`, in the order they start; `removed`, the last removed first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TreeOf {
    pub live: Vec<Node>,
    pub queued: Vec<Node>,
    pub removed: Vec<Node>,
}

/// `→ 242-overview after !256` / `→ 242-overview now`.
fn taken_text(to: &str, open: &[i64]) -> String {
    if open.is_empty() {
        format!("→ {to} now")
    } else {
        format!("→ {to} after {}", mrs_text(open))
    }
}

/// What a queued issue waits for (#250): the announced agent, the MRs it
/// starts after, why it is back, the free agent UP NEXT suggests, else what
/// its fresh start does.
fn queued_text(issue: &Entry, facts: &TreeFacts) -> String {
    if let Some(handover) = facts
        .announced
        .iter()
        .find(|handover| Some(handover.number) == issue.number)
    {
        return taken_text(&handover.to, &handover.open);
    }
    let open: Vec<i64> = issue
        .after
        .iter()
        .copied()
        .filter(|iid| !facts.merged.is_some_and(|merged| merged.contains(iid)))
        .collect();
    if !open.is_empty() {
        return format!("waits for {} to be merged", mrs_text(&open));
    }
    let suggested = issue
        .number
        .and_then(|number| facts.suggested.and_then(|suggested| suggested.get(&number)));
    let fresh = issue
        .number
        .and_then(|number| facts.fresh.and_then(|fresh| fresh.get(&number)));
    if let Some(requeued) = issue.requeued.as_deref().filter(|_| suggested.is_none()) {
        return format!(
            "back in the queue: {requeued}; {}",
            fresh.map(String::as_str).unwrap_or("waits for an agent")
        );
    }
    if let Some(found) = suggested {
        let worker = &found.worker;
        return if worker.is_merged {
            format!("→ {} (free, {} %)", worker.name, (worker.share * 100.0).round() as i64)
        } else {
            format!("→ {} once its MR is merged", worker.name)
        };
    }
    fresh.cloned().unwrap_or_else(|| "start: waits for an agent".into())
}

/// The facts of a node: issue, MR and pipeline, work, whether its HEAD is
/// tested, its agent's name after a handover, which issue of its agent this
/// is and its context, a wait its state doesn't name, a requested change,
/// what isn't pushed, the next issue announced for its agent.
fn facts_of(
    issue: &Entry,
    found: Option<&Issue>,
    mr: Option<&Mr>,
    state: &Lifecycle,
    handover: Option<&NextIssue>,
    facts: &TreeFacts,
) -> Vec<Fact> {
    let tree = issue.tree.as_ref();
    let now = facts.now;
    let mut all: Vec<Fact> = Vec::new();
    if let Some(number) = issue.number {
        let link = Span::link(format!("#{number}"), found.map(|found| found.url.as_str()));
        let state = match found {
            Some(found) if found.state == "closed" => Span::new(" closed", Paint::Cyan),
            Some(_) => Span::plain(" open"),
            None => Span::dim(if facts.has_remote { " ?" } else { " …" }),
        };
        all.push(vec![link, state]);
    }
    let merged_at = issue
        .merged_at
        .clone()
        .or_else(|| mr.and_then(|mr| mr.merged_at.clone()))
        .and_then(|at| parse_iso(&at));
    let mr_fact: Fact = match mr {
        None => merged_at
            .map(|at| vec![Span::dim(format!("merged {}", clock(at, now)))])
            .unwrap_or_default(),
        Some(mr) => {
            let mut fact = vec![Span::link(format!("!{}", mr.iid), Some(&mr.url)), mr_state(mr)];
            if mr.is_open() {
                fact.push(Span::plain(" "));
                fact.push(pipeline_mark(mr.pipeline.as_ref()));
            }
            if let Some(at) = merged_at.filter(|_| mr.is_merged()) {
                fact.push(Span::dim(format!(" {}", clock(at, now))));
            }
            fact
        }
    };
    if !mr_fact.is_empty() {
        all.push(mr_fact);
    } else if tree.is_some() {
        all.push(vec![Span::dim("no MR yet")]);
    }
    if let Some(tree) = tree {
        let mut work = Vec::new();
        if tree.ahead > 0 {
            work.push(Span::plain(format!("{} ahead", tree.ahead)));
        }
        if tree.changed > 0 {
            if !work.is_empty() {
                work.push(Span::plain(" · "));
            }
            work.push(Span::new(format!("{} changed", tree.changed), Paint::Yellow));
        }
        if !work.is_empty() {
            all.push(work);
        }
        if let Some(fact) = check_fact(&tree.checked) {
            all.push(vec![fact]);
        }
    }
    // The agent by its name when that isn't the worktree's (a handover, #251), its name before for a day.
    if let Some(claude) = tree.and_then(|tree| tree.claude.as_ref()) {
        let worker = worker_of(facts.known, &claude.id);
        let mut agent = Vec::new();
        if let Some(name) = worker
            .and_then(|worker| worker.name.as_deref())
            .filter(|name| Some(*name) != tree.map(|tree| tree.name.as_str()))
        {
            agent.push(Span::plain("agent "));
            agent.push(Span::new(name, Paint::Bold));
        }
        if let Some(was) = was_of(facts.known, &claude.id, now) {
            if !agent.is_empty() {
                agent.push(Span::plain(" "));
            }
            agent.push(Span::dim(format!("was {was}")));
        }
        if !agent.is_empty() {
            all.push(agent);
        }
    }
    if let Some(tree) = tree {
        if let Some(claude) = &tree.claude {
            all.push(agent_fact(claude, now));
        }
        if let Some(wait) = tree.wait.as_ref().filter(|_| state.key != "waitsForMr") {
            let none = HashSet::new();
            all.push(vec![Span::new(
                waits_text(wait, facts.merged.unwrap_or(&none)),
                Paint::Yellow,
            )]);
        }
        if let Some(fact) = change_fact(tree, mr) {
            all.push(vec![fact]);
        }
        if let Some(fact) = unpushed_fact(tree, now) {
            all.push(vec![fact]);
        }
    }
    if let Some(handover) = handover.filter(|_| state.key != "next") {
        all.push(vec![Span::new(next_text(handover.number, &handover.open), Paint::Cyan)]);
    }
    all
}

/// The handover announced for the agent of a worktree.
fn handover_of(tree: Option<&Tree>, announced: &[Announced]) -> Option<NextIssue> {
    let dir = &tree?.dir;
    let handover = announced
        .iter()
        .find(|handover| handover.agent.as_ref().and_then(|agent| agent.dir.as_ref()) == Some(dir))?;
    Some(NextIssue {
        number: handover.number,
        open: handover.open.clone(),
    })
}

/// The line of a node's agent, None without one.
fn agent_line(
    issue: &Entry,
    state: &Lifecycle,
    status: Option<&str>,
    names: &Names,
    known: &Known,
    is_merged: &dyn Fn(i64) -> bool,
) -> Option<AgentLine> {
    let tree = issue.tree.as_ref()?;
    // Without its conversation (not read yet, or gone), the record's worker
    // whose latest issue this is, unless it went free (#289): it is still
    // the one in this worktree.
    let id = match &tree.claude {
        Some(claude) => &claude.id,
        None => {
            let number = issue.number?;
            &known
                .iter()
                .filter(|worker| worker.free.is_none())
                .find(|worker| history_of(worker).last().is_some_and(|start| start.number == number))?
                .id
        }
    };
    let name = names.by_id.get(id)?.clone();
    let (issues, merged) = counts_of(known, id, issue.number, is_merged);
    let dot = match state.tone {
        Some(tone) => Paint::Wait(tone),
        None => status.map_or(Paint::Dim, |status| Paint::State(status.into())),
    };
    Some(AgentLine {
        name,
        dot,
        issues,
        merged,
    })
}

/// treeOf: each issue a node, numbered; `remote` its part of GitLab.
pub fn tree_of(issues: &[Entry], remote: Option<&Remote>, facts: &TreeFacts) -> TreeOf {
    let of = |issue: &Entry| remote.map(|remote| remote.of(&issue.branch));
    let numbers = numbers_of(issues, facts.work, facts.merge, |issue| {
        of(issue).and_then(|part| part.mr.as_ref())
    });
    let names = names_of(issues, facts.known);
    let is_merged = |number: i64| {
        issues
            .iter()
            .find(|issue| issue.number == Some(number))
            .is_some_and(|issue| {
                issue.merged_at.is_some() || of(issue).and_then(|part| part.mr.as_ref()).is_some_and(Mr::is_merged)
            })
    };
    let mut rows: Vec<Node> = issues
        .iter()
        .enumerate()
        .map(|(index, issue)| {
            let part = of(issue);
            let mr = part.and_then(|part| part.mr.as_ref());
            let found = part.and_then(|part| part.issue.as_ref());
            let tree = issue.tree.as_ref();
            let status = tree
                .and_then(|tree| tree.claude.as_ref())
                .and_then(|claude| facts.agents.and_then(|agents| agents.get(&claude.id)))
                .and_then(|agent| agent.status.as_deref());
            let handover = handover_of(tree, facts.announced);
            let boxes = found.map(|found| &found.boxes[..]).unwrap_or(&[]);
            let checks = tree
                .map(|tree| checks_of(boxes, &tree.actions, &tree.answered))
                .unwrap_or_else(|| checks_of(boxes, &[], &[]));
            let lifecycle = lifecycle_of(
                issue,
                &LifeFacts {
                    mr,
                    status,
                    hold: tree
                        .and_then(|tree| facts.holds.and_then(|holds| holds.get(&tree.dir)))
                        .map(String::as_str),
                    removal: tree.and_then(|tree| facts.removals.and_then(|removals| removals.get(&tree.dir))),
                    merged: facts.merged,
                    setup: tree.and_then(|tree| {
                        facts
                            .setups
                            .iter()
                            .find(|setup| setup.dir.as_deref().map(PathBuf::from).as_ref() == Some(&tree.dir))
                    }),
                    handover: handover.as_ref(),
                    checks: &checks,
                },
            );
            let state = match lifecycle.key {
                "queued" => Lifecycle {
                    text: queued_text(issue, facts),
                    ..lifecycle
                },
                "removed" if issue.removed_at.is_some() => {
                    let at = issue.removed_at.as_deref().and_then(parse_iso).unwrap_or(facts.now);
                    let merged = issue.merged_at.is_some() || mr.is_some_and(Mr::is_merged);
                    Lifecycle {
                        text: format!(
                            "{}removed {}",
                            if merged { "merged: " } else { "" },
                            clock(at, facts.now)
                        ),
                        ..lifecycle
                    }
                }
                _ => lifecycle,
            };
            let go = match (tree, found) {
                (Some(tree), _) => Go::FocusSpace(tree.dir.clone()),
                (None, Some(found)) => Go::Show {
                    url: found.url.clone(),
                    dir: None,
                },
                (None, None) => Go::Nothing,
            };
            let agent = agent_line(issue, &state, status, &names, facts.known, &is_merged);
            Node {
                index,
                number: issue.number,
                agent,
                facts: facts_of(issue, found, mr, &state, handover.as_ref(), facts),
                state,
                place: numbers[index],
                name: issue.feature.clone(),
                has_tree: tree.is_some(),
                title: found
                    .map(|found| found.title.clone())
                    .or_else(|| issue.title.clone())
                    .unwrap_or_else(|| issue.branch.clone()),
                go,
            }
        })
        .collect();
    let rank = |node: &Node| {
        node.number
            .and_then(|number| facts.work.iter().position(|known| *known == number))
            .unwrap_or(usize::MAX)
    };
    let (mut queued, rest): (Vec<Node>, Vec<Node>) = rows.drain(..).partition(|node| node.state.key == "queued");
    let (mut removed, mut live): (Vec<Node>, Vec<Node>) =
        rest.into_iter().partition(|node| node.state.key == "removed");
    live.sort_by_key(|node| node.place);
    queued.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| issues[a.index].queued_at.cmp(&issues[b.index].queued_at))
            .then_with(|| a.place.cmp(&b.place))
    });
    removed.sort_by(|a, b| issues[b.index].removed_at.cmp(&issues[a.index].removed_at));
    TreeOf { live, queued, removed }
}

/// The head line of WORKTREES (#260): how many agents are at work, wait for
/// GitLab, wait for you and are free, the ones about to go, queued and
/// removed, then the free memory, red below the threshold.
pub fn head_of(tree: &TreeOf, free: Option<u64>) -> Fact {
    let count = |tone: Tone| tree.live.iter().filter(|node| node.state.tone == Some(tone)).count();
    let counts: Vec<String> = [
        (count(Tone::Work), "at work"),
        (count(Tone::Gitlab), "wait for GitLab"),
        (count(Tone::You), "wait for you"),
        (count(Tone::Free), "free"),
        (
            tree.live.iter().filter(|node| node.state.key == "removable").count(),
            "will be removed",
        ),
        (tree.queued.len(), "queued"),
        (tree.removed.len(), "removed"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    let mut head = Vec::new();
    if !counts.is_empty() {
        head.push(Span::dim(counts.join(" · ")));
    }
    if let Some(free) = free {
        if !head.is_empty() {
            head.push(Span::dim(" · "));
        }
        head.push(if free < THRESHOLD {
            Span::new(
                format!("{} free: fresh agents on the shared stack", gb(free)),
                Paint::Red,
            )
        } else {
            Span::dim(format!("{} free (shared stack below {})", gb(free), gb(THRESHOLD)))
        });
    }
    head
}

/// WORKTREES: the orchestration at the root with how reuse works (#264), its
/// nodes, the queue, the gone ones; its head line counts them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Worktrees {
    /// `⇶ <name>` and its record's path.
    pub name: String,
    pub record: String,
    /// `agents 14 · issues 22 · reused 5, started at 34% · handovers failed 2`.
    pub reuse: String,
    pub tree: TreeOf,
    pub head: Fact,
    /// Whether "done and removed" starts folded: past FOLD_OVER.
    pub is_removed_folded: bool,
}

pub fn worktrees_of(
    name: &str,
    issues: &[Entry],
    remote: Option<&Remote>,
    facts: &TreeFacts,
    free: Option<u64>,
) -> Worktrees {
    let tree = tree_of(issues, remote, facts);
    Worktrees {
        name: name.to_string(),
        record: format!(".claude/orchestrations/{name}.json"),
        reuse: reuse_text(&reuse_of(issues, facts.known)),
        head: head_of(&tree, free),
        is_removed_folded: tree.removed.len() > FOLD_OVER,
        tree,
    }
}

/// The facts of a node as one line, as a person reads them.
pub fn facts_text(facts: &[Fact]) -> String {
    facts.iter().map(|fact| text_of(fact)).collect::<Vec<_>>().join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Action, Change};
    use crate::gitlab::BranchRemote;
    use crate::model::display_names::pick_name;
    use crate::model::handovers::tests::claude_of;
    use crate::record::{Start, Worker};
    use crate::wait::Wait;

    fn tree() -> Tree {
        Tree {
            dir: "/w".into(),
            claude: Some(claude_of("c1")),
            ..Default::default()
        }
    }
    fn issue(number: i64) -> Entry {
        Entry {
            number: Some(number),
            feature: format!("{number}-feature"),
            branch: format!("feature/{number}-feature"),
            tree: Some(tree()),
            ..Default::default()
        }
    }
    fn gone(number: i64) -> Entry {
        Entry {
            tree: None,
            ..issue(number)
        }
    }
    fn merged() -> Mr {
        Mr {
            state: "merged".into(),
            iid: 1,
            url: "u".into(),
            ..Default::default()
        }
    }
    fn green() -> Mr {
        Mr {
            iid: 9,
            state: "opened".into(),
            pipeline: Some(Pipeline {
                status: "success".into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn state_of(issue: &Entry, facts: &LifeFacts) -> Lifecycle {
        lifecycle_of(issue, facts)
    }

    #[test]
    fn waits_for_your_merge_only_once_no_check_is_open() {
        let mr = green();
        assert_eq!(
            state_of(
                &issue(1),
                &LifeFacts {
                    mr: Some(&mr),
                    status: Some("idle"),
                    ..Default::default()
                }
            )
            .text,
            "waits for you: merge"
        );
        let checks = vec!["try #1 broken".to_string()];
        let facts = LifeFacts {
            mr: Some(&mr),
            status: Some("idle"),
            checks: &checks,
            ..Default::default()
        };
        assert_eq!(state_of(&issue(1), &facts).text, "checks open: try #1 broken");
        let draft = Mr {
            is_draft: true,
            ..green()
        };
        let state = state_of(
            &issue(1),
            &LifeFacts {
                mr: Some(&draft),
                status: Some("idle"),
                ..Default::default()
            },
        );
        assert_eq!(
            (state.text.as_str(), state.tone),
            ("waits for you: merge", Some(Tone::You))
        );
    }

    #[test]
    fn tells_a_merged_worktree_the_clean_up_takes_from_one_it_keeps() {
        let mr = merged();
        let with = |tree: Tree, facts: LifeFacts| {
            state_of(
                &Entry {
                    tree: Some(tree),
                    ..issue(1)
                },
                &facts,
            )
        };
        assert_eq!(
            with(
                tree(),
                LifeFacts {
                    mr: Some(&mr),
                    ..Default::default()
                }
            )
            .key,
            "removable"
        );
        let kept = with(
            Tree { changed: 2, ..tree() },
            LifeFacts {
                mr: Some(&mr),
                ..Default::default()
            },
        );
        assert_eq!((kept.key, kept.text.as_str()), ("kept", "kept: 2 changed"));
        assert_eq!(
            with(
                Tree {
                    own_commits: 1,
                    ..tree()
                },
                LifeFacts {
                    mr: Some(&mr),
                    ..Default::default()
                }
            )
            .text,
            "kept: 1 commit not in develop"
        );
        assert_eq!(
            with(
                tree(),
                LifeFacts {
                    mr: Some(&mr),
                    status: Some("working"),
                    ..Default::default()
                }
            )
            .text,
            "waits: its agent is working"
        );
        let hold = LifeFacts {
            mr: Some(&mr),
            status: Some("idle"),
            hold: Some("its agent takes #226 after !232"),
            ..Default::default()
        };
        assert_eq!(with(tree(), hold).text, "waits: its agent takes #226 after !232");
        let removing = Removal {
            is_running: true,
            ..Default::default()
        };
        assert_eq!(
            with(
                tree(),
                LifeFacts {
                    mr: Some(&mr),
                    removal: Some(&removing),
                    ..Default::default()
                }
            )
            .text,
            "merged: removing …"
        );
    }

    #[test]
    fn says_what_the_agent_waits_for() {
        let text = |entry: &Entry, status: Option<&str>| {
            state_of(
                entry,
                &LifeFacts {
                    status,
                    ..Default::default()
                },
            )
            .text
        };
        assert_eq!(text(&issue(1), Some("blocked")), "asks you");
        assert_eq!(text(&issue(1), Some("working")), "at work");
        let actions = vec![
            Action {
                kind: "try".into(),
                id: 4,
                ..Default::default()
            },
            Action {
                kind: "merge".into(),
                ..Default::default()
            },
        ];
        let wait = Some(Wait {
            mrs: vec![271],
            ..Default::default()
        });
        let asked = Entry {
            tree: Some(Tree {
                actions,
                wait: wait.clone(),
                ..tree()
            }),
            ..issue(1)
        };
        assert_eq!(text(&asked, Some("idle")), "waits for you: try #4, merge");
        let waiting = Entry {
            tree: Some(Tree { wait, ..tree() }),
            ..issue(1)
        };
        assert_eq!(
            state_of(
                &waiting,
                &LifeFacts {
                    status: Some("idle"),
                    ..Default::default()
                }
            )
            .key,
            "waitsForMr"
        );
        assert_eq!(text(&waiting, Some("idle")), "waits for !271");
        assert_eq!(text(&issue(1), Some("idle")), "free");
        assert_eq!(
            text(
                &Entry {
                    tree: Some(Tree { claude: None, ..tree() }),
                    ..issue(1)
                },
                None
            ),
            "no agent"
        );
    }

    #[test]
    fn is_removed_without_a_worktree_skipped_with_a_reason() {
        assert_eq!(state_of(&gone(1), &LifeFacts::default()).key, "removed");
        let skipped = state_of(
            &Entry {
                why: Some("claimed".into()),
                ..gone(1)
            },
            &LifeFacts::default(),
        );
        assert_eq!((skipped.key, skipped.text.as_str()), ("skipped", "claimed"));
    }

    fn remote(parts: Vec<(&str, BranchRemote)>) -> Remote {
        Remote {
            of: parts
                .into_iter()
                .map(|(branch, part)| (branch.to_string(), part))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn shows_a_requested_change_and_what_isnt_pushed_in_the_facts() {
        let changed = Entry {
            tree: Some(Tree {
                change: Some(Change {
                    text: "less padding".into(),
                    head: Some("a".into()),
                    ..Default::default()
                }),
                unpushed: 2,
                ..tree()
            }),
            ..issue(1)
        };
        let mr = Mr {
            state: "opened".into(),
            iid: 5,
            url: "u".into(),
            is_draft: true,
            ..Default::default()
        };
        let gitlab = remote(vec![(
            "feature/1-feature",
            BranchRemote {
                mr: Some(mr),
                ..Default::default()
            },
        )]);
        let found = tree_of(
            &[changed],
            Some(&gitlab),
            &TreeFacts {
                has_remote: true,
                ..Default::default()
            },
        );
        let facts = facts_text(&found.live[0].facts);
        assert!(facts.contains("change requested: less padding"), "{facts}");
        assert!(facts.contains("2 commits not pushed (no remote branch)"), "{facts}");
    }

    #[test]
    fn names_the_wait_once() {
        let waiting = Entry {
            tree: Some(Tree {
                wait: Some(Wait {
                    mrs: vec![271],
                    ..Default::default()
                }),
                ..tree()
            }),
            ..issue(1)
        };
        let facts_at = |status: &str| {
            let agents: HashMap<String, Agent> = [(
                "c1".to_string(),
                Agent {
                    pane: "p".into(),
                    tab: "t".into(),
                    status: Some(status.into()),
                },
            )]
            .into();
            let found = tree_of(
                std::slice::from_ref(&waiting),
                None,
                &TreeFacts {
                    agents: Some(&agents),
                    ..Default::default()
                },
            );
            facts_text(&found.live[0].facts)
        };
        assert!(!facts_at("idle").contains("waits for !271"));
        assert!(facts_at("blocked").contains("waits for !271"));
    }

    #[test]
    fn names_the_agent_when_it_isnt_the_worktrees() {
        let renamed = "2026-09-30T10:00:00.000Z";
        let entry = Entry {
            tree: Some(Tree {
                name: "1-feature".into(),
                ..tree()
            }),
            ..issue(1)
        };
        let facts_of = |known: &[Worker], now: i64| {
            let found = tree_of(
                std::slice::from_ref(&entry),
                None,
                &TreeFacts {
                    known,
                    now,
                    ..Default::default()
                },
            );
            facts_text(&found.live[0].facts)
        };
        let now = parse_iso(renamed).unwrap() + 60_000;
        let same = [Worker {
            id: "c1".into(),
            name: Some("1-feature".into()),
            ..Default::default()
        }];
        assert!(!facts_of(&same, now).contains("agent 1-feature"));
        assert!(!facts_of(&[], now).contains("agent "));
        let known = [Worker {
            id: "c1".into(),
            name: Some("2-other".into()),
            was: Some("1-feature".into()),
            renamed: Some(renamed.into()),
            ..Default::default()
        }];
        assert!(facts_of(&known, now).contains("agent 2-other was 1-feature"));
        let later = facts_of(&known, now + 2 * 86_400_000);
        assert!(later.contains("agent 2-other") && !later.contains("was 1-feature"));
    }

    #[test]
    fn names_the_agent_of_a_node_with_its_issues_and_merged_mrs() {
        let fresh = Entry {
            tree: Some(Tree {
                name: "1-feature".into(),
                ..tree()
            }),
            ..issue(1)
        };
        let found = tree_of(&[fresh], None, &TreeFacts::default());
        let agent = found.live[0].agent.clone().unwrap();
        assert_eq!(agent.name, pick_name("c1", []));
        assert_eq!(agent.counts(), "· 1 issue · 0 MRs merged");
        assert!(!facts_text(&found.live[0].facts).contains("agent: "));
        let known = [Worker {
            id: "c1".into(),
            name: Some("3-third".into()),
            history: Some(vec![
                Start {
                    number: 1,
                    how: "fresh".into(),
                    at: None,
                    context: Some(0.0),
                },
                Start {
                    number: 3,
                    how: "handover".into(),
                    at: None,
                    context: Some(0.31),
                },
            ]),
            ..Default::default()
        }];
        let first = Entry {
            tree: Some(Tree {
                name: "1-feature".into(),
                claude: None,
                ..tree()
            }),
            merged_at: Some("2026-09-30T10:00:00Z".into()),
            ..issue(1)
        };
        let third = Entry {
            tree: Some(Tree {
                name: "3-third".into(),
                ..tree()
            }),
            ..issue(3)
        };
        let found = tree_of(
            &[first, third],
            None,
            &TreeFacts {
                known: &known,
                ..Default::default()
            },
        );
        let of = |number| found.live.iter().find(|node| node.number == Some(number)).unwrap();
        // Its agent moved on to #3: #1's worktree has none.
        assert_eq!(of(1).agent, None);
        let agent = of(3).agent.clone().unwrap();
        assert_eq!(agent.counts(), "· 2 issues · 1 MR merged");
        assert_eq!(agent.dot, Paint::Wait(Tone::Free));
        // No conversation read for #3's worktree: the record's worker still names it.
        let third = Entry {
            tree: Some(Tree {
                name: "3-third".into(),
                claude: None,
                ..tree()
            }),
            ..issue(3)
        };
        let found = tree_of(
            &[third],
            None,
            &TreeFacts {
                known: &known,
                ..Default::default()
            },
        );
        assert_eq!(found.live[0].agent.as_ref().map(|agent| agent.issues), Some(2));
    }

    #[test]
    fn puts_the_gone_ones_under_removed_the_last_removed_first() {
        let now = parse_iso("2026-09-30T13:00:00Z").unwrap();
        let found = tree_of(
            &[
                issue(1),
                Entry {
                    removed_at: Some("2026-09-30T10:00:00Z".into()),
                    ..gone(2)
                },
                Entry {
                    why: Some("failed at the start".into()),
                    ..gone(3)
                },
                Entry {
                    removed_at: Some("2026-09-30T12:00:00Z".into()),
                    ..gone(4)
                },
            ],
            None,
            &TreeFacts {
                now,
                ..Default::default()
            },
        );
        assert_eq!(
            found.live.iter().map(|node| node.number.unwrap()).collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(
            found
                .removed
                .iter()
                .map(|node| node.number.unwrap())
                .collect::<Vec<_>>(),
            [4, 2]
        );
        assert!(found.removed[0].state.text.starts_with("removed "));
    }

    #[test]
    fn queues_the_issues_without_an_agent() {
        use crate::model::workers::FreeWorker;
        let queued = |number: i64, at: &str| Entry {
            is_queued: true,
            queued_at: Some(at.into()),
            ..gone(number)
        };
        let announced = [Announced {
            number: 4,
            to: "2-feature".into(),
            after: vec![256],
            at: None,
            agent: None,
            open: vec![256],
            is_due: false,
            why: None,
        }];
        let suggested: HashMap<i64, Suggestion> = [(
            5,
            Suggestion {
                worker: FreeWorker {
                    name: "1-feature".into(),
                    ..Default::default()
                },
                shared: vec![],
            },
        )]
        .into();
        let found = tree_of(
            &[
                issue(1),
                queued(5, "2026-09-30T10:00:00Z"),
                queued(4, "2026-09-30T09:00:00Z"),
                queued(6, "2026-09-30T08:00:00Z"),
                queued(7, "2026-09-30T07:00:00Z"),
            ],
            None,
            &TreeFacts {
                work: &[1, 6],
                announced: &announced,
                suggested: Some(&suggested),
                ..Default::default()
            },
        );
        assert_eq!(found.live.len(), 1);
        let queue: Vec<(i64, String)> = found
            .queued
            .iter()
            .map(|node| (node.number.unwrap(), node.state.text.clone()))
            .collect();
        assert_eq!(
            queue,
            [
                (6, "start: waits for an agent".to_string()),
                (7, "start: waits for an agent".to_string()),
                (4, "→ 2-feature after !256".to_string()),
                (5, "→ 1-feature once its MR is merged".to_string()),
            ]
        );
    }

    #[test]
    fn says_what_a_queued_issue_waits_for() {
        let merged: HashSet<i64> = [283].into();
        let fresh: HashMap<i64, String> = [(269, "starts a fresh agent".to_string())].into();
        let found = tree_of(
            &[
                Entry {
                    is_queued: true,
                    after: vec![282, 283],
                    ..gone(267)
                },
                Entry {
                    is_queued: true,
                    requeued: Some("after the setup, 268-x no claude runs it".into()),
                    ..gone(269)
                },
            ],
            None,
            &TreeFacts {
                merged: Some(&merged),
                fresh: Some(&fresh),
                ..Default::default()
            },
        );
        assert_eq!(
            found
                .queued
                .iter()
                .map(|node| node.state.text.as_str())
                .collect::<Vec<_>>(),
            [
                "waits for !282 to be merged",
                "back in the queue: after the setup, 268-x no claude runs it; starts a fresh agent",
            ]
        );
    }

    #[test]
    fn shows_a_worktree_a_start_sets_up() {
        let made = Entry {
            tree: Some(Tree {
                dir: "/w-267".into(),
                claude: None,
                ..tree()
            }),
            ..issue(267)
        };
        let setup = Setup {
            number: 267,
            dir: Some("/w-267".into()),
            to: Some("268-x".into()),
            from: None,
            pid: 1,
            at: String::new(),
        };
        let found = tree_of(
            std::slice::from_ref(&made),
            None,
            &TreeFacts {
                setups: std::slice::from_ref(&setup),
                ..Default::default()
            },
        );
        assert_eq!(found.live[0].state.key, "setup");
        assert_eq!(found.live[0].state.text, "being set up for 268-x (wt:new) …, leave it");
        assert_eq!(lifecycle_of(&made, &LifeFacts::default()).text, "no agent");
    }

    #[test]
    fn numbers_every_node_the_work_order_first() {
        let found = tree_of(
            &[issue(7), issue(3), issue(9)],
            None,
            &TreeFacts {
                work: &[3, 7],
                ..Default::default()
            },
        );
        let named: Vec<String> = found
            .live
            .iter()
            .map(|node| format!("{}. {}", node.place, node.name))
            .collect();
        assert_eq!(named, ["1. 3-feature", "2. 7-feature", "3. 9-feature"]);
    }

    fn node(key: &'static str, tone: Option<Tone>) -> Node {
        Node {
            index: 0,
            number: None,
            state: Lifecycle {
                key,
                text: String::new(),
                paint: Paint::Plain,
                tone,
            },
            place: 0,
            name: String::new(),
            has_tree: true,
            agent: None,
            facts: vec![],
            title: String::new(),
            go: Go::Nothing,
        }
    }

    #[test]
    fn counts_the_agents_by_whom_they_wait_for_then_the_memory() {
        let tones = [
            Tone::Work,
            Tone::Gitlab,
            Tone::You,
            Tone::Gitlab,
            Tone::You,
            Tone::You,
            Tone::Free,
        ];
        let mut live: Vec<Node> = tones.iter().map(|tone| node("x", Some(*tone))).collect();
        live.push(node("removable", None));
        let tree = TreeOf {
            live,
            queued: vec![node("queued", None)],
            removed: (0..14).map(|_| node("removed", None)).collect(),
        };
        assert_eq!(
            text_of(&head_of(&tree, Some(THRESHOLD + super::super::memory::STACK))),
            "1 at work · 2 wait for GitLab · 3 wait for you · 1 free · 1 will be removed · 1 queued · 14 removed · 2.5 GB free (shared stack below 1.5 GB)"
        );
        let few = TreeOf {
            live: vec![node("x", Some(Tone::You))],
            ..Default::default()
        };
        assert_eq!(
            text_of(&head_of(&few, Some(super::super::memory::AGENT))),
            "1 wait for you · 0.5 GB free: fresh agents on the shared stack"
        );
        let free = TreeOf {
            live: vec![node("x", Some(Tone::Free))],
            ..Default::default()
        };
        assert_eq!(text_of(&head_of(&free, None)), "1 free");
    }

    #[test]
    fn gives_a_node_its_facts_and_title() {
        let found_issue = |number: i64| Issue {
            state: "opened".into(),
            url: "i".into(),
            title: format!("the title of {number}"),
            ..Default::default()
        };
        let gitlab = remote(vec![
            (
                "feature/1-feature",
                BranchRemote {
                    issue: Some(found_issue(1)),
                    ..Default::default()
                },
            ),
            (
                "feature/2-feature",
                BranchRemote {
                    issue: Some(found_issue(2)),
                    ..Default::default()
                },
            ),
        ]);
        let first = Entry {
            tree: Some(Tree {
                ahead: 2,
                claude: None,
                ..tree()
            }),
            ..issue(1)
        };
        let found = worktrees_of(
            "orch",
            &[first, issue(2)],
            Some(&gitlab),
            &TreeFacts {
                has_remote: true,
                ..Default::default()
            },
            None,
        );
        assert_eq!(found.reuse, "agents 2 · issues 2 · reused 0 · handovers failed 0");
        let [one, two] = &found.tree.live[..] else { panic!() };
        assert_eq!(
            (
                one.state.text.as_str(),
                facts_text(&one.facts).as_str(),
                one.title.as_str()
            ),
            ("no agent", "#1 open · no MR yet · 2 ahead", "the title of 1")
        );
        assert_eq!(two.state.text, "free");
        assert!(facts_text(&two.facts).starts_with("#2 open · no MR yet · no answer yet"));
        assert!(!found.is_removed_folded);
    }

    #[test]
    fn folds_a_long_done_and_removed() {
        let found = worktrees_of(
            "orch",
            &[issue(1), gone(2), gone(3), gone(4), gone(5)],
            None,
            &TreeFacts::default(),
            None,
        );
        assert!(found.is_removed_folded);
        assert_eq!(found.tree.removed.len(), 4);
    }
}
