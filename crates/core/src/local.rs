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

// ---------------------------------------------------------------------------
// docker (#331): one read of every container of the machine, at most once in
// CONTAINERS_EVERY_MS for all processes of the user together, shared through
// a file; the stacks and each stack's containers are taken from it. Never a
// `compose ls` / `compose ps` per process and stack.
// ---------------------------------------------------------------------------

/// How long one read of docker holds for every process (#331: every 10–15 s).
pub const CONTAINERS_EVERY_MS: i64 = 12_000;
/// A reader that died holding the lock: taken over after this.
const CONTAINERS_LOCK_STALE: std::time::Duration = std::time::Duration::from_secs(30);

/// A container of a compose stack, as `docker ps` lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StackContainer {
    pub project: String,
    pub service: String,
    /// `running`, `exited`, …
    pub state: String,
    /// `healthy`, `unhealthy`, `starting`, or empty without a health check.
    pub health: String,
}

/// What the file holds: when docker was read, and what it said (None: it didn't answer).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Containers {
    pub at: i64,
    pub containers: Option<Vec<StackContainer>>,
}

/// The user's folder of breadboard's runtime files: `$XDG_RUNTIME_DIR/breadboard`, else `$TMPDIR/breadboard-<uid>`.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("breadboard"),
        // SAFETY: getuid never fails.
        None => std::env::temp_dir().join(format!("breadboard-{}", unsafe { libc::getuid() })),
    }
}

/// The file docker's read is shared through (#331).
pub fn containers_file() -> PathBuf {
    runtime_dir().join("containers.json")
}

/// The template of `docker ps`: one container a line, tab-separated.
const PS_FORMAT: &str = "{{.Label \"com.docker.compose.project\"}}\t{{.Label \"com.docker.compose.service\"}}\t{{.Label \"com.docker.compose.oneoff\"}}\t{{.State}}\t{{.Status}}";

/// The containers of compose stacks in what `docker ps --format PS_FORMAT` printed; one-off ones (`compose run`) left out, as `compose ps` does.
pub fn containers_in(text: &str) -> Vec<StackContainer> {
    let mut found: Vec<StackContainer> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let (project, service, oneoff, state, status) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next().unwrap_or(""),
            );
            if project.is_empty() || oneoff.eq_ignore_ascii_case("true") {
                return None;
            }
            let health = ["unhealthy", "healthy", "health: starting"]
                .into_iter()
                .find(|health| status.contains(&format!("({health})")))
                .map(|health| health.trim_start_matches("health: ").to_string())
                .unwrap_or_default();
            Some(StackContainer {
                project: project.to_string(),
                service: service.to_string(),
                state: state.to_lowercase(),
                health,
            })
        })
        .collect();
    found.sort_by(|a, b| (&a.project, &a.service).cmp(&(&b.project, &b.service)));
    found
}

/// One read of docker: every container of a compose stack; None when docker doesn't answer.
fn read_docker() -> Option<Vec<StackContainer>> {
    let output = Command::new("docker")
        .args([
            "ps",
            "--all",
            "--filter",
            "label=com.docker.compose.project",
            "--format",
            PS_FORMAT,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| containers_in(&String::from_utf8_lossy(&output.stdout)))
}

/// docker's containers, read at most once in CONTAINERS_EVERY_MS by all processes together: the one that finds the file old reads and writes it, the others take what it holds.
pub fn containers() -> Option<Vec<StackContainer>> {
    shared_containers(&containers_file(), crate::time::now_ms(), read_docker)
}

fn read_containers(file: &Path) -> Option<Containers> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// `containers` with its file, clock and read given (the tests').
pub fn shared_containers(
    file: &Path,
    now: i64,
    read: impl FnOnce() -> Option<Vec<StackContainer>>,
) -> Option<Vec<StackContainer>> {
    let known = read_containers(file);
    if let Some(known) = &known
        && (0..CONTAINERS_EVERY_MS).contains(&(now - known.at))
    {
        return known.containers.clone();
    }
    let lock = crate::lock::lock_of(file);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let is_stale_lock = std::fs::metadata(&lock)
        .and_then(|meta| meta.modified())
        .is_ok_and(|at| at.elapsed().unwrap_or_default() > CONTAINERS_LOCK_STALE);
    if is_stale_lock {
        let _ = std::fs::remove_dir_all(&lock);
    }
    let is_ours = std::fs::create_dir(&lock).is_ok();
    if !is_ours && let Some(known) = known {
        // Another process reads docker right now: what the file holds until then.
        return known.containers;
    }
    let containers = read();
    let fresh = Containers {
        at: now,
        containers: containers.clone(),
    };
    if let Ok(text) = serde_json::to_string(&fresh) {
        let part = file.with_extension(format!("{}.part", std::process::id()));
        if std::fs::write(&part, text).is_ok() {
            let _ = std::fs::rename(&part, file);
        }
    }
    if is_ours {
        let _ = std::fs::remove_dir(&lock);
    }
    containers
}

/// compose's word for a stack's state from its containers' (`compose ls`): `exited(1), running(2)`.
pub fn status_of<'a>(states: impl IntoIterator<Item = &'a str>) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for state in states {
        *counts.entry(state).or_default() += 1;
    }
    counts
        .iter()
        .map(|(state, count)| format!("{state}({count})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The compose projects docker knows, by name → status; None when docker doesn't answer.
pub fn compose_projects() -> Option<BTreeMap<String, String>> {
    Some(projects_in(&containers()?))
}

/// The stacks of `containers`, by name → status.
pub fn projects_in(containers: &[StackContainer]) -> BTreeMap<String, String> {
    let mut by_project: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for container in containers {
        by_project
            .entry(container.project.as_str())
            .or_default()
            .push(container.state.as_str());
    }
    by_project
        .into_iter()
        .map(|(project, states)| (project.to_string(), status_of(states)))
        .collect()
}

/// A container of a checkout's stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub service: String,
    pub state: String,
    pub health: String,
}

/// The containers of one checkout's stack (gitlab.mjs `containersOf`), from the shared read.
pub fn containers_of(_dir: &Path, project: &str) -> Vec<Container> {
    containers()
        .unwrap_or_default()
        .into_iter()
        .filter(|container| container.project == project)
        .map(|container| Container {
            service: container.service,
            state: container.state,
            health: container.health,
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
    tree.on_develop = git::on_develop(&dir, &tree.files);
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
    fn takes_every_stack_and_its_containers_from_one_docker_read() {
        let printed = "vita-2\tapp\t\trunning\tUp 2 hours (healthy)\n\
                       vita-2\tdb\tFalse\trunning\tUp 2 hours (health: starting)\n\
                       vita-2\tapp\tTrue\texited\tExited (0) 1 hour ago\n\
                       vita\tdb\t\texited\tExited (1) 3 days ago\n\
                       vita\tapp\t\trunning\tUp 1 minute (unhealthy)\n\
                       \tlonely\t\trunning\tUp\n";
        let containers = containers_in(printed);
        let line = |container: &StackContainer| {
            format!(
                "{} {} {} {}",
                container.project, container.service, container.state, container.health
            )
        };
        assert_eq!(
            containers.iter().map(line).collect::<Vec<_>>(),
            [
                "vita app running unhealthy",
                "vita db exited ",
                "vita-2 app running healthy",
                "vita-2 db running starting"
            ]
        );
        let projects = projects_in(&containers);
        assert_eq!(projects["vita"], "exited(1), running(1)");
        assert_eq!(projects["vita-2"], "running(2)");
        assert!(containers_in("").is_empty());
    }

    #[test]
    fn reads_docker_once_for_every_process_within_its_interval() {
        let scratch = Scratch::new();
        let file = scratch.join("breadboard/containers.json");
        let one = || {
            Some(vec![StackContainer {
                project: "vita".into(),
                service: "db".into(),
                state: "running".into(),
                health: String::new(),
            }])
        };
        let reads = std::cell::Cell::new(0);
        let counted = |answer: Option<Vec<StackContainer>>| {
            reads.set(reads.get() + 1);
            answer
        };
        assert_eq!(shared_containers(&file, 1_000, || counted(one())), one());
        // Another process within the interval: the file, docker not asked.
        assert_eq!(
            shared_containers(&file, 1_000 + CONTAINERS_EVERY_MS - 1, || counted(None)),
            one()
        );
        assert_eq!(reads.get(), 1);
        // While one reads (its lock), the others take the old answer.
        std::fs::create_dir(crate::lock::lock_of(&file)).unwrap();
        assert_eq!(
            shared_containers(&file, 1_000 + CONTAINERS_EVERY_MS, || counted(None)),
            one()
        );
        assert_eq!(reads.get(), 1);
        std::fs::remove_dir(crate::lock::lock_of(&file)).unwrap();
        // Past the interval: read again; docker not answering is shared too.
        assert_eq!(
            shared_containers(&file, 1_000 + CONTAINERS_EVERY_MS, || counted(None)),
            None
        );
        assert_eq!(
            shared_containers(&file, 1_001 + CONTAINERS_EVERY_MS, || counted(one())),
            None
        );
        assert_eq!(reads.get(), 2);
        assert!(!crate::lock::lock_of(&file).exists());
    }

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
