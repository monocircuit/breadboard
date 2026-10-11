//! The spaces of the orchestrations (worktree/orchestrator.mjs, origins.mjs,
//! orchestrator-chat.mjs, free.mjs, order.mjs): `⇶ <title>` on its own
//! checkout, the marks in the orchestration's colour, the orchestrator's
//! chat and the free agents back after a restart, the order of the group,
//! and the space that goes with the last worktree.

use super::super::claude::{is_conversation_running, running_claudes};
use super::super::herdr::{self, call, did, has_agent, s};
use super::super::proc::{exe, git, quote, text};
use super::super::repo::Repo;
use super::nvim::{can_edit, editor_command, ide_flag, lua};
use super::spaces::space_of;
use super::tabs::{
    CHANGELOG, CHANGES, ORCHESTRATOR_TAB, OVERVIEW, ROOT, TOKEN_SOURCE, View, Want, drop_duplicates, ensure_space,
    ensure_tab, ensure_worktree_space, keep_order, move_into, overview_command,
};
use breadboard_core::git::orchestrator_checkouts;
use breadboard_core::record::{self, ORCHESTRATOR, Origin, Record, SLOTS, SPACE, is_session_id};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

/// The tab order of the space `⇶ <name>`.
pub const ORDER: [&str; 5] = [ROOT, OVERVIEW, ORCHESTRATOR_TAB, CHANGELOG, CHANGES];

/// The sidebar token of a slot: `orch1`.
fn token_of(slot: u32) -> String {
    format!("orch{}", slot + 1)
}

/// The checkout of the orchestration `name`, made on first use (`git
/// worktree add --detach` on origin/develop); None when git refuses.
pub fn orchestrator_checkout(repo: &Repo, name: &str) -> Option<PathBuf> {
    let dir = repo.checkout_of(name);
    if orchestrator_checkouts(&repo.main).contains(&dir) {
        return Some(dir);
    }
    fs::create_dir_all(dir.parent()?).ok()?;
    let is_made = git(
        &repo.main,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            &text(&dir),
            &breadboard_core::git::base(&repo.main),
        ],
        None,
    )
    .is_ok;
    is_made.then_some(dir)
}

/// The checkout goes with its space; a changed file keeps it, and says so.
pub fn remove_orchestrator_checkout(repo: &Repo, name: &str) -> Option<String> {
    let dir = repo.checkout_of(name);
    if !orchestrator_checkouts(&repo.main).contains(&dir) {
        return None;
    }
    if git(&repo.main, &["worktree", "remove", &text(&dir)], None).is_ok {
        return None;
    }
    Some(format!(
        "kept the checkout of {}, it has changes: {}",
        repo.orchestrator_label(name),
        text(&dir)
    ))
}

/// What the tab `changelog` of `⇶ <name>` runs: its changelog, live.
fn changelog_command(name: &str) -> String {
    format!("{} wt:orchestrate {name} --changelog --watch", quote(&exe()))
}

/// The space `⇶ <name>`: `root`, its `overview`, the orchestrator's tab,
/// `changelog` and, with nvim, `changes`, in that order; renamed after a new
/// title, an older space's tabs taken over. `should_resume` (a restart): its
/// free agents run again in their tabs.
pub fn orchestrator_space(repo: &Repo, name: &str, should_resume: bool) -> Option<String> {
    let main = repo.main.as_path();
    let label = repo.orchestrator_label(name);
    let all = herdr::spaces();
    if !all.iter().any(|space| s(space, "label") == label) {
        let prefix = format!("{ORCHESTRATOR} ");
        let retitled = all.iter().find(|space| {
            s(space, "label").starts_with(&prefix) && repo.orchestration_of(s(space, "label")).as_deref() == Some(name)
        });
        if let Some(retitled) = retitled {
            call(&["workspace", "rename", s(retitled, "workspace_id"), &label]);
        }
    }
    let root = Want {
        label: ROOT,
        cwd: main,
        command: None,
    };
    let workspace = match orchestrator_checkout(repo, name) {
        Some(checkout) => {
            let first = Want {
                label: ROOT,
                cwd: &checkout,
                command: None,
            };
            ensure_worktree_space(&label, &checkout, space_of(repo, main).as_deref(), &first)
        }
        None => ensure_space(&label, main, &root),
    }?;
    let olds: Vec<String> = herdr::spaces()
        .iter()
        .filter(|space| {
            s(space, "workspace_id") != workspace && repo.orchestration_of(s(space, "label")).as_deref() == Some(name)
        })
        .map(|space| s(space, "workspace_id").to_string())
        .collect();
    for old in olds {
        if let Some(overview) = herdr::tabs_of(&old).iter().find(|tab| s(tab, "label") == OVERVIEW) {
            call(&["tab", "close", s(overview, "tab_id")]);
        }
        move_into(&old, &workspace);
        drop_duplicates(&workspace, ROOT);
    }
    let tabs = herdr::tabs_of(&workspace);
    let overview = overview_command(View::Orchestration(name));
    ensure_tab(
        &workspace,
        &Want {
            label: OVERVIEW,
            cwd: main,
            command: Some(overview),
        },
        &tabs,
    );
    ensure_tab(&workspace, &root, &tabs);
    ensure_tab(
        &workspace,
        &Want {
            label: CHANGELOG,
            cwd: main,
            command: Some(changelog_command(name)),
        },
        &tabs,
    );
    if can_edit() {
        let commands = vec![
            format!("luafile {}", lua("orchestration.lua")),
            format!("OrchestrationChanges {name}"),
        ];
        let command = editor_command(main, Some(name), Some(commands), None);
        ensure_tab(
            &workspace,
            &Want {
                label: CHANGES,
                cwd: main,
                command: Some(command),
            },
            &tabs,
        );
    }
    keep_order(&workspace, &ORDER);
    if should_resume {
        resume_free(repo, name, &workspace);
    }
    Some(workspace)
}

/// After a herdr restart: every free agent of `name` runs again in its tab
/// `free <agent>`, in the orchestration's checkout, unless its conversation
/// runs somewhere already (#289). The names resumed.
pub fn resume_free(repo: &Repo, name: &str, workspace: &str) -> Vec<String> {
    let checkout = repo.checkout_of(name);
    let Some(record) = record::record_of(&repo.main, name).filter(|_| checkout.exists()) else {
        return Vec::new();
    };
    let tabs = herdr::tabs_of(workspace);
    record
        .free_agents()
        .into_iter()
        .filter(|worker| !is_conversation_running(&worker.id))
        .filter_map(|worker| {
            let agent = worker.name.clone().unwrap_or_default();
            let label = format!("free {agent}");
            let command = format!("cd {} && claude --resume {}", quote(&text(&checkout)), worker.id);
            ensure_tab(
                workspace,
                &Want {
                    label: &label,
                    cwd: &checkout,
                    command: Some(command),
                },
                &tabs,
            )
            .then_some(agent)
        })
        .collect()
}

/// The slot a space's tokens show.
fn slot_of_tokens(space: &Value) -> Option<u32> {
    let tokens = space.get("tokens")?;
    (0..SLOTS).find(|slot| breadboard_core::json::is_truthy(tokens.get(token_of(*slot))))
}

/// The colour slot of the orchestration `name`: the one its worktrees or its
/// space carry already, else the least used (origins.mjs `slotOf`).
pub fn slot_of(repo: &Repo, name: &str, trees: &[(PathBuf, Origin)]) -> u32 {
    if let Some((_, origin)) = trees.iter().find(|(_, origin)| origin.name == name) {
        return origin.slot;
    }
    let marked: Vec<(String, Value)> = if herdr::is_in() {
        herdr::spaces()
            .into_iter()
            .filter_map(|space| Some((repo.orchestration_of(s(&space, "label"))?, space)))
            .collect()
    } else {
        Vec::new()
    };
    if let Some(slot) = marked
        .iter()
        .find(|(of, _)| of == name)
        .and_then(|(_, space)| slot_of_tokens(space))
    {
        return slot;
    }
    let mut uses = vec![0usize; SLOTS as usize];
    for (_, origin) in trees {
        uses[origin.slot as usize] += 1;
    }
    for slot in marked.iter().filter_map(|(_, space)| slot_of_tokens(space)) {
        uses[slot as usize] += 1;
    }
    let least = uses.iter().min().copied().unwrap_or(0);
    uses.iter().position(|count| *count == least).unwrap_or(0) as u32
}

/// The arguments that make a space's tokens show `origin` (its slot's token
/// `⇶ <title>`, the others cleared); none when it shows that already.
pub fn mark_args(space: &Value, origin: Option<(&Origin, &str)>) -> Vec<String> {
    let mut args = Vec::new();
    for slot in 0..SLOTS {
        let token = token_of(slot);
        let wanted = origin
            .filter(|(origin, _)| origin.slot == slot)
            .map(|(_, title)| format!("{ORCHESTRATOR} {title}"));
        let shown = space
            .get("tokens")
            .and_then(|tokens| tokens.get(&token))
            .and_then(Value::as_str);
        match wanted {
            Some(wanted) if shown != Some(wanted.as_str()) => {
                args.extend(["--token".into(), format!("{token}={wanted}")])
            }
            None if shown.is_some_and(|shown| !shown.is_empty()) => args.extend(["--clear-token".into(), token]),
            _ => {}
        }
    }
    args
}

/// The origin tokens of a space as `origin` wants them (origins.mjs `markSpace`).
pub fn mark_space(repo: &Repo, space: &Value, origin: Option<&Origin>) {
    let title = origin.map(|origin| record::title_of(&repo.main, &origin.name));
    let args = mark_args(space, origin.zip(title.as_deref()));
    if args.is_empty() {
        return;
    }
    // herdr wants the space first: `--source` in front takes the id for an option.
    let source = TOKEN_SOURCE.get();
    let mut all = vec![
        "workspace",
        "report-metadata",
        s(space, "workspace_id"),
        "--source",
        &source,
    ];
    all.extend(args.iter().map(String::as_str));
    call(&all);
}

/// Every space shows the orchestration it belongs to: a worktree's space its
/// origin, an orchestrator's space its own. herdr forgets the marks on a restart.
pub fn mark_origins(repo: &Repo) {
    let trees = repo.orchestrated();
    let by_feature: HashMap<String, Origin> = trees
        .iter()
        .map(|(dir, origin)| (repo.feature_of(dir), origin.clone()))
        .collect();
    for space in herdr::spaces() {
        let origin = match repo.orchestration_of(s(&space, "label")) {
            Some(name) => Some(Origin {
                slot: slot_of(repo, &name, &trees),
                name,
            }),
            None => by_feature.get(s(&space, "label")).cloned(),
        };
        mark_space(repo, &space, origin.as_ref());
    }
}

/// The space of an orchestration closes once its last worktree is gone, but
/// never while something runs in it; the record goes with the space, and so
/// does one whose space and worktrees are both gone. What closed.
pub fn prune_orchestrators(repo: &Repo) -> Vec<String> {
    let live: HashSet<String> = repo.orchestrated().into_iter().map(|(_, origin)| origin.name).collect();
    let mut closed = Vec::new();
    let mut open: HashSet<String> = HashSet::new();
    for space in herdr::spaces() {
        let Some(name) = repo.orchestration_of(s(&space, "label")) else {
            continue;
        };
        open.insert(name.clone());
        if live.contains(&name) {
            continue;
        }
        let workspace = s(&space, "workspace_id");
        let overview = herdr::tabs_of(workspace)
            .into_iter()
            .find(|tab| s(tab, "label") == OVERVIEW);
        let overview = overview.as_ref().map(|tab| s(tab, "tab_id")).unwrap_or("");
        let panes: Vec<Value> = herdr::panes_of(workspace)
            .into_iter()
            .filter(|pane| s(pane, "tab_id") != overview)
            .collect();
        if panes
            .iter()
            .any(|pane| has_agent(pane) || !herdr::is_idle(s(pane, "pane_id")))
        {
            continue;
        }
        if call(&["workspace", "close", workspace]).is_some() {
            open.remove(&name);
            closed.push(format!(
                "closed the space {}, its worktrees are gone",
                s(&space, "label")
            ));
            closed.extend(remove_orchestrator_checkout(repo, &name));
        }
    }
    for name in record::recorded(&repo.main) {
        if !live.contains(&name) && !open.contains(&name) {
            let _ = fs::remove_file(record::record_file(&repo.main, &name));
        }
    }
    // A checkout whose space and worktrees are gone (a record may never have been written).
    for dir in orchestrator_checkouts(&repo.main) {
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !live.contains(&name) && !open.contains(&name) {
            closed.extend(remove_orchestrator_checkout(repo, &name));
        }
    }
    closed
}

// ---------------------------------------------------------------------------
// The orchestrator's chat (#287)
// ---------------------------------------------------------------------------

/// The orchestrator's conversation: `(session id, where it works)`.
pub type Chat = (String, PathBuf);

fn chat_of(repo: &Repo, record: Option<&Record>) -> Option<Chat> {
    let chat = record?.chat.as_ref()?;
    let cwd = chat
        .cwd
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.main.clone());
    Some((chat.session_id.clone(), cwd))
}

/// The conversation of a pane, where its claude stands now.
pub fn chat_of_pane(repo: &Repo, pane: &Value) -> Option<Chat> {
    if s(pane, "agent") != "claude" {
        return None;
    }
    let id = pane
        .get("agent_session")
        .and_then(|session| session.get("value"))
        .and_then(Value::as_str)?;
    if !is_session_id(id) {
        return None;
    }
    let running = running_claudes()
        .into_iter()
        .find(|claude| claude.id == id)
        .map(|claude| claude.cwd);
    let cwd = running
        .filter(|cwd| !cwd.as_os_str().is_empty())
        .or_else(|| {
            [s(pane, "foreground_cwd"), s(pane, "cwd")]
                .into_iter()
                .find(|cwd| !cwd.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| repo.main.clone());
    Some((id.to_string(), cwd))
}

/// `chat` into the record of `name`, only on a change.
pub fn record_chat(repo: &Repo, name: &str, chat: Option<Chat>) {
    let Some((id, cwd)) = chat else { return };
    let known = chat_of(repo, record::record_of(&repo.main, name).as_ref());
    if known
        .as_ref()
        .is_some_and(|(known_id, known_cwd)| *known_id == id && *known_cwd == cwd)
    {
        return;
    }
    let mut fields = Map::new();
    fields.insert("orchestrator".into(), json!({ "sessionId": id, "cwd": text(&cwd) }));
    let _ = record::update_record(&repo.main, name, fields);
}

fn session_of(pane: &Value) -> &str {
    pane.get("agent_session")
        .and_then(|session| session.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// The orchestrator's claude in its space: the claude of the tab
/// `orchestrator`; without that tab the one claude of the space.
fn chat_pane_of<'a>(tabs: &[Value], panes: &'a [Value]) -> Option<&'a Value> {
    let free: HashSet<&str> = tabs
        .iter()
        .filter(|tab| s(tab, "label").starts_with("free "))
        .map(|tab| s(tab, "tab_id"))
        .collect();
    let claudes: Vec<&Value> = panes
        .iter()
        .filter(|pane| {
            s(pane, "agent") == "claude" && !session_of(pane).is_empty() && !free.contains(s(pane, "tab_id"))
        })
        .collect();
    match tabs.iter().find(|tab| s(tab, "label") == ORCHESTRATOR_TAB) {
        Some(tab) => claudes.into_iter().find(|pane| s(pane, "tab_id") == s(tab, "tab_id")),
        None if claudes.len() == 1 => claudes.into_iter().next(),
        None => None,
    }
}

/// Where the chat of a space stands (orchestrator-chat.mjs `chatStateOf`).
#[derive(Debug, PartialEq)]
pub enum ChatState {
    /// Its conversation runs, or a claude in the tab; `is_lone`: the one
    /// claude of a space without that tab.
    Alive {
        pane: Value,
        is_lone: bool,
    },
    /// The tab is there, a shell idles in it alone (its claude died).
    Idle {
        pane: Value,
    },
    /// The tab is there, something else runs in it.
    Busy,
    Missing,
}

pub fn chat_state_of(
    tabs: &[Value],
    panes: &[Value],
    all: &[Value],
    session: Option<&str>,
    is_idle: impl Fn(&str) -> bool,
) -> ChatState {
    if let Some(running) = session.and_then(|id| all.iter().find(|pane| session_of(pane) == id)) {
        return ChatState::Alive {
            pane: running.clone(),
            is_lone: false,
        };
    }
    let Some(tab) = tabs.iter().find(|tab| s(tab, "label") == ORCHESTRATOR_TAB) else {
        return match chat_pane_of(tabs, panes) {
            Some(lone) => ChatState::Alive {
                pane: lone.clone(),
                is_lone: true,
            },
            None => ChatState::Missing,
        };
    };
    let own: Vec<&Value> = panes
        .iter()
        .filter(|pane| s(pane, "tab_id") == s(tab, "tab_id"))
        .collect();
    if let Some(claude) = own.iter().find(|pane| has_agent(pane)) {
        return ChatState::Alive {
            pane: (*claude).clone(),
            is_lone: false,
        };
    }
    if own.len() == 1 && is_idle(s(own[0], "pane_id")) {
        return ChatState::Idle { pane: own[0].clone() };
    }
    ChatState::Busy
}

/// After a herdr restart: the orchestrator's chat back in its tab, if that
/// idles and a conversation is recorded; a lone claude of the space becomes
/// the orchestrator. What happened, None for nothing.
pub fn restore_chat(repo: &Repo, name: &str) -> Option<String> {
    if !herdr::is_in() {
        return None;
    }
    let label = repo.orchestrator_label(name);
    let workspace = herdr::space_labelled(&label).map(|space| s(&space, "workspace_id").to_string())?;
    let chat = chat_of(repo, record::record_of(&repo.main, name).as_ref());
    let tabs = herdr::tabs_of(&workspace);
    let state = chat_state_of(
        &tabs,
        &herdr::panes_of(&workspace),
        &herdr::all_panes(),
        chat.as_ref().map(|c| c.0.as_str()),
        herdr::is_idle,
    );
    match state {
        ChatState::Alive { pane, is_lone } => {
            if is_lone {
                call(&["tab", "rename", s(&pane, "tab_id"), ORCHESTRATOR_TAB]);
                record_chat(repo, name, chat_of_pane(repo, &pane));
            }
            None
        }
        ChatState::Busy => None,
        ChatState::Missing | ChatState::Idle { .. } => {
            let (id, cwd) = chat?;
            // A second claude in the same conversation would write into the same transcript.
            if is_conversation_running(&id) {
                return Some(format!("the conversation {id} runs elsewhere, outside herdr's panes"));
            }
            let ChatState::Idle { pane } = state else { return None };
            let command = format!("cd {} && claude --resume {id}{}", quote(&text(&cwd)), ide_flag(&cwd));
            did(&["pane", "run", s(&pane, "pane_id"), &command]).then(|| format!("resumed {id} in its tab"))
        }
    }
}

// ---------------------------------------------------------------------------
// The order of the group (#255)
// ---------------------------------------------------------------------------

/// The features of an orchestration's worktrees in its work order: the
/// issues of `plan.work` first, then the rest as they were started.
pub fn work_order_of(record: Option<&Record>) -> Vec<String> {
    let Some(record) = record else { return Vec::new() };
    let planned = record
        .plan
        .iter()
        .flat_map(|plan| plan.work.iter())
        .filter_map(|number| record.issues.iter().find(|issue| issue.number == Some(*number)));
    let mut seen = Vec::new();
    for issue in planned.chain(record.issues.iter()) {
        if let Some(feature) = &issue.feature
            && !seen.contains(feature)
        {
            seen.push(feature.clone());
        }
    }
    seen
}

fn tree_of(space: &Value) -> Option<&Value> {
    space.get("worktree")
}

/// The move that brings the group of the main space in `list` (sidebar
/// order) into its order, `{ workspace_ids, before_workspace_id }` for
/// `workspace.move_block`; None when it stands so already.
pub fn group_move(
    list: &[Value],
    origin_of: impl Fn(&str) -> Option<String>,
    work_order_of: impl Fn(&str) -> Vec<String>,
    orchestration_of: impl Fn(&str) -> Option<String>,
) -> Option<Value> {
    let main = list.iter().position(|space| s(space, "label") == SPACE)?;
    let repo = tree_of(&list[main])
        .and_then(|tree| tree.get("repo_key"))
        .and_then(Value::as_str)?;
    let group: Vec<&Value> = list
        .iter()
        .filter(|space| {
            tree_of(space).is_some_and(|tree| {
                tree.get("is_linked_worktree").and_then(Value::as_bool) == Some(true)
                    && tree.get("repo_key").and_then(Value::as_str) == Some(repo)
            })
        })
        .collect();
    let orchestrations: Vec<(&Value, String)> = group
        .iter()
        .filter_map(|space| Some((*space, orchestration_of(s(space, "label"))?)))
        .collect();
    let open: HashSet<&str> = orchestrations.iter().map(|(_, name)| name.as_str()).collect();
    let trees: Vec<&Value> = group
        .iter()
        .copied()
        .filter(|space| orchestration_of(s(space, "label")).is_none())
        .collect();
    let mut wanted: Vec<&Value> = trees
        .iter()
        .copied()
        .filter(|space| origin_of(s(space, "label")).is_none_or(|origin| !open.contains(origin.as_str())))
        .collect();
    for (space, name) in &orchestrations {
        let order = work_order_of(name);
        let rank = |space: &Value| {
            order
                .iter()
                .position(|feature| feature == s(space, "label"))
                .unwrap_or(order.len())
        };
        let mut own: Vec<&Value> = trees
            .iter()
            .copied()
            .filter(|tree| origin_of(s(tree, "label")).as_deref() == Some(name.as_str()))
            .collect();
        own.sort_by_key(|tree| rank(tree));
        wanted.push(space);
        wanted.extend(own);
    }
    let ids: Vec<&str> = wanted.iter().map(|space| s(space, "workspace_id")).collect();
    if ids == group.iter().map(|space| s(space, "workspace_id")).collect::<Vec<_>>() {
        return None;
    }
    // Behind the main space and where the group starts, before the first space outside it.
    let first = group
        .first()
        .and_then(|first| list.iter().position(|space| std::ptr::eq(space, *first)));
    let from = first.map_or(main, |first| first.max(main));
    let before = list[from..]
        .iter()
        .enumerate()
        .find(|(at, space)| from + at != main && !ids.contains(&s(space, "workspace_id")))
        .map(|(_, space)| s(space, "workspace_id").to_string());
    Some(json!({ "workspace_ids": ids, "before_workspace_id": before }))
}

/// The group of the main space into its order when it is out of it; a line
/// for what moved.
pub fn sort_spaces(repo: &Repo) -> Option<String> {
    let list = herdr::spaces();
    let origins: HashMap<String, String> = repo
        .orchestrated()
        .into_iter()
        .map(|(dir, origin)| (repo.feature_of(&dir), origin.name))
        .collect();
    let titles = record::titles(&repo.main);
    let orchestration_of = |label: &str| {
        if label.contains(ORCHESTRATOR) {
            record::orchestration_of_label(label, &titles)
        } else {
            None
        }
    };
    let moved = group_move(
        &list,
        |label| origins.get(label).cloned(),
        |name| work_order_of(record::record_of(&repo.main, name).as_ref()),
        orchestration_of,
    )?;
    herdr::api("workspace.move_block", moved).map(|_| format!("spaces of {SPACE} sorted by orchestration"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(id: &str, label: &str, is_linked: bool) -> Value {
        json!({ "workspace_id": id, "label": label, "worktree": { "is_linked_worktree": is_linked, "repo_key": "/r/.git" } })
    }

    #[test]
    fn orchestrations_sort_below_the_plain_worktrees() {
        let list = vec![
            space("m", &SPACE.get(), false),
            space("a", "⇶ o", true),
            space("b", "2-b", true),
            space("c", "1-c", true),
            space("d", "plain", true),
            json!({ "workspace_id": "x", "label": "elsewhere" }),
        ];
        let origin_of = |label: &str| ["1-c", "2-b"].contains(&label).then(|| "o".to_string());
        let order = |_: &str| vec!["1-c".to_string(), "2-b".to_string()];
        let of = |label: &str| (label == "⇶ o").then(|| "o".to_string());
        let moved = group_move(&list, origin_of, order, of).unwrap();
        assert_eq!(
            moved,
            json!({ "workspace_ids": ["d", "a", "c", "b"], "before_workspace_id": "x" })
        );
        let sorted = vec![
            list[0].clone(),
            list[4].clone(),
            list[1].clone(),
            list[3].clone(),
            list[2].clone(),
        ];
        assert!(group_move(&sorted, origin_of, order, of).is_none());
    }

    #[test]
    fn marks_only_what_changes() {
        let origin = Origin {
            name: "o".into(),
            slot: 1,
        };
        let shown = json!({ "tokens": { "orch1": "⇶ old", "orch2": "⇶ T" } });
        assert_eq!(mark_args(&shown, Some((&origin, "T"))), vec!["--clear-token", "orch1"]);
        assert_eq!(
            mark_args(&json!({}), Some((&origin, "T"))),
            vec!["--token", "orch2=⇶ T"]
        );
        assert_eq!(
            mark_args(&shown, None),
            vec!["--clear-token", "orch1", "--clear-token", "orch2"]
        );
        assert!(mark_args(&json!({ "tokens": {} }), None).is_empty());
    }

    #[test]
    fn the_chat_is_the_claude_of_its_tab_or_the_lone_one() {
        let tabs = [
            json!({ "tab_id": "t1", "label": "root" }),
            json!({ "tab_id": "t2", "label": "free 3-x" }),
        ];
        let id = "0123456789abcdef0123456789abcdef0123";
        let panes = [
            json!({ "pane_id": "p1", "tab_id": "t1", "agent": "claude", "agent_session": { "value": id } }),
            json!({ "pane_id": "p2", "tab_id": "t2", "agent": "claude", "agent_session": { "value": "other" } }),
        ];
        let state = chat_state_of(&tabs, &panes, &panes, None, |_| true);
        assert_eq!(
            state,
            ChatState::Alive {
                pane: panes[0].clone(),
                is_lone: true
            }
        );
        let with_tab = [json!({ "tab_id": "t3", "label": "orchestrator" })];
        let idle = [json!({ "pane_id": "p3", "tab_id": "t3" })];
        assert_eq!(
            chat_state_of(&with_tab, &idle, &idle, None, |_| true),
            ChatState::Idle { pane: idle[0].clone() }
        );
        assert_eq!(chat_state_of(&with_tab, &idle, &idle, None, |_| false), ChatState::Busy);
        assert_eq!(chat_state_of(&[], &[], &[], None, |_| true), ChatState::Missing);
    }
}
