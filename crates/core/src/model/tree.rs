//! WORKTREES of an orchestration (tree.mjs): its worktrees as a folder tree
//! under the orchestration, each with where it stands in its life, from what
//! the overview reads anyway: at work, waiting for you, ready, merged and
//! about to go, gone. Every node has its number; its name is in the pink of
//! herdr's sidebar (#227). A node reads like a folder listing: name and state,
//! its facts and title in the lines below (#223).

use super::agent_state::{AgentState, Facts, NextIssue, agent_state_of};
use super::auto_actions::Removal;
use super::boxes::{BoxSection, all_sections_of};
use super::change::{change_fact, unpushed_fact};
use super::clean_up::{CleanUp, clean_up_of};
use super::develop::Migrations;
use super::diff::Overlap;
use super::display_names::{Names, counts_of, names_of};
use super::everything::app_cell;
use super::handovers::{Announced, Known, mrs_text, next_text, worker_of};
use super::history::{history_of, reuse_of, reuse_text};
use super::memory::{THRESHOLD, gb};
use super::merge::checks_of;
use super::merge_turn::MergeTurn;
use super::numbering::numbers_of;
use super::order::Go;
use super::workers::{Suggestion, TRIES_AGAIN};
use super::{Docker, Entry, Fact, Paint, Span, Tone, Tree, text_of};
use crate::check::{CheckState, failures_text};
use crate::conversations::Claude;
use crate::gitlab::{Issue, Mr, Pipeline, Remote};
use crate::herdr::Agent;
use crate::said::{Now, now_of};
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
    /// Its MR's merge turn (merge_turn.rs): offered, waits for other MRs, lacks.
    pub turn: Option<&'a MergeTurn>,
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
            turn: facts.turn,
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

/// Where an MR stands: ` in epic` (#350), ` merged`, ` closed`, ` draft`, ` ready` (render-worktree.mjs `mrState`).
pub fn mr_state(mr: &Mr, issue: Option<&Issue>) -> Span {
    if crate::epic::is_in_epic(mr, issue) {
        return Span::new(" in epic", Paint::Cyan);
    }
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
                crate::config::in_repo("see pnpm wt:check")
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
    /// Its app and its stack (#332, render.mjs's columns): None without a worktree or its .env.local.
    pub app: Option<Span>,
    pub stack: Option<Span>,
    /// Whether docker answered for its stack (#331): `docker does not answer` is said once, over the tree.
    pub docker: Docker,
    /// Where its open work collides (#332, DIFF's `⚠ also changed …`):
    /// develop first (named `develop`), then the orchestration's other
    /// worktrees, each by its issue (`#304`), else its feature.
    pub collides: Vec<Overlap>,
    /// The files of its work and of the worktrees it collides with: a
    /// colliding file is named by the shortest end of its path none of the
    /// others shares, so `index.ts` says which one.
    pub near: Vec<String>,
    /// Its worktree, where a click on a colliding file opens it.
    pub dir: Option<PathBuf>,
    /// Its own stack can be started (#332, STACK's `▶ start the stack`):
    /// an .env.local and ports of its own, docker told it isn't all running.
    pub can_start_stack: bool,
    /// Whether a merge of develop would conflict (#332, DEVELOP's `git
    /// merge-tree`): the files, empty when it merges cleanly; None where
    /// nothing collides with develop or git can't tell.
    pub conflicts: Option<Vec<String>>,
    /// The migrations its database lacks (#332, STACK's line), where that
    /// database runs: its own stack all up, or the shared one.
    pub migrations: Option<Migrations>,
    /// It runs on the main checkout's stack.
    pub is_shared: bool,
    /// The port its app answers on, where it has an .env.local: its own, or the shared 3000.
    pub app_port: Option<i64>,
    /// When its migrations and docker's answer were asked (ms): what an
    /// act changed after them isn't in them yet.
    pub migrations_at: i64,
    pub docker_at: i64,
    pub title: String,
    /// Its issue's boxes one by one, as its fold lists them (#346); none without a worktree.
    pub boxes: Vec<BoxSection>,
    /// What is wrong or wanted (#341), from its issue's `## Goal` / `## Bug`;
    /// only where its worktree is.
    pub problem: Option<String>,
    /// What its agent is on now (#341): its note or its last answer's first line.
    pub now: Option<Now>,
    /// Its tries a checker answered or holds (#384).
    pub checked: Vec<crate::try_check::Checked>,
    /// A click: its space, else its issue's page.
    pub go: Go,
}

/// A worktree's stack as WORKTREES says it (#332): `stack running`, `stack
/// down`, `stack 2 running, 1 exited`; `stack …` while docker hasn't
/// answered, `stack ?` when it didn't (#331); `shared` on the main
/// checkout's. Its `+N` is left out: the app's port says it (`○ :3002`).
pub fn stack_of(tree: &Tree) -> Span {
    if !tree.has_own_port {
        return Span::dim("shared");
    }
    let Some(status) = tree.stack.as_deref().filter(|_| tree.docker == Docker::Answered) else {
        return match tree.docker {
            Docker::NotYet => Span::dim("stack …"),
            Docker::Silent => Span::new("stack ?", Paint::Yellow),
            Docker::Answered => Span::dim("stack down"),
        };
    };
    // compose's `exited(1), running(2)`: the running first; one state alone without its count.
    let mut states: Vec<(&str, &str)> = status
        .split(", ")
        .map(|part| part.trim_end_matches(')').split_once('(').unwrap_or((part, "")))
        .collect();
    states.sort_by_key(|(state, _)| *state != "running");
    let is_running = states.iter().all(|(state, _)| *state == "running");
    let text = match states.as_slice() {
        [(state, _)] => format!("stack {state}"),
        _ => format!(
            "stack {}",
            states
                .iter()
                .map(|(state, count)| format!("{count} {state}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    Span::new(text, if is_running { Paint::Green } else { Paint::Yellow })
}

/// Whether docker told every container of the worktree's own stack runs.
pub fn is_stack_running(tree: &Tree) -> bool {
    tree.docker == Docker::Answered
        && tree
            .stack
            .as_deref()
            .is_some_and(|status| status.split(", ").all(|part| part.starts_with("running")))
}

/// The agent of a node (#292, tree.mjs `agentLine`): `● Mira Albrecht · 3
/// issues · 2 merged`, the dot in the colour of whom it waits for, else
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
    /// `· 3 issues · 2 merged`: of its issues, how many MRs are merged.
    pub fn counts(&self) -> String {
        format!(
            "· {} · {} merged",
            super::counted(self.issues as u64, "issue"),
            self.merged
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
    /// The open MRs' turns by issue, None before GitLab answered.
    pub turns: Option<&'a super::orchestration::Turns>,
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
fn queued_text(issue: &Entry, facts: &TreeFacts, names: &Names) -> String {
    if let Some(handover) = facts
        .announced
        .iter()
        .find(|handover| Some(handover.number) == issue.number)
    {
        return taken_text(&names.shown(&handover.to), &handover.open);
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
        // Its handover failed (workers.rs puts why into `fresh`).
        if let Some(failed) = fresh {
            return format!("→ {}: {failed}", names.shown(&worker.name));
        }
        return if worker.is_merged {
            format!(
                "→ {} (free, {} %)",
                names.shown(&worker.name),
                (worker.share * 100.0).round() as i64
            )
        } else {
            format!("→ {} once its MR is merged", names.shown(&worker.name))
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
        // Open is what an issue of a worktree is: only the exception is said
        // (lazygit's marks), `?` / `…` while GitLab doesn't tell.
        let state = match found {
            Some(found) if found.state == "closed" => Some(Span::new(" closed", Paint::Cyan)),
            Some(_) => None,
            None => Some(Span::dim(if facts.has_remote { " ?" } else { " …" })),
        };
        all.push([link].into_iter().chain(state).collect());
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
            let mut fact = vec![Span::link(format!("!{}", mr.iid), Some(&mr.url)), mr_state(mr, found)];
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
        // Before GitLab answered nothing is known: `MR …`, as its issue's `#303 …`.
        all.push(vec![Span::dim(if facts.has_remote { "no MR yet" } else { "MR …" })]);
    }
    if let Some(tree) = tree {
        let mut work = Vec::new();
        // How far it is from develop (#332), as lazygit's branches: `↑1 ↓13`,
        // commits develop lacks and develop's it lacks; nothing to merge in once merged.
        if tree.ahead > 0 {
            work.push(Span::plain(format!("↑{}", tree.ahead)));
        }
        let is_merged = issue.merged_at.is_some() || mr.is_some_and(Mr::is_merged);
        if tree.behind > 0 && !is_merged {
            if !work.is_empty() {
                work.push(Span::plain(" "));
            }
            work.push(Span::new(format!("↓{}", tree.behind), Paint::Yellow));
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
    // The open work of each worktree, by its issue (`#304`, short beside a long feature), for where they collide.
    let works: Vec<(usize, String, &std::collections::BTreeSet<String>)> = issues
        .iter()
        .enumerate()
        .filter(|(_, issue)| {
            issue.merged_at.is_none() && !of(issue).and_then(|part| part.mr.as_ref()).is_some_and(Mr::is_merged)
        })
        .filter_map(|(index, issue)| {
            let name = issue
                .number
                .map_or_else(|| issue.feature.clone(), |number| format!("#{number}"));
            Some((index, name, &issue.tree.as_ref()?.files))
        })
        .collect();
    let collides_of = |index: usize| -> Vec<Overlap> {
        let Some((_, _, mine)) = works.iter().find(|(at, _, _)| *at == index) else {
            return Vec::new();
        };
        let tree = issues[index].tree.as_ref();
        let develop = tree.map(|tree| tree.on_base.clone()).unwrap_or_default();
        // Named after its base: develop, or its epic (#349).
        let develop = (!develop.is_empty()).then(|| Overlap {
            name: tree.map_or_else(|| crate::git::default_branch().into(), |tree| tree.base.local.clone()),
            files: develop,
        });
        let others = works
            .iter()
            .filter(|(at, _, _)| *at != index)
            .map(|(_, name, files)| Overlap {
                name: name.clone(),
                files: files.intersection(mine).cloned().collect(),
            })
            .filter(|other| !other.files.is_empty());
        develop.into_iter().chain(others).collect()
    };
    let near_of = |index: usize, collides: &[Overlap]| -> Vec<String> {
        let Some((_, _, mine)) = works.iter().find(|(at, _, _)| *at == index) else {
            return Vec::new();
        };
        let mut near: std::collections::BTreeSet<&String> = mine.iter().collect();
        for (_, name, files) in &works {
            if collides.iter().any(|overlap| &overlap.name == name) {
                near.extend(files.iter());
            }
        }
        near.into_iter().cloned().collect()
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
                    turn: issue
                        .number
                        .and_then(|number| facts.turns.and_then(|turns| turns.get(&number)))
                        .map(|(_, turn)| turn),
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
                "queued" => {
                    let text = queued_text(issue, facts, &names);
                    // Its start failed: no wait like the others, it needs a click.
                    let paint = if text.ends_with(TRIES_AGAIN) {
                        Paint::Red
                    } else {
                        lifecycle.paint
                    };
                    Lifecycle {
                        text,
                        paint,
                        ..lifecycle
                    }
                }
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
            let collides = collides_of(index);
            Node {
                index,
                number: issue.number,
                agent,
                facts: facts_of(issue, found, mr, &state, handover.as_ref(), facts),
                app: tree.filter(|tree| tree.has_env).map(app_cell),
                stack: tree.filter(|tree| tree.has_env).map(stack_of),
                docker: tree.map(|tree| tree.docker).unwrap_or_default(),
                near: near_of(index, &collides),
                collides,
                dir: tree.map(|tree| tree.dir.clone()),
                can_start_stack: tree.is_some_and(|tree| {
                    tree.has_env && tree.has_own_port && tree.docker == Docker::Answered && !is_stack_running(tree)
                }),
                conflicts: tree.and_then(|tree| tree.conflicts.clone()),
                migrations: tree
                    .filter(|tree| tree.has_env && (!tree.has_own_port || is_stack_running(tree)))
                    .and_then(|tree| tree.migrations.clone()),
                is_shared: tree.is_some_and(|tree| tree.has_env && !tree.has_own_port),
                app_port: tree
                    .filter(|tree| tree.has_env)
                    .map(|tree| tree.app_port + if tree.has_own_port { tree.offset } else { 0 }),
                migrations_at: tree.map_or(0, |tree| tree.migrations_at),
                docker_at: tree.map_or(0, |tree| tree.docker_at),
                state,
                place: numbers[index],
                name: issue.feature.clone(),
                has_tree: tree.is_some(),
                title: found
                    .map(|found| found.title.clone())
                    .or_else(|| issue.title.clone())
                    .unwrap_or_else(|| issue.branch.clone()),
                boxes: match (tree, found) {
                    (Some(tree), Some(found)) => all_sections_of(
                        &found.boxes,
                        mr.map(|mr| (mr.iid, &mr.boxes[..])),
                        &tree.actions,
                        &tree.answered,
                    ),
                    _ => Vec::new(),
                },
                problem: tree.and(found).and_then(|found| found.problem.clone()),
                checked: tree
                    .map(|tree| {
                        crate::try_check::checked_of(
                            &tree.dir,
                            &tree.checks,
                            &tree.actions,
                            &tree.answered,
                            mr.is_some_and(|mr| mr.is_open()),
                        )
                    })
                    .unwrap_or_default(),
                now: tree.and_then(|tree| {
                    let claude = tree.claude.as_ref();
                    now_of(
                        tree.note.as_ref(),
                        claude.and_then(|claude| claude.answer.as_deref()),
                        claude.and_then(|claude| claude.answer_at.or(claude.at)),
                    )
                }),
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
    let (mut removed, mut live): (Vec<Node>, Vec<Node>) = rest.into_iter().partition(is_gone);
    live.sort_by_key(|node| node.place);
    queued.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| issues[a.index].queued_at.cmp(&issues[b.index].queued_at))
            .then_with(|| a.place.cmp(&b.place))
    });
    // The one being removed first, where it stays once it is gone: the last removed first.
    let at = |node: &Node| (node.state.key != "removed", issues[node.index].removed_at.clone());
    removed.sort_by_key(|node| std::cmp::Reverse(at(node)));
    TreeOf { live, queued, removed }
}

/// Whether a node goes under "done and removed": gone, or merged with its
/// removal due and not failed. A due one is there from GitLab's first
/// answer, so the automatic wt:remove that follows changes no layout: the
/// node keeps its place and the fold its count (rule 1: the hover checks
/// read the screen once it was quiet for a second).
fn is_gone(node: &Node) -> bool {
    node.state.key == "removed" || (node.state.key == "removable" && node.state.paint != Paint::Red)
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
            tree.live
                .iter()
                .chain(&tree.removed)
                .filter(|node| node.state.key == "removable")
                .count(),
            "will be removed",
        ),
        (tree.queued.len(), "queued"),
        (
            tree.removed.iter().filter(|node| node.state.key == "removed").count(),
            "removed",
        ),
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
    use crate::model::display_names::{display_names_of, pick_name};
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
                    turn: Some(&MergeTurn::Offered),
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
                turn: Some(&MergeTurn::Offered),
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
    fn says_how_far_develop_moved_on_where_the_work_collides_its_app_and_stack() {
        let with = |number: i64, files: &[&str], on_develop: &[&str]| Entry {
            tree: Some(Tree {
                ahead: 1,
                behind: 3,
                files: files.iter().map(|file| file.to_string()).collect(),
                on_base: on_develop.iter().map(|file| file.to_string()).collect(),
                has_env: true,
                app_port: 3000,
                has_own_port: true,
                offset: number,
                stack: Some("running(5)".into()),
                ..tree()
            }),
            ..issue(number)
        };
        let issues = [
            with(1, &["a.md", "b.md"], &["b.md"]),
            with(2, &["a.md"], &[]),
            with(3, &["c.md"], &[]),
            // Merged: its work is in develop, nothing of it collides or is behind.
            Entry {
                merged_at: Some("2026-10-01T10:00:00Z".into()),
                ..with(4, &["a.md"], &["a.md"])
            },
        ];
        let found = tree_of(&issues, None, &TreeFacts::default());
        let node = |number: i64| found.live.iter().find(|node| node.number == Some(number)).unwrap();
        let collides = |number: i64| {
            node(number)
                .collides
                .iter()
                .map(|overlap| format!("{}: {}", overlap.name, overlap.files.join(", ")))
                .collect::<Vec<_>>()
        };
        assert_eq!(collides(1), ["develop: b.md", "#2: a.md"]);
        assert_eq!(collides(2), ["#1: a.md"]);
        assert!(collides(3).is_empty() && collides(4).is_empty());
        // Named against its own files and those of whom it collides with; c.md is no one's it meets.
        assert_eq!(node(1).near, ["a.md", "b.md"]);
        assert_eq!(node(3).near, ["c.md"]);
        assert!(facts_text(&node(1).facts).contains("↑1 ↓3"));
        assert!(!facts_text(&node(4).facts).contains('↓'));
        assert_eq!(node(3).app.as_ref().map(|app| app.text.as_str()), Some("○ :3003"));
        assert_eq!(
            node(3).stack.as_ref().map(|stack| stack.text.as_str()),
            Some("stack running")
        );
        assert_eq!(tree_of(&[gone(5)], None, &TreeFacts::default()).removed[0].app, None);
    }

    #[test]
    fn a_stack_says_its_states_in_words_and_never_down_while_docker_has_not_answered() {
        let own = |docker: Docker, stack: Option<&str>| Tree {
            has_env: true,
            app_port: 3000,
            has_own_port: true,
            offset: 2,
            docker,
            stack: stack.map(str::to_string),
            ..tree()
        };
        let cell = |tree: Tree| {
            let span = stack_of(&tree);
            (span.text, span.paint)
        };
        assert_eq!(cell(own(Docker::NotYet, None)), ("stack …".into(), Paint::Dim));
        assert_eq!(cell(own(Docker::Silent, None)), ("stack ?".into(), Paint::Yellow));
        assert_eq!(cell(own(Docker::Answered, None)), ("stack down".into(), Paint::Dim));
        assert_eq!(
            cell(own(Docker::Answered, Some("running(5)"))),
            ("stack running".into(), Paint::Green)
        );
        assert_eq!(
            cell(own(Docker::Answered, Some("exited(1), running(2)"))),
            ("stack 2 running, 1 exited".into(), Paint::Yellow)
        );
        assert_eq!(
            cell(own(Docker::Answered, Some("exited(3)"))),
            ("stack exited".into(), Paint::Yellow)
        );
        let shared = Tree {
            has_own_port: false,
            ..own(Docker::NotYet, None)
        };
        assert_eq!(cell(shared), ("shared".into(), Paint::Dim));
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
        assert_eq!(agent.counts(), "· 1 issue · 0 merged");
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
        assert_eq!(agent.counts(), "· 2 issues · 1 merged");
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
    fn a_queued_issue_names_its_agent_by_display_name_and_says_a_failed_handover() {
        use crate::model::workers::FreeWorker;
        let queued = |number: i64| Entry {
            is_queued: true,
            ..gone(number)
        };
        let known = [Worker {
            id: "c1".into(),
            name: Some("1-feature".into()),
            display_name: Some("Elif Kowalczyk".into()),
            ..Default::default()
        }];
        let announced = [Announced {
            number: 4,
            to: "1-feature".into(),
            after: vec![],
            at: None,
            agent: None,
            open: vec![256],
            is_due: false,
            why: None,
        }];
        let worker = FreeWorker {
            name: "1-feature".into(),
            is_merged: true,
            ..Default::default()
        };
        let suggested: HashMap<i64, Suggestion> = [5, 6]
            .map(|number| {
                (
                    number,
                    Suggestion {
                        worker: worker.clone(),
                        shared: vec![],
                    },
                )
            })
            .into();
        let fresh: HashMap<i64, String> = [(
            6,
            "its handover failed: wt:orchestrate: not built yet; a click tries again".to_string(),
        )]
        .into();
        let found = tree_of(
            &[issue(1), queued(4), queued(5), queued(6)],
            None,
            &TreeFacts {
                work: &[4, 5, 6],
                known: &known,
                announced: &announced,
                suggested: Some(&suggested),
                fresh: Some(&fresh),
                ..Default::default()
            },
        );
        let shown = display_names_of(&known, &["c1"])["c1"].clone();
        let queue: Vec<String> = found.queued.iter().map(|node| node.state.text.clone()).collect();
        assert_eq!(
            queue,
            [
                format!("→ {shown} after !256"),
                format!("→ {shown} (free, 0 %)"),
                format!("→ {shown}: its handover failed: wt:orchestrate: not built yet; a click tries again"),
            ]
        );
        // The failed one needs a click: red, the others wait in their own colour.
        assert_eq!(found.queued[2].state.paint, Paint::Red);
        assert_ne!(found.queued[1].state.paint, Paint::Red);
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
            app: None,
            stack: None,
            docker: Docker::Answered,
            collides: vec![],
            near: vec![],
            dir: None,
            can_start_stack: false,
            conflicts: None,
            migrations: None,
            is_shared: false,
            app_port: None,
            migrations_at: 0,
            docker_at: 0,
            title: String::new(),
            boxes: Vec::new(),
            problem: None,
            checked: Vec::new(),
            now: None,
            go: Go::Nothing,
        }
    }

    #[test]
    fn a_due_removal_waits_under_done_and_removed_a_failed_one_stays() {
        let due = node("removable", None);
        let failed = Node {
            state: Lifecycle {
                paint: Paint::Red,
                ..due.state.clone()
            },
            ..due.clone()
        };
        assert!(is_gone(&due) && is_gone(&node("removed", None)));
        assert!(!is_gone(&failed) && !is_gone(&node("x", Some(Tone::Work))));
        let tree = TreeOf {
            removed: vec![due, node("removed", None), node("removed", None)],
            ..Default::default()
        };
        assert_eq!(text_of(&head_of(&tree, None)), "1 will be removed · 2 removed");
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
            "1 at work · 2 wait for GitLab · 3 wait for you · 1 free · 1 will be removed · 1 queued · 14 removed · 2.0 GB free (shared stack below 1.0 GB)"
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
            ("no agent", "#1 · no MR yet · ↑2", "the title of 1")
        );
        assert_eq!(two.state.text, "free");
        assert!(facts_text(&two.facts).starts_with("#2 · no MR yet · no answer yet"));
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
