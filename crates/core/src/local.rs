//! What there is on this machine (read.mjs `readLocal`, view.mjs `readView`):
//! the worktrees with what git, their `.claude` files and their agents'
//! transcripts say, the stacks docker runs, what herdr says. Read only.

use crate::actions::{actions_file, actions_of, answered_of, change_of};
use crate::check::check_of;
use crate::conversations::{bound_session, context_share_of, left_in, pending_of_file, read_claude, running_claudes};
use crate::git::{
    self, Worktree, ahead_of_develop, behind_develop, branch_files, changed_count, feature_of, is_inside, own_commits,
    push_state, work_files,
};
use crate::herdr;
use crate::model::Tree;
use crate::model::auto_actions::sent_of;
use crate::model::develop::VITA;
use crate::model::orchestration::{Chat, ChatState};
use crate::record::{self, Chat as RecordChat, origin_of};
use crate::wait::wait_of;
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The .env.local of a checkout (stack.mjs `envFileOf`; an older one in products/vita is left where it is).
pub fn env_file_of(dir: &Path) -> PathBuf {
    dir.join(VITA).join(".env.local")
}

/// `KEY=value` lines of an env file, None without one (stack.mjs `readEnv`).
pub fn read_env(file: &Path) -> Option<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(file).ok()?;
    Some(
        text.split('\n')
            .filter(|line| line.contains('=') && !line.trim_start().starts_with('#'))
            .filter_map(|line| {
                line.split_once('=')
                    .map(|(key, value)| (key.to_string(), value.to_string()))
            })
            .collect(),
    )
}

/// The offset of a checkout's own stack, 0 for the shared one.
pub fn offset_of(env: Option<&BTreeMap<String, String>>) -> i64 {
    let Some(env) = env else { return 0 };
    match (
        env.get("COMPOSE_PROJECT_NAME"),
        env.get("POSTGRES_PORT")
            .and_then(|port| port.trim().parse::<i64>().ok()),
    ) {
        (Some(_), Some(port)) => port - 5432,
        _ => 0,
    }
}

/// Whether something answers on the port: it can't be bound (stack.mjs `isPortFree`, negated).
pub fn is_port_taken(port: i64) -> bool {
    u16::try_from(port).is_ok_and(|port| TcpListener::bind(("127.0.0.1", port)).is_err())
}

/// The compose projects docker knows, by name → status; None when docker doesn't answer.
pub fn compose_projects() -> Option<BTreeMap<String, String>> {
    let output = Command::new("docker")
        .args(["compose", "ls", "--all", "--format", "json"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let list: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    Some(
        list.as_array()?
            .iter()
            .filter_map(|project| {
                Some((
                    project.get("Name")?.as_str()?.to_string(),
                    project.get("Status")?.as_str().unwrap_or("").to_string(),
                ))
            })
            .collect(),
    )
}

/// A container of a checkout's stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub service: String,
    pub state: String,
    pub health: String,
}

/// The containers of one checkout's stack (gitlab.mjs `containersOf`).
pub fn containers_of(dir: &Path, project: &str) -> Vec<Container> {
    let output = Command::new("docker")
        .args(["compose", "-p", project, "ps", "--all", "--format", "json"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else { return Vec::new() };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .map(|container| {
            let text = |key: &str| {
                container
                    .get(key)
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            Container {
                service: text("Service"),
                state: text("State"),
                health: text("Health"),
            }
        })
        .collect()
}

/// A worktree's basic facts (readLocal): its name, branch, origin, stack, work.
/// `index` 0 is the main checkout.
pub fn tree_of(
    main: &Path,
    here: &Path,
    worktree: &Worktree,
    is_main: bool,
    projects: Option<&BTreeMap<String, String>>,
) -> Tree {
    let dir = &worktree.dir;
    let env = read_env(&env_file_of(dir));
    let offset = if is_main { 0 } else { offset_of(env.as_ref()) };
    let has_own_port = is_main || offset > 0;
    let project = env
        .as_ref()
        .and_then(|env| env.get("COMPOSE_PROJECT_NAME").cloned())
        .unwrap_or_else(|| "vita".into());
    Tree {
        dir: dir.clone(),
        name: if is_main {
            main.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            feature_of(main, dir)
        },
        branch: worktree.branch.clone().unwrap_or_else(|| "(detached)".into()),
        origin: if is_main { None } else { origin_of(dir) },
        is_here: is_inside(here, dir),
        is_main,
        has_env: env.is_some(),
        is_up: env.is_some() && has_own_port && is_port_taken(3000 + offset),
        stack: projects.and_then(|projects| projects.get(&project).cloned()),
        project,
        env: env.unwrap_or_default(),
        offset,
        has_own_port,
        changed: changed_count(dir),
        ahead: ahead_of_develop(dir),
        checked: check_of(dir),
        ..Default::default()
    }
}

/// What an orchestration's read adds to a worktree (view.mjs): its agent's
/// conversation, the claudes left in it, its actions, files, how far develop
/// moved on, its own commits, its wait, a requested change, what isn't pushed.
pub fn with_orchestration_facts(mut tree: Tree, claudes: &[crate::conversations::Running]) -> Tree {
    let dir = tree.dir.clone();
    let file = actions_file(&dir);
    tree.claude = read_claude(&dir);
    tree.left = left_in(&dir, claudes, bound_session(&dir).as_deref());
    tree.actions = actions_of(&file);
    tree.answered = answered_of(&file);
    tree.change = change_of(&file);
    tree.files = work_files(&dir);
    tree.branch_files = branch_files(&dir);
    tree.behind = behind_develop(&dir);
    tree.own_commits = own_commits(&dir, &tree.branch);
    tree.wait = wait_of(&dir);
    let push = push_state(&dir, Some(&tree.branch));
    tree.pushed = push.pushed;
    tree.unpushed = push.unpushed;
    tree.unpushed_at = push.unpushed_at;
    tree.has_remote_branch = push.has_remote_branch;
    tree.sent = sent_of(&dir).into_iter().collect();
    tree.pending = tree
        .claude
        .as_ref()
        .and_then(|claude| pending_of_file(&claude.transcript));
    tree
}

/// The claudes running on this machine, for `left`.
pub fn claudes() -> Vec<crate::conversations::Running> {
    running_claudes()
}

/// Where the orchestrator's chat stands (orchestrator-chat.mjs `chatStatus`); None outside herdr.
pub fn chat_status(main: &Path, name: &str, chat: Option<&RecordChat>, all: &[herdr::Pane]) -> Option<Chat> {
    if !herdr::is_in_herdr() {
        return None;
    }
    let session_id = chat.map(|chat| chat.session_id.clone());
    let label = record::orchestrator_label(main, name);
    let Some(space) = herdr::spaces().into_iter().find(|space| space.label == label) else {
        return Some(Chat {
            state: ChatState::Gone,
            session_id,
        });
    };
    let (tabs, panes) = std::thread::scope(|scope| {
        let tabs = scope.spawn(|| herdr::tabs_of(&space.id));
        let panes = herdr::panes_of(&space.id);
        (tabs.join().unwrap_or_default(), panes)
    });
    let state = chat_state_of(&tabs, &panes, all, session_id.as_deref(), herdr::is_idle);
    Some(Chat { state, session_id })
}

/// The tab of the orchestrator's chat.
pub const ORCHESTRATOR_TAB: &str = "orchestrator";

/// chatStateOf: the recorded conversation runs (anywhere), or a claude in the
/// tab `orchestrator`, or without that tab the one claude of the space; else
/// the tab idles, runs something else, or is missing.
pub fn chat_state_of(
    tabs: &[herdr::Tab],
    panes: &[herdr::Pane],
    all: &[herdr::Pane],
    session_id: Option<&str>,
    is_idle: impl Fn(&str) -> bool,
) -> ChatState {
    if session_id.is_some_and(|id| all.iter().any(|pane| pane.session.as_deref() == Some(id))) {
        return ChatState::Alive;
    }
    let Some(tab) = tabs.iter().find(|tab| tab.label == ORCHESTRATOR_TAB) else {
        // A free agent's tab (#289) is never the chat.
        let free: Vec<&str> = tabs
            .iter()
            .filter(|tab| tab.label.starts_with("free "))
            .map(|tab| tab.id.as_str())
            .collect();
        let claudes = panes
            .iter()
            .filter(|pane| {
                pane.agent.as_deref() == Some("claude") && pane.session.is_some() && !free.contains(&pane.tab.as_str())
            })
            .count();
        return if claudes == 1 {
            ChatState::Alive
        } else {
            ChatState::Missing
        };
    };
    let own: Vec<&herdr::Pane> = panes.iter().filter(|pane| pane.tab == tab.id).collect();
    if own.iter().any(|pane| pane.agent.is_some()) {
        return ChatState::Alive;
    }
    if own.len() == 1 && is_idle(&own[0].id) {
        return ChatState::Idle;
    }
    ChatState::Busy
}

/// The free agents of a record with the share of their context used.
pub fn free_agents(record: Option<&record::Record>) -> Vec<(record::Worker, f64)> {
    record
        .map(|record| {
            record
                .free_agents()
                .into_iter()
                .map(|worker| (worker.clone(), context_share_of(&worker.id).unwrap_or(0.0)))
                .collect()
        })
        .unwrap_or_default()
}

/// The origin of the test instance: `$VITA_TEST_ORIGIN`, else in the main checkout's .env.local (try.mjs `testOrigin`).
pub fn test_origin(main: &Path) -> Option<String> {
    let origin = std::env::var("VITA_TEST_ORIGIN")
        .ok()
        .filter(|origin| !origin.is_empty())
        .or_else(|| read_env(&env_file_of(main)).and_then(|env| env.get("VITA_TEST_ORIGIN").cloned()))?;
    Some(origin.trim().trim_end_matches('/').to_string()).filter(|origin| !origin.is_empty())
}

/// Every worktree of the repo `main` stands in, the main checkout first; `git::worktrees` skips the orchestrations' checkouts.
pub fn worktrees(main: &Path) -> Vec<Worktree> {
    git::worktrees(main)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, worktree};
    use crate::lock::scratch::Scratch;
    use crate::record::{Origin, write_origin};

    #[test]
    fn reads_env_and_the_offset_of_a_stack() {
        let scratch = Scratch::new();
        let file = scratch.join("products/vita/web/.env.local");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            "# a comment\nCOMPOSE_PROJECT_NAME=vita-2\nPOSTGRES_PORT=5434\nURL=a=b\n",
        )
        .unwrap();
        let env = read_env(&env_file_of(scratch.path())).unwrap();
        assert_eq!(env["URL"], "a=b");
        assert_eq!(offset_of(Some(&env)), 2);
        assert_eq!(offset_of(None), 0);
        assert!(read_env(&scratch.join("none")).is_none());
    }

    #[test]
    fn reads_a_worktree_with_what_an_orchestration_adds() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "5-five", true);
        write_origin(
            &dir,
            &Origin {
                name: "o".into(),
                slot: 0,
            },
        )
        .unwrap();
        let all = worktrees(&main);
        let tree = tree_of(&main, &dir, &all[1], false, None);
        assert_eq!(
            (tree.name.as_str(), tree.branch.as_str(), tree.ahead, tree.is_here),
            ("5-five", "feature/5-five", 1, true)
        );
        assert_eq!(tree.origin.as_ref().map(|origin| origin.name.as_str()), Some("o"));
        let tree = with_orchestration_facts(tree, &[]);
        assert_eq!(tree.branch_files, ["work/5-five.md"]);
        assert!(tree.has_remote_branch);
        assert_eq!(tree.own_commits, 1);
        let main_tree = tree_of(&main, &dir, &all[0], true, None);
        assert_eq!(main_tree.name, "platform");
        assert!(!main_tree.is_here);
    }

    #[test]
    fn tells_where_the_chat_stands() {
        let tab = |id: &str, label: &str| herdr::Tab {
            id: id.into(),
            label: label.into(),
            pane_count: 1,
        };
        let pane = |id: &str, tab: &str, agent: Option<&str>, session: Option<&str>| herdr::Pane {
            id: id.into(),
            tab: tab.into(),
            agent: agent.map(String::from),
            session: session.map(String::from),
            ..Default::default()
        };
        let tabs = [tab("t1", "overview"), tab("t2", ORCHESTRATOR_TAB)];
        let alive = [pane("p2", "t2", Some("claude"), Some("s1"))];
        assert_eq!(chat_state_of(&tabs, &alive, &alive, None, |_| true), ChatState::Alive);
        let shell = [pane("p2", "t2", None, None)];
        assert_eq!(
            chat_state_of(&tabs, &shell, &shell, Some("s1"), |_| true),
            ChatState::Idle
        );
        assert_eq!(
            chat_state_of(&tabs, &shell, &shell, Some("s1"), |_| false),
            ChatState::Busy
        );
        assert_eq!(
            chat_state_of(&tabs, &shell, &alive, Some("s1"), |_| false),
            ChatState::Alive
        );
        assert_eq!(chat_state_of(&tabs[..1], &[], &[], None, |_| true), ChatState::Missing);
        let lone = [pane("p3", "t1", Some("claude"), Some("s2"))];
        assert_eq!(
            chat_state_of(&tabs[..1], &lone, &lone, None, |_| true),
            ChatState::Alive
        );
    }
}
