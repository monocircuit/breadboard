//! The view of one orchestration (--orchestration, render-orchestration.mjs)
//! as data: NEXT, what the agents ask for (ACTIONS, UP NEXT), its worktrees as
//! a tree (WORKTREES), the free agents, the merge requests in the merge order
//! the overview computes, the merged below, and whether it can end. Each
//! section is derived on its own (#306): one that fails says so in its place.

use super::auto_actions::{AutoFacts, Removal, Tell, auto_actions_of, conflicts_of, failed_pipelines_of};
use super::display_names::names_of;
use super::free_agents::{FreeRow, free_rows_of};
use super::handovers::announced_of;
use super::merge::{blockers_of, checks_of, to_check_of};
use super::merge_order::{GraphRow, MergeEntry, MergeOrder, graph_of, merge_order_of};
use super::mr_blockers::{Progress, Reason, progress_of, row_blockers_of};
use super::next::{IssueState, Next, NextMerge, next_of, nothing_text, stale_of, updated_plan, why_of};
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
    /// What herdr says each agent does, by conversation; empty outside herdr.
    pub agents: HashMap<String, Agent>,
    pub chat: Option<Chat>,
    pub setups: Vec<Setup>,
    /// The test instance's origin (`VITA_TEST_ORIGIN`).
    pub test_origin: Option<String>,
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
}

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
    /// What it still lacks, who acts on it; its conflicts said by `conflict`.
    pub reasons: Vec<Reason>,
    /// Its issue's boxes, ticked of all (#294); None for a merged one or an issue without boxes.
    pub progress: Option<Progress>,
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

/// Whether a posted merge shows in ACTIONS: not once its MR is merged, nor
/// while a try or an acceptance box is open (#280).
fn is_merge_offered(issue: &Entry, part: &BranchRemote) -> bool {
    let boxes = part.issue.as_ref().map(|found| &found.boxes[..]).unwrap_or(&[]);
    let (actions, answered) = issue
        .tree
        .as_ref()
        .map(|tree| (&tree.actions[..], &tree.answered[..]))
        .unwrap_or((&[], &[]));
    !part.mr.as_ref().is_some_and(Mr::is_merged) && checks_of(boxes, actions, answered).is_empty()
}

/// The open MRs after `number` in the merge order, the issue next first (merge.mjs `openAfter`).
fn open_after(issues: &[Entry], remote: Option<&Remote>, number: Option<i64>, order: &[i64]) -> Vec<i64> {
    let mut after: Vec<&Entry> = issues
        .iter()
        .filter(|issue| {
            issue.number != number
                && issue.tree.is_some()
                && of(remote, issue)
                    .mr
                    .as_ref()
                    .is_some_and(|mr| mr.state != "merged" && mr.state != "closed")
        })
        .collect();
    after.sort_by_key(|issue| (place_in(order, issue.number), issue.number.unwrap_or(i64::MAX)));
    after.iter().filter_map(|issue| issue.number).collect()
}

/// What a merge row offers: its MR, what its two clicks need.
fn merge_offer(mr: &Mr, part: &BranchRemote, issue: &Entry, can_mark_ready: bool, others: Vec<i64>) -> MergeOffer {
    let boxes = part.issue.as_ref().map(|found| &found.boxes[..]).unwrap_or(&[]);
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
        others,
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

/// The page of an action's url: a path on the app of the main checkout, anything else as it is.
fn page_of(url: &str, offset: i64) -> String {
    if url.starts_with('/') {
        format!("http://localhost:{}{url}", 3000 + offset)
    } else {
        url.to_string()
    }
}

/// Every open action, posted and automatic, split into ACTIONS and UP NEXT.
fn actions_of(
    local: &OrchestrationLocal,
    issues: &[Entry],
    remote: Option<&Remote>,
    order: &MergeOrder,
    workers: &super::workers::WorkerActions,
    state: &ViewState,
    now: i64,
) -> ActionsSection {
    let merge = &order.numbers;
    let mut items: Vec<Item> = Vec::new();
    // The orchestrator's own (`wt:action --orchestration`); a merge it posted for an MR (`--mr !N`) is that MR's.
    for action in local.record.as_ref().map(|record| &record.actions[..]).unwrap_or(&[]) {
        let issue = action.mr.filter(|_| action.kind == "merge").and_then(|iid| {
            issues
                .iter()
                .find(|issue| of(remote, issue).mr.as_ref().is_some_and(|mr| mr.iid == iid))
        });
        if issue.is_some_and(|issue| !is_merge_offered(issue, of(remote, issue))) {
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
                    url: page_of(url, 0),
                    dir: None,
                },
                (None, None) => Go::Nothing,
            },
            ..Default::default()
        };
        if let Some(issue) = issue
            && let Some(mr) = &of(remote, issue).mr
        {
            item.merge = Some(merge_offer(
                mr,
                of(remote, issue),
                issue,
                true,
                open_after(issues, remote, issue.number, merge),
            ));
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
            if action.kind == "merge" && !is_merge_offered(issue, part) {
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
                Go::FocusSpace(tree.dir.clone())
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
                item.merge = Some(merge_offer(
                    mr,
                    part,
                    issue,
                    true,
                    open_after(issues, remote, issue.number, merge),
                ));
            }
            items.push(item);
        }
    }
    // Drafts nothing but the draft holds up, next in the merge order (#242).
    let ready_drafts: HashSet<i64> = order
        .open
        .iter()
        .filter(|item| item.hold_up.len() == 1 && item.hold_up[0] == "draft")
        .map(|item| item.number)
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
            item.merge = Some(merge_offer(
                mr,
                of(remote, issue),
                issue,
                offer.can_mark_ready,
                open_after(issues, remote, issue.number, merge),
            ));
        }
        items.push(item);
    }
    items.extend(workers.actions.iter().cloned());
    // The end: once all is merged and no worktree is left.
    let has_next_merge = next_merge(issues, remote, order).is_some();
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
    // A stale plan: a click writes the computed merge order into it.
    let plan = local.record.as_ref().and_then(|record| record.plan.as_ref());
    let states = states_of(issues, remote);
    let stale = if remote.is_some() {
        stale_of(plan, &states, order.is_off_plan)
    } else {
        Vec::new()
    };
    if !stale.is_empty() {
        let numbers: Vec<String> = order.open.iter().map(|item| format!("#{}", item.number)).collect();
        let why = if plan.is_some_and(|plan| !plan.merge.is_empty()) {
            format!("the plan is stale: {}", stale.join("; "))
        } else {
            "no merge order in the plan yet".into()
        };
        items.push(Item {
            key: format!(
                "order:{}:{}:{}",
                local.name,
                order.planned.iter().map(i64::to_string).collect::<Vec<_>>().join(","),
                stale.join(";")
            ),
            kind: "order".into(),
            text: format!(
                "{why}: {} · click to write it into the plan (wt:plan)",
                numbers.join(" → ")
            ),
            who: format!("{ORCHESTRATOR} orchestrator"),
            is_auto: true,
            rank: -1,
            go: Go::WritePlan(updated_plan(plan, &states, &order.planned)),
            ..Default::default()
        });
    }
    let (actions, up_next) = order_actions(items);
    ActionsSection { actions, up_next }
}

/// The MR next in the merge order: the first nothing holds up, else the first and what holds it.
fn next_merge(issues: &[Entry], remote: Option<&Remote>, order: &MergeOrder) -> Option<NextMerge> {
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
    let ready: Vec<_> = open.iter().filter(|item| item.hold_up.is_empty()).collect();
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

/// MERGE REQUESTS: the open ones as a graph in the merge order, the merged below.
fn merge_requests_of(
    issues: &[Entry],
    remote: &Remote,
    order: &MergeOrder,
    agents: &HashMap<String, Agent>,
    resolves: &mut Resolves,
    done: &HashSet<i64>,
    now: i64,
) -> MergeRequests {
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
        } else {
            let boxes = part.issue.as_ref().map(|found| &found.boxes[..]).unwrap_or(&[]);
            let after = issue
                .number
                .and_then(|number| after_of.get(&number).copied())
                .unwrap_or(&[]);
            row_blockers_of(mr, boxes, behind, after, &part.jobs, tree, done)
        };
        let status = if is_merged {
            let when = mr
                .merged_at
                .as_deref()
                .and_then(crate::time::parse_iso)
                .map(|at| ago(at, now))
                .unwrap_or_default();
            Some(Span::dim(format!("merged {when}").trim_end().to_string()))
        } else if reasons.is_empty() {
            Some(Span::new("ready to merge", super::Paint::Green))
        } else {
            None
        };
        MrFacts {
            iid: mr.iid,
            url: mr.url.clone(),
            pipeline: pipeline_mark(mr.pipeline.as_ref()),
            title: mr.title.clone(),
            branch: issue.branch.clone(),
            dir: tree.map(|tree| tree.dir.clone()),
            is_merged,
            status,
            conflict: (!is_merged).then(|| resolves.state_of(mr, tree, agents, behind, now)),
            reasons: reasons.into_iter().filter(|reason| !reason.is_conflict).collect(),
            progress: if is_merged {
                None
            } else {
                part.issue.as_ref().and_then(|found| progress_of(&found.boxes))
            },
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
            },
        )
    })
    .unwrap_or_default();
    model.orders = workers.orders.clone();
    model.holds = workers.holds.clone();
    model.actions = section("ACTIONS", || {
        actions_of(local, &issues, remote, &order, &workers, state, now)
    });
    model.stale = if remote.is_some() {
        stale_of(plan, &states_of(&issues, remote), order.is_off_plan)
    } else {
        Vec::new()
    };
    let merge = remote.and_then(|_| next_merge(&issues, remote, &order));
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
            None if remote.is_none() => NextBand::Nothing("GitLab …".into()),
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
                NextBand::Nothing(nothing_text(at_work, merge.as_ref()))
            }
        };
        NextSection { band, why }
    });
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
    let merge_numbers: Vec<i64> = order.open.iter().map(|item| item.number).collect();
    model.worktrees = section("WORKTREES", || {
        worktrees_of(
            &local.name,
            &issues,
            remote,
            &TreeFacts {
                agents: Some(&local.agents),
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
                setups: &local.setups,
                now,
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
            merge_requests_of(&issues, remote, &order, &local.agents, resolves, &done, now)
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
                    merge: vec![1, 2, 3],
                    why: Some("#1 first".into()),
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
        assert_eq!(model.next.unwrap().band, NextBand::Nothing("GitLab …".into()));
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
}
