//! `--herdr`: the main space's overview keeps herdr in step (overview.mjs
//! `keepInStep`), after the first read and after every local read, never two
//! at once: the conversation of each worktree's space and each
//! orchestrator's written down for a restart to resume, each agent's line in
//! the sidebar, a notification when a worktree needs you; the spaces of
//! removed worktrees pruned and the group's order kept on every read, the
//! whole layout (wt:herdr's, `cli::lifecycle::layout`, in this process) when the set of worktrees
//! changed, a stack's tabs when it came up or went down.

use super::layout_log::{Logged, said};
use super::{Effect, glab, herdr};
use crate::cli::lifecycle::herdr::journaled;
use crate::cli::lifecycle::layout;
use crate::cli::lifecycle::layout::spaces::space_of;
use crate::cli::lifecycle::repo::Repo;
use crate::tui::overview::Overview;
use breadboard_core::git::feature_of;
use breadboard_core::gitlab::{Mr, Pipeline};
use breadboard_core::herdr::{Pane, all_panes, spaces, tabs_of};
use breadboard_core::local::ORCHESTRATOR_TAB;
use breadboard_core::model::agent_state::{Facts, NextIssue, agent_state_of, sidebar_line_of};
use breadboard_core::model::change::is_change_open;
use breadboard_core::model::merge::checks_of;
use breadboard_core::model::{Paint, Tree};
use breadboard_core::reader::{Local, RemoteOf};
use breadboard_core::record::{SPACE, orchestration_of_label, orchestrator_label, record_of, recorded, titles};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The source breadboard reports its tokens under (the plugin's id), and the tokens.
use breadboard_core::record::PLUGIN as SOURCE;
const STATE_TOKEN: &str = "wt_state";
const REPO_TOKEN: &str = "wt_repo";
/// Tokens of an older breadboard, always cleared.
const GONE: [&str; 2] = ["wt_head", "wt_tree"];

/// A pane's tokens with their values, None to clear, in the order they are reported.
type Tokens = Vec<(String, Option<String>)>;

/// An open MR as the sync needs it: its branch, iid, pipeline (its status and the commit it ran for).
type OpenMr = (String, i64, Option<Pipeline>);

/// What a worktree looked like at the last tick, for its notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    is_asking: bool,
    mr: Option<(i64, Option<String>)>,
    has_remote: bool,
}

/// What the sync remembers between ticks, shared with its job.
#[derive(Debug, Default)]
struct Memory {
    /// pane → its tokens, as reported last.
    reported: HashMap<String, Tokens>,
    /// The panes reported to: one not in it gets every token, clears too.
    known: HashSet<String>,
    /// worktree → its state at the last tick.
    last: HashMap<PathBuf, Seen>,
}

#[derive(Debug, Default)]
pub struct Sync {
    is_busy: Arc<AtomicBool>,
    /// A read came in while a tick was busy: one more tick as soon as it ends
    /// (Node's `r` during a read queues one more), so GitLab's answer is never
    /// left waiting for the next local change or poll.
    is_due: bool,
    memory: Arc<Mutex<Memory>>,
    /// The worktrees and whether their stack runs, at the last tick; None before the first.
    known: Option<BTreeMap<PathBuf, bool>>,
}

/// After a read of the view of everything with `--herdr`.
pub fn after_read(app: &mut Overview) {
    if !app.actor.is_sync {
        return;
    }
    if app.actor.sync.is_busy.load(Ordering::SeqCst) {
        app.actor.sync.is_due = true;
        return;
    }
    app.actor.sync.is_due = false;
    let Some(Local::Everything(local)) = app.local.clone() else {
        return;
    };
    let mrs: Option<Vec<OpenMr>> = match &app.remote {
        Some(RemoteOf::Everything(remote)) => Some(
            remote
                .mrs
                .iter()
                .map(|mr| (mr.branch.clone(), mr.iid, mr.pipeline.clone()))
                .collect(),
        ),
        _ => None,
    };
    let now: BTreeMap<PathBuf, bool> = local
        .trees
        .iter()
        .map(|tree| {
            (
                tree.dir.clone(),
                tree.stack.as_deref().is_some_and(|stack| stack.contains("running")),
            )
        })
        .collect();
    let before = app.actor.sync.known.replace(now.clone());
    // Only on a change: the read runs every 2 s, herdr's layout need not.
    let is_layout = before
        .as_ref()
        .is_none_or(|before| before.keys().collect::<Vec<_>>() != now.keys().collect::<Vec<_>>());
    let stacks: Vec<PathBuf> = match &before {
        Some(before) if !is_layout => now
            .iter()
            .filter(|(dir, is_up)| before.get(*dir) != Some(*is_up))
            .map(|(dir, _)| dir.clone())
            .collect(),
        _ => Vec::new(),
    };
    let lab = Lab::of(mrs.as_deref(), app.gitlab.error.is_some());
    let trees = local.trees.clone();
    let main = app.actor.main.clone();
    let memory = app.actor.sync.memory.clone();
    let is_busy = app.actor.sync.is_busy.clone();
    is_busy.store(true, Ordering::SeqCst);
    app.actor.queue(move || {
        // Free again however the job ends: a panic costs this tick, not every next one.
        let _free = Free(is_busy);
        let (mut logged, mut known) = tick(&main, &trees, mrs.as_deref(), lab, is_layout, &memory);
        if is_layout || !stacks.is_empty() {
            // One change of the layout at a time, with wt:herdr and the other overviews.
            let repo = Repo { main: main.clone() };
            // What the layout's prune removes has no line of its own either.
            let before = is_layout.then(|| (Leftovers::of(&main), titles(&main)));
            let (lines, journal) = journaled(|| {
                layout::exclusive(|| {
                    let mut lines = Vec::new();
                    if is_layout && let Some((_, report)) = layout::layout(&repo, false) {
                        lines.extend(report);
                    }
                    for dir in &stacks {
                        let changed = layout::sync_stack_tabs(&repo, dir, space_of(&repo, dir).as_deref());
                        if !changed.is_empty() {
                            lines.push(format!("{}: {}", feature_of(&main, dir), changed.join(" ")));
                        }
                    }
                    lines
                })
            });
            // The spaces it opened, by their ids, for its commands: asked now, never the shared answer from before them.
            if !journal.writes.is_empty() {
                known.extend(
                    breadboard_core::herdr::spaces_now()
                        .into_iter()
                        .map(|space| (space.id, space.label)),
                );
            }
            logged.extend(said(&lines, &journal, &known));
            if let Some((before, names)) = before {
                logged.extend(before.gone(&Leftovers::of(&main), &main, &names));
            }
        }
        Some(Box::new(move |app: &mut Overview| {
            log_acts(&mut app.actor, logged);
            // What came in meanwhile, at once: the newest local and GitLab read.
            if app.actor.sync.is_due {
                after_read(app);
            }
        }) as Effect)
    });
}

/// What a tick did into the log: an act the log already has (the same words
/// and commands) is counted on its entry, once for each time its commands
/// ran, whatever ticks came between; DONE's `· N times` is the number of
/// rounds of those calls (the world's herdr never drops a closed tab, so a
/// prune repeats on each tick that prunes).
fn log_acts(actor: &mut super::Actor, logged: Vec<Logged>) {
    for act in logged {
        let key = act.key();
        if !actor.again(&key) {
            actor.log_as(&key, act.paint, act.text, act.detail);
        }
    }
}

/// Where GitLab stands for the sidebar's lines. In: they say what it said.
/// Waiting (its first read under way): none written, the lines herdr holds
/// stay, so the first write is the right one, never a guess followed by a
/// second write (#262: `waits for !309` of a merged !309; even `at work`
/// gets its ` · !316` from GitLab). Out (its read failed, nothing came yet):
/// every line, of the facts that need no GitLab, an awaited MR named by none
/// of its iids (it may be merged), and `GitLab out, its MR unknown` where the
/// state hangs on an MR only GitLab knows (`free` of a branch ahead of
/// develop, a change whose fix is pushed), so a line herdr holds from before
/// never outlives its fact and none says a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lab {
    Waiting,
    Out,
    In,
}

impl Lab {
    fn of(mrs: Option<&[OpenMr]>, has_failed: bool) -> Lab {
        match (mrs, has_failed) {
            (Some(_), _) => Lab::In,
            (None, true) => Lab::Out,
            (None, false) => Lab::Waiting,
        }
    }
}

/// An agent's sidebar line as GitLab stands (`Lab`), None to leave it as herdr holds it.
fn line_of(tree: &Tree, facts: &Facts, lab: Lab) -> Option<String> {
    let state = agent_state_of(tree, facts);
    match (lab, state.key) {
        (Lab::In, _) => Some(sidebar_line_of(tree, facts)),
        (Lab::Waiting, _) => None,
        (Lab::Out, "waitsForMr") => Some("waits for other MRs".into()),
        (Lab::Out, "next") => Some(match facts.handover {
            Some(handover) if !handover.open.is_empty() => format!("→ next #{} after other MRs", handover.number),
            _ => state.text,
        }),
        (Lab::Out, key) if hangs_on_its_mr(tree, key) => Some(UNKNOWN_MR.into()),
        (Lab::Out, _) => Some(state.text),
    }
}

/// The sidebar's line of an agent whose state only its MR can tell while GitLab is out.
const UNKNOWN_MR: &str = "GitLab out, its MR unknown";

/// Whether an agent's `free` or `change requested` hangs on an MR GitLab
/// didn't tell: a branch ahead of develop may have one (its pipeline may run
/// or have failed), and a requested change whose fix is pushed is done once
/// its MR's pipeline passed on it.
fn hangs_on_its_mr(tree: &Tree, key: &str) -> bool {
    let passed = Mr {
        state: "opened".into(),
        pipeline: Some(Pipeline {
            status: "success".into(),
            sha: tree.pushed.clone(),
            ..Pipeline::default()
        }),
        ..Mr::default()
    };
    match key {
        "free" => tree.ahead > 0,
        "change" => !is_change_open(Some(tree), Some(&passed)),
        _ => false,
    }
}

/// Marks the sync free when dropped, at the end of its job.
struct Free(Arc<AtomicBool>);

impl Drop for Free {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// One tick: sessions, chats, the spaces pruned and sorted (not when the
/// whole layout follows, `is_layout`: it prunes and sorts itself, and a
/// second pass in one tick would close what the first just closed), the
/// sidebar's lines as GitLab stands (`Lab`), the notifications.
/// What it did, for the log, and the spaces it read (id → label) for the
/// layout's commands.
fn tick(
    main: &Path,
    trees: &[Tree],
    mrs: Option<&[OpenMr]>,
    lab: Lab,
    is_layout: bool,
    memory: &Mutex<Memory>,
) -> (Vec<Logged>, HashMap<String, String>) {
    let mut logged = Vec::new();
    let panes = all_panes();
    let all_spaces = spaces();
    let known: HashMap<String, String> = all_spaces
        .iter()
        .map(|space| (space.id.clone(), space.label.clone()))
        .collect();
    let names = titles(main);
    remember_chats(main, &panes, &all_spaces, &names, &mut logged);
    let repo = Repo {
        main: main.to_path_buf(),
    };
    if !is_layout {
        let before = Leftovers::of(main);
        let (lines, journal) = journaled(|| {
            layout::exclusive(|| {
                let mut lines = layout::prune_spaces(&repo);
                // Cheap when the order stands: a plan or a handover may have changed it (#255).
                lines.extend(layout::orchestrations::sort_spaces(&repo));
                lines
            })
        });
        logged.extend(said(&lines, &journal, &known));
        logged.extend(before.gone(&Leftovers::of(main), main, &names));
    }
    let mut memory = memory.lock().unwrap_or_else(|e| e.into_inner());
    report_states(main, trees, mrs, lab, &panes, &all_spaces, &mut memory);
    notify_changes(main, trees, mrs, &panes, &all_spaces, &names, &mut memory, &mut logged);
    (logged, known)
}

/// The records and checkouts of the orchestrations, before and after a
/// prune: what it removed has no line of its own (`pruneOrchestrators`).
struct Leftovers {
    records: BTreeSet<String>,
    checkouts: BTreeSet<String>,
}

impl Leftovers {
    fn of(main: &Path) -> Leftovers {
        let dir = breadboard_core::git::orchestrations_dir(main);
        let checkouts = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        Leftovers {
            records: recorded(main).into_iter().collect(),
            checkouts,
        }
    }

    /// What went between `self` and `now`, in words: `removed the record of ⇶ x`.
    fn gone(&self, now: &Leftovers, main: &Path, names: &[(String, String)]) -> Vec<Logged> {
        let label = |name: &str| match names.iter().find(|(known, _)| known == name) {
            Some((_, title)) => format!("⇶ {title}"),
            None => orchestrator_label(main, name),
        };
        let dir = breadboard_core::git::orchestrations_dir(main);
        let mut out = Vec::new();
        for name in self.records.difference(&now.records) {
            out.push(Logged::new(
                Paint::Green,
                format!(
                    "removed the record of {} · neither a worktree nor a space of it is left",
                    label(name)
                ),
                Some(format!(
                    "rm {}",
                    herdr::quote(&dir.join(format!("{name}.json")).to_string_lossy())
                )),
            ));
        }
        for name in self.checkouts.difference(&now.checkouts) {
            out.push(Logged::new(
                Paint::Green,
                format!("removed the checkout of {} · its worktrees are gone", label(name)),
                Some(format!(
                    "git worktree remove {}",
                    herdr::quote(&dir.join(name).to_string_lossy())
                )),
            ));
        }
        out
    }
}

/// The orchestrations of a space: a product's every one (#405), else the one of `⇶ <title>`.
fn orchestrations_of(main: &Path, label: &str, names: &[(String, String)]) -> Vec<String> {
    if breadboard_core::record::is_product(main, label) {
        breadboard_core::record::orchestrations_of_product(main, label)
    } else {
        orchestration_of_label(label, names).into_iter().collect()
    }
}

/// Every orchestration's chat written down (`rememberChats`): the claude of
/// its tab `orchestrator`; a lone claude in a space without it gets that name.
fn remember_chats(
    main: &Path,
    panes: &[Pane],
    all_spaces: &[breadboard_core::herdr::Space],
    names: &[(String, String)],
    logged: &mut Vec<Logged>,
) {
    for space in all_spaces {
        // A product's space (#405) has one chat for its orchestrations: written as the first's, into the product's.
        let Some(name) = orchestrations_of(main, &space.label, names).into_iter().next() else {
            continue;
        };
        if record_of(main, &name).is_none() {
            continue;
        }
        let tabs = tabs_of(&space.id);
        let own: Vec<Pane> = panes
            .iter()
            .filter(|pane| pane.workspace == space.id)
            .cloned()
            .collect();
        let free: Vec<&str> = tabs
            .iter()
            .filter(|tab| tab.label.starts_with("free "))
            .map(|tab| tab.id.as_str())
            .collect();
        let claudes: Vec<&Pane> = own
            .iter()
            .filter(|pane| {
                pane.agent.as_deref() == Some("claude") && pane.session.is_some() && !free.contains(&pane.tab.as_str())
            })
            .collect();
        let tab = tabs.iter().find(|tab| tab.label == ORCHESTRATOR_TAB);
        let pane = match tab {
            Some(tab) => claudes.iter().find(|pane| pane.tab == tab.id).copied(),
            None => (claudes.len() == 1).then(|| claudes[0]),
        };
        let Some(pane) = pane else { continue };
        if tab.is_none() && herdr::did(&["tab", "rename", &pane.tab, ORCHESTRATOR_TAB]) {
            logged.push(Logged::new(
                Paint::Green,
                format!(
                    "named the tab of the chat of {} {ORCHESTRATOR_TAB} · its lone claude",
                    space.label
                ),
                Some(format!("herdr tab rename {} {ORCHESTRATOR_TAB}", pane.tab)),
            ));
        }
        let cwd = breadboard_core::conversations::running_claudes()
            .into_iter()
            .find(|claude| Some(&claude.id) == pane.session.as_ref())
            .map(|claude| claude.cwd)
            .or_else(|| pane.cwd.clone().map(PathBuf::from))
            .unwrap_or_else(|| main.to_path_buf());
        super::chat::record_chat(main, &name, pane.session.clone().map(|id| (id, cwd)));
    }
}

/// Each agent's tokens in herdr (`reportStates`): its line in a worktree's
/// space, its repo in a linked worktree's; only what changed, all to a pane
/// first seen; legacy tokens cleared.
fn report_states(
    main: &Path,
    trees: &[Tree],
    mrs: Option<&[OpenMr]>,
    lab: Lab,
    panes: &[Pane],
    all_spaces: &[breadboard_core::herdr::Space],
    memory: &mut Memory,
) {
    let agents: Vec<&Pane> = panes.iter().filter(|pane| pane.agent.is_some()).collect();
    let open: HashSet<i64> = mrs.unwrap_or_default().iter().map(|(_, iid, _)| *iid).collect();
    let mut lines: HashMap<String, String> = HashMap::new();
    // Panes whose line waits for GitLab's first answer: left as herdr holds them.
    let mut held: HashSet<String> = HashSet::new();
    let claudes = breadboard_core::local::claudes();
    // A headless agent has no pane (#361): its line goes onto its worktree's
    // space, its state the status table's.
    let headless = breadboard_core::runs::agents_of(main);
    let mut hosts: Vec<String> = Vec::new();
    for listed in trees.iter().filter(|tree| tree.dir != main) {
        let label = herdr::space_label(main, &listed.dir);
        let Some(space) = all_spaces.iter().find(|space| space.label == label) else {
            continue;
        };
        let mut own: Vec<(String, Option<String>)> = agents
            .iter()
            .filter(|pane| pane.workspace == space.id)
            .map(|pane| (pane.id.clone(), pane.status.clone()))
            .collect();
        if own.is_empty()
            && let Some(agent) =
                breadboard_core::conversations::bound_session(&listed.dir).and_then(|id| headless.get(&id))
        {
            // The sidebar shows agent rows for agent panes only: the space's row says it.
            let host = format!("{SPACE_KEY}{}", space.id);
            own.push((host.clone(), agent.status.clone()));
            hosts.push(host);
        }
        if own.is_empty() {
            continue;
        }
        // git answered from the reader's memo of the worktree while its key holds (daemon round 4).
        let tree = breadboard_core::reader::with_orchestration_facts_remembered(listed.clone(), &claudes);
        let mr = mrs
            .and_then(|mrs| mrs.iter().find(|(branch, _, _)| *branch == tree.branch))
            .map(|(_, iid, pipeline)| Mr {
                iid: *iid,
                state: "opened".into(),
                pipeline: pipeline.clone(),
                ..Mr::default()
            });
        // Before GitLab's first answer an awaited MR counts as open.
        let merged: HashSet<i64> = match mrs {
            Some(_) => tree
                .wait
                .as_ref()
                .map(|wait| wait.mrs.iter().copied().filter(|iid| !open.contains(iid)).collect())
                .unwrap_or_default(),
            None => HashSet::new(),
        };
        let handover = handover_of(main, &tree, mrs.map(|_| &open));
        let checks = checks_of(&[], &tree.actions, &tree.answered);
        // For the commit GitLab's pipeline ran on, while what it waits for is open; else no merge is claimed.
        let turn = tree.merge_turn.as_ref().and_then(|record| {
            let sha = mr
                .as_ref()
                .and_then(|mr| mr.pipeline.as_ref())
                .and_then(|pipeline| pipeline.sha.as_deref());
            record.turn_now(sha, &open)
        });
        for (pane, status) in own {
            let facts = Facts {
                mr: mr.as_ref(),
                status: status.as_deref(),
                merged: Some(&merged),
                handover: handover.as_ref(),
                checks: &checks,
                // The merge order isn't known here: the orchestration's overview left the turn (#344).
                turn: turn.as_ref(),
                blocker: None,
            };
            match line_of(&tree, &facts, lab) {
                Some(line) => {
                    lines.insert(pane, line);
                }
                None => {
                    held.insert(pane);
                }
            }
        }
    }
    let repos = repos_of(all_spaces, &agents);
    let mut ids: HashSet<String> = agents.iter().map(|pane| pane.id.clone()).collect();
    ids.extend(hosts);
    ids.extend(memory.reported.keys().cloned());
    // A held pane gets all its tokens in one write once GitLab answers, as Node's one write.
    for pane in ids.iter().filter(|pane| !held.contains(*pane)) {
        // In Node's order: the state, the repo, the gone ones.
        let mut tokens: Tokens = vec![
            (STATE_TOKEN.into(), lines.get(pane).cloned()),
            (REPO_TOKEN.into(), repos.get(pane).cloned()),
        ];
        tokens.extend(GONE.iter().map(|gone| (gone.to_string(), None)));
        report(pane, tokens, memory);
    }
    // A pane that is gone is forgotten: herdr may give its id to a new one.
    memory.known.retain(|pane| ids.contains(pane));
}

/// The handover announced for the agent of a worktree (agent-state.mjs
/// `handoverOf`): from the record of the orchestration that started it, by
/// the names its agent goes by; `open`: the iids GitLab has open, None
/// before it answered (every awaited one counts as open then).
fn handover_of(main: &Path, tree: &Tree, open: Option<&HashSet<i64>>) -> Option<NextIssue> {
    let record = record_of(main, &tree.origin.as_ref()?.name)?;
    let worker = tree
        .claude
        .as_ref()
        .and_then(|claude| record.workers.iter().find(|worker| worker.id == claude.id));
    let names: Vec<&str> = [
        Some(tree.name.as_str()),
        worker.and_then(|worker| worker.name.as_deref()),
        worker.and_then(|worker| worker.was.as_deref()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let handover = record
        .handovers
        .iter()
        .find(|handover| names.contains(&handover.to.as_str()))?;
    Some(NextIssue {
        number: handover.number,
        open: handover
            .after
            .iter()
            .copied()
            .filter(|iid| open.is_none_or(|open| open.contains(iid)))
            .collect(),
    })
}

/// pane → the repo of its agent, for one in a linked worktree's space (`repoOfAgents`).
fn repos_of(all_spaces: &[breadboard_core::herdr::Space], agents: &[&Pane]) -> HashMap<String, String> {
    let listed = herdr::ask(&["workspace", "list"])
        .and_then(|result| result.get("workspaces").and_then(|list| list.as_array()).cloned())
        .unwrap_or_default();
    let worktree_of = |id: &str| {
        listed
            .iter()
            .find(|space| space.get("workspace_id").and_then(|v| v.as_str()) == Some(id))
            .and_then(|space| space.get("worktree"))
            .cloned()
    };
    let repos: HashMap<String, String> = all_spaces
        .iter()
        .filter_map(|space| {
            let worktree = worktree_of(&space.id)?;
            let is_linked = worktree
                .get("is_linked_worktree")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let root = worktree.get("repo_root").and_then(|v| v.as_str())?;
            (!is_linked).then(|| (root.to_string(), space.label.clone()))
        })
        .collect();
    agents
        .iter()
        .filter_map(|agent| {
            let worktree = worktree_of(&agent.workspace)?;
            if !worktree
                .get("is_linked_worktree")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return None;
            }
            let repo = repos.get(worktree.get("repo_root").and_then(|v| v.as_str())?)?;
            repo.starts_with(&SPACE.get()).then(|| (agent.id.clone(), repo.clone()))
        })
        .collect()
}

/// The tokens of one pane: set what changed, clear what went.
/// What a reported key starts with when it is a space, not a pane: a
/// headless agent's line goes onto its worktree's space (#361).
const SPACE_KEY: &str = "space:";

fn report(pane: &str, tokens: Tokens, memory: &mut Memory) {
    let is_new = memory.known.insert(pane.to_string());
    let before = memory.reported.get(pane).cloned().unwrap_or_default();
    let was = |token: &str| {
        before
            .iter()
            .find(|(known, _)| known == token)
            .and_then(|(_, value)| value.clone())
    };
    let mut args: Vec<String> = Vec::new();
    for (token, value) in &tokens {
        if !is_new && was(token) == *value {
            continue;
        }
        match value {
            Some(value) => args.extend(["--token".into(), format!("{token}={value}")]),
            None => args.extend(["--clear-token".into(), token.clone()]),
        }
    }
    if !args.is_empty() {
        let source = SOURCE.get();
        let mut all = match pane.strip_prefix(SPACE_KEY) {
            Some(space) => vec!["workspace", "report-metadata", space, "--source", &source],
            None => vec!["pane", "report-metadata", pane, "--source", &source],
        };
        all.extend(args.iter().map(String::as_str));
        herdr::did(&all);
    }
    if tokens.iter().any(|(_, value)| value.is_some()) {
        memory.reported.insert(pane.to_string(), tokens);
    } else {
        memory.reported.remove(pane);
    }
}

/// Notifications when a worktree needs you (`notifyChanges`): its agent
/// asks, its pipeline failed, its MR got merged; on a change only, not for a
/// worktree whose orchestration's space is open (its overview notifies).
#[allow(clippy::too_many_arguments)]
fn notify_changes(
    main: &Path,
    trees: &[Tree],
    mrs: Option<&[OpenMr]>,
    panes: &[Pane],
    all_spaces: &[breadboard_core::herdr::Space],
    names: &[(String, String)],
    memory: &mut Memory,
    logged: &mut Vec<Logged>,
) {
    let open: HashSet<String> = all_spaces
        .iter()
        .flat_map(|space| orchestrations_of(main, &space.label, names))
        .collect();
    for tree in trees.iter().filter(|tree| tree.dir != main) {
        if tree.origin.as_ref().is_some_and(|origin| open.contains(&origin.name)) {
            continue;
        }
        let label = herdr::space_label(main, &tree.dir);
        let workspace = all_spaces
            .iter()
            .find(|space| space.label == label)
            .map(|space| &space.id);
        let now = Seen {
            is_asking: panes.iter().any(|pane| {
                Some(&pane.workspace) == workspace && pane.agent.is_some() && pane.status.as_deref() == Some("blocked")
            }),
            mr: mrs
                .and_then(|mrs| mrs.iter().find(|(branch, _, _)| *branch == tree.branch))
                .map(|(_, iid, pipeline)| (*iid, pipeline.as_ref().map(|pipeline| pipeline.status.clone()))),
            has_remote: mrs.is_some(),
        };
        let before = memory.last.insert(tree.dir.clone(), now.clone());
        let feature = feature_of(main, &tree.dir);
        for (body, sound) in events_of(before.as_ref(), &now, |iid| {
            glab::get(main, &glab::mr_endpoint(iid))
                .is_ok_and(|mr| mr.get("state").and_then(|state| state.as_str()) == Some("merged"))
        }) {
            if herdr::notify(&feature, &body, sound) {
                logged.push(Logged::new(
                    Paint::Green,
                    format!("notified you about {feature}: {body}"),
                    Some(format!(
                        "herdr notification show {} --body {} --sound {sound}",
                        herdr::quote(&feature),
                        herdr::quote(&body)
                    )),
                ));
            }
        }
    }
    let current: HashSet<&PathBuf> = trees.iter().map(|tree| &tree.dir).collect();
    memory.last.retain(|dir, _| current.contains(dir));
}

/// The notifications of one worktree between two ticks (`eventsOf`); none at the first.
fn events_of(before: Option<&Seen>, now: &Seen, was_merged: impl Fn(i64) -> bool) -> Vec<(String, &'static str)> {
    let Some(before) = before else { return Vec::new() };
    let mut events = Vec::new();
    if now.is_asking && !before.is_asking {
        events.push(("its agent asks you something".to_string(), "request"));
    }
    if before.has_remote && now.has_remote {
        if let (Some((was, was_status)), Some((iid, status))) = (&before.mr, &now.mr)
            && was == iid
            && status.as_deref() == Some("failed")
            && was_status.as_deref() != Some("failed")
        {
            events.push((format!("the pipeline of !{iid} failed"), "request"));
        }
        if let (Some((was, _)), None) = (&before.mr, &now.mr)
            && was_merged(*was)
        {
            events.push((format!("!{was} is merged"), "done"));
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(is_asking: bool, mr: Option<(i64, &str)>) -> Seen {
        Seen {
            is_asking,
            mr: mr.map(|(iid, status)| (iid, Some(status.to_string()))),
            has_remote: true,
        }
    }

    #[test]
    fn notifies_what_changed_never_at_the_first_tick() {
        let now = seen(true, Some((5, "failed")));
        assert!(events_of(None, &now, |_| true).is_empty());
        let events = events_of(Some(&seen(false, Some((5, "running")))), &now, |_| true);
        assert_eq!(
            events,
            [
                ("its agent asks you something".to_string(), "request"),
                ("the pipeline of !5 failed".to_string(), "request")
            ]
        );
        let gone = events_of(Some(&seen(false, Some((5, "success")))), &seen(false, None), |iid| {
            iid == 5
        });
        assert_eq!(gone, [("!5 is merged".to_string(), "done")]);
        assert!(events_of(Some(&seen(false, Some((5, "success")))), &seen(false, None), |_| false).is_empty());
    }

    #[test]
    fn counts_an_act_once_for_each_round_of_its_calls() {
        let mut actor = super::super::Actor::new(
            &breadboard_core::reader::View::Everything,
            Path::new("/r"),
            false,
            false,
        );
        let close = || {
            Logged::new(
                Paint::Green,
                "closed 8 tabs of 299-x · its worktree is gone",
                Some("herdr tab close w4P:t1; herdr tab close w4P:t2".into()),
            )
        };
        let other = || Logged::new(Paint::Green, "wrote down the conversation of 307-x", None);
        // Round 1, a tick with something else, a tick with nothing, round 2, then round 3 twice in one tick.
        log_acts(&mut actor, vec![close()]);
        log_acts(&mut actor, vec![other()]);
        log_acts(&mut actor, vec![]);
        log_acts(&mut actor, vec![close()]);
        log_acts(&mut actor, vec![close(), close()]);
        let closes: Vec<&super::super::Did> = actor.did.iter().filter(|did| did.text.starts_with("closed")).collect();
        assert_eq!(closes.len(), 1, "one entry");
        assert_eq!(closes[0].times, 4, "as often as its calls ran");
        assert_eq!(actor.did.len(), 2);
    }

    #[test]
    fn writes_no_guess_before_gitlab_answers_and_no_awaited_iid_while_it_is_out() {
        assert_eq!(Lab::of(None, false), Lab::Waiting);
        assert_eq!(Lab::of(None, true), Lab::Out);
        assert_eq!(Lab::of(Some(&[]), true), Lab::In);
        let waiting = Tree {
            wait: Some(breadboard_core::wait::Wait {
                mrs: vec![309, 310, 311],
                ..Default::default()
            }),
            ..Default::default()
        };
        let none = HashSet::new();
        let facts = |status| Facts {
            status,
            merged: Some(&none),
            ..Default::default()
        };
        // The first read under way: nothing, not even `at work` (GitLab adds its MR).
        assert_eq!(line_of(&waiting, &facts(None), Lab::Waiting), None);
        assert_eq!(line_of(&Tree::default(), &facts(Some("working")), Lab::Waiting), None);
        assert_eq!(
            line_of(&Tree::default(), &facts(Some("working")), Lab::Out).as_deref(),
            Some("at work")
        );
        // GitLab out: every line, no awaited MR counted open (#262).
        assert_eq!(
            line_of(&waiting, &facts(None), Lab::Out).as_deref(),
            Some("waits for other MRs")
        );
        // `free` only where no MR can be (nothing ahead of develop); else GitLab is said out.
        assert_eq!(
            line_of(&Tree::default(), &facts(None), Lab::Out).as_deref(),
            Some("free")
        );
        let ahead = Tree {
            ahead: 1,
            pushed: Some("b".into()),
            ..Default::default()
        };
        assert_eq!(line_of(&ahead, &facts(None), Lab::Out).as_deref(), Some(UNKNOWN_MR));
        assert_eq!(line_of(&ahead, &facts(None), Lab::In).as_deref(), Some("free"));
        // A requested change: open while its fix isn't pushed, unknown once it is.
        let change = |head: &str| breadboard_core::actions::Change {
            text: "rename it".into(),
            head: Some(head.into()),
            ..Default::default()
        };
        let unfixed = Tree {
            change: Some(change("b")),
            ..ahead.clone()
        };
        assert_eq!(
            line_of(&unfixed, &facts(None), Lab::Out).as_deref(),
            Some("change requested")
        );
        let fixed = Tree {
            change: Some(change("a")),
            ..ahead.clone()
        };
        assert_eq!(line_of(&fixed, &facts(None), Lab::Out).as_deref(), Some(UNKNOWN_MR));
        let handover = NextIssue {
            number: 226,
            open: vec![232],
        };
        let next = Facts {
            handover: Some(&handover),
            ..facts(None)
        };
        assert_eq!(
            line_of(&Tree::default(), &next, Lab::Out).as_deref(),
            Some("→ next #226 after other MRs")
        );
        // GitLab in: its words, the merged ones left out.
        let merged: HashSet<i64> = [309, 310].into();
        let known = Facts {
            merged: Some(&merged),
            ..facts(None)
        };
        assert_eq!(
            line_of(&waiting, &known, Lab::In).as_deref(),
            Some("waits for !311 · 2 merged")
        );
    }

    #[test]
    fn reports_only_what_changed_all_to_a_new_pane() {
        let mut memory = Memory::default();
        let tokens = |state: Option<&str>| vec![(STATE_TOKEN.to_string(), state.map(String::from))];
        // Outside herdr nothing is called; the memory still follows.
        report("p1", tokens(Some("at work")), &mut memory);
        assert!(memory.known.contains("p1"));
        assert_eq!(memory.reported["p1"][0].1.as_deref(), Some("at work"));
        report("p1", tokens(None), &mut memory);
        assert!(!memory.reported.contains_key("p1"));
    }
}
