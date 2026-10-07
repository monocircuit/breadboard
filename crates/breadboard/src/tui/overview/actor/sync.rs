//! `--herdr`: the main space's overview keeps herdr in step (overview.mjs
//! `keepInStep`), after the first read and after every local read, never two
//! at once: the conversation of each worktree's space and each
//! orchestrator's written down for a restart to resume, each agent's line in
//! the sidebar, a notification when a worktree needs you; the spaces of
//! removed worktrees pruned and the group's order kept on every read, the
//! whole layout (wt:herdr's, `cli::lifecycle::layout`, in this process) when the set of worktrees
//! changed, a stack's tabs when it came up or went down.

use super::{Effect, glab, herdr};
use crate::cli::lifecycle::layout;
use crate::cli::lifecycle::layout::spaces::space_of;
use crate::cli::lifecycle::repo::Repo;
use crate::tui::overview::Overview;
use breadboard_core::conversations::{bound_session, session_file};
use breadboard_core::git::feature_of;
use breadboard_core::gitlab::Mr;
use breadboard_core::herdr::{Pane, all_panes, spaces, tabs_of};
use breadboard_core::local::ORCHESTRATOR_TAB;
use breadboard_core::model::agent_state::{Facts, sidebar_line_of};
use breadboard_core::model::merge::checks_of;
use breadboard_core::model::{Paint, Tree};
use breadboard_core::reader::{Local, RemoteOf};
use breadboard_core::record::{SPACE, orchestration_of_label, record_of, titles};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The source breadboard reports its tokens under, and the tokens.
const SOURCE: &str = "monocircuit.platform";
const STATE_TOKEN: &str = "wt_state";
const REPO_TOKEN: &str = "wt_repo";
/// Tokens of an older breadboard, always cleared.
const GONE: [&str; 2] = ["wt_head", "wt_tree"];

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
    /// pane → token → value, as reported last.
    reported: HashMap<String, BTreeMap<String, Option<String>>>,
    /// The panes reported to: one not in it gets every token, clears too.
    known: HashSet<String>,
    /// worktree → its state at the last tick.
    last: HashMap<PathBuf, Seen>,
}

#[derive(Debug, Default)]
pub struct Sync {
    is_busy: Arc<AtomicBool>,
    memory: Arc<Mutex<Memory>>,
    /// The worktrees and whether their stack runs, at the last tick; None before the first.
    known: Option<BTreeMap<PathBuf, bool>>,
}

/// After a read of the view of everything with `--herdr`.
pub fn after_read(app: &mut Overview) {
    if !app.actor.is_sync || app.actor.sync.is_busy.load(Ordering::SeqCst) {
        return;
    }
    let Some(Local::Everything(local)) = app.local.clone() else {
        return;
    };
    let mrs: Option<Vec<(String, i64, Option<String>)>> = match &app.remote {
        Some(RemoteOf::Everything(remote)) => Some(
            remote
                .mrs
                .iter()
                .map(|mr| {
                    (
                        mr.branch.clone(),
                        mr.iid,
                        mr.pipeline.as_ref().map(|pipeline| pipeline.status.clone()),
                    )
                })
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
    let trees = local.trees.clone();
    let main = app.actor.main.clone();
    let memory = app.actor.sync.memory.clone();
    let is_busy = app.actor.sync.is_busy.clone();
    is_busy.store(true, Ordering::SeqCst);
    app.actor.queue(move || {
        let said = tick(&main, &trees, mrs.as_deref(), &memory);
        // One change of the layout at a time, with wt:herdr and the other overviews.
        let repo = Repo { main: main.clone() };
        layout::exclusive(|| {
            if is_layout {
                layout::layout(&repo, false);
            }
            for dir in &stacks {
                layout::sync_stack_tabs(&repo, dir, space_of(&repo, dir).as_deref());
            }
        });
        is_busy.store(false, Ordering::SeqCst);
        Some(Box::new(move |app: &mut Overview| {
            for (paint, text) in said {
                app.actor.log(paint, text);
            }
        }) as Effect)
    });
}

/// One tick: sessions, chats, the sidebar's lines, the notifications. What it did, for the log.
fn tick(
    main: &Path,
    trees: &[Tree],
    mrs: Option<&[(String, i64, Option<String>)]>,
    memory: &Mutex<Memory>,
) -> Vec<(Paint, String)> {
    let mut said = Vec::new();
    let panes = all_panes();
    let all_spaces = spaces();
    let names = titles(main);
    remember_sessions(main, trees, &panes, &all_spaces, &mut said);
    remember_chats(main, &panes, &all_spaces, &names, &mut said);
    let repo = Repo {
        main: main.to_path_buf(),
    };
    layout::exclusive(|| {
        layout::prune_spaces(&repo);
        // Cheap when the order stands: a plan or a handover may have changed it (#255).
        layout::orchestrations::sort_spaces(&repo);
    });
    let mut memory = memory.lock().unwrap_or_else(|e| e.into_inner());
    report_states(main, trees, mrs, &panes, &all_spaces, &mut memory);
    notify_changes(main, trees, mrs, &panes, &all_spaces, &names, &mut memory, &mut said);
    said
}

/// Each worktree's conversation bound to it (`rememberSessions`): the claude
/// panes of its space; a bound id none of them runs is replaced by the first.
fn remember_sessions(
    main: &Path,
    trees: &[Tree],
    panes: &[Pane],
    all_spaces: &[breadboard_core::herdr::Space],
    said: &mut Vec<(Paint, String)>,
) {
    for tree in trees.iter().filter(|tree| tree.dir != main) {
        // Never into a worktree being removed.
        if !tree.dir.join(".git").exists() || !tree.dir.join(".claude").is_dir() {
            continue;
        }
        let label = feature_of(main, &tree.dir);
        let Some(space) = all_spaces.iter().find(|space| space.label == label) else {
            continue;
        };
        let ids: Vec<&str> = panes
            .iter()
            .filter(|pane| pane.workspace == space.id && pane.agent.as_deref() == Some("claude"))
            .filter_map(|pane| pane.session.as_deref())
            .collect();
        let Some(first) = ids.first() else { continue };
        if bound_session(&tree.dir).is_some_and(|bound| ids.contains(&bound.as_str())) {
            continue;
        }
        if std::fs::write(session_file(&tree.dir), format!("{first}\n")).is_ok() {
            said.push((Paint::Dim, format!("{label}: its conversation {first} written down")));
        }
    }
}

/// Every orchestration's chat written down (`rememberChats`): the claude of
/// its tab `orchestrator`; a lone claude in a space without it gets that name.
fn remember_chats(
    main: &Path,
    panes: &[Pane],
    all_spaces: &[breadboard_core::herdr::Space],
    names: &[(String, String)],
    said: &mut Vec<(Paint, String)>,
) {
    for space in all_spaces {
        let Some(name) = orchestration_of_label(&space.label, names) else {
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
            said.push((
                Paint::Dim,
                format!("{}: its chat's tab named {ORCHESTRATOR_TAB}", space.label),
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
    mrs: Option<&[(String, i64, Option<String>)]>,
    panes: &[Pane],
    all_spaces: &[breadboard_core::herdr::Space],
    memory: &mut Memory,
) {
    let agents: Vec<&Pane> = panes.iter().filter(|pane| pane.agent.is_some()).collect();
    let open: HashSet<i64> = mrs.unwrap_or_default().iter().map(|(_, iid, _)| *iid).collect();
    let mut lines: HashMap<String, String> = HashMap::new();
    let claudes = breadboard_core::local::claudes();
    for listed in trees.iter().filter(|tree| tree.dir != main) {
        let label = herdr::space_label(main, &listed.dir);
        let Some(space) = all_spaces.iter().find(|space| space.label == label) else {
            continue;
        };
        let own: Vec<&&Pane> = agents.iter().filter(|pane| pane.workspace == space.id).collect();
        if own.is_empty() {
            continue;
        }
        let tree = breadboard_core::local::with_orchestration_facts(listed.clone(), &claudes);
        let mr = mrs
            .and_then(|mrs| mrs.iter().find(|(branch, _, _)| *branch == tree.branch))
            .map(|(_, iid, status)| Mr {
                iid: *iid,
                state: "opened".into(),
                pipeline: status.as_ref().map(|status| breadboard_core::gitlab::Pipeline {
                    status: status.clone(),
                    ..Default::default()
                }),
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
        let checks = checks_of(&[], &tree.actions, &tree.answered);
        for pane in own {
            let line = sidebar_line_of(
                &tree,
                &Facts {
                    mr: mr.as_ref(),
                    status: pane.status.as_deref(),
                    merged: Some(&merged),
                    handover: None,
                    checks: &checks,
                },
            );
            lines.insert(pane.id.clone(), line);
        }
    }
    let repos = repos_of(all_spaces, &agents);
    let mut ids: HashSet<String> = agents.iter().map(|pane| pane.id.clone()).collect();
    ids.extend(memory.reported.keys().cloned());
    for pane in &ids {
        let mut tokens: BTreeMap<String, Option<String>> = BTreeMap::new();
        tokens.insert(STATE_TOKEN.into(), lines.get(pane).cloned());
        tokens.insert(REPO_TOKEN.into(), repos.get(pane).cloned());
        for gone in GONE {
            tokens.insert(gone.into(), None);
        }
        report(pane, tokens, memory);
    }
    // A pane that is gone is forgotten: herdr may give its id to a new one.
    memory.known.retain(|pane| ids.contains(pane));
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
            repo.starts_with(SPACE).then(|| (agent.id.clone(), repo.clone()))
        })
        .collect()
}

/// The tokens of one pane: set what changed, clear what went.
fn report(pane: &str, tokens: BTreeMap<String, Option<String>>, memory: &mut Memory) {
    let is_new = memory.known.insert(pane.to_string());
    let before = memory.reported.get(pane).cloned().unwrap_or_default();
    let mut args: Vec<String> = Vec::new();
    for (token, value) in &tokens {
        if !is_new && before.get(token).cloned().flatten() == *value {
            continue;
        }
        match value {
            Some(value) => args.extend(["--token".into(), format!("{token}={value}")]),
            None => args.extend(["--clear-token".into(), token.clone()]),
        }
    }
    if !args.is_empty() {
        let mut all = vec!["pane", "report-metadata", pane, "--source", SOURCE];
        all.extend(args.iter().map(String::as_str));
        herdr::did(&all);
    }
    if tokens.values().any(Option::is_some) {
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
    mrs: Option<&[(String, i64, Option<String>)]>,
    panes: &[Pane],
    all_spaces: &[breadboard_core::herdr::Space],
    names: &[(String, String)],
    memory: &mut Memory,
    said: &mut Vec<(Paint, String)>,
) {
    let open: HashSet<String> = all_spaces
        .iter()
        .filter_map(|space| orchestration_of_label(&space.label, names))
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
                .map(|(_, iid, status)| (*iid, status.clone())),
            has_remote: mrs.is_some(),
        };
        let before = memory.last.insert(tree.dir.clone(), now.clone());
        let feature = feature_of(main, &tree.dir);
        for (body, sound) in events_of(before.as_ref(), &now, |iid| {
            glab::get(main, &glab::mr_endpoint(iid))
                .is_ok_and(|mr| mr.get("state").and_then(|state| state.as_str()) == Some("merged"))
        }) {
            herdr::notify(&feature, &body, sound);
            said.push((Paint::Dim, format!("notified {feature}: {body}")));
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
    fn reports_only_what_changed_all_to_a_new_pane() {
        let mut memory = Memory::default();
        let tokens = |state: Option<&str>| {
            let mut tokens = BTreeMap::new();
            tokens.insert(STATE_TOKEN.to_string(), state.map(String::from));
            tokens
        };
        // Outside herdr nothing is called; the memory still follows.
        report("p1", tokens(Some("at work")), &mut memory);
        assert!(memory.known.contains("p1"));
        assert_eq!(memory.reported["p1"][STATE_TOKEN].as_deref(), Some("at work"));
        report("p1", tokens(None), &mut memory);
        assert!(!memory.reported.contains_key("p1"));
    }
}
