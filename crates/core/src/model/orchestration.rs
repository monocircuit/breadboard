//! The view of one orchestration (--orchestration, render-orchestration.mjs)
//! as data: NEXT, what the agents ask for (ACTIONS, UP NEXT), its worktrees as
//! a tree (WORKTREES), the free agents, the merge requests in the merge order
//! the overview computes, the merged below, and whether it can end. Each
//! section is derived on its own (#306): one that fails says so in its place.

use super::auto_actions::{AutoFacts, Removal, Tell, auto_actions_of, conflicts_of, failed_pipelines_of};
use super::boxes::{BoxSection, all_sections_of, open_sections_of};
use super::display_names::names_of;
use super::free_agents::{FreeRow, free_rows_of};
use super::handovers::announced_of;
use super::merge::{blockers_of, to_check_of};
use super::merge_order::{GraphRow, MergeEntry, MergeOrder, graph_of, merge_order_of};
use super::merge_turn::{MergeTurn, TurnRecord, merge_verdict_of};
use super::mr_blockers::{Progress, Reason, files_text, progress_of, row_blockers_of};
use super::next::{IssueState, Next, NextMerge, next_of, nothing_text, stale_of, updated_plan, why_of};
use super::orchestrator::{Orchestrator, hold_of};
use super::order::{Go, Item, MergeOffer, Target, order_actions, place_in};
use super::resolve::{Conflict, Resolves};
use super::tree::{TreeFacts, Worktrees, pipeline_mark, worktrees_of};
use super::try_state::{try_spot, try_state_of};
use super::workers::{StartRun, WorkerFacts, worker_actions_of};
use super::{Entry, Section, Span, section};
use crate::actions::Action;
use crate::gitlab::{BranchRemote, Mr, Remote};
use crate::herdr::Agent;
use crate::record::{ORCHESTRATOR, Plan, Record, Worker};
use crate::setups::Setup;
use crate::time::ago;
use crate::wait::{Awaited, Verdict, is_waiting, verdict_of};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Where the orchestrator's chat stands (#287, orchestrator-chat.mjs `chatStateOf`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatState {
    /// The orchestrator runs: a click focuses it.
    Alive,
    /// Something else runs in its tab.
    Busy,
    /// Its claude is gone, its tab idles.
    Idle,
    /// Its tab is gone.
    Missing,
    /// Its space is gone.
    Gone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub state: ChatState,
    pub session_id: Option<String>,
}

impl Chat {
    /// What the row says next to the button (chat.mjs `chatText`).
    pub fn text(&self) -> String {
        let what = match &self.session_id {
            Some(id) => format!("resumes {}…", id.get(..8).unwrap_or(id)),
            None => "starts a fresh claude, none is recorded".into(),
        };
        match self.state {
            ChatState::Alive => "the orchestrator runs · a click focuses it".into(),
            ChatState::Busy => "something else runs in its tab · a click focuses it".into(),
            ChatState::Idle => format!("its claude is gone · a click {what}"),
            ChatState::Missing => format!("its tab is gone · a click {what}"),
            ChatState::Gone => format!("its space is gone · a click opens it and {what}"),
        }
    }
}

/// What the local read of an orchestration found (view.mjs `readView`).
#[derive(Debug, Clone, Default)]
pub struct OrchestrationLocal {
    pub name: String,
    /// Its record's title, else its name.
    pub title: String,
    /// Its worktrees, the ones it started.
    pub trees: Vec<super::Tree>,
    /// Its record, None without one.
    pub record: Option<Record>,
    pub record_file: PathBuf,
    /// Its free agents (#289) with the share of their context used.
    pub free: Vec<(Worker, f64)>,
    pub free_memory: Option<u64>,
    /// breadboard.toml's `[agents] fresh_above` as a share (#381); None: the default.
    pub context_limit: Option<f64>,
    /// What herdr says each agent does, by conversation; empty outside herdr.
    pub agents: HashMap<String, Agent>,
    pub chat: Option<Chat>,
    pub setups: Vec<Setup>,
    /// The test instance's origin (`VITA_TEST_ORIGIN`).
    pub test_origin: Option<String>,
    /// The CI queue of the repo while a daemon keeps it (#357).
    pub ci: Option<crate::ci::Queue>,
    /// What the CI job watcher found while a daemon keeps it (#379).
    pub ci_watch: Option<crate::ci_watch::Report>,
    /// What the daemon's marks did to the repo's MRs (#399).
    pub ready: crate::ready::Marks,
    /// Its epic branch (#350), None for one that works on develop.
    pub epic: Option<super::epic_row::EpicLocal>,
    /// The share of its context the orchestrator's chat used (#382), None without one recorded.
    pub orchestrator_share: Option<f64>,
    /// The last lines of its journal (#382), oldest first.
    pub journal: Vec<String>,
    /// Its settings as they hold now (#415), the panel's rows.
    pub settings: Vec<crate::settings::Row>,
    /// When the orchestrator gets a fresh start (`[orchestrator]`).
    pub fresh: crate::config::Orchestrator,
    /// What its agents and the repo's spent (#372).
    pub spent: crate::spend::Spent,
    /// The runs paused (#397): `agents paused since 13:05 · 3 lines waiting`.
    pub paused: Option<String>,
}

/// What the writing side keeps between frames: the removals, starts and
/// lines it runs, the conflicts' clicks. Empty for a print.
#[derive(Debug, Clone, Default)]
pub struct ViewState {
    pub removals: HashMap<PathBuf, Removal>,
    pub runs: HashMap<i64, StartRun>,
    pub pipeline_tries: HashMap<String, String>,
    pub resolves: Resolves,
    /// The one live overview that delivers and acts by itself (not a print, not a second overview).
    pub is_deliverer: bool,
    /// The worktrees whose ended wait the overview tried to type and could
    /// not (its agent blocked, no pane): their `waits for` stays drawn until
    /// the line gets in, as Node's.
    pub undelivered_waits: HashSet<PathBuf>,
}

/// NEXT before GitLab's first answer: what is for you depends on it.
pub const GITLAB_PENDING: &str = "GitLab …";

/// NEXT: the answer to "what now?" (#278).
#[derive(Debug, Clone, PartialEq)]
pub enum NextBand {
    /// The first action not answered yet, as in ACTIONS.
    Action(Item),
    /// The first row of UP NEXT not under way.
    UpNext(Item),
    /// The merge that is ready.
    Merge(NextMerge),
    /// Nothing for the user: one dim line.
    Nothing(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NextSection {
    pub band: NextBand,
    /// The plan's why under a merge, never once the plan is stale.
    pub why: Option<String>,
}

/// ACTIONS and UP NEXT.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ActionsSection {
    pub actions: Vec<Item>,
    pub up_next: Vec<Item>,
}

/// A row of MERGE REQUESTS: an MR's or a line of the graph alone.
#[derive(Debug, Clone, PartialEq)]
pub struct MrRow {
    pub number: Option<i64>,
    /// Its lane mark (`●`, `│ ●`) and the lanes on its further lines.
    pub graph: String,
    pub below: String,
    pub mr: Option<MrFacts>,
}

/// An MR in its row.
#[derive(Debug, Clone, PartialEq)]
pub struct MrFacts {
    pub iid: i64,
    pub url: String,
    pub pipeline: Span,
    pub title: String,
    pub branch: String,
    pub dir: Option<PathBuf>,
    pub is_merged: bool,
    /// `merged 5 min ago`, `ready to merge`, None while it lacks something.
    pub status: Option<Span>,
    /// Where its conflicts stand: leads its second line; None for a merged one.
    pub conflict: Option<Conflict>,
    /// `conflicts with develop (README.md, view.rs)` while GitLab's settled
    /// answer is a conflict: the row says it red, its agent's (#416).
    pub conflicts: Option<String>,
    /// What it still lacks, who acts on it; its conflicts said by `conflict`.
    pub reasons: Vec<Reason>,
    /// Its issue's boxes, ticked of all (#294); None for a merged one or an issue without boxes.
    pub progress: Option<Progress>,
    /// Its place in the CI queue (#357) while its newest commit waits for a pipeline: its mark says it, a click runs it now.
    pub ci: Option<crate::ci::Place>,
    /// Its issue's boxes one by one, then its own description's, as its fold
    /// lists them (#346); none for a merged one.
    pub boxes: Vec<BoxSection>,
    /// The boxes that still hold its merge, shown under its row before any
    /// click (#354); none for a merged one.
    pub open_boxes: Vec<BoxSection>,
    /// The issue its bar counts the boxes of: the bar names it (#346).
    pub issue: Option<i64>,
    /// Every issue of its task whose boxes the bar counts (#321), `issue` first.
    pub issues: Vec<i64>,
    /// What its issue's agents spent on it (#372), None before they answered.
    pub spent: Option<crate::spend::Spend>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MergeRequests {
    /// The open ones as a graph in the merge order.
    pub open: Vec<MrRow>,
    /// The merged ones, then the closed, in the order they were merged.
    pub merged: Vec<MrRow>,
    /// `3 open · 40 merged · 1 without one yet · in merge order, side by side where independent`.
    pub counts: String,
}

/// The view of one orchestration.
#[derive(Debug, Clone, PartialEq)]
pub struct OrchestrationModel {
    pub name: String,
    /// `⇶ <title>`.
    pub label: String,
    /// Its auto mode (#384): a checker answers its tries; the model that does.
    pub is_auto: bool,
    pub checker: String,
    /// Its settings (#415): what the settings panel shows.
    pub settings: Vec<crate::settings::Row>,
    pub chat: Option<Chat>,
    /// No issues recorded: the view says how to start one, nothing else.
    pub is_empty: bool,
    /// Whether GitLab has answered yet.
    pub has_remote: bool,
    pub next: Section<NextSection>,
    pub actions: Section<ActionsSection>,
    pub worktrees: Section<Worktrees>,
    pub free_agents: Section<Vec<FreeRow>>,
    pub merge_requests: Section<MergeRequests>,
    /// What keeps the orchestration from ending: its worktrees and queued issues; none: it can end.
    pub left: Vec<String>,
    /// What the live overview does by itself now: hand-outs and fresh starts (workers.rs).
    pub orders: Vec<Go>,
    /// The lines it sends by itself: conflicts, failed pipelines (auto_actions.rs).
    pub tells: Vec<Tell>,
    /// Worktrees whose agent a hand-over takes now, and why: not removed meanwhile.
    pub holds: HashMap<PathBuf, String>,
    /// Why the plan is stale.
    pub stale: Vec<String>,
    /// The plan the live overview writes by itself while it is stale (#376):
    /// the computed merge order; None while the plan holds or GitLab hasn't answered.
    pub plan_update: Option<Plan>,
    /// The runner's load for the band (#357): `CI 3/4 slots · 2 queued`.
    pub ci_load: Option<String>,
    /// What the agents spent, for the band (#372): `today 4.2 M tokens · ≈ $12 · budget $80 of $200`.
    pub spent: Option<String>,
    /// Why no headless run starts (#372): `daily limit reached: $41 of $40`.
    pub held: Option<String>,
    /// The user paused every headless run (#397): `agents paused since 13:05 · 3 lines waiting`.
    pub paused: Option<String>,
    /// The Claude usage limit holding the headless runs (#393): `usage limit until 23:30`.
    pub usage: Option<String>,
    /// The CI job watcher's line under the band (#379): `CI: check:x holds pipelines up · …`.
    pub ci_watch: Option<String>,
    /// What the live overviews publish for the CI queue (#357); None until GitLab answered.
    pub ci_published: Option<crate::ci::Published>,
    /// Each worktree's merge turn for herdr's sidebar, which can't find it
    /// itself: the live overview writes them into the worktrees (#344).
    pub turn_records: Vec<(PathBuf, TurnRecord)>,
    /// The epic's head row (#350), None for one that works on develop.
    pub epic: Option<Vec<Vec<Span>>>,
    /// The orchestrator's context, quiet point and journal (#382).
    pub orchestrator: Orchestrator,
}

/// An action of ACTIONS: a try opens its spot, a merge merges on two clicks.
fn is_try(action: &Action) -> bool {
    action.is_try()
}

/// The MR and issue of an entry, empty without GitLab.
fn of<'a>(remote: Option<&'a Remote>, issue: &Entry) -> &'a BranchRemote {
    static NONE: std::sync::LazyLock<BranchRemote> = std::sync::LazyLock::new(BranchRemote::default);
    remote.map(|remote| remote.of(&issue.branch)).unwrap_or(&NONE)
}

/// Each open MR's row reasons and merge turn (merge_turn.rs), by its issue:
/// the one rule of what is offered and what waits (#344).
pub type Turns = HashMap<i64, (Vec<Reason>, MergeTurn)>;

/// The turn of an issue's MR; None without an open one (merged, none, GitLab not read yet).
fn turn_of<'a>(issue: &Entry, turns: &'a Turns) -> Option<&'a MergeTurn> {
    issue.number.and_then(|number| turns.get(&number)).map(|(_, turn)| turn)
}

/// Whether a merge shows in ACTIONS, posted or automatic: only while its
/// turn says offered: not once merged, nor while a try or a box is open
/// (#280), nor while it comes after another MR (#344).
fn is_merge_offered(issue: &Entry, turns: &Turns) -> bool {
    turn_of(issue, turns) == Some(&MergeTurn::Offered)
}

/// The turns of the worktrees whose open MR has a pipeline, tied to its commit.
fn turn_records_of(issues: &[Entry], remote: Option<&Remote>, turns: &Turns) -> Vec<(PathBuf, TurnRecord)> {
    issues
        .iter()
        .filter_map(|issue| {
            let tree = issue.tree.as_ref()?;
            let sha = of(remote, issue).mr.as_ref()?.pipeline.as_ref()?.sha.clone()?;
            let turn = turn_of(issue, turns)?.clone();
            Some((tree.dir.clone(), TurnRecord { sha, turn }))
        })
        .collect()
}

/// `develop red (check:vita)` while develop's last finished pipeline failed:
/// a merge now would land on a base that is already broken (#392).
fn develop_red_text(develop: Option<&crate::gitlab::DevelopPipeline>) -> Option<String> {
    let develop = develop.filter(|pipeline| pipeline.status == "failed")?;
    let jobs: Vec<&str> = develop
        .jobs
        .iter()
        .filter(|job| job.status == "failed")
        .map(|job| job.name.as_str())
        .take(2)
        .collect();
    Some(if jobs.is_empty() {
        "develop red".to_string()
    } else {
        format!("develop red ({})", jobs.join(", "))
    })
}

/// While develop is red no merge is offered: the turn that would be `Offered`
/// lacks it, and the row says why. The safety net of merging without a new
/// pipeline (#392); the overview's offer is held, not a merge by itself.
fn hold_while_develop_is_red(
    (mut reasons, turn): (Vec<Reason>, MergeTurn),
    develop: Option<&crate::gitlab::DevelopPipeline>,
) -> (Vec<Reason>, MergeTurn) {
    let Some(text) = develop_red_text(develop).filter(|_| turn == MergeTurn::Offered) else {
        return (reasons, turn);
    };
    for reason in reasons
        .iter_mut()
        .filter(|reason| reason.text == super::merge_turn::MARK_READY)
    {
        reason.who = "agent".into();
    }
    reasons.push(Reason {
        text,
        who: "develop".into(),
        is_conflict: false,
    });
    (reasons, MergeTurn::Lacks)
}

/// `push blocked: develop red (renders the row)`: its commits wait unpushed
/// because wt:check fails at its HEAD while develop's last pipeline failed
/// too (#416, #403): a test red on develop holds the push, and only a fix
/// on develop gets it out, not its agent.
fn push_blocked_text(tree: &super::Tree, develop: Option<&crate::gitlab::DevelopPipeline>) -> Option<String> {
    develop.filter(|pipeline| pipeline.status == "failed")?;
    let crate::check::CheckState::Failed { failures, .. } = &tree.checked else {
        return None;
    };
    if tree.unpushed == 0 {
        return None;
    }
    let test = failures
        .iter()
        .find_map(|failure| failure.tests.first().cloned())
        .or_else(|| {
            failures
                .first()
                .map(|failure| format!("{} {}", failure.package, failure.task).trim().to_string())
        })
        .filter(|test| !test.is_empty());
    Some(match test {
        Some(test) => format!("push blocked: develop red ({test})"),
        None => "push blocked: develop red".into(),
    })
}

/// Who gets a push out that a red develop holds: a fix of develop of its own (#403).
pub const DEVELOP_FIX: &str = "a hotfix on develop";

/// Its unpushed commits as what holds them, where develop's red test does.
fn hold_push_while_develop_is_red(
    (mut reasons, turn): (Vec<Reason>, MergeTurn),
    tree: Option<&super::Tree>,
    develop: Option<&crate::gitlab::DevelopPipeline>,
) -> (Vec<Reason>, MergeTurn) {
    if let Some(text) = tree.and_then(|tree| push_blocked_text(tree, develop))
        && let Some(reason) = reasons.iter_mut().find(|reason| reason.text.ends_with(" not pushed"))
    {
        reason.text = text;
        reason.who = DEVELOP_FIX.into();
    }
    (reasons, turn)
}

/// The turns of the open MRs, from their row's reasons.
fn turns_of(issues: &[Entry], remote: Option<&Remote>, order: &MergeOrder, done: &HashSet<i64>) -> Turns {
    let Some(remote) = remote else { return Turns::new() };
    let after_of: HashMap<i64, &[super::merge_order::After]> =
        order.open.iter().map(|item| (item.number, &item.after[..])).collect();
    let mut turns = Turns::new();
    for issue in issues {
        let Some(number) = issue.number else { continue };
        let part = remote.of(&issue.branch);
        let Some(mr) = part.mr.as_ref().filter(|mr| mr.is_open()) else {
            continue;
        };
        let tree = issue.tree.as_ref();
        let verdict = merge_verdict_of(
            mr,
            &part.boxes(),
            tree.map(|tree| tree.behind).unwrap_or(0),
            after_of.get(&number).copied().unwrap_or(&[]),
            &part.jobs,
            tree,
            done,
        );
        // Only a merge onto develop waits for it: a task into its epic does not.
        let develop = remote
            .develop
            .as_ref()
            .filter(|_| mr.target() == crate::git::default_branch());
        let verdict = hold_push_while_develop_is_red(verdict, tree, remote.develop.as_ref());
        turns.insert(number, hold_while_develop_is_red(verdict, develop));
    }
    turns
}

/// What a merge row offers: its MR, what its two clicks need.
fn merge_offer(mr: &Mr, part: &BranchRemote, issue: &Entry, can_mark_ready: bool) -> MergeOffer {
    let boxes = &part.boxes()[..];
    let (actions, answered) = issue
        .tree
        .as_ref()
        .map(|tree| (&tree.actions[..], &tree.answered[..]))
        .unwrap_or((&[], &[]));
    MergeOffer {
        iid: mr.iid,
        url: mr.url.clone(),
        is_draft: mr.is_draft,
        can_mark_ready,
        blockers: blockers_of(Some(mr), boxes, can_mark_ready, actions, answered),
        to_check: to_check_of(boxes, actions),
    }
}

/// Each issue's state for the plan's staleness.
fn states_of(issues: &[Entry], remote: Option<&Remote>) -> HashMap<i64, IssueState> {
    issues
        .iter()
        .filter_map(|issue| {
            let part = of(remote, issue);
            let state = if part.mr.as_ref().is_some_and(Mr::is_merged) {
                IssueState::Merged
            } else if part.issue.as_ref().is_some_and(|found| found.state == "closed") {
                IssueState::Closed
            } else {
                IssueState::Open
            };
            Some((issue.number?, state))
        })
        .collect()
}

/// The page of an action's url: a path on the app on `port`, anything else as it is.
fn page_of(url: &str, port: i64) -> String {
    if url.starts_with('/') {
        format!("http://localhost:{port}{url}")
    } else {
        url.to_string()
    }
}

/// Every open action, posted and automatic, split into ACTIONS and UP NEXT.
#[allow(clippy::too_many_arguments)]
fn actions_of(
    local: &OrchestrationLocal,
    issues: &[Entry],
    remote: Option<&Remote>,
    order: &MergeOrder,
    turns: &Turns,
    workers: &super::workers::WorkerActions,
    state: &ViewState,
    now: i64,
) -> ActionsSection {
    // The orchestrator's pages are on the main checkout's app: its port is any worktree's (one config).
    let main_port = local
        .trees
        .first()
        .map(|tree| tree.app_port)
        .or_else(|| crate::stack::app_port(&std::env::current_dir().unwrap_or_default()))
        .unwrap_or(0);
    let merge = &order.numbers;
    let mut items: Vec<Item> = Vec::new();
    // The orchestrator's own (`wt:action --orchestration`); a merge it posted for an MR (`--mr !N`) is that MR's.
    for action in local.record.as_ref().map(|record| &record.actions[..]).unwrap_or(&[]) {
        let issue = action.mr.filter(|_| action.kind == "merge").and_then(|iid| {
            issues
                .iter()
                .find(|issue| of(remote, issue).mr.as_ref().is_some_and(|mr| mr.iid == iid))
        });
        if issue.is_some_and(|issue| !is_merge_offered(issue, turns)) {
            continue;
        }
        let file = local.record_file.clone();
        let mut item = Item {
            key: format!("{}#{}", file.display(), action.id),
            kind: action.kind.clone(),
            text: action.text.clone(),
            at: action.at.clone(),
            action: Some(action.clone()),
            who: format!("{ORCHESTRATOR} orchestrator"),
            rank: -1,
            number: issue.and_then(|issue| issue.number),
            branch: issue.map(|issue| issue.branch.clone()),
            file: Some(file.clone()),
            to: Some(Target::Orchestration(local.name.clone())),
            go: match (issue, &action.url) {
                (Some(issue), _) => Go::Merge {
                    branch: issue.branch.clone(),
                    action: Some((file.clone(), action.id)),
                },
                (None, Some(url)) => Go::Show {
                    url: page_of(url, main_port),
                    dir: None,
                },
                (None, None) => Go::Nothing,
            },
            ..Default::default()
        };
        if let Some(issue) = issue
            && let Some(mr) = &of(remote, issue).mr
        {
            item.merge = Some(merge_offer(mr, of(remote, issue), issue, true));
        }
        if is_try(action) {
            item.text = format!("{}  {}", action.text, try_spot(action, None));
        }
        items.push(item);
    }
    // The agents' own (`wt:action`).
    for issue in issues {
        let Some(tree) = &issue.tree else { continue };
        let part = of(remote, issue);
        let mr = part.mr.as_ref();
        let file = crate::actions::actions_file(&tree.dir);
        for action in &tree.actions {
            if action.kind == "merge" && !is_merge_offered(issue, turns) {
                continue;
            }
            let try_state = is_try(action).then(|| {
                let origin = local
                    .test_origin
                    .as_deref()
                    .filter(|_| action.on.as_deref() == Some("test"));
                try_state_of(action, Some(tree), mr, origin, remote.is_some(), now)
            });
            let go = if action.kind == "merge" && mr.is_some() {
                Go::Merge {
                    branch: issue.branch.clone(),
                    action: Some((file.clone(), action.id)),
                }
            } else if is_try(action) {
                Go::Try {
                    dir: Some(tree.dir.clone()),
                    id: action.id,
                }
            } else {
                Go::Detail(tree.dir.clone())
            };
            let mut item = Item {
                key: format!("{}#{}", file.display(), action.id),
                kind: action.kind.clone(),
                text: if is_try(action) {
                    format!("{}  {}", action.text, try_spot(action, Some(tree)))
                } else {
                    action.text.clone()
                },
                at: action.at.clone(),
                action: Some(action.clone()),
                who: issue.feature.clone(),
                rank: place_in(merge, issue.number),
                number: issue.number,
                branch: Some(issue.branch.clone()),
                file: Some(file.clone()),
                to: Some(Target::Dir(tree.dir.clone())),
                try_state,
                go,
                ..Default::default()
            };
            if action.kind == "merge"
                && let Some(mr) = mr
            {
                item.merge = Some(merge_offer(mr, part, issue, true));
            }
            items.push(item);
        }
    }
    // Drafts whose merge is offered: nothing but the draft holds them up (#242, #344).
    let ready_drafts: HashSet<i64> = issues
        .iter()
        .filter(|issue| is_merge_offered(issue, turns) && of(remote, issue).mr.as_ref().is_some_and(|mr| mr.is_draft))
        .filter_map(|issue| issue.number)
        .collect();
    let auto = auto_actions_of(
        issues,
        remote,
        &local.agents,
        &AutoFacts {
            ready_drafts: Some(&ready_drafts),
            develop: remote.and_then(|remote| remote.develop.as_ref()),
            removals: Some(&state.removals),
            pipeline_tries: Some(&state.pipeline_tries),
            now,
        },
    );
    for mut item in auto {
        let issue = issues.iter().find(|issue| Some(&issue.branch) == item.branch.as_ref());
        // The overview's own merge follows the same rule as a posted one.
        if item.kind == "merge" && !issue.is_some_and(|issue| is_merge_offered(issue, turns)) {
            continue;
        }
        if item.key.ends_with(":remove")
            && issue
                .and_then(|issue| issue.tree.as_ref())
                .is_some_and(|tree| workers.holds.contains_key(&tree.dir))
        {
            continue;
        }
        item.rank = place_in(merge, item.number);
        if let (Some(issue), Some(offer)) = (issue, item.merge.take())
            && let Some(mr) = &of(remote, issue).mr
        {
            item.merge = Some(merge_offer(mr, of(remote, issue), issue, offer.can_mark_ready));
        }
        items.push(item);
    }
    items.extend(workers.actions.iter().cloned());
    items.extend(super::milestone::milestone_of(local, remote));
    // The end: once all is merged and no worktree is left.
    let has_next_merge = next_merge(issues, remote, order, turns).is_some();
    if remote.is_some() && local.trees.is_empty() && !issues.iter().any(|issue| issue.is_queued) && !has_next_merge {
        items.push(Item {
            key: format!("end:{}", local.name),
            kind: "end".into(),
            text: "all merged, no worktree left: end the orchestration".into(),
            who: format!("{ORCHESTRATOR} orchestrator"),
            is_auto: true,
            rank: i64::MAX,
            go: Go::End,
            ..Default::default()
        });
    }
    let (actions, up_next) = order_actions(items);
    ActionsSection { actions, up_next }
}

/// The MR next in the merge order: the first nothing holds up, else the first and what holds it.
fn next_merge(issues: &[Entry], remote: Option<&Remote>, order: &MergeOrder, turns: &Turns) -> Option<NextMerge> {
    let with_mr = |number: i64| {
        issues
            .iter()
            .rev()
            .find(|issue| issue.number == Some(number) && of(remote, issue).mr.is_some())
    };
    let open: Vec<_> = order
        .open
        .iter()
        .filter(|item| with_mr(item.number).is_some())
        .collect();
    // Ready: its merge is offered, the same rule as ACTIONS (#344).
    let ready: Vec<_> = open
        .iter()
        .filter(|item| {
            turns
                .get(&item.number)
                .is_some_and(|(_, turn)| *turn == MergeTurn::Offered)
        })
        .collect();
    let first = ready.first().copied().or(open.first())?;
    let issue = with_mr(first.number)?;
    let mr = of(remote, issue).mr.as_ref()?;
    Some(NextMerge {
        is_ready: !ready.is_empty(),
        iid: mr.iid,
        url: mr.url.clone(),
        feature: issue.feature.clone(),
        branch: issue.branch.clone(),
        hold_up: first.hold_up.clone(),
        more_ready: ready.len().saturating_sub(1),
    })
}

/// When an MR was merged: `3 h ago` within a day, older by its day and time
/// (`1 Oct 14:01`, tree.mjs `clock`), as git log dates its commits: at 42 rows
/// `6 days ago` on 35 of them told no merge from another.
fn merged_when(at: i64, now: i64) -> String {
    if now - at < 24 * 3_600_000 {
        ago(at, now)
    } else {
        crate::time::clock(at, now)
    }
}

/// MERGE REQUESTS: the open ones as a graph in the merge order, the merged below.
#[allow(clippy::too_many_arguments)]
fn merge_requests_of(
    issues: &[Entry],
    remote: &Remote,
    order: &MergeOrder,
    turns: &Turns,
    local: &OrchestrationLocal,
    resolves: &mut Resolves,
    done: &HashSet<i64>,
    now: i64,
) -> MergeRequests {
    let (agents, queue) = (&local.agents, local.ci.as_ref());
    let with_mr = |number: i64| {
        issues
            .iter()
            .rev()
            .find(|issue| issue.number == Some(number) && remote.of(&issue.branch).mr.is_some())
    };
    let after_of: HashMap<i64, &[super::merge_order::After]> =
        order.open.iter().map(|item| (item.number, &item.after[..])).collect();
    let open_iids: Vec<i64> = issues
        .iter()
        .filter_map(|issue| remote.of(&issue.branch).mr.as_ref())
        .filter(|mr| mr.is_open())
        .map(|mr| mr.iid)
        .collect();
    resolves.forget_resolved(&open_iids);
    let facts_of = |issue: &Entry, is_merged: bool, resolves: &mut Resolves| -> MrFacts {
        let part = remote.of(&issue.branch);
        let mr = part.mr.as_ref().expect("a row with its MR");
        let tree = issue.tree.as_ref();
        let behind = tree.map(|tree| tree.behind).unwrap_or(0);
        let reasons = if is_merged {
            Vec::new()
        } else if let Some((reasons, _)) = issue.number.and_then(|number| turns.get(&number)) {
            reasons.clone()
        } else {
            let boxes = &part.boxes()[..];
            let after = issue
                .number
                .and_then(|number| after_of.get(&number).copied())
                .unwrap_or(&[]);
            row_blockers_of(mr, boxes, behind, after, &part.jobs, tree, done)
        };
        let status = if is_merged && mr.state == "closed" {
            // Closed without a merge: never `merged` (Node said so for both).
            Some(Span::dim("closed"))
        } else if is_merged {
            let when = mr
                .merged_at
                .as_deref()
                .and_then(crate::time::parse_iso)
                .map(|at| merged_when(at, now))
                .unwrap_or_default();
            let word = if crate::epic::is_in_epic(mr, part.issue.as_ref()) {
                "in epic"
            } else {
                "merged"
            };
            Some(Span::dim(format!("{word} {when}").trim_end().to_string()))
        } else if reasons.is_empty() {
            Some(Span::new("ready to merge", super::Paint::Green))
        } else {
            None
        };
        // Waiting in the CI queue: its place stands for the pipeline it has not got yet.
        let ci = queue
            .filter(|_| !is_merged)
            .and_then(|queue| queue.places.get(&mr.iid))
            .filter(|place| place.is_waiting())
            .cloned();
        let hung = local
            .ci_watch
            .as_ref()
            .filter(|_| !is_merged)
            .and_then(|report| report.hung_of(mr.iid));
        let mut reasons = reasons;
        // Marked ready by its bar, then set back by a person: theirs until the next push.
        if local.ready.is_drafted_by_hand(mr.iid, mr.is_draft, mr.sha.as_deref())
            && let Some(reason) = reasons
                .iter_mut()
                .find(|reason| reason.text == super::mr_blockers::READY_BAR_FULL)
        {
            reason.text = super::mr_blockers::DRAFT_BY_HAND.into();
            reason.who = "you".into();
        }
        if let Some(place) = &ci
            && let Some(reason) = reasons.iter_mut().find(|reason| reason.text == "no pipeline yet")
        {
            reason.text = place.text();
            reason.who = "breadboard".into();
        }
        MrFacts {
            iid: mr.iid,
            url: mr.url.clone(),
            pipeline: match (&ci, hung) {
                (Some(place), _) => ci_mark(place, mr.pipeline.as_ref()),
                // A job of its pipeline far past its median (#379): the mark says which, a click opens it.
                // ⧗, not ⏳: narrow in every terminal, a wide one leaves a stale cell where it is drawn narrow (#404).
                // Quiet while it packs its cache (#424): working, dim and no alarm.
                (None, Some(hung)) => Span {
                    text: format!(
                        "⧗ {} {}{}",
                        hung.job,
                        crate::ci_watch::minutes(hung.elapsed),
                        if hung.is_caching() { " caching" } else { "" }
                    ),
                    paint: if hung.is_caching() {
                        super::Paint::Dim
                    } else {
                        super::Paint::Red
                    },
                    url: hung
                        .url
                        .clone()
                        .or_else(|| mr.pipeline.as_ref().and_then(|pipeline| pipeline.web_url.clone())),
                },
                (None, None) => pipeline_mark(mr.pipeline.as_ref()),
            },
            ci,
            title: mr.title.clone(),
            branch: issue.branch.clone(),
            dir: tree.map(|tree| tree.dir.clone()),
            is_merged,
            status,
            conflict: (!is_merged).then(|| resolves.state_of(mr, tree, agents, behind, now)),
            conflicts: reasons
                .iter()
                .any(|reason| reason.is_conflict && reason.who == "agent")
                .then(|| {
                    match tree
                        .and_then(|tree| tree.conflicts.as_deref())
                        .filter(|files| !files.is_empty())
                    {
                        Some(files) => format!("conflicts with {} ({})", mr.target(), files_text(files)),
                        None => format!("conflicts with {}", mr.target()),
                    }
                }),
            reasons: reasons.into_iter().filter(|reason| !reason.is_conflict).collect(),
            progress: if is_merged {
                None
            } else {
                part.issue.as_ref().and_then(|_| progress_of(&part.boxes()))
            },
            boxes: match (&part.issue, is_merged) {
                (Some(_), false) => {
                    let (actions, answered) = tree
                        .map(|tree| (&tree.actions[..], &tree.answered[..]))
                        .unwrap_or((&[], &[]));
                    all_sections_of(&part.task_boxes(), Some((mr.iid, &mr.boxes[..])), actions, answered)
                }
                _ => Vec::new(),
            },
            open_boxes: match (&part.issue, is_merged) {
                (Some(_), false) => {
                    let (actions, answered) = tree
                        .map(|tree| (&tree.actions[..], &tree.answered[..]))
                        .unwrap_or((&[], &[]));
                    open_sections_of(&part.task_boxes(), actions, answered)
                }
                _ => Vec::new(),
            },
            issue: issue.number,
            issues: part.issues().map(|found| found.iid).collect(),
            spent: issue
                .number
                .and_then(|number| local.spent.by_issue.get(&number))
                .filter(|spent| !spent.is_empty())
                .copied(),
        }
    };
    let open: Vec<MrRow> = graph_of(&order.open)
        .into_iter()
        .map(|GraphRow { number, graph, below }| {
            // A row of the graph alone has no issue (#306): nothing to look up.
            let mr = number.and_then(with_mr).map(|issue| facts_of(issue, false, resolves));
            MrRow {
                number,
                graph,
                below,
                mr,
            }
        })
        .collect();
    let merged: Vec<MrRow> = order
        .merged
        .iter()
        .filter_map(|number| with_mr(*number))
        .map(|issue| MrRow {
            number: issue.number,
            graph: String::new(),
            below: String::new(),
            mr: Some(facts_of(issue, true, resolves)),
        })
        .collect();
    let waiting = issues
        .iter()
        .filter(|issue| issue.tree.is_some() && remote.of(&issue.branch).mr.is_none())
        .count();
    let merged_count = merged
        .iter()
        .filter(|row| {
            row.number
                .and_then(with_mr)
                .is_some_and(|issue| remote.of(&issue.branch).mr.as_ref().is_some_and(Mr::is_merged))
        })
        .count();
    let mut counts = vec![format!("{} open", order.open.len())];
    if merged_count > 0 {
        counts.push(format!("{merged_count} merged"));
    }
    if waiting > 0 {
        counts.push(format!("{waiting} without one yet"));
    }
    if !order.open.is_empty() {
        counts.push("in merge order, side by side where independent".into());
    }
    MergeRequests {
        open,
        merged,
        counts: counts.join(" · "),
    }
}

/// What the CI queue takes of an orchestration (#357): its open MRs in the
/// merge order and each one's bar, its boxes (those after the deploy left
/// out) and whether `wt:check` passed at its HEAD.
fn ci_published_of(issues: &[Entry], remote: Option<&Remote>, order: &MergeOrder) -> crate::ci::Published {
    let open_iid = |issue: &Entry| of(remote, issue).mr.as_ref().filter(|mr| mr.is_open()).map(|mr| mr.iid);
    let order_iids: Vec<i64> = order
        .open
        .iter()
        .filter_map(|item| {
            issues
                .iter()
                .rev()
                .filter(|issue| issue.number == Some(item.number))
                .find_map(open_iid)
        })
        .collect();
    let bars = issues
        .iter()
        .filter_map(|issue| {
            let iid = open_iid(issue)?;
            let boxes: Vec<_> = of(remote, issue)
                .issue
                .as_ref()
                .map(|found| {
                    found
                        .boxes
                        .iter()
                        .filter(|found| !super::merge::is_after_deploy(found))
                        .collect()
                })
                .unwrap_or_default();
            // Without its worktree no one can check it here: its boxes say it.
            let tested = issue.tree.as_ref().map(|tree| match &tree.checked {
                crate::check::CheckState::Tested { sha, .. } => Some(sha.clone()),
                _ => None,
            });
            let is_checked = tested.as_ref().is_none_or(Option::is_some);
            Some((
                iid,
                crate::ci::Bar {
                    ticked: boxes.iter().filter(|found| found.is_ticked).count(),
                    total: boxes.len(),
                    is_checked,
                    sha: tested.flatten(),
                    holds: ready_holds_of(issue, of(remote, issue).mr.as_ref()),
                },
            ))
        })
        .collect();
    // A hard wait holds the pipeline of an MR (#392): the prerequisite it
    // needs, or an MR it `wt:wait`s for, while that one is open here. Shared
    // files only rank: they are no entry.
    let mut held: std::collections::BTreeMap<i64, String> = std::collections::BTreeMap::new();
    for item in &order.open {
        let Some(iid) = issues
            .iter()
            .rev()
            .filter(|issue| issue.number == Some(item.number))
            .find_map(open_iid)
        else {
            continue;
        };
        let mut awaited: Vec<i64> = item
            .after
            .iter()
            .filter(|pred| pred.is_hard)
            .map(|pred| pred.iid)
            .collect();
        if let Some(wait) = issues
            .iter()
            .rev()
            .filter(|issue| issue.number == Some(item.number))
            .find_map(|issue| issue.tree.as_ref().and_then(|tree| tree.wait.as_ref()))
        {
            for waited in &wait.mrs {
                if order_iids.contains(waited) && !awaited.contains(waited) {
                    awaited.push(*waited);
                }
            }
        }
        if !awaited.is_empty() {
            let list: Vec<String> = awaited.iter().map(|iid| format!("!{iid}")).collect();
            held.insert(iid, format!("after {}", list.join(", ")));
        }
    }
    crate::ci::Published {
        order: order_iids,
        bars,
        held,
    }
}

/// What keeps an MR a draft though its boxes are all ticked (#399): a try
/// still open or broken (one after the deploy holds nothing), a requested
/// change not yet pushed and green.
fn ready_holds_of(issue: &Entry, mr: Option<&Mr>) -> Option<String> {
    let tree = issue.tree.as_ref()?;
    if let Some(open) = super::merge::open_tries_of(&tree.actions, &tree.answered).first() {
        let word = if open.answer.as_deref() == Some("broken") {
            "broken"
        } else {
            "open"
        };
        return Some(format!("try #{} {word}", open.id));
    }
    super::change::is_change_open(Some(tree), mr).then(|| "change requested".to_string())
}

/// The mark of an MR waiting in the CI queue: its place, a click runs it
/// now; the page of its last pipeline stays a click away on the MR's.
fn ci_mark(place: &crate::ci::Place, last: Option<&crate::gitlab::Pipeline>) -> Span {
    Span {
        text: format!("{} ▸", place.text()),
        paint: super::Paint::Yellow,
        url: last.and_then(|pipeline| pipeline.web_url.clone()),
    }
}

/// Leaves out the waits whose command is gone and that ended (all merged, one
/// closed, past `until`): the live overview types their line and clears them
/// a moment after this read (actor/auto.rs `deliver_waits`), as Node does
/// before its first frame with GitLab's data. Drawn, `waits for` would come
/// and go and everything below it jump (#296). Without GitLab's data only
/// `until` tells. A wait whose line could not get in (`undelivered`) stays:
/// its agent still waits, so the row says so, as Node's.
fn drop_ended_waits(issues: &mut [Entry], remote: Option<&Remote>, now: i64, undelivered: &HashSet<PathBuf>) {
    let mut found: HashMap<i64, Awaited> = HashMap::new();
    if let Some(remote) = remote {
        for (iid, state) in &remote.awaited {
            found.insert(
                *iid,
                Awaited {
                    iid: *iid,
                    state: state.clone(),
                    has_conflicts: false,
                },
            );
        }
        for mr in remote.of.values().filter_map(|part| part.mr.as_ref()) {
            found.insert(
                mr.iid,
                Awaited {
                    iid: mr.iid,
                    state: mr.state.clone(),
                    has_conflicts: mr.has_conflicts,
                },
            );
        }
    }
    let found: Vec<Awaited> = found.into_values().collect();
    for tree in issues
        .iter_mut()
        .filter_map(|issue| issue.tree.as_mut())
        .filter(|tree| !undelivered.contains(&tree.dir))
    {
        let is_ended = tree.wait.as_ref().is_some_and(|wait| {
            !is_waiting(wait) && !matches!(verdict_of(wait, &found, now, &mut HashMap::new()), Verdict::Waiting(_))
        });
        if is_ended {
            tree.wait = None;
        }
    }
}

/// The orchestrator's context and whether now is a quiet point for its fresh start (#382).
fn orchestrator_of(local: &OrchestrationLocal, actions: &[Item]) -> Orchestrator {
    let session = local.chat.as_ref().and_then(|chat| chat.session_id.as_deref());
    let has_chat = local.chat.as_ref().is_some_and(|chat| chat.state == ChatState::Alive);
    Orchestrator {
        share: local.orchestrator_share,
        fresh_above: f64::from(local.fresh.fresh_above) / 100.0,
        is_auto: local.fresh.auto_fresh,
        hold: hold_of(has_chat, session.and_then(|id| local.agents.get(id)), actions),
        journal: local.journal.clone(),
    }
}

/// The view of one orchestration from what was read: `remote` None until
/// GitLab answers; `state` what the writing side keeps.
pub fn orchestration_of(
    local: &OrchestrationLocal,
    remote: Option<&Remote>,
    state: &mut ViewState,
    now: i64,
) -> OrchestrationModel {
    let record = local.record.as_ref();
    let plan: Option<&Plan> = record.and_then(|record| record.plan.as_ref());
    let work: &[i64] = plan.map(|plan| &plan.work[..]).unwrap_or(&[]);
    let mut issues = super::issues_of(record.map(|record| &record.issues[..]).unwrap_or(&[]), &local.trees);
    issues.sort_by_key(|issue| place_in(work, issue.number));
    if state.is_deliverer {
        drop_ended_waits(&mut issues, remote, now, &state.undelivered_waits);
    }
    let label = format!("{ORCHESTRATOR} {}", local.title);
    let left: Vec<String> = local
        .trees
        .iter()
        .map(|tree| tree.name.clone())
        .chain(
            issues
                .iter()
                .filter(|issue| issue.is_queued)
                .map(|issue| format!("#{} (queued)", issue.number.unwrap_or(0))),
        )
        .collect();
    let mut model = OrchestrationModel {
        name: local.name.clone(),
        label,
        chat: local.chat.clone(),
        is_auto: record.is_some_and(|record| record.is_auto),
        settings: local.settings.clone(),
        checker: crate::config::here().config.tries.checker.clone(),
        is_empty: issues.is_empty(),
        has_remote: remote.is_some(),
        next: Err(String::new()),
        actions: Ok(ActionsSection::default()),
        worktrees: Ok(Worktrees::default()),
        free_agents: Ok(Vec::new()),
        merge_requests: Ok(MergeRequests::default()),
        left,
        orders: Vec::new(),
        tells: Vec::new(),
        holds: HashMap::new(),
        stale: Vec::new(),
        plan_update: None,
        ci_load: local.ci.as_ref().map(crate::ci::Queue::load_text),
        spent: crate::spend::band_text(&local.spent),
        held: local.spent.held.clone(),
        paused: local.paused.clone(),
        usage: local.spent.usage.clone(),
        ci_watch: local.ci_watch.as_ref().and_then(crate::ci_watch::Report::line),
        ci_published: None,
        turn_records: Vec::new(),
        epic: super::epic_row::epic_row_of(local, remote),
        orchestrator: orchestrator_of(local, &[]),
    };
    if issues.is_empty() {
        model.next = Ok(NextSection {
            band: NextBand::Nothing(String::new()),
            why: None,
        });
        return model;
    }
    let known: &[Worker] = record.map(|record| &record.workers[..]).unwrap_or(&[]);
    let merge_plan: &[i64] = plan.map(|plan| &plan.merge[..]).unwrap_or(&[]);
    // The merge order, computed on each read; by number only the issues with an MR (#221).
    let order = section("the merge order", || {
        let no_files = Default::default();
        let mut by_number: Vec<&Entry> = Vec::new();
        for issue in issues
            .iter()
            .filter(|issue| issue.number.is_some() && of(remote, issue).mr.is_some())
        {
            by_number.retain(|known| known.number != issue.number);
            by_number.push(issue);
        }
        let entries: Vec<MergeEntry> = by_number
            .iter()
            .map(|issue| {
                let part = of(remote, issue);
                let found = part.issue.as_ref();
                let tree = issue.tree.as_ref();
                MergeEntry {
                    number: issue.number.unwrap_or(0),
                    mr: part.mr.as_ref().expect("an MR"),
                    needs: found.map(|found| &found.needs[..]).unwrap_or(&[]),
                    files: tree.map(|tree| &tree.files).unwrap_or(&no_files),
                    boxes: found.map(|found| &found.boxes[..]).unwrap_or(&[]),
                    behind: tree.map(|tree| tree.behind).unwrap_or(0),
                    actions: tree.map(|tree| &tree.actions[..]).unwrap_or(&[]),
                    answered: tree.map(|tree| &tree.answered[..]).unwrap_or(&[]),
                }
            })
            .collect();
        merge_order_of(&entries, merge_plan)
    })
    .unwrap_or_default();
    if remote.is_some() {
        model.ci_published = Some(ci_published_of(&issues, remote, &order));
    }
    // Conflicts and failed pipelines reach their agents by themselves, from the one live overview.
    if let Some(remote) = remote.filter(|_| state.is_deliverer) {
        model.tells = conflicts_of(&issues, remote);
        model
            .tells
            .extend(failed_pipelines_of(&issues, remote, remote.develop.as_ref()));
    }
    let free_workers: Vec<Worker> = local.free.iter().map(|(worker, _)| worker.clone()).collect();
    let announced = announced_of(
        record.map(|record| &record.handovers[..]).unwrap_or(&[]),
        &issues,
        known,
        &local.agents,
        remote.map(|remote| &remote.awaited),
        &free_workers,
    );
    let workers = section("the workers", || {
        worker_actions_of(
            &issues,
            remote,
            &local.agents,
            known,
            &WorkerFacts {
                should_hand_out: remote.is_some() && state.is_deliverer,
                announced: &announced,
                memory: local.free_memory.unwrap_or(0),
                setups: &local.setups,
                awaited: remote.map(|remote| &remote.awaited),
                work,
                free: &local.free,
                runs: Some(&state.runs),
                now,
                limit: local.context_limit,
            },
        )
    })
    .unwrap_or_default();
    model.orders = workers.orders.clone();
    model.holds = workers.holds.clone();
    // The MRs a wait may name that are over already: the orchestration's own and the awaited (#262).
    let mut done: HashSet<i64> = HashSet::new();
    let mut merged: HashSet<i64> = HashSet::new();
    for issue in &issues {
        if let Some(mr) = of(remote, issue).mr.as_ref().filter(|mr| !mr.is_open()) {
            done.insert(mr.iid);
            if mr.is_merged() {
                merged.insert(mr.iid);
            }
        }
    }
    for (iid, found) in remote.map(|remote| &remote.awaited).into_iter().flatten() {
        if found != "opened" {
            done.insert(*iid);
            if found == "merged" {
                merged.insert(*iid);
            }
        }
    }
    let turns = turns_of(&issues, remote, &order, &done);
    model.turn_records = turn_records_of(&issues, remote, &turns);
    model.actions = section("ACTIONS", || {
        actions_of(local, &issues, remote, &order, &turns, &workers, state, now)
    });
    if let Ok(actions) = &model.actions {
        model.orchestrator = orchestrator_of(local, &actions.actions);
    }
    if remote.is_some() {
        let states = states_of(&issues, remote);
        model.stale = stale_of(plan, &states, order.is_off_plan);
        model.plan_update = (!model.stale.is_empty()).then(|| updated_plan(plan, &states, &order.planned));
    }
    let merge = remote.and_then(|_| next_merge(&issues, remote, &order, &turns));
    model.next = section("NEXT", || {
        let (actions, up_next) = match &model.actions {
            Ok(section) => (&section.actions[..], &section.up_next[..]),
            Err(_) => (&[][..], &[][..]),
        };
        let next = next_of(actions, up_next, merge.as_ref());
        let why = why_of(plan, &model.stale, next.as_ref(), actions);
        let band = match next {
            Some(Next::Action(at)) => NextBand::Action(actions[at].clone()),
            Some(Next::UpNext(at)) => NextBand::UpNext(up_next[at].clone()),
            Some(Next::Merge(merge)) => NextBand::Merge(merge),
            None if remote.is_none() => NextBand::Nothing(GITLAB_PENDING.into()),
            None => {
                let at_work = issues
                    .iter()
                    .filter(|issue| {
                        issue
                            .tree
                            .as_ref()
                            .and_then(|tree| tree.claude.as_ref())
                            .and_then(|claude| local.agents.get(&claude.id))
                            .and_then(|agent| agent.status.as_deref())
                            == Some("working")
                    })
                    .count();
                NextBand::Nothing(nothing_text(
                    at_work,
                    merge.as_ref(),
                    remote
                        .and_then(|remote| develop_red_text(remote.develop.as_ref()))
                        .filter(|_| {
                            turns
                                .values()
                                .any(|(reasons, _)| reasons.iter().any(|reason| reason.who == "develop"))
                        })
                        .as_deref(),
                ))
            }
        };
        NextSection { band, why }
    });
    let merge_numbers: Vec<i64> = order.open.iter().map(|item| item.number).collect();
    model.worktrees = section("WORKTREES", || {
        worktrees_of(
            &local.name,
            &issues,
            remote,
            &TreeFacts {
                agents: Some(&local.agents),
                spent: Some(&local.spent),
                work,
                merge: &merge_numbers,
                announced: &announced,
                holds: Some(&workers.holds),
                suggested: Some(&workers.suggested),
                fresh: Some(&workers.fresh),
                removals: Some(&state.removals),
                has_remote: remote.is_some(),
                known,
                merged: Some(&merged),
                turns: Some(&turns),
                setups: &local.setups,
                now,
                limit: local.context_limit,
            },
            local.free_memory,
        )
    });
    model.free_agents = section("FREE AGENTS", || {
        free_rows_of(&local.free, &local.agents, &names_of(&issues, known))
    });
    if let Some(remote) = remote {
        let resolves = &mut state.resolves;
        model.merge_requests = section("MERGE REQUESTS", || {
            merge_requests_of(&issues, remote, &order, &turns, local, resolves, &done, now)
        });
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::{Issue, Pipeline};
    use crate::model::Tree;
    use crate::model::handovers::tests::claude_of;
    use crate::record::IssueEntry;

    const NOW: i64 = 1_790_000_000_000;

    #[test]
    fn a_merge_older_than_a_day_says_its_day_and_time() {
        assert_eq!(merged_when(NOW - 3 * 3_600_000, NOW), "3 h ago");
        // Two merges of the same week no longer both say `6 days ago`.
        let (a, b) = (
            merged_when(NOW - 6 * 86_400_000, NOW),
            merged_when(NOW - 6 * 86_400_000 - 3_600_000, NOW),
        );
        assert_ne!(a, b);
        assert!(
            !a.contains("ago") && a.ends_with(&crate::time::hm(NOW - 6 * 86_400_000)),
            "{a}"
        );
    }

    fn entry(number: i64, branch: &str) -> IssueEntry {
        IssueEntry {
            number: Some(number),
            title: Some(format!("issue {number}")),
            feature: Some(branch.trim_start_matches("feature/").into()),
            branch: Some(branch.into()),
            ..Default::default()
        }
    }
    fn tree(feature: &str) -> Tree {
        Tree {
            dir: format!("/w/{feature}").into(),
            name: feature.into(),
            branch: format!("feature/{feature}"),
            claude: Some(claude_of(feature)),
            ..Default::default()
        }
    }
    fn local() -> OrchestrationLocal {
        OrchestrationLocal {
            name: "breadboard".into(),
            title: "@monocircuit/breadboard".into(),
            trees: vec![tree("1-one"), tree("2-two")],
            record: Some(Record {
                issues: vec![
                    entry(1, "feature/1-one"),
                    entry(2, "feature/2-two"),
                    entry(3, "feature/3-three"),
                ],
                plan: Some(Plan {
                    work: vec![2, 1],
                    merge: vec![1, 2],
                    why: Some("#1 first".into()),
                    ..Plan::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn mr(iid: i64, branch: &str, state: &str, status: &str) -> Mr {
        Mr {
            iid,
            title: format!("MR {iid}"),
            state: state.into(),
            url: format!("https://gitlab.com/x/-/merge_requests/{iid}"),
            branch: branch.into(),
            merge_status: Some("mergeable".into()),
            merged_at: (state == "merged").then(|| "2026-09-30T10:00:00Z".to_string()),
            pipeline: Some(Pipeline {
                status: status.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn remote() -> Remote {
        let part = |mr: Mr, number: i64| BranchRemote {
            issue: Some(Issue {
                iid: number,
                title: format!("issue {number}"),
                state: "opened".into(),
                url: "i".into(),
                ..Default::default()
            }),
            mr: Some(mr),
            jobs: vec![],
            also: vec![],
        };
        Remote {
            of: [
                (
                    "feature/1-one".to_string(),
                    part(mr(11, "feature/1-one", "opened", "success"), 1),
                ),
                (
                    "feature/2-two".to_string(),
                    part(mr(12, "feature/2-two", "opened", "running"), 2),
                ),
                (
                    "feature/3-three".to_string(),
                    part(mr(13, "feature/3-three", "merged", "success"), 3),
                ),
            ]
            .into(),
            ..Default::default()
        }
    }

    /// #344, !324 of one-codebase: done itself, behind another MR in the merge
    /// order. Sharing `package.json` alone only ranks them (#392): `needs` is
    /// what makes #2 wait for #1.
    fn behind_another_sharing_files() -> (OrchestrationLocal, Remote) {
        let mut local = local();
        let files: std::collections::BTreeSet<String> = ["package.json".to_string()].into();
        for tree in &mut local.trees {
            tree.files = files.clone();
            tree.pushed = Some("a".into());
        }
        // Its agent posted the merge anyway: no merge is offered while it waits.
        local.trees[1].actions = vec![Action {
            id: 1,
            kind: "merge".into(),
            text: "the work".into(),
            ..Default::default()
        }];
        let mut remote = remote();
        let draft = |iid: i64, branch: &str, status: &str| Mr {
            is_draft: true,
            merge_status: Some("draft_status".into()),
            pipeline: Some(Pipeline {
                status: status.into(),
                sha: Some("a".into()),
                ..Default::default()
            }),
            ..mr(iid, branch, "opened", status)
        };
        remote.of.get_mut("feature/1-one").unwrap().mr = Some(draft(11, "feature/1-one", "running"));
        remote.of.get_mut("feature/2-two").unwrap().mr = Some(draft(12, "feature/2-two", "success"));
        (local, remote)
    }

    fn behind_another() -> (OrchestrationLocal, Remote) {
        let (local, mut remote) = behind_another_sharing_files();
        remote
            .of
            .get_mut("feature/2-two")
            .unwrap()
            .issue
            .as_mut()
            .unwrap()
            .needs = vec![1];
        (local, remote)
    }

    #[test]
    fn sharing_a_code_file_only_ranks_it_the_merge_is_offered_in_parallel() {
        let (local, remote) = behind_another_sharing_files();
        let model = orchestration_of(&local, Some(&remote), &mut ViewState::default(), NOW);
        assert!(model.turn_records.contains(&(
            PathBuf::from("/w/2-two"),
            TurnRecord {
                sha: "a".into(),
                turn: MergeTurn::Offered,
            }
        )));
        let actions = model.actions.unwrap();
        assert_eq!(
            actions
                .actions
                .iter()
                .filter(|item| item.kind == "merge")
                .map(|item| item.who.as_str())
                .collect::<Vec<_>>(),
            ["2-two"]
        );
        let mrs = model.merge_requests.unwrap();
        let row = mrs
            .open
            .iter()
            .filter_map(|row| row.mr.as_ref())
            .find(|mr| mr.iid == 12)
            .unwrap();
        assert_eq!(
            row.reasons
                .iter()
                .map(|reason| format!("{} → {}", reason.text, reason.who))
                .collect::<Vec<_>>(),
            ["draft: mark ready → you"]
        );
    }

    #[test]
    fn a_done_mr_behind_another_never_waits_for_you() {
        let (local, remote) = behind_another();
        let model = orchestration_of(&local, Some(&remote), &mut ViewState::default(), NOW);
        // herdr's sidebar gets the same turn, for the commit the pipeline ran on.
        assert!(model.turn_records.contains(&(
            PathBuf::from("/w/2-two"),
            TurnRecord {
                sha: "a".into(),
                turn: MergeTurn::Waits(vec![11]),
            }
        )));
        let actions = model.actions.unwrap();
        assert!(
            !actions.actions.iter().any(|item| item.kind == "merge"),
            "{:?}",
            actions.actions
        );
        let next = model.next.unwrap();
        assert!(
            matches!(&next.band, NextBand::Nothing(text) if text.contains("next merge: !11")),
            "{:?}",
            next.band
        );
        let worktrees = model.worktrees.unwrap();
        let two = worktrees.tree.live.iter().find(|node| node.name == "2-two").unwrap();
        assert_eq!(two.state.text, "waits for !11");
        let mrs = model.merge_requests.unwrap();
        let row = mrs
            .open
            .iter()
            .filter_map(|row| row.mr.as_ref())
            .find(|mr| mr.iid == 12)
            .unwrap();
        assert_eq!(
            row.reasons
                .iter()
                .map(|reason| format!("{} → {}", reason.text, reason.who))
                .collect::<Vec<_>>(),
            ["draft: mark ready → agent", "after !11 (needs #1) → !11"]
        );
    }

    #[test]
    fn a_red_develop_holds_the_merge_offers_and_the_band_says_why() {
        let (local, mut remote) = behind_another_sharing_files();
        let offered = |remote: &Remote| {
            let model = orchestration_of(&local, Some(remote), &mut ViewState::default(), NOW);
            let merges = model
                .actions
                .as_ref()
                .unwrap()
                .actions
                .iter()
                .filter(|item| item.kind == "merge")
                .count();
            (merges, model)
        };
        assert_eq!(offered(&remote).0, 1);
        remote.develop = Some(crate::gitlab::DevelopPipeline {
            id: 9,
            status: "failed".into(),
            jobs: vec![crate::gitlab::Job {
                name: "check:vita".into(),
                status: "failed".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let (merges, model) = offered(&remote);
        assert_eq!(merges, 0);
        let next = model.next.unwrap();
        assert!(
            matches!(&next.band, NextBand::Nothing(text) if text.contains("develop red (check:vita): merges held")),
            "{:?}",
            next.band
        );
        let row = model
            .merge_requests
            .unwrap()
            .open
            .iter()
            .filter_map(|row| row.mr.clone())
            .find(|mr| mr.iid == 12)
            .unwrap();
        assert!(
            row.reasons
                .iter()
                .any(|reason| reason.text == "develop red (check:vita)"),
            "{:?}",
            row.reasons
        );
        // Green again: offered as before.
        remote.develop.as_mut().unwrap().status = "success".into();
        assert_eq!(offered(&remote).0, 1);
    }

    #[test]
    fn a_push_a_red_develop_holds_says_so_not_only_its_count() {
        let unpushed = || {
            (
                vec![Reason {
                    text: "2 commits not pushed".into(),
                    who: "agent".into(),
                    is_conflict: false,
                }],
                MergeTurn::Lacks,
            )
        };
        let failed = Tree {
            unpushed: 2,
            checked: crate::check::CheckState::Failed {
                sha: "a".into(),
                failures: vec![crate::check::Failure {
                    package: "@monocircuit/breadboard".into(),
                    task: "test:rs".into(),
                    tests: vec!["tui::overview::work::tests::renders_the_row".into()],
                }],
            },
            ..Default::default()
        };
        let red = crate::gitlab::DevelopPipeline {
            id: 9,
            status: "failed".into(),
            ..Default::default()
        };
        let (reasons, _) = hold_push_while_develop_is_red(unpushed(), Some(&failed), Some(&red));
        assert_eq!(
            (reasons[0].text.as_str(), reasons[0].who.as_str()),
            (
                "push blocked: develop red (tui::overview::work::tests::renders_the_row)",
                DEVELOP_FIX
            )
        );
        // develop green, or its own check passed: its agent pushes.
        let green = crate::gitlab::DevelopPipeline {
            status: "success".into(),
            ..red.clone()
        };
        let (reasons, _) = hold_push_while_develop_is_red(unpushed(), Some(&failed), Some(&green));
        assert_eq!(reasons[0].text, "2 commits not pushed");
        let tested = Tree {
            checked: crate::check::CheckState::None,
            ..failed.clone()
        };
        let (reasons, _) = hold_push_while_develop_is_red(unpushed(), Some(&tested), Some(&red));
        assert_eq!(reasons[0].who, "agent");
    }

    #[test]
    fn its_merge_is_offered_once_the_one_before_is_merged() {
        let (local, mut remote) = behind_another();
        remote.of.get_mut("feature/1-one").unwrap().mr = Some(mr(11, "feature/1-one", "merged", "success"));
        let model = orchestration_of(&local, Some(&remote), &mut ViewState::default(), NOW);
        let actions = model.actions.unwrap();
        assert_eq!(
            actions
                .actions
                .iter()
                .filter(|item| item.kind == "merge")
                .map(|item| item.who.as_str())
                .collect::<Vec<_>>(),
            ["2-two"]
        );
        let worktrees = model.worktrees.unwrap();
        let two = worktrees.tree.live.iter().find(|node| node.name == "2-two").unwrap();
        assert_eq!(two.state.text, "waits for you: merge");
        let mrs = model.merge_requests.unwrap();
        let row = mrs
            .open
            .iter()
            .filter_map(|row| row.mr.as_ref())
            .find(|mr| mr.iid == 12)
            .unwrap();
        assert_eq!(row.reasons[0].who, "you");
    }

    #[test]
    fn publishes_a_hard_wait_as_a_held_pipeline_and_shared_files_not() {
        // #392: sharing package.json only ranks #2 behind #1: no hold.
        let (local, remote) = behind_another_sharing_files();
        let model = orchestration_of(&local, Some(&remote), &mut ViewState::default(), NOW);
        assert!(model.ci_published.as_ref().unwrap().held.is_empty());
        // #2 needs #1: its pipeline waits for !11 to be merged.
        let (local, remote) = behind_another();
        let model = orchestration_of(&local, Some(&remote), &mut ViewState::default(), NOW);
        assert_eq!(
            model.ci_published.as_ref().unwrap().held,
            [(12, "after !11".to_string())].into()
        );
    }

    #[test]
    fn says_where_an_mr_stands_in_the_ci_queue_and_publishes_the_merge_order() {
        use crate::ci::{Place, Queue};
        let mut remote = remote();
        remote
            .of
            .get_mut("feature/2-two")
            .unwrap()
            .mr
            .as_mut()
            .unwrap()
            .pipeline = None;
        let mut local = local();
        let mut queue = Queue {
            at: NOW,
            ..Default::default()
        };
        queue.places.insert(12, Place::Next { at: 2 });
        local.ci = Some(queue);
        let model = orchestration_of(&local, Some(&remote), &mut ViewState::default(), NOW);
        assert_eq!(model.ci_published.as_ref().unwrap().order, vec![11, 12]);
        assert_eq!(model.ci_load.as_deref(), Some("CI 0 running · 1 queued"));
        let mrs = model.merge_requests.unwrap();
        let two = mrs
            .open
            .iter()
            .filter_map(|row| row.mr.as_ref())
            .find(|mr| mr.iid == 12)
            .unwrap();
        assert_eq!(two.pipeline.text, "CI: next (2nd) ▸");
        assert_eq!(two.ci, Some(Place::Next { at: 2 }));
        assert!(
            two.reasons
                .iter()
                .any(|reason| reason.text == "CI: next (2nd)" && reason.who == "breadboard"),
            "{:?}",
            two.reasons
        );
        // One with its pipeline: no place, its mark as before.
        let one = mrs
            .open
            .iter()
            .filter_map(|row| row.mr.as_ref())
            .find(|mr| mr.iid == 11)
            .unwrap();
        assert_eq!((one.ci.as_ref(), one.pipeline.text.as_str()), (None, "✓ passed"));
    }

    #[test]
    fn a_closed_mr_says_closed_not_merged() {
        let mut remote = remote();
        let part = remote.of.get_mut("feature/3-three").unwrap();
        part.mr = Some(mr(13, "feature/3-three", "closed", "success"));
        let mut state = ViewState::default();
        let model = orchestration_of(&local(), Some(&remote), &mut state, NOW);
        let mrs = model.merge_requests.unwrap();
        let closed = mrs.merged[0].mr.as_ref().unwrap();
        assert_eq!(closed.status.as_ref().unwrap().text, "closed");
        assert!(!mrs.counts.contains("merged"), "{}", mrs.counts);
    }

    /// The plan of the fixture's record, replaced.
    fn with_plan(merge: Vec<i64>) -> OrchestrationLocal {
        let mut local = local();
        if let Some(plan) = local.record.as_mut().and_then(|record| record.plan.as_mut()) {
            plan.merge = merge;
        }
        local
    }

    #[test]
    fn a_stale_plan_is_written_by_itself_never_offered_as_a_row() {
        // #3 merged, still in the plan: the merge order to write without it, the why kept.
        let model = orchestration_of(
            &with_plan(vec![1, 2, 3]),
            Some(&remote()),
            &mut ViewState::default(),
            NOW,
        );
        assert_eq!(model.stale, ["#3 merged"]);
        assert_eq!(
            model.plan_update,
            Some(Plan {
                work: vec![2, 1],
                merge: vec![1, 2],
                why: Some("#1 first".into()),
                ..Plan::default()
            })
        );
        let actions = model.actions.unwrap();
        assert!(
            !actions
                .actions
                .iter()
                .chain(&actions.up_next)
                .any(|item| item.text.contains("plan")),
            "{actions:?}"
        );
        // No merge order in the plan yet: the computed one goes in.
        let model = orchestration_of(&with_plan(vec![]), Some(&remote()), &mut ViewState::default(), NOW);
        assert_eq!(model.plan_update.map(|plan| plan.merge), Some(vec![1, 2]));
        // Before GitLab answers nothing is known to write.
        let model = orchestration_of(&with_plan(vec![1, 2, 3]), None, &mut ViewState::default(), NOW);
        assert_eq!(model.plan_update, None);
    }

    #[test]
    fn a_hand_set_order_of_independent_mrs_survives() {
        // !12 shares no file with !11: the plan's order holds, nothing to write.
        let model = orchestration_of(&with_plan(vec![2, 1]), Some(&remote()), &mut ViewState::default(), NOW);
        assert!(model.stale.is_empty(), "{:?}", model.stale);
        assert_eq!(model.plan_update, None);
        let open: Vec<i64> = model
            .merge_requests
            .unwrap()
            .open
            .iter()
            .filter_map(|row| row.number)
            .collect();
        assert_eq!(open, [2, 1]);
    }

    #[test]
    fn draws_every_section_of_an_orchestration() {
        let mut state = ViewState::default();
        let model = orchestration_of(&local(), Some(&remote()), &mut state, NOW);
        assert_eq!(model.label, "⇶ @monocircuit/breadboard");
        let actions = model.actions.unwrap();
        assert_eq!(
            actions
                .actions
                .iter()
                .map(|item| (item.kind.as_str(), item.who.as_str()))
                .collect::<Vec<_>>(),
            [("merge", "1-one")]
        );
        let next = model.next.unwrap();
        assert!(matches!(&next.band, NextBand::Action(item) if item.kind == "merge"));
        assert_eq!(next.why.as_deref(), Some("#1 first"));
        let worktrees = model.worktrees.unwrap();
        assert_eq!(
            worktrees
                .tree
                .live
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["2-two", "1-one"]
        );
        assert_eq!(worktrees.tree.removed.len(), 1);
        let mrs = model.merge_requests.unwrap();
        assert_eq!(
            mrs.open
                .iter()
                .filter_map(|row| row.mr.as_ref().map(|mr| mr.iid))
                .collect::<Vec<_>>(),
            [11, 12]
        );
        assert_eq!(
            mrs.merged
                .iter()
                .filter_map(|row| row.mr.as_ref().map(|mr| mr.iid))
                .collect::<Vec<_>>(),
            [13]
        );
        assert_eq!(
            mrs.counts,
            "2 open · 1 merged · in merge order, side by side where independent"
        );
        let first = mrs.open[0].mr.as_ref().unwrap();
        assert_eq!(first.status.as_ref().unwrap().text, "ready to merge");
        assert_eq!(first.conflict, Some(Conflict::None));
        let second = mrs.open[1].mr.as_ref().unwrap();
        assert_eq!(
            second
                .reasons
                .iter()
                .map(|reason| reason.text.as_str())
                .collect::<Vec<_>>(),
            ["pipeline running"]
        );
        assert_eq!(model.left, ["1-one", "2-two"]);
    }

    #[test]
    fn waits_for_gitlab_before_it_answers() {
        let model = orchestration_of(&local(), None, &mut ViewState::default(), NOW);
        assert!(!model.has_remote);
        assert_eq!(model.next.unwrap().band, NextBand::Nothing(GITLAB_PENDING.into()));
        assert_eq!(model.merge_requests.unwrap(), MergeRequests::default());
        assert_eq!(model.worktrees.unwrap().tree.live.len(), 2);
    }

    #[test]
    fn an_mr_without_issue_or_pipeline_costs_nothing_but_its_detail() {
        let mut gitlab = remote();
        let part = gitlab.of.get_mut("feature/2-two").unwrap();
        part.issue = None;
        part.mr.as_mut().unwrap().pipeline = None;
        let model = orchestration_of(&local(), Some(&gitlab), &mut ViewState::default(), NOW);
        let mrs = model.merge_requests.unwrap();
        assert_eq!(
            mrs.open
                .iter()
                .filter_map(|row| row.mr.as_ref().map(|mr| mr.iid))
                .collect::<Vec<_>>(),
            [11, 12]
        );
        assert!(model.worktrees.is_ok() && model.actions.is_ok() && model.next.is_ok());
    }

    #[test]
    fn offers_to_end_once_everything_is_merged() {
        let mut gone = local();
        gone.trees.clear();
        let mut gitlab = remote();
        for part in gitlab.of.values_mut() {
            part.mr.as_mut().unwrap().state = "merged".into();
        }
        let model = orchestration_of(&gone, Some(&gitlab), &mut ViewState::default(), NOW);
        let actions = model.actions.unwrap();
        assert!(actions.up_next.iter().any(|item| item.kind == "end"));
        assert!(model.left.is_empty());
    }

    #[test]
    fn says_so_without_issues() {
        let model = orchestration_of(
            &OrchestrationLocal {
                name: "x".into(),
                title: "x".into(),
                ..Default::default()
            },
            None,
            &mut ViewState::default(),
            NOW,
        );
        assert!(model.is_empty);
    }

    #[test]
    fn an_ended_wait_is_never_drawn() {
        // 1-one waits for !12 (open) until after NOW: drawn. 2-two waits for !13 (merged): ended.
        let wait = |mrs: Vec<i64>, until: &str| crate::wait::Wait {
            mrs,
            until: Some(until.into()),
            ..Default::default()
        };
        let mut local = local();
        local.trees[0].wait = Some(wait(vec![12], "2099-01-01T00:00:00Z"));
        local.trees[1].wait = Some(wait(vec![13], "2099-01-01T00:00:00Z"));
        let reasons = |state: &mut ViewState, local: &OrchestrationLocal| -> Vec<Vec<String>> {
            let model = orchestration_of(local, Some(&remote()), state, NOW);
            let mrs = model.merge_requests.unwrap();
            mrs.open
                .iter()
                .filter_map(|row| row.mr.as_ref())
                .map(|mr| {
                    mr.reasons
                        .iter()
                        .filter(|reason| reason.text.starts_with("waits for"))
                        .map(|reason| reason.text.clone())
                        .collect()
                })
                .collect()
        };
        let mut deliverer = ViewState {
            is_deliverer: true,
            ..Default::default()
        };
        // !11 waits for !12, still open; !12's wait for !13 is over (and !13 counts as done anyway).
        assert_eq!(
            reasons(&mut deliverer, &local),
            [vec!["waits for !12".to_string()], vec![]]
        );
        // Past its `until`: ended, though !12 is still open; a print draws it as Node's --once.
        local.trees[0].wait = Some(wait(vec![12], "2020-01-01T00:00:00Z"));
        assert_eq!(reasons(&mut deliverer, &local), [Vec::<String>::new(), vec![]]);
        assert_eq!(
            reasons(&mut ViewState::default(), &local),
            [vec!["waits for !12".to_string()], vec![]]
        );
        // Its line could not get in (the agent blocked): its agent still waits, the row says so.
        deliverer.undelivered_waits.insert(local.trees[0].dir.clone());
        assert_eq!(
            reasons(&mut deliverer, &local),
            [vec!["waits for !12".to_string()], vec![]]
        );
    }
}
