//! The reads behind a view: the local one (disk, git, herdr) cheap enough to
//! run every couple of seconds, the GitLab one for a refresh every half
//! minute, and the model of the view built from both.
//!
//! Local: each worktree's facts are read again only when one of the files
//! they come from changed (its `.claude` files, its git dir's HEAD and index,
//! the refs of its branch and develop, its agent's transcript) or after
//! STALE_MS (edits in the working tree touch no file git or we watch). A stat
//! of a few dozen files is all an idle read costs: idle CPU near zero without
//! a watcher thread or crate (rewrite rules 4 and 7).
//!
//! GitLab: `RemoteReader` holds the client, the disk cache of what is final
//! and what the fast reads keep; it runs in a task of its own while the view
//! draws, asked by an `Ask` the local read made.

use crate::conversations::{bound_session, transcript_of};
use crate::git::{self, git_dirs};
use crate::gitlab::Remote;
use crate::gitlab::cache::{Cache, cache_file};
use crate::gitlab::client::{Gitlab, GitlabError};
use crate::gitlab::read::{self, Ask, EverythingRemote, Memory, WorktreeRemote};
use crate::herdr;
use crate::local::{
    self, claudes, compose_projects, containers_of, free_agents, is_port_taken, test_origin, tree_of,
    with_orchestration_facts,
};
use crate::model::everything::{EverythingLocal, EverythingModel, everything_of};
use crate::model::memory::free_memory;
use crate::model::orchestration::{OrchestrationLocal, OrchestrationModel, ViewState, orchestration_of};
use crate::model::waiting::waiting_for;
use crate::model::worktree::{STORYBOOK_PORT, STUDIO_PORT, WorktreeLocal, WorktreeModel, worktree_of};
use crate::model::{Docker, Tree, develop, diff};
use crate::record::{self, record_of};
use crate::setups::live_setups_of;
use crate::time::now_ms;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// How long a worktree's facts hold without a change of its files: edits in the working tree touch none.
pub const STALE_MS: i64 = 15_000;
/// How long docker's stacks hold for WORKTREES (#331: one docker read every 10–15 s, never a poll per read).
pub const DOCKER_EVERY_MS: i64 = 12_000;

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
    stamp: u64,
    at: i64,
    /// Read with what an orchestration adds.
    is_full: bool,
    tree: Tree,
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
    git_dirs: HashMap<PathBuf, Option<(PathBuf, PathBuf)>>,
    /// Where each conversation's transcript is: it stays where claude put it.
    transcripts: HashMap<String, PathBuf>,
    diffs: diff::DiffCache,
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
            git_dirs: HashMap::new(),
            transcripts: HashMap::new(),
            diffs: diff::DiffCache::default(),
            docker: Arc::default(),
            should_wait_for_docker: false,
        })
    }

    /// The files a worktree's facts come from.
    fn files_of(&mut self, dir: &Path, branch: &str) -> Vec<PathBuf> {
        let claude = dir.join(".claude");
        let mut files: Vec<PathBuf> = [
            "actions.json",
            "wait.json",
            "origin",
            "checked.json",
            "herdr-session",
            "overview-sent.json",
        ]
        .iter()
        .map(|name| claude.join(name))
        .collect();
        let dirs = self
            .git_dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| git_dirs(dir))
            .clone();
        if let Some((own, common)) = dirs {
            files.extend(
                ["HEAD", "index", "logs/HEAD", "MERGE_HEAD"]
                    .iter()
                    .map(|name| own.join(name)),
            );
            files.extend(
                ["packed-refs", "FETCH_HEAD", "refs/remotes/origin/develop"]
                    .iter()
                    .map(|name| common.join(name)),
            );
            files.push(common.join("refs/remotes/origin").join(branch));
            files.push(common.join("refs/heads").join(branch));
        }
        if let Some(id) = bound_session(dir) {
            let found = match self.transcripts.get(&id) {
                Some(transcript) => Some(transcript.clone()),
                None => transcript_of(&id),
            };
            if let Some(transcript) = found {
                self.transcripts.insert(id, transcript.clone());
                files.push(transcript);
            }
        }
        files
    }

    /// The worktrees' facts: from the last read while their files hold, else
    /// read again, side by side (a dozen git calls each).
    fn trees(
        &mut self,
        list: &[(&git::Worktree, bool)],
        projects: Option<&std::collections::BTreeMap<String, String>>,
        is_full: bool,
        claudes: &[crate::conversations::Running],
        now: i64,
    ) -> Vec<Tree> {
        let stamps: Vec<u64> = list
            .iter()
            .map(|(worktree, _)| {
                let files = self.files_of(&worktree.dir, worktree.branch.as_deref().unwrap_or_default());
                stamp_of(&files)
            })
            .collect();
        let is_known = |at: usize, known: &HashMap<PathBuf, Known>| {
            known.get(&list[at].0.dir).is_some_and(|known| {
                known.stamp == stamps[at] && now - known.at < STALE_MS && (known.is_full || !is_full)
            })
        };
        let (main, here) = (&self.main, &self.here);
        let read: Vec<Option<Tree>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..list.len())
                .map(|at| {
                    (!is_known(at, &self.known)).then(|| {
                        let (worktree, is_main) = list[at];
                        scope.spawn(move || {
                            let tree = tree_of(main, here, worktree, is_main, projects);
                            if is_full {
                                with_orchestration_facts(tree, claudes)
                            } else {
                                tree
                            }
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
                Some(tree) => {
                    let known = Known {
                        stamp: stamps[at],
                        at: now,
                        is_full,
                        tree: tree.clone(),
                    };
                    self.known.insert(list[at].0.dir.clone(), known);
                    tree
                }
                None => {
                    let mut tree = self.known[&list[at].0.dir].tree.clone();
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
        }
        let known: Vec<(PathBuf, String)> = self
            .known
            .values()
            .map(|known| (known.tree.dir.clone(), known.tree.branch.clone()))
            .collect();
        for (dir, branch) in known {
            files.extend(self.files_of(&dir, &branch));
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
                    Some(None) => (Docker::Silent, None),
                    Some(Some(projects)) => (Docker::Answered, projects.get(&tree.project).cloned()),
                };
            }
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
            agents: herdr::agents_in(&panes),
            chat,
            setups: live_setups_of(&self.main, name),
            test_origin: test_origin(&self.main),
            record,
        }
    }

    /// docker's stacks as known now (None: no answer yet), never waiting for
    /// docker: an answer older than DOCKER_EVERY_MS is asked for again on a
    /// thread, one another process wrote to the shared file is taken at once.
    fn stacks(&mut self, now: i64) -> Option<Option<BTreeMap<String, String>>> {
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
        stacks.held.as_ref().map(|(_, projects)| projects.clone())
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
        let own_commits = if is_main {
            0
        } else {
            git::own_commits(dir, &tree.branch)
        };
        let offset = if tree.has_own_port { tree.offset } else { 0 };
        let diff = if is_main {
            None
        } else {
            let others: Vec<(String, BTreeSet<String>)> = all
                .iter()
                .skip(1)
                .filter(|other| other.dir != dir)
                .map(|other| (git::feature_of(&self.main, &other.dir), git::work_files(&other.dir)))
                .collect();
            self.diffs.own_diff_of(dir).map(|mut found| {
                let mine = git::work_files(dir);
                found.with_worktrees = others
                    .into_iter()
                    .map(|(name, files)| diff::Overlap {
                        name,
                        files: files.intersection(&mine).cloned().collect(),
                    })
                    .filter(|other| !other.files.is_empty())
                    .collect();
                found
            })
        };
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
            develop: (!is_main).then(|| develop::develop_of(dir)),
            // Only where the view shows them: a shared stack, or an own one that runs (no psql against a stack that is down).
            migrations: if tree.has_env
                && (!tree.has_own_port || tree.stack.as_deref().is_some_and(|stack| stack.starts_with("running")))
            {
                develop::migrations_of(dir)
            } else {
                None
            },
            is_app_up: is_port_taken(3000 + offset),
            is_studio_up: is_port_taken(STUDIO_PORT + offset),
            is_storybook_up: is_port_taken(STORYBOOK_PORT + offset),
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
        EverythingLocal { trees, projects }
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
                RemoteAsk::Orchestration(Ask {
                    branches,
                    awaited,
                    deploys,
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
                let head = refs
                    .iter()
                    .find(|(_, branch)| branch == "develop")
                    .map(|(sha, _)| sha.clone());
                let heads: Vec<String> = refs.into_iter().map(|(_, branch)| branch).collect();
                read::read_everything(&self.gitlab, &heads)
                    .await
                    .map(|remote| RemoteOf::Everything(EverythingRemote { head, ..remote }))
            }
        }
    }
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
                (Some(tree), Some(remote)) => {
                    waiting_for(&local.dir, Some(&tree.branch), &remote.open, "origin/develop")
                }
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
