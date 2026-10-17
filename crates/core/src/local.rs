//! What there is on this machine (read.mjs `readLocal`, view.mjs `readView`):
//! the worktrees with what git, their `.claude` files and their agents'
//! transcripts say, the stacks docker runs, what herdr says. Read only.

use crate::actions::{actions_file, actions_of, answered_of, change_of};
use crate::check::check_of;
use crate::conversations::{bound_session, context_share_of, left_in, pending_of_file, read_claude, running_claudes};
use crate::git::{
    self, Worktree, ahead_of_base, behind_base, branch_files, changed_count, feature_of, is_inside, own_commits,
    push_state, work_files,
};
use crate::herdr;
use crate::model::Tree;
use crate::model::auto_actions::sent_of;
use crate::model::orchestration::{Chat, ChatState};
use crate::record::{self, Chat as RecordChat, origin_of};
use crate::wait::wait_of;
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The env file of a checkout's stack (breadboard.toml's `[stack]`); a
/// path nothing is at without a stack, so every reader finds none.
pub fn env_file_of(dir: &Path) -> PathBuf {
    crate::stack::env_file(dir).unwrap_or_else(|| dir.join(".breadboard").join("no-stack"))
}

pub use crate::stack::read_env;

/// Whether something answers on the port: it can't be bound (stack.mjs `isPortFree`, negated).
pub fn is_port_taken(port: i64) -> bool {
    u16::try_from(port).is_ok_and(|port| TcpListener::bind(("127.0.0.1", port)).is_err())
}

/// Whether the app on `port` is the one of the checkout `dir` (#332): the
/// port is taken, and where Linux's /proc tells who listens on it, a process
/// of this user that runs in `dir` holds it. A dev server of another checkout
/// on the same port (an offset used twice, a test world beside the real
/// ones) is not this checkout's app; where /proc can't tell, the port says it.
pub fn is_app_of(port: i64, dir: &Path) -> bool {
    is_port_taken(port) && owner_of(Path::new("/proc"), port, dir) != Some(false)
}

/// Whether one of the listeners on `port` runs in `dir`: None where `proc`
/// can't tell (no /proc, no listener in it, only other users' processes hold it).
fn owner_of(proc: &Path, port: i64, dir: &Path) -> Option<bool> {
    let sockets = listening_sockets(proc, port)?;
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    // The processes this user may look into, those in `dir` first: its own app is found after a few.
    let (mine, others): (Vec<_>, Vec<_>) = std::fs::read_dir(proc)
        .ok()?
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|byte| byte.is_ascii_digit())
        })
        .filter_map(|entry| Some((std::fs::read_link(entry.path().join("cwd")).ok()?, entry.path())))
        .partition(|(cwd, _)| cwd.starts_with(&dir));
    let holds = |pid: &Path| {
        std::fs::read_dir(pid.join("fd")).is_ok_and(|fds| {
            fds.flatten().any(|fd| {
                std::fs::read_link(fd.path())
                    .is_ok_and(|link| sockets.iter().any(|socket| link.as_os_str() == socket.as_str()))
            })
        })
    };
    if mine.iter().any(|(_, pid)| holds(pid)) {
        return Some(true);
    }
    others.iter().any(|(_, pid)| holds(pid)).then_some(false)
}

/// `socket:[inode]` of each socket listening on `port` (/proc/net/tcp, tcp6); None without them.
fn listening_sockets(proc: &Path, port: i64) -> Option<Vec<String>> {
    let port = format!(":{port:04X}");
    let mut sockets = Vec::new();
    for table in ["net/tcp", "net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(proc.join(table)) else {
            continue;
        };
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // local_address is field 1, st 3 (0A: LISTEN), inode 9.
            if let [_, local, _, "0A", _, _, _, _, _, inode, ..] = fields.as_slice()
                && local.ends_with(&port)
            {
                sockets.push(format!("socket:[{inode}]"));
            }
        }
    }
    (!sockets.is_empty()).then_some(sockets)
}

// ---------------------------------------------------------------------------
// docker (#331): one read of every container of the machine, at most once in
// CONTAINERS_EVERY_MS for all processes of the user together, shared through
// a file; the stacks and each stack's containers are taken from it. Never a
// `compose ls` / `compose ps` per process and stack.
// ---------------------------------------------------------------------------

/// How long one read of docker holds for every process (#331: every 10–15 s).
pub const CONTAINERS_EVERY_MS: i64 = 12_000;
/// How long one read of docker may take: a dockerd under load (#331) holds no process longer; it counts as not answering.
const DOCKER_WAIT: Duration = Duration::from_secs(10);
/// A reader that died holding the lock: taken over after this (its read ends after DOCKER_WAIT).
const CONTAINERS_LOCK_STALE: Duration = Duration::from_secs(15);
/// With no answer on file yet, how long another process waits for the one that reads docker.
const CONTAINERS_FIRST_WAIT: Duration = Duration::from_secs(2);

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

/// `dir` made (0700) where it is missing, and whether it is this user's
/// alone: a real directory, owned by this user, no one else's access. One in
/// a shared `/tmp` that someone else made first is never used: what it holds
/// (a socket, docker's answer) would be theirs.
pub fn own_dir(dir: &Path) -> bool {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0)
}

/// The file docker's read is shared through (#331): `containers-<key>.json`,
/// keyed like the serve socket by what decides which docker answers: the
/// `docker` that PATH runs (resolved), DOCKER_HOST, DOCKER_CONTEXT,
/// DOCKER_CONFIG and HOME (its `.docker` holds the current context). Two
/// processes share an answer only when they would run the same docker: a
/// gauntlet world's fake never answers for the user's real one, nor the other way.
pub fn containers_file() -> PathBuf {
    static FILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        let env = ["DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_CONFIG", "HOME"]
            .map(|name| std::env::var_os(name).unwrap_or_default());
        let docker = std::env::var_os("PATH").and_then(|path| docker_on(&path));
        runtime_dir().join(containers_name(docker.as_deref(), &env))
    })
    .clone()
}

/// The `docker` executable `path` (a PATH) runs, its symlinks resolved; None without one.
fn docker_on(path: &std::ffi::OsStr) -> Option<PathBuf> {
    executable_on(path, "docker")
}

/// The executable `name` that `path` (a PATH) runs, its symlinks resolved; None without one.
pub fn executable_on(path: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|file| std::fs::metadata(file).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0))
        .map(|file| std::fs::canonicalize(&file).unwrap_or(file))
}

fn containers_name(docker: Option<&Path>, env: &[std::ffi::OsString]) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    docker.hash(&mut hasher);
    env.hash(&mut hasher);
    format!("containers-{:016x}.json", hasher.finish())
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

/// One read of docker: every container of a compose stack; None when docker doesn't answer within DOCKER_WAIT.
fn read_docker() -> Option<Vec<StackContainer>> {
    let mut command = Command::new("docker");
    command.args([
        "ps",
        "--all",
        "--filter",
        "label=com.docker.compose.project",
        "--format",
        PS_FORMAT,
    ]);
    let printed = output_within(command, DOCKER_WAIT)?;
    Some(containers_in(&printed))
}

/// What `command` prints when it succeeds within `wait`; one still running then is killed (None).
fn output_within(mut command: Command, wait: Duration) -> Option<String> {
    use std::io::Read;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    // Read meanwhile: a full pipe would hold the command.
    let printed = std::thread::spawn(move || {
        let mut printed = Vec::new();
        let _ = stdout.read_to_end(&mut printed);
        printed
    });
    let deadline = Instant::now() + wait;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let printed = printed.join().ok()?;
    status
        .filter(std::process::ExitStatus::success)
        .map(|_| String::from_utf8_lossy(&printed).into_owned())
}

/// docker's containers, read at most once in CONTAINERS_EVERY_MS by all processes together: the one that finds the file old reads and writes it, the others take what it holds.
pub fn containers() -> Option<Vec<StackContainer>> {
    // Never the machine's docker, nor its file, from a test: `shared_containers` is tested with its own.
    if cfg!(test) {
        return None;
    }
    // No folder of this user's own to share it through: read here.
    let Some(file) = shared_file() else {
        return read_docker();
    };
    shared_containers(&file, crate::time::now_ms(), read_docker)
}

/// The file docker's read is shared through, None where there is no folder of this user's own for it (or in a test).
pub fn shared_file() -> Option<PathBuf> {
    let file = containers_file();
    (!cfg!(test) && file.parent().is_some_and(own_dir)).then_some(file)
}

/// What the shared file holds, None without one: no docker call, no lock.
pub fn read_containers(file: &Path) -> Option<Containers> {
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
    if !is_ours {
        // Another process reads docker right now: what the file holds until then.
        if let Some(known) = known {
            return known.containers;
        }
        // Nothing on file yet (the first look of many): its answer, not a docker call each.
        let deadline = Instant::now() + CONTAINERS_FIRST_WAIT;
        while Instant::now() < deadline && lock.exists() {
            std::thread::sleep(Duration::from_millis(20));
            if let Some(known) = read_containers(file) {
                return known.containers;
            }
        }
        if lock.exists() {
            // Still reading (a docker under load): none yet, docker is asked by it alone.
            return None;
        }
        return read_containers(file).and_then(|known| known.containers);
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
    let stack = crate::stack::of(dir);
    let env = read_env(&env_file_of(dir));
    let offset = if is_main {
        0
    } else {
        crate::stack::offset_in(stack.as_ref(), env.as_ref())
    };
    let has_own_port = is_main || offset > 0;
    let project = crate::stack::project_in(stack.as_ref(), env.as_ref());
    let app_port = stack.as_ref().map_or(0, |stack| i64::from(stack.app.port));
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
        base: git::base_of(dir),
        is_here: is_inside(here, dir),
        is_main,
        has_env: env.is_some(),
        is_up: env.is_some() && has_own_port && is_app_of(app_port + offset, dir),
        app_port,
        stack: projects.and_then(|projects| projects.get(&project).cloned()),
        project,
        env: env.unwrap_or_default(),
        offset,
        has_own_port,
        changed: changed_count(dir),
        ahead: ahead_of_base(dir),
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
    tree.note = crate::said::note_of(&dir);
    tree.left = left_in(&dir, claudes, bound_session(&dir).as_deref());
    tree.actions = actions_of(&file);
    tree.answered = answered_of(&file);
    tree.checks = crate::try_check::checks_of(&dir);
    tree.change = change_of(&file);
    tree.files = work_files(&dir);
    tree.on_base = git::on_base(&dir, &tree.files);
    // Whether a merge of its base would conflict (#332): only where both changed a file, else it can't.
    tree.conflicts = (!tree.on_base.is_empty())
        .then(|| crate::model::develop::conflicts_of(&dir))
        .flatten();
    tree.branch_files = branch_files(&dir);
    tree.behind = behind_base(&dir);
    tree.own_commits = own_commits(&dir, &tree.branch);
    tree.wait = wait_of(&dir);
    tree.wake = crate::wake::wake_of_dir(&dir);
    tree.merge_turn = crate::model::merge_turn::turn_record_of(&dir);
    let push = push_state(&dir, Some(&tree.branch));
    tree.pushed = push.pushed;
    tree.unpushed = push.unpushed;
    tree.unpushed_at = push.unpushed_at;
    tree.has_remote_branch = push.has_remote_branch;
    tree.merge_head_at = git::merge_head_at(&dir);
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

/// The origin of the test instance: `$<[tries] test_origin_env>`, else in the main checkout's env file (try.mjs `testOrigin`).
pub fn test_origin(main: &Path) -> Option<String> {
    let variable = crate::config::of(main).config.tries.test_origin_env.clone();
    let origin = std::env::var(&variable)
        .ok()
        .filter(|origin| !origin.is_empty())
        .or_else(|| read_env(&env_file_of(main)).and_then(|env| env.get(&variable).cloned()))?;
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
    fn finds_the_sockets_listening_on_a_port_in_procs_tables() {
        let scratch = Scratch::new();
        let proc = scratch.path();
        std::fs::create_dir_all(proc.join("net")).unwrap();
        let head = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";
        let row = |local: &str, state: &str, inode: u32| {
            format!(
                "   0: {local} 00000000:0000 {state} 00000000:00000000 00:00000000 00000000  1000        0 {inode} 1 0 100 0 0 10 0\n"
            )
        };
        // 3007 is 0BBF: one listening on v4, one on v6, a connection to it that is no listener.
        std::fs::write(
            proc.join("net/tcp"),
            format!(
                "{head}{}{}",
                row("0100007F:0BBF", "0A", 11),
                row("0100007F:0BBF", "01", 12)
            ),
        )
        .unwrap();
        std::fs::write(
            proc.join("net/tcp6"),
            format!("{head}{}", row("00000000000000000000000000000000:0BBF", "0A", 13)),
        )
        .unwrap();
        assert_eq!(
            listening_sockets(proc, 3007),
            Some(vec!["socket:[11]".to_string(), "socket:[13]".to_string()])
        );
        assert_eq!(listening_sockets(proc, 3008), None);
        // No such table (not Linux): nothing can be told.
        assert_eq!(owner_of(&proc.join("none"), 3007, proc), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_app_is_the_checkouts_only_where_a_process_in_it_listens() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = i64::from(listener.local_addr().unwrap().port());
        // This test's process listens, its cwd is the crate.
        let here = std::env::current_dir().unwrap();
        assert_eq!(owner_of(Path::new("/proc"), port, &here), Some(true));
        assert!(is_app_of(port, &here));
        // Another checkout on the same port: not its app (the gauntlet's world beside the real worktrees).
        let scratch = Scratch::new();
        assert_eq!(owner_of(Path::new("/proc"), port, scratch.path()), Some(false));
        assert!(!is_app_of(port, scratch.path()));
        // No assertion after `drop(listener)`: the ephemeral port can be taken
        // again at once by a parallel test of this same process (same cwd), so
        // "free after the drop" is a race of the test, not a fact of the product.
    }

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
    fn waits_for_the_first_read_of_another_instead_of_asking_docker_too() {
        let scratch = Scratch::new();
        let file = scratch.join("breadboard/containers.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let lock = crate::lock::lock_of(&file);
        std::fs::create_dir(&lock).unwrap();
        let asked = std::cell::Cell::new(false);
        // The one that holds the lock still reads (docker under load): none yet, docker not asked here.
        let started = Instant::now();
        assert_eq!(
            shared_containers(&file, 1_000, || {
                asked.set(true);
                None
            }),
            None
        );
        assert!(started.elapsed() >= CONTAINERS_FIRST_WAIT);
        // Its answer comes while this one waits: taken.
        let writer = {
            let (file, lock) = (file.clone(), lock.clone());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                let answer = Containers {
                    at: 1_000,
                    containers: Some(Vec::new()),
                };
                std::fs::write(&file, serde_json::to_string(&answer).unwrap()).unwrap();
                std::fs::remove_dir(&lock).unwrap();
            })
        };
        assert_eq!(
            shared_containers(&file, 1_000, || {
                asked.set(true);
                None
            }),
            Some(Vec::new())
        );
        writer.join().unwrap();
        assert!(!asked.get());
    }

    #[test]
    fn gives_up_a_command_that_hangs() {
        let mut slow = Command::new("sh");
        slow.args(["-c", "echo started; exec sleep 5"]);
        let started = Instant::now();
        assert_eq!(output_within(slow, Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let mut quick = Command::new("sh");
        quick.args(["-c", "echo vita"]);
        assert_eq!(output_within(quick, Duration::from_secs(5)).as_deref(), Some("vita\n"));
        assert_eq!(output_within(Command::new("false"), Duration::from_secs(5)), None);
    }

    #[test]
    fn keeps_one_answer_per_docker() {
        use std::ffi::OsString;
        let env = |host: &str, context: &str| [host, context, "", "/home/me"].map(OsString::from);
        let real = Some(Path::new("/usr/bin/docker"));
        let name = containers_name(real, &env("", ""));
        assert!(name.starts_with("containers-") && name.ends_with(".json"), "{name}");
        // The same docker, the same answer: every process of the user shares one read.
        assert_eq!(name, containers_name(real, &env("", "")));
        // Another docker on PATH (a gauntlet world's fake), none at all, another host or context: never shared.
        let fake = Some(Path::new("/tmp/world/bin/docker"));
        let others = [
            containers_name(fake, &env("", "")),
            containers_name(None, &env("", "")),
            containers_name(real, &env("ssh://build", "")),
            containers_name(real, &env("", "remote")),
            containers_name(
                real,
                &[
                    OsString::new(),
                    OsString::new(),
                    OsString::new(),
                    "/tmp/world/home".into(),
                ],
            ),
        ];
        for (index, other) in others.iter().enumerate() {
            assert_ne!(&name, other, "{index}");
            assert!(others[index + 1..].iter().all(|each| each != other), "{index}");
        }
    }

    #[test]
    fn finds_the_docker_path_runs() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        let (first, second, empty) = (scratch.join("a"), scratch.join("b"), scratch.join("c"));
        for dir in [&first, &second, &empty] {
            std::fs::create_dir_all(dir).unwrap();
        }
        // Not executable: skipped, as the shell would.
        std::fs::write(first.join("docker"), "").unwrap();
        std::fs::write(second.join("docker"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(second.join("docker"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([&empty, &first, &second]).unwrap();
        let found = docker_on(&path).unwrap();
        assert_eq!(found, std::fs::canonicalize(second.join("docker")).unwrap());
        assert_eq!(docker_on(&std::env::join_paths([&empty]).unwrap()), None);
    }

    #[test]
    fn uses_only_a_folder_of_its_own() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        let mine = scratch.join("run/breadboard");
        assert!(own_dir(&mine));
        assert_eq!(std::fs::metadata(&mine).unwrap().permissions().mode() & 0o777, 0o700);
        // One others can read and write (someone else's would fail the owner the same way): not used.
        let open = scratch.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!own_dir(&open));
        let link = scratch.join("link");
        std::os::unix::fs::symlink(&mine, &link).unwrap();
        assert!(!own_dir(&link));
    }

    #[test]
    fn reads_the_env_and_the_offset_of_a_stack_where_breadboard_toml_puts_it() {
        let scratch = Scratch::new();
        // No [stack]: no env file, whatever lies around.
        std::fs::write(scratch.join(".env.local"), "COMPOSE_PROJECT_NAME=p-2\n").unwrap();
        assert!(read_env(&env_file_of(scratch.path())).is_none());
        std::fs::write(
            scratch.join(crate::config::FILE),
            "[stack]\ndir = \"web\"\nenv_file = \".env\"\nproject = \"p\"\n[stack.app]\nport = 3000\n\
             [[stack.services]]\nname = \"db\"\nport = 5432\nenv = \"DB_PORT\"\n",
        )
        .unwrap();
        let file = scratch.join("web/.env");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "# a comment\nCOMPOSE_PROJECT_NAME=p-2\nDB_PORT=5434\nURL=a=b\n").unwrap();
        assert_eq!(env_file_of(scratch.path()), file);
        let env = read_env(&file).unwrap();
        assert_eq!(env["URL"], "a=b");
        assert_eq!(crate::stack::offset(scratch.path()), 2);
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
