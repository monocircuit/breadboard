//! The reads behind a view: the local one (disk, git, herdr) cheap enough to
//! run every couple of seconds, the GitLab one for a refresh every half
//! minute, and the model of the view built from both.
//!
//! Local: each worktree's git facts are cached by its key (`Key`): the stat
//! of its git files (HEAD, index, the refs of its branch and develop) and its
//! `git status` with each listed file's size and time, looked at again after
//! STALE_MS (edits in the working tree touch no file git or we watch). Under
//! one key every git read of the worktree is answered from its memo
//! (`git::remembered`): ahead, behind, merge base, files, cherry, develop's
//! log, the diff, the overlaps. Its other files (`.claude`, env, its agent's
//! transcript) move while its agent works: they make its facts read again,
//! git asked only for its status (what the agent edited shows at once). An idle worktree costs one `git status` in
//! STALE_MS, idle CPU near zero without a watcher thread or crate (rewrite
//! rules 4 and 7; spec: "each is cached by its key").
//!
//! GitLab: `RemoteReader` holds the client, the disk cache of what is final
//! and what the fast reads keep; it runs in a task of its own while the view
//! draws, asked by an `Ask` the local read made.

use crate::conversations::{bound_session, transcript_of};
use crate::git::{self, git_dirs, path_of_status};
use crate::gitlab::Remote;
use crate::gitlab::cache::{Cache, cache_file};
use crate::gitlab::client::{Gitlab, GitlabError};
use crate::gitlab::read::{self, Ask, EverythingRemote, Memory, OpenMr, WorktreeRemote};
use crate::herdr;
use crate::local::{
    self, claudes, compose_projects, containers_of, env_file_of, free_agents, is_port_taken, test_origin, tree_of,
    with_orchestration_facts,
};
use crate::model::everything::{EverythingLocal, EverythingModel, everything_of};
use crate::model::memory::free_memory;
use crate::model::orchestration::{OrchestrationLocal, OrchestrationModel, ViewState, orchestration_of};
use crate::model::waiting::{Waiting, waiting_for};
use crate::model::worktree::{WorktreeLocal, WorktreeModel, worktree_of};
use crate::model::{Docker, Tree, develop, diff};
use crate::record::{self, record_of};
use crate::setups::live_setups_of;
use crate::time::now_ms;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// How long a worktree's `git status` holds without a change of its files: edits in the working tree touch none.
pub const STALE_MS: i64 = 15_000;
/// How long docker's stacks hold for WORKTREES (#331: one docker read every 10–15 s, never a poll per read).
pub const DOCKER_EVERY_MS: i64 = 12_000;
/// How many of the journal's last lines the orchestration's view shows (#382).
const JOURNAL_LINES: usize = 3;

/// Which view is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Orchestration(String),
    Worktree(PathBuf),
    Everything,
}

/// What a local read found, by view.
#[derive(Debug, Clone)]
pub enum Local {
    Orchestration(Box<OrchestrationLocal>),
    Worktree(Box<WorktreeLocal>),
    Everything(EverythingLocal),
}

/// What a GitLab read found, by view; it goes over the socket of `breadboard serve` as JSON.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum RemoteOf {
    Orchestration(Remote),
    Worktree(Box<WorktreeRemote>),
    Everything(EverythingRemote),
}

impl RemoteOf {
    pub fn at(&self) -> i64 {
        match self {
            RemoteOf::Orchestration(remote) => remote.at,
            RemoteOf::Worktree(remote) => remote.at,
            RemoteOf::Everything(remote) => remote.at,
        }
    }
}

/// What a GitLab read asks for, from the local read.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RemoteAsk {
    Orchestration(Ask),
    Worktree { branch: Option<String> },
    Everything,
}

/// The model of a view.
#[derive(Debug, Clone)]
pub enum Model {
    Orchestration(Box<OrchestrationModel>),
    Worktree(Box<WorktreeModel>),
    Everything(Box<EverythingModel>),
}

/// A stamp of files: their size and time of change, hashed; a missing one counts too.
pub fn stamp_of<'a>(files: impl IntoIterator<Item = &'a PathBuf>) -> u64 {
    let mut hasher = DefaultHasher::new();
    for file in files {
        file.hash(&mut hasher);
        if let Ok(meta) = std::fs::metadata(file) {
            meta.len().hash(&mut hasher);
            meta.modified().ok().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// A worktree's facts as last read.
#[derive(Debug, Clone)]
struct Known {
    /// Its git key when they were read (`Key::of`).
    key: u64,
    /// The stamp of its other files then (`Files::other`).
    other: u64,
    /// Read with what an orchestration adds.
    is_full: bool,
    tree: Tree,
}

/// What git's facts of a worktree are cached by (spec: "each is cached by its
/// key": HEAD + develop + status): the stamp of the git files they come from
/// (HEAD, index, its branch's refs and develop's), and what `git status` said
/// with the size and time of each file it listed (a second edit of a file
/// already ` M` counts), looked at again after STALE_MS or when a stamp
/// moved: its other files too (an agent at work edits what its transcript
/// says, and `N changed` shows at once).
#[derive(Debug, Clone, Copy)]
struct Key {
    stamp: u64,
    other: u64,
    status: u64,
    looked_at: i64,
}

impl Key {
    /// The key as one number: equal, and git's facts of the worktree are as read.
    fn of(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (self.stamp, self.status).hash(&mut hasher);
        hasher.finish()
    }
}

/// The `git status` a worktree's key is read from (`status_key`); its answer
/// also stands for the same call with `--untracked-files=all`.
pub const STATUS: [&str; 3] = ["status", "--porcelain", "-uall"];

/// What answers a worktree's `git status` (`STATUS`) in place of a call of
/// this process: the overviews' shared reader (`breadboard serve`), asked
/// with the worktree and the stamp of its files (`Key`). Its answer: when
/// that status was read (ms; never more than STALE_MS before) and what it
/// said; None: ask git here.
pub type SharedStatus = fn(&Path, u64) -> Option<(i64, String)>;

static SHARED_STATUS: std::sync::OnceLock<SharedStatus> = std::sync::OnceLock::new();

/// From now on this process's worktree keys take `git status` from `shared` first (a live overview does this).
pub fn share_status(shared: SharedStatus) {
    let _ = SHARED_STATUS.set(shared);
}

/// `git status` of `dir` under the stamp of its files, and when it was read:
/// another overview's read of the same stamp (the shared reader) when it has
/// one, else this process's at `now`.
fn status_of(dir: &Path, stamp: u64, now: i64) -> (i64, git::Out) {
    if let Some((at, stdout)) = SHARED_STATUS.get().and_then(|shared| shared(dir, stamp)) {
        let out = git::Out {
            is_ok: true,
            stdout,
            ..Default::default()
        };
        return (at, out);
    }
    (now, git::git(dir, &STATUS))
}

/// The stamps of a worktree's git files and of its other files as one: what a shared `git status` is asked under.
fn stamp_pair(stamp: u64, other: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    (stamp, other).hash(&mut hasher);
    hasher.finish()
}

/// `git status` (`STATUS`) of `dir` with each listed file's size and time, hashed.
fn status_key(dir: &Path, status: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    for line in status.split('\n').filter(|line| !line.is_empty()) {
        line.hash(&mut hasher);
        diff::stamp_of(&dir.join(path_of_status(line))).hash(&mut hasher);
    }
    hasher.finish()
}

/// The files a worktree's facts come from: git's (its key) and the others
/// (its `.claude` files, env, its agent's transcript), which change while
/// its agent works and git's facts don't.
#[derive(Debug, Default)]
struct Files {
    git: Vec<PathBuf>,
    other: Vec<PathBuf>,
}

/// Git answers by worktree while their key holds (`git::remembered`): read
/// again, its facts ask git nothing.
#[derive(Debug, Default)]
struct Memos(HashMap<PathBuf, (u64, Arc<Mutex<git::Memo>>)>);

impl Memos {
    /// The memo of `dir` under `key`: a new one when the key moved.
    fn of(&mut self, dir: &Path, key: u64) -> Arc<Mutex<git::Memo>> {
        let entry = self.0.entry(dir.to_path_buf()).or_default();
        if entry.0 != key || entry.1.lock().is_err() {
            *entry = (key, Arc::default());
        }
        Arc::clone(&entry.1)
    }
}

/// The memos of the worktrees under the keys this process's reader last
/// found: what else reads a worktree's facts in it (the actor's sync) takes
/// git's answers from there too.
static MEMOS: std::sync::LazyLock<Mutex<Memos>> = std::sync::LazyLock::new(Mutex::default);

/// The memo of `dir` under `key` (`MEMOS`).
fn memo_of(dir: &Path, key: u64) -> Arc<Mutex<git::Memo>> {
    MEMOS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .of(dir, key)
}

/// `with_orchestration_facts` of a tree this process's reader listed, git
/// answered from its memo under the key the reader last found (as fresh as
/// the tree itself); a tree it never keyed is read from git.
pub fn with_orchestration_facts_remembered(tree: Tree, claudes: &[crate::conversations::Running]) -> Tree {
    let memo = MEMOS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .0
        .get(&tree.dir)
        .map(|(_, memo)| Arc::clone(memo));
    match memo {
        Some(memo) => git::remembered(&memo, || with_orchestration_facts(tree, claudes)),
        None => with_orchestration_facts(tree, claudes),
    }
}

/// The local side: worktrees, records, herdr; cheap when nothing changed.
#[derive(Debug)]
pub struct LocalReader {
    pub main: PathBuf,
    pub here: PathBuf,
    pub view: View,
    /// Whether herdr is asked (`HERDR_ENV=1`); off, its agents and chat are unknown.
    pub is_in_herdr: bool,
    known: HashMap<PathBuf, Known>,
    /// Each worktree's git key, by its dir.
    keys: HashMap<PathBuf, Key>,
    git_dirs: HashMap<PathBuf, Option<(PathBuf, PathBuf)>>,
    /// Where each conversation's transcript is: it stays where claude put it.
    transcripts: HashMap<String, PathBuf>,
    /// docker's stacks for WORKTREES, read on a thread of their own.
    docker: Arc<Mutex<Stacks>>,
    /// Whether a read waits for docker's answer (a print, `--once`); a live view draws without it.
    pub should_wait_for_docker: bool,
}

/// docker's stacks as WORKTREES knows them (#331, #332): asked on a thread
/// of their own, so git's facts never wait for a slow docker; the stack
/// cells fill in once it answered, here or in another process (the shared
/// file).
#[derive(Debug, Default)]
struct Stacks {
    /// When the answer held was read (the shared file's `at`), and the
    /// stacks by name → status (None: docker didn't answer). None: no answer yet.
    held: Option<(i64, Option<BTreeMap<String, String>>)>,
    is_reading: bool,
    /// Counts the answers taken: the local read's stamp moves with it.
    taken: u64,
}

impl Stacks {
    /// The shared file's answer, when it is newer than the one held.
    fn take_shared(&mut self, file: &Path) {
        let Some(shared) = local::read_containers(file) else {
            return;
        };
        if self.held.as_ref().is_none_or(|(at, _)| *at < shared.at) {
            let projects = shared.containers.as_deref().map(local::projects_in);
            self.held = Some((shared.at, projects));
            self.taken += 1;
        }
    }
}

/// `stacks` locked; a reader that panicked left nothing half written in it.
fn locked(stacks: &Mutex<Stacks>) -> std::sync::MutexGuard<'_, Stacks> {
    stacks.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl LocalReader {
    /// The reader of `view` in the repo `here` stands in; None outside a repo.
    pub fn new(here: &Path, view: View) -> Option<LocalReader> {
        let main = git::main_dir(here)?;
        Some(LocalReader {
            is_in_herdr: herdr::is_in_herdr(),
            main,
            here: here.to_path_buf(),
            view,
            known: HashMap::new(),
            keys: HashMap::new(),
            git_dirs: HashMap::new(),
            transcripts: HashMap::new(),
            docker: Arc::default(),
            should_wait_for_docker: false,
        })
    }

    /// The files a worktree's facts come from.
    fn files_of(&mut self, dir: &Path, branch: &str) -> Files {
        let claude = dir.join(".claude");
        let other = [
            "actions.json",
            "wait.json",
            "wake.json",
            "origin",
            "checked.json",
            "merge-turn.json",
            "herdr-session",
            "overview-sent.json",
            "note.json",
            "try-checks.json",
            "reviews.json",
        ]
        .iter()
        .map(|name| claude.join(name))
        // Its stack's ports and project.
        .chain([env_file_of(dir)])
        .collect();
        let mut files = Files { git: Vec::new(), other };
        let dirs = self
            .git_dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| git_dirs(dir))
            .clone();
        if let Some((own, common)) = dirs {
            files.git.extend(
                ["HEAD", "index", "logs/HEAD", "MERGE_HEAD"]
                    .iter()
                    .map(|name| own.join(name)),
            );
            files
                .git
                .extend(["packed-refs", "FETCH_HEAD"].iter().map(|name| common.join(name)));
            files
                .git
                .push(common.join("refs/remotes").join(&git::base_of(dir).remote));
            files.git.push(common.join("refs/remotes/origin").join(branch));
            files.git.push(common.join("refs/heads").join(branch));
        }
        if let Some(id) = bound_session(dir) {
            let found = match self.transcripts.get(&id) {
                Some(transcript) => Some(transcript.clone()),
                None => transcript_of(&id),
            };
            if let Some(transcript) = found {
                self.transcripts.insert(id, transcript.clone());
                files.other.push(transcript);
            }
        }
        files
    }

    /// Each worktree's git key (`Key::of`) and the stamp of its other files,
    /// by `(dir, branch)`: the key from the stat of its git files while they
    /// hold and STALE_MS hasn't passed, else with one `git status` each, side by side.
    fn keys_of(&mut self, dirs: &[(PathBuf, String)], now: i64) -> Vec<(u64, u64)> {
        self.keys_with(dirs, now, status_of)
    }

    /// `keys_of` with `git status` asked of `status` (tests).
    fn keys_with(
        &mut self,
        dirs: &[(PathBuf, String)],
        now: i64,
        status: impl Fn(&Path, u64, i64) -> (i64, git::Out) + Copy + Send,
    ) -> Vec<(u64, u64)> {
        let stamps: Vec<(u64, u64)> = dirs
            .iter()
            .map(|(dir, branch)| {
                let files = self.files_of(dir, branch);
                (stamp_of(&files.git), stamp_of(&files.other))
            })
            .collect();
        let keys = &self.keys;
        let looked: Vec<Option<(i64, git::Out)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = dirs
                .iter()
                .zip(&stamps)
                .map(|((dir, _), (stamp, other))| {
                    let is_held = keys.get(dir).is_some_and(|key| {
                        key.stamp == *stamp && key.other == *other && now - key.looked_at < STALE_MS
                    });
                    let both = stamp_pair(*stamp, *other);
                    (!is_held).then(|| scope.spawn(move || status(dir, both, now)))
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.and_then(|handle| handle.join().ok()))
                .collect()
        });
        dirs.iter()
            .zip(stamps)
            .zip(looked)
            .map(|(((dir, _), (stamp, other)), status)| {
                if let Some((looked_at, status)) = status {
                    let key = Key {
                        stamp,
                        other,
                        status: status_key(dir, &status.stdout),
                        // Another overview's read counts from when it was made: never held longer than STALE_MS in all.
                        looked_at,
                    };
                    self.keys.insert(dir.clone(), key);
                    // The facts read under this key take the same answer: no second `git status`.
                    if status.is_ok {
                        let memo = memo_of(dir, key.of());
                        let mut memo = memo.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        for args in [&STATUS[..], &["status", "--porcelain", "--untracked-files=all"]] {
                            let words = args.iter().map(|word| word.to_string()).collect();
                            memo.entry((dir.clone(), words)).or_insert_with(|| status.clone());
                        }
                    }
                }
                (self.keys[dir].of(), other)
            })
            .collect()
    }

    /// The worktrees' facts: from the last read while their files and git
    /// key hold (STALE_MS at most: whether its app is up, a check still runs,
    /// its database's migrations hold no file), else read again, side by
    /// side, git's answers from its memo while its key holds (a dozen git
    /// calls each when it moved).
    fn trees(
        &mut self,
        list: &[(&git::Worktree, bool)],
        projects: Option<&std::collections::BTreeMap<String, String>>,
        is_full: bool,
        claudes: &[crate::conversations::Running],
        now: i64,
    ) -> Vec<Tree> {
        let dirs: Vec<(PathBuf, String)> = list
            .iter()
            .map(|(worktree, _)| (worktree.dir.clone(), worktree.branch.clone().unwrap_or_default()))
            .collect();
        let keys = self.keys_of(&dirs, now);
        let memos: Vec<Option<Arc<Mutex<git::Memo>>>> = (0..list.len())
            .map(|at| {
                let (key, other) = keys[at];
                let is_held = self.known.get(&dirs[at].0).is_some_and(|known| {
                    known.key == key
                        && known.other == other
                        && now - known.tree.read_at < STALE_MS
                        && (known.is_full || !is_full)
                });
                (!is_held).then(|| memo_of(&dirs[at].0, key))
            })
            .collect();
        let (main, here) = (&self.main, &self.here);
        let read: Vec<Option<Tree>> = std::thread::scope(|scope| {
            let handles: Vec<_> = memos
                .iter()
                .enumerate()
                .map(|(at, memo)| {
                    memo.as_ref().map(|memo| {
                        let (worktree, is_main) = list[at];
                        scope.spawn(move || {
                            git::remembered(memo, || {
                                let tree = tree_of(main, here, worktree, is_main, projects);
                                if is_full {
                                    with_orchestration_facts(tree, claudes)
                                } else {
                                    tree
                                }
                            })
                        })
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.and_then(|handle| handle.join().ok()))
                .collect()
        });
        read.into_iter()
            .enumerate()
            .map(|(at, fresh)| match fresh {
                Some(mut tree) => {
                    tree.read_at = now;
                    let (key, other) = keys[at];
                    let known = Known {
                        key,
                        other,
                        is_full,
                        tree: tree.clone(),
                    };
                    self.known.insert(dirs[at].0.clone(), known);
                    tree
                }
                None => {
                    let mut tree = self.known[&dirs[at].0].tree.clone();
                    // The cheap parts move on their own: its stack, who stands in it.
                    tree.stack = projects.and_then(|projects| projects.get(&tree.project).cloned());
                    if is_full {
                        let bound = bound_session(&tree.dir);
                        tree.left = crate::conversations::left_in(&tree.dir, claudes, bound.as_deref());
                    }
                    tree
                }
            })
            .collect()
    }

    /// A stamp of everything the local read comes from that a file holds: the
    /// worktrees of the repo, the records, each worktree's files. A view reads
    /// again when it changed (or after a while: herdr's states hold no file).
    pub fn stamp(&mut self) -> u64 {
        let mut files: Vec<PathBuf> = vec![record::records_dir(&self.main)];
        if let Some((_, common)) = self
            .git_dirs
            .entry(self.main.clone())
            .or_insert_with(|| git_dirs(&self.main))
            .clone()
        {
            files.push(common.join("worktrees"));
        }
        if let View::Orchestration(name) = &self.view {
            files.push(record::record_file(&self.main, name));
            files.push(crate::journal::journal_file(&self.main, name));
            // The band's pause and usage limit and the lines waiting (#397,
            // #393): a click on `pause agents` shows at the next look (#460).
            files.push(crate::runs::pause_file(&self.main));
            files.push(crate::runs::usage_file(&self.main));
            files.push(crate::runs::queue_file(&self.main));
        }
        let known: Vec<(PathBuf, String)> = self
            .known
            .values()
            .map(|known| (known.tree.dir.clone(), known.tree.branch.clone()))
            .collect();
        for (dir, branch) in known {
            let Files { git, other } = self.files_of(&dir, &branch);
            files.extend(git);
            files.extend(other);
        }
        // docker's answer, in this process or another's (the shared file): its stack cells fill in.
        let taken = locked(&self.docker).taken;
        if taken > 0 || locked(&self.docker).is_reading {
            files.extend(local::shared_file());
        }
        stamp_of(&files) ^ taken
    }

    /// The local read of the view.
    pub fn read(&mut self) -> Local {
        let now = now_ms();
        match self.view.clone() {
            View::Orchestration(name) => Local::Orchestration(Box::new(self.read_orchestration(&name, now))),
            View::Worktree(dir) => Local::Worktree(Box::new(self.read_worktree(&dir, now))),
            View::Everything => Local::Everything(self.read_everything(now)),
        }
    }

    pub fn read_orchestration(&mut self, name: &str, now: i64) -> OrchestrationLocal {
        let record = record_of(&self.main, name);
        let all = local::worktrees(&self.main);
        let running = claudes();
        let list: Vec<(&git::Worktree, bool)> = all
            .iter()
            .skip(1)
            .filter(|worktree| record::origin_of(&worktree.dir).is_some_and(|origin| origin.name == name))
            .map(|worktree| (worktree, false))
            .collect();
        // herdr answers while git does: each is a process of its own.
        let main = self.main.clone();
        let chat = record.as_ref().and_then(|record| record.chat.clone());
        let is_in_herdr = self.is_in_herdr;
        let herdr_read = std::thread::spawn({
            let name = name.to_string();
            move || {
                if !is_in_herdr {
                    return (Vec::new(), None);
                }
                let panes = herdr::all_panes();
                let chat = local::chat_status(&main, &name, chat.as_ref(), &panes);
                (panes, chat)
            }
        });
        let mut trees = self.trees(&list, None, true, &running, now);
        // The stacks of their own (#332): docker only for those, once in DOCKER_EVERY_MS (#331).
        if trees.iter().any(|tree| tree.has_env && tree.has_own_port) {
            let stacks = self.stacks(now);
            for tree in &mut trees {
                (tree.docker, tree.stack) = match &stacks {
                    None => (Docker::NotYet, None),
                    Some((_, None)) => (Docker::Silent, None),
                    Some((_, Some(projects))) => (Docker::Answered, projects.get(&tree.project).cloned()),
                };
                tree.docker_at = stacks.as_ref().map_or(0, |(at, _)| *at);
            }
        }
        // The migrations where a database runs (#332, STACK's line): its state is in no file, so
        // asked beside git's held facts, each worktree's once in MIGRATIONS_HOLD_MS.
        for tree in &mut trees {
            let is_db_up = tree.has_env && (!tree.has_own_port || crate::model::tree::is_stack_running(tree));
            tree.migrations = is_db_up.then(|| develop::migrations_held(&tree.dir, now)).flatten();
            tree.migrations_at = now;
        }
        let (panes, chat) = herdr_read.join().unwrap_or_default();
        OrchestrationLocal {
            name: name.to_string(),
            title: record
                .as_ref()
                .and_then(|record| record.title.clone())
                .unwrap_or_else(|| name.to_string()),
            trees,
            record_file: record::record_file(&self.main, name),
            free: free_agents(record.as_ref()),
            free_memory: free_memory(),
            fresh_rule: Some(crate::settings::of(&self.main, name).agent_fresh),
            agents: agents_of(&self.main, &panes),
            chat,
            setups: live_setups_of(&self.main, name),
            test_origin: test_origin(&self.main),
            ci: crate::ci::Queue::read(&self.main, now),
            ci_watch: crate::ci_watch::Report::read(&self.main, now),
            ready: crate::ready::Marks::read(&self.main),
            epic: epic_local(&self.main, record.as_ref()),
            orchestrator_share: record
                .as_ref()
                .and_then(|record| record.chat.as_ref())
                .and_then(|chat| crate::conversations::context_share_of(&chat.session_id)),
            journal: crate::journal::last(&self.main, name, JOURNAL_LINES),
            settings: crate::settings::rows_of(&self.main, name),
            merges_on_check: crate::model::merge::OnCheck::of_main(&self.main),
            fresh: crate::config::Orchestrator {
                // A setting (#415): the panel's, else `[orchestrator] auto_fresh`.
                auto_fresh: crate::settings::of(&self.main, name).auto_fresh,
                ..crate::config::of_main(&self.main).config.orchestrator.clone()
            },
            spent: crate::spend::spent_of(&self.main, name, record.as_ref(), now),
            paused: crate::runs::paused_now(&self.main),
            record,
        }
    }

    /// docker's stacks as known now (None: no answer yet), never waiting for
    /// docker: an answer older than DOCKER_EVERY_MS is asked for again on a
    /// thread, one another process wrote to the shared file is taken at once.
    fn stacks(&mut self, now: i64) -> Option<(i64, Option<BTreeMap<String, String>>)> {
        let shared = local::shared_file();
        let mut stacks = locked(&self.docker);
        if let Some(file) = &shared {
            stacks.take_shared(file);
        }
        let is_old = stacks.held.as_ref().is_none_or(|(at, _)| now - at >= DOCKER_EVERY_MS);
        if is_old && !stacks.is_reading {
            stacks.is_reading = true;
            drop(stacks);
            let slot = Arc::clone(&self.docker);
            let reading = std::thread::spawn(move || {
                let found = local::containers();
                let mut stacks = locked(&slot);
                stacks.is_reading = false;
                match &shared {
                    // Its answer, or the one of the process that read docker meanwhile; none yet while it still reads.
                    Some(file) => stacks.take_shared(file),
                    None => {
                        stacks.held = Some((now, found.as_deref().map(local::projects_in)));
                        stacks.taken += 1;
                    }
                }
            });
            if self.should_wait_for_docker {
                let _ = reading.join();
            }
            stacks = locked(&self.docker);
        }
        stacks.held.clone()
    }

    pub fn read_worktree(&mut self, dir: &Path, now: i64) -> WorktreeLocal {
        let all = local::worktrees(&self.main);
        let Some(at) = all.iter().position(|worktree| worktree.dir == dir) else {
            return WorktreeLocal {
                dir: dir.to_path_buf(),
                ..Default::default()
            };
        };
        let projects = compose_projects();
        let is_main = at == 0;
        let tree = self
            .trees(&[(&all[at], is_main)], projects.as_ref(), false, &[], now)
            .remove(0);
        let claude = if is_main {
            None
        } else {
            crate::conversations::read_claude(dir)
        };
        let agent_status = claude.as_ref().filter(|_| self.is_in_herdr).and_then(|claude| {
            herdr::all_panes()
                .into_iter()
                .find(|pane| pane.session.as_deref() == Some(claude.id.as_str()))
                .and_then(|pane| pane.status)
        });
        let offset = if tree.has_own_port { tree.offset } else { 0 };
        let stack = crate::stack::of(dir);
        // git's facts of the view besides the tree, from the tree's memo while its key holds.
        let memo = memo_of(dir, self.known[dir].key);
        let (own_commits, own_diff, develop) = if is_main {
            (0, None, None)
        } else {
            git::remembered(&memo, || {
                let own_diff = diff::own_diff_of(dir).map(|found| (found, git::work_files(dir)));
                (
                    git::own_commits(dir, &tree.branch),
                    own_diff,
                    Some(develop::develop_of(dir)),
                )
            })
        };
        let diff = own_diff.map(|(mut found, mine)| {
            // The files the other worktrees' work touches, each from its own memo while its key holds.
            let others: Vec<(PathBuf, String)> = all
                .iter()
                .skip(1)
                .filter(|other| other.dir != dir)
                .map(|other| (other.dir.clone(), other.branch.clone().unwrap_or_default()))
                .collect();
            let keys = self.keys_of(&others, now);
            let memos: Vec<_> = others
                .iter()
                .zip(keys)
                .map(|((other, _), (key, _))| memo_of(other, key))
                .collect();
            let files: Vec<BTreeSet<String>> = std::thread::scope(|scope| {
                let handles: Vec<_> = others
                    .iter()
                    .zip(&memos)
                    .map(|((other, _), memo)| scope.spawn(move || git::remembered(memo, || git::work_files(other))))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap_or_default())
                    .collect()
            });
            found.with_worktrees = others
                .iter()
                .zip(files)
                .map(|((other, _), files)| diff::Overlap {
                    name: git::feature_of(&self.main, other),
                    files: files.intersection(&mine).cloned().collect(),
                })
                .filter(|other| !other.files.is_empty())
                .collect();
            found
        });
        WorktreeLocal {
            dir: dir.to_path_buf(),
            is_main,
            is_done: !is_main && tree.branch != "(detached)" && tree.changed == 0 && own_commits == 0,
            own_commits,
            claude,
            agent_status,
            has_docker: projects.is_some(),
            containers: if tree.has_env && tree.has_own_port {
                containers_of(dir, &tree.project)
            } else {
                Vec::new()
            },
            diff,
            develop,
            // Only where the view shows them: a shared stack, or an own one that runs (no psql against a stack that is down).
            migrations: if tree.has_env
                && (!tree.has_own_port || tree.stack.as_deref().is_some_and(|stack| stack.starts_with("running")))
            {
                develop::migrations_of(dir)
            } else {
                None
            },
            is_app_up: tree.app_port > 0 && is_port_taken(tree.app_port + offset),
            tools_up: stack
                .iter()
                .flat_map(|stack| &stack.tools)
                .map(|tool| (tool.name.clone(), is_port_taken(i64::from(tool.port) + offset)))
                .collect(),
            stack,
            tree: Some(tree),
        }
    }

    pub fn read_everything(&mut self, now: i64) -> EverythingLocal {
        let all = local::worktrees(&self.main);
        let projects = compose_projects();
        let list: Vec<(&git::Worktree, bool)> = all
            .iter()
            .enumerate()
            .map(|(at, worktree)| (worktree, at == 0))
            .collect();
        let trees = self.trees(&list, projects.as_ref(), false, &[], now);
        let epics = git::orchestrator_checkouts(&self.main)
            .iter()
            .filter_map(|dir| local::read_env(&env_file_of(dir))?.get("COMPOSE_PROJECT_NAME").cloned())
            .collect();
        EverythingLocal { trees, projects, epics }
    }

    /// What a GitLab read of the view asks for, from a local read.
    pub fn ask(&self, local: &Local) -> RemoteAsk {
        match local {
            Local::Orchestration(local) => {
                let mut branches: Vec<String> = local
                    .record
                    .as_ref()
                    .map(|record| record.issues.iter().filter_map(|issue| issue.branch.clone()).collect())
                    .unwrap_or_default();
                for tree in &local.trees {
                    if !branches.contains(&tree.branch) && tree.branch != "(detached)" {
                        branches.push(tree.branch.clone());
                    }
                }
                let mut awaited: Vec<i64> = Vec::new();
                let mut add = |iids: &[i64]| {
                    for iid in iids {
                        if !awaited.contains(iid) {
                            awaited.push(*iid);
                        }
                    }
                };
                if let Some(record) = &local.record {
                    for handover in &record.handovers {
                        add(&handover.after);
                    }
                    for issue in record.issues.iter().filter(|issue| issue.is_queued) {
                        add(&issue.after);
                    }
                }
                for tree in &local.trees {
                    if let Some(wait) = &tree.wait {
                        add(&wait.mrs);
                    }
                }
                let deploys = local
                    .trees
                    .iter()
                    .filter(|tree| {
                        tree.actions
                            .iter()
                            .any(|action| action.is_try() && action.on.as_deref() == Some("test"))
                    })
                    .map(|tree| tree.branch.clone())
                    .collect();
                let mut bundles = std::collections::BTreeMap::new();
                if let Some(record) = &local.record {
                    for issue in record.issues.iter().filter(|issue| issue.numbers.len() > 1) {
                        if let Some(branch) = &issue.branch {
                            bundles.insert(branch.clone(), issue.numbers.clone());
                        }
                    }
                }
                for tree in local.trees.iter().filter(|tree| tree.issues.len() > 1) {
                    let known = bundles.get(&tree.branch).cloned().unwrap_or_default();
                    let numbers = record::bundle_of(tree.issues.first().copied(), &[&tree.issues[..], &known].concat());
                    bundles.insert(tree.branch.clone(), numbers);
                }
                RemoteAsk::Orchestration(Ask {
                    branches,
                    awaited,
                    deploys,
                    bundles,
                    epic: local
                        .record
                        .as_ref()
                        .and_then(|record| crate::batch::branch_of_record(&self.main, record)),
                    shipping: local
                        .record
                        .as_ref()
                        .and_then(|record| record.product.as_deref())
                        .map(|product| crate::batch::shipping_of(&self.main, product))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|entry| entry.branch)
                        .collect(),
                })
            }
            Local::Worktree(local) => RemoteAsk::Worktree {
                branch: local
                    .tree
                    .as_ref()
                    .map(|tree| tree.branch.clone())
                    .filter(|branch| branch != "(detached)"),
            },
            Local::Everything(_) => RemoteAsk::Everything,
        }
    }
}

/// The GitLab side: the client, the disk cache of what is final, what the fast reads keep.
#[derive(Debug)]
pub struct RemoteReader {
    pub gitlab: Gitlab,
    main: PathBuf,
    cache: Cache,
    memory: Memory,
}

impl RemoteReader {
    /// What is final is final for every overview, a print's too: the cache file is shared.
    pub fn new(main: &Path) -> RemoteReader {
        RemoteReader {
            gitlab: Gitlab::new(main),
            main: main.to_path_buf(),
            cache: Cache::open(Some(cache_file(main))),
            memory: Memory::default(),
        }
    }

    /// The read `ask` names; `is_full` looks again at what moves slowly (the first, `r`, after a merge).
    pub async fn read(&mut self, ask: &RemoteAsk, is_full: bool) -> Result<RemoteOf, GitlabError> {
        self.gitlab.begin_read();
        if is_full {
            self.gitlab.fresh();
        }
        match ask {
            RemoteAsk::Orchestration(ask) => {
                read::read_orchestration(&self.gitlab, &mut self.cache, &mut self.memory, ask, is_full)
                    .await
                    .map(RemoteOf::Orchestration)
            }
            RemoteAsk::Worktree { branch } => {
                read::read_worktree(&self.gitlab, &mut self.cache, &mut self.memory, branch.as_deref())
                    .await
                    .map(|remote| RemoteOf::Worktree(Box::new(remote)))
            }
            RemoteAsk::Everything => {
                let main = self.main.clone();
                let refs = tokio::task::spawn_blocking(move || git::remote_heads(&main))
                    .await
                    .unwrap_or_default();
                let develop = git::branch(&self.main);
                let head = refs
                    .iter()
                    .find(|(_, branch)| *branch == develop)
                    .map(|(sha, _)| sha.clone());
                let heads: Vec<String> = refs.into_iter().map(|(_, branch)| branch).collect();
                read::read_everything(&self.gitlab, &heads)
                    .await
                    .map(|remote| RemoteOf::Everything(EverythingRemote { head, ..remote }))
            }
        }
    }
}

/// The open MRs `dir` builds on (`waiting_for`), its git answers kept while
/// the refs they read hold (HEAD, its base's and each MR branch's): the model
/// is built again on every read and GitLab answer, git asked only when one moved.
fn waiting_of(dir: &Path, branch: &str, open: &[OpenMr]) -> Vec<Waiting> {
    /// The git dirs of each worktree and its answers, by dir.
    type Held = (HashMap<PathBuf, Option<(PathBuf, PathBuf)>>, Memos);
    static HELD: std::sync::LazyLock<Mutex<Held>> = std::sync::LazyLock::new(Mutex::default);
    let base = git::base_of(dir).remote;
    let memo = {
        let mut held = HELD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (dirs, memos) = &mut *held;
        let Some((own, common)) = dirs.entry(dir.to_path_buf()).or_insert_with(|| git_dirs(dir)).clone() else {
            return waiting_for(dir, Some(branch), open, &base);
        };
        let mut files: Vec<PathBuf> = ["HEAD", "logs/HEAD"].iter().map(|name| own.join(name)).collect();
        files.extend(["packed-refs", "FETCH_HEAD"].iter().map(|name| common.join(name)));
        files.push(common.join("refs/remotes").join(&base));
        files.extend(
            open.iter()
                .map(|mr| common.join("refs/remotes/origin").join(&mr.branch)),
        );
        memos.of(dir, stamp_of(&files))
    };
    git::remembered(&memo, || waiting_for(dir, Some(branch), open, &base))
}

/// The model of a view from what was read; `remote` None until GitLab answers.
pub fn model_of(local: &Local, remote: Option<&RemoteOf>, state: &mut ViewState, now: i64) -> Model {
    match local {
        Local::Orchestration(local) => {
            let remote = match remote {
                Some(RemoteOf::Orchestration(remote)) => Some(remote),
                _ => None,
            };
            Model::Orchestration(Box::new(orchestration_of(local, remote, state, now)))
        }
        Local::Worktree(local) => {
            let remote = match remote {
                Some(RemoteOf::Worktree(remote)) => Some((**remote).clone()),
                _ => None,
            };
            // The open MRs it builds on: git knows, GitLab names them.
            let waiting = match (&local.tree, &remote) {
                (Some(tree), Some(remote)) => waiting_of(&local.dir, &tree.branch, &remote.open),
                _ => Vec::new(),
            };
            Model::Worktree(Box::new(worktree_of((**local).clone(), remote, waiting)))
        }
        Local::Everything(local) => {
            let remote = match remote {
                Some(RemoteOf::Everything(remote)) => Some(remote.clone()),
                _ => None,
            };
            Model::Everything(Box::new(everything_of(local, remote)))
        }
    }
}

/// The agents by conversation: the run engine's status table (#358, an
/// agent is a session that runs only when something is queued for it), and
/// over it herdr's panes, where a click opened one in a tab (#361); one
/// such whose lines wait past ten minutes `held` (#386).
fn agents_of(main: &Path, panes: &[herdr::Pane]) -> HashMap<String, herdr::Agent> {
    let mut agents = crate::runs::agents_of(main);
    agents.extend(herdr::agents_in(panes));
    let queue: crate::runs::Queue = crate::runs::read_or_default(&crate::runs::queue_file(main));
    crate::runs::mark_held(&mut agents, &queue, crate::time::now_ms());
    agents
}

/// The epic branch of an orchestration's record (#350), else its product's
/// batch (#411), and how far develop has gone on since it, from what origin
/// was last fetched.
fn epic_local(main: &Path, record: Option<&record::Record>) -> Option<crate::model::epic_row::EpicLocal> {
    let branch = crate::batch::branch_of_record(main, record?)?;
    let behind = git::git(
        main,
        &[
            "rev-list",
            "--count",
            &format!("origin/{branch}..{}", git::Base::of(&git::branch(main)).remote),
        ],
    );
    let develop = git::default_branch();
    let shipped = record?
        .product
        .as_deref()
        .map(|product| crate::batch::shipping_of(main, product))
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| crate::batch_red::shipped_of(main, &entry.branch, develop))
        .collect();
    Some(crate::model::epic_row::EpicLocal {
        behind: (behind.status == 0)
            .then(|| behind.stdout.trim().parse().ok())
            .flatten(),
        fixes: record?
            .product
            .as_deref()
            .map(|product| crate::batch::fixes_of(main, product))
            .unwrap_or_default(),
        branch,
        shipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, run, worktree};
    use crate::lock::scratch::Scratch;
    use crate::record::{Origin, write_origin};

    #[test]
    fn takes_docker_s_shared_answer_only_when_it_is_newer_than_the_one_held() {
        let scratch = Scratch::new();
        let file = scratch.join("containers.json");
        let write = |at: i64, containers: Option<Vec<local::StackContainer>>| {
            let text = serde_json::to_string(&local::Containers { at, containers }).unwrap();
            std::fs::write(&file, text).unwrap();
        };
        let mut stacks = Stacks::default();
        // No file yet (another process still asks docker): no answer, not `down`.
        stacks.take_shared(&file);
        assert_eq!((stacks.held.is_none(), stacks.taken), (true, 0));
        let db = local::StackContainer {
            project: "vita-1".into(),
            service: "db".into(),
            state: "running".into(),
            health: String::new(),
            id: String::new(),
        };
        write(1_000, Some(vec![db]));
        stacks.take_shared(&file);
        let running = BTreeMap::from([("vita-1".to_string(), "running(1)".to_string())]);
        assert_eq!(stacks.held, Some((1_000, Some(running.clone()))));
        // The same answer again counts once: the stamp doesn't move.
        stacks.take_shared(&file);
        assert_eq!(stacks.taken, 1);
        // An older one never replaces it; a newer one that says docker is silent does.
        write(500, None);
        stacks.take_shared(&file);
        assert_eq!(stacks.held, Some((1_000, Some(running))));
        write(13_000, None);
        stacks.take_shared(&file);
        assert_eq!((stacks.held, stacks.taken), (Some((13_000, None)), 2));
    }

    #[test]
    fn reads_a_worktree_again_only_when_its_files_changed() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "7-seven", true);
        write_origin(
            &dir,
            &Origin {
                name: "o".into(),
                slot: 0,
            },
        )
        .unwrap();
        let mut reader = LocalReader::new(&main, View::Orchestration("o".into())).unwrap();
        // Never the live herdr from a test.
        reader.is_in_herdr = false;
        let now = now_ms();
        let first = reader.read_orchestration("o", now);
        assert_eq!(first.trees.len(), 1);
        assert_eq!(first.trees[0].ahead, 1);
        // A commit changes HEAD: read again.
        std::fs::write(dir.join("more.md"), "x\n").unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "-q", "-m", "feat: more"]);
        assert_eq!(reader.read_orchestration("o", now).trees[0].ahead, 2);
        let stamp = reader.stamp();
        assert_eq!(reader.stamp(), stamp, "nothing changed");
        // An edit in the working tree shows after STALE_MS.
        std::fs::write(dir.join("edit.md"), "y\n").unwrap();
        assert_eq!(reader.read_orchestration("o", now).trees[0].changed, 0);
        run(&dir, &["add", "edit.md"]);
        assert_ne!(reader.stamp(), stamp, "the index changed");
        assert_eq!(reader.read_orchestration("o", now + STALE_MS).trees[0].changed, 1);
        let Local::Orchestration(local) = reader.read() else {
            panic!()
        };
        let RemoteAsk::Orchestration(ask) = reader.ask(&Local::Orchestration(local)) else {
            panic!()
        };
        assert_eq!(ask.branches, ["feature/7-seven"]);
    }

    /// `dir` with its `.git` moved away: any git call in it fails from now on (`back` returns it).
    fn without_git(dir: &Path) -> impl FnOnce() + use<> {
        let (link, hidden) = (dir.join(".git"), dir.join(".git-hidden"));
        std::fs::rename(&link, &hidden).unwrap();
        move || std::fs::rename(&hidden, &link).unwrap()
    }

    #[test]
    fn takes_a_shared_status_for_no_longer_than_stale_ms_from_its_read() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "14-fourteen", false);
        std::fs::write(dir.join("new.md"), "new\n").unwrap();
        let mut reader = LocalReader::new(&main, View::Everything).unwrap();
        let dirs = [(dir.clone(), "feature/14-fourteen".to_string())];
        let now = now_ms();
        let asked = std::sync::atomic::AtomicUsize::new(0);
        // Another overview read it 10 s ago (the shared reader's answer): held 5 s more, not 15.
        let shared = |dir: &Path, _stamp: u64, _now: i64| {
            asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (now - 10_000, git::git(dir, &STATUS))
        };
        let key = reader.keys_with(&dirs, now, shared)[0].0;
        reader.keys_with(&dirs, now + 4_000, shared);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
        reader.keys_with(&dirs, now + 5_000, shared);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 2);
        // The facts under that key take its answer: no second `git status` (git gone here, the memo answers).
        let back = without_git(&dir);
        let memo = memo_of(&dir, key);
        assert_eq!(git::remembered(&memo, || git::changed_count(&dir)), 1);
        assert_eq!(git::remembered(&memo, || git::git(&dir, &STATUS).lines().count()), 1);
        back();
    }

    #[test]
    fn asks_git_nothing_while_a_worktrees_key_holds() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "9-nine", true);
        write_origin(
            &dir,
            &Origin {
                name: "o".into(),
                slot: 0,
            },
        )
        .unwrap();
        let mut reader = LocalReader::new(&main, View::Orchestration("o".into())).unwrap();
        reader.is_in_herdr = false;
        let now = now_ms();
        let first = reader.read_orchestration("o", now);
        assert_eq!((first.trees[0].ahead, first.trees[0].own_commits), (1, 1));
        let back = without_git(&dir);
        // Its agent's files move (here: its env), git's don't: read again, git answered from memory.
        std::fs::create_dir_all(env_file_of(&dir).parent().unwrap()).unwrap();
        std::fs::write(env_file_of(&dir), "COMPOSE_PROJECT_NAME=vita-9\n").unwrap();
        let again = reader.read_orchestration("o", now + 1);
        assert!(again.trees[0].has_env);
        assert_eq!((again.trees[0].ahead, again.trees[0].own_commits), (1, 1));
        assert_eq!(again.trees[0].files, first.trees[0].files);
        back();
        // A commit moves its key: git is asked again.
        std::fs::write(dir.join("more.md"), "x\n").unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "-q", "-m", "feat: more"]);
        assert_eq!(reader.read_orchestration("o", now + 2).trees[0].ahead, 2);
        // An agent at work: its files move with what it edits, which shows at once, not after STALE_MS.
        std::fs::write(dir.join("edit.md"), "y\n").unwrap();
        std::fs::write(dir.join(".claude/checked.json"), "{}").unwrap();
        assert_eq!(reader.read_orchestration("o", now + 3).trees[0].changed, 1);
    }

    #[test]
    fn reads_the_view_of_a_worktree_from_memory_while_no_key_moved() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "10-ten", true);
        let other = worktree(&main, "11-eleven", false);
        // Both touch one file: an overlap.
        std::fs::write(dir.join("README.md"), "ten\n").unwrap();
        std::fs::write(other.join("README.md"), "eleven\n").unwrap();
        let mut reader = LocalReader::new(&main, View::Worktree(dir.clone())).unwrap();
        reader.is_in_herdr = false;
        let now = now_ms();
        let first = reader.read_worktree(&dir, now);
        let overlaps = |local: &WorktreeLocal| local.diff.as_ref().unwrap().with_worktrees.clone();
        assert_eq!(overlaps(&first).len(), 1);
        assert_eq!(first.own_commits, 1);
        let (back, back_other) = (without_git(&dir), without_git(&other));
        let again = reader.read_worktree(&dir, now + 1);
        assert_eq!(overlaps(&again), overlaps(&first));
        assert_eq!(again.diff.as_ref().unwrap().files, first.diff.as_ref().unwrap().files);
        assert_eq!(again.develop, first.develop);
        assert_eq!(again.own_commits, 1);
        back();
        back_other();
        // The other's work moves on (a commit): its overlap is read again, after STALE_MS at the latest.
        run(&other, &["checkout", "-q", "--", "README.md"]);
        let later = reader.read_worktree(&dir, now + STALE_MS);
        assert!(overlaps(&later).is_empty(), "{:?}", overlaps(&later));
    }

    #[test]
    fn asks_git_what_a_worktree_waits_for_only_when_a_ref_moved() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let base = worktree(&main, "12-base", true);
        let dir = worktree(&main, "13-on-base", false);
        run(&dir, &["reset", "-q", "--hard", "feature/12-base"]);
        let mrs = vec![OpenMr {
            iid: 12,
            branch: "feature/12-base".into(),
            ..Default::default()
        }];
        let counts = || -> Vec<u64> {
            waiting_of(&dir, "feature/13-on-base", &mrs)
                .iter()
                .map(|found| found.count)
                .collect()
        };
        assert_eq!(counts(), [1]);
        let back = without_git(&dir);
        assert_eq!(counts(), [1], "from memory");
        back();
        // The branch it builds on got a commit, pushed: its ref moved, git is asked again.
        std::fs::write(base.join("b.md"), "b\n").unwrap();
        run(&base, &["add", "."]);
        run(&base, &["commit", "-q", "-m", "feat: b"]);
        run(&base, &["push", "-q", "origin", "feature/12-base"]);
        run(&dir, &["fetch", "-q", "origin"]);
        run(&dir, &["merge", "-q", "--ff-only", "origin/feature/12-base"]);
        assert_eq!(counts(), [2]);
    }

    #[test]
    fn builds_the_model_of_each_view_without_gitlab() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let dir = worktree(&main, "8-eight", false);
        let mut state = ViewState::default();
        let mut reader = LocalReader::new(&dir, View::Everything).unwrap();
        reader.is_in_herdr = false;
        let local = reader.read();
        let Model::Everything(model) = model_of(&local, None, &mut state, now_ms()) else {
            panic!()
        };
        assert_eq!(model.trees.unwrap().len(), 2);
        let mut reader = LocalReader::new(&main, View::Worktree(dir.clone())).unwrap();
        reader.is_in_herdr = false;
        let local = reader.read();
        let Model::Worktree(model) = model_of(&local, None, &mut state, now_ms()) else {
            panic!()
        };
        assert_eq!(model.local.tree.as_ref().unwrap().name, "8-eight");
        assert_eq!(model.local.diff.as_ref().unwrap().files, 1);
        assert_eq!(
            reader.ask(&local),
            RemoteAsk::Worktree {
                branch: Some("feature/8-eight".into())
            }
        );
    }
}
