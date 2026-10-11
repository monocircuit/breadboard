//! git as the overview reads it: the worktrees of the repo, the main checkout,
//! and what each worktree's branch holds against its base (develop or an epic). Read only, and
//! without git's optional locks: a `git status` every few seconds would
//! otherwise take the index lock under a commit of the same worktree.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// What a git call answered: `ok` its exit code was 0, `out` its output
/// trimmed (proc.mjs `git`), `stdout` as it came.
#[derive(Debug, Clone, Default)]
pub struct Out {
    pub is_ok: bool,
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    /// The output trimmed, as proc.mjs `git` gives it.
    pub fn out(&self) -> &str {
        self.stdout.trim()
    }
    /// The non-empty lines of the output.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.stdout.split('\n').filter(|line| !line.is_empty())
    }
}

/// What git answered, by its folder and words: `remembered` answers from it.
pub type Memo = HashMap<(PathBuf, Vec<String>), Out>;

thread_local! {
    /// The memo `remembered` set for this thread's reads.
    static MEMO: RefCell<Option<Arc<Mutex<Memo>>>> = const { RefCell::new(None) };
}

/// The calls whose answer depends only on what a worktree's key holds (its
/// HEAD, index, refs, the status of its files): never one that asks origin.
const REMEMBERED: [&str; 10] = [
    "status",
    "reflog",
    "rev-parse",
    "rev-list",
    "merge-base",
    "diff",
    "cherry",
    "log",
    "merge-tree",
    "symbolic-ref",
];

/// `read` with this thread's git reads answered from `memo` where it holds
/// the same call, and kept there otherwise. The caller keeps a memo only
/// while what the calls depend on holds (reader.rs: a worktree's key), so a
/// worktree that didn't change is read without a git process.
pub fn remembered<T>(memo: &Arc<Mutex<Memo>>, read: impl FnOnce() -> T) -> T {
    let was = MEMO.with(|slot| slot.replace(Some(Arc::clone(memo))));
    let value = read();
    MEMO.with(|slot| *slot.borrow_mut() = was);
    value
}

/// `git <args>` in `dir`, quiet, never failing: a git that can't run answers
/// a failed `Out`. Inside `remembered`, a read is answered from its memo.
pub fn git(dir: &Path, args: &[&str]) -> Out {
    let memo = MEMO
        .with(|slot| slot.borrow().clone())
        .filter(|_| args.first().is_some_and(|first| REMEMBERED.contains(first)));
    let Some(memo) = memo else {
        return run_git(dir, args);
    };
    let key = (dir.to_path_buf(), args.iter().map(|arg| arg.to_string()).collect());
    if let Some(out) = memo.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return out.clone();
    }
    let out = run_git(dir, args);
    memo.lock().unwrap_or_else(|e| e.into_inner()).insert(key, out.clone());
    out
}

fn run_git(dir: &Path, args: &[&str]) -> Out {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) => Out {
            is_ok: output.status.success(),
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(error) => Out {
            is_ok: false,
            status: -1,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

/// A count git prints (`rev-list --count`), None when it fails.
pub fn count(dir: &Path, range: &str) -> Option<u64> {
    let out = git(dir, &["rev-list", "--count", range]);
    if out.is_ok { out.out().parse().ok() } else { None }
}

/// A worktree of the repo: its directory and branch (None when detached).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub dir: PathBuf,
    pub branch: Option<String>,
}

/// The worktrees in `git worktree list --porcelain`, the main checkout first.
pub fn parse_worktrees(porcelain: &str) -> Vec<Worktree> {
    porcelain
        .split("\n\n")
        .filter_map(|block| {
            let get = |key: &str| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix(key).and_then(|rest| rest.strip_prefix(' ')))
            };
            let dir = get("worktree")?;
            Some(Worktree {
                dir: PathBuf::from(dir),
                branch: get("branch").map(|branch| branch.trim_start_matches("refs/heads/").to_string()),
            })
        })
        .collect()
}

/// Every worktree of the repo `from` stands in, the main checkout first; none
/// outside a repo.
pub fn all_worktrees(from: &Path) -> Vec<Worktree> {
    parse_worktrees(&git(from, &["worktree", "list", "--porcelain"]).stdout)
}

/// The folder of the orchestrations' checkouts in the main checkout: they
/// carry no work, only the place of a space in herdr's sidebar
/// (orchestrator.mjs `checkoutOf`).
pub fn orchestrations_dir(main: &Path) -> PathBuf {
    main.join(".claude").join("orchestrations")
}

/// The worktrees without the orchestrations' checkouts (worktrees.mjs `worktrees`).
pub fn worktrees(from: &Path) -> Vec<Worktree> {
    let all = all_worktrees(from);
    let Some(main) = all.first().map(|tree| tree.dir.clone()) else {
        return all;
    };
    let skip = orchestrations_dir(&main);
    all.into_iter()
        .filter(|tree| tree.dir == main || !tree.dir.starts_with(&skip))
        .collect()
}

/// The checkouts of the orchestrations (orchestrator.mjs `orchestratorCheckouts`).
pub fn orchestrator_checkouts(from: &Path) -> Vec<PathBuf> {
    let all = all_worktrees(from);
    let Some(main) = all.first().map(|tree| tree.dir.clone()) else {
        return Vec::new();
    };
    let folder = orchestrations_dir(&main);
    all.into_iter()
        .map(|tree| tree.dir)
        .filter(|dir| dir.parent() == Some(folder.as_path()))
        .collect()
}

/// The main checkout of the repo `from` stands in, None outside one.
pub fn main_dir(from: &Path) -> Option<PathBuf> {
    all_worktrees(from).into_iter().next().map(|tree| tree.dir)
}

/// The name of a worktree after the main checkout's: `platform-292-x` → `292-x`
/// (tabs.mjs `featureOf`).
pub fn feature_of(main: &Path, dir: &Path) -> String {
    let base = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let prefix = format!(
        "{}-",
        main.file_name().map(|name| name.to_string_lossy()).unwrap_or_default()
    );
    base.replacen(&prefix, "", 1)
}

/// The worktree of a feature next to the main checkout: `<main>-<feature>`.
pub fn dir_of(main: &Path, feature: &str) -> PathBuf {
    PathBuf::from(format!("{}-{feature}", main.display()))
}

/// Whether `here` is `dir` or lies in it (prune.mjs `isInside`).
pub fn is_inside(here: &Path, dir: &Path) -> bool {
    here == dir || here.starts_with(dir)
}

/// The issue a branch claims: `feature/34-x` → 34 (`/(\d+)-/`).
pub fn issue_of_branch(branch: &str) -> Option<u64> {
    let bytes = branch.as_bytes();
    (0..bytes.len()).filter(|&at| bytes[at] == b'/').find_map(|at| {
        let digits = bytes[at + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
        (digits > 0 && bytes.get(at + 1 + digits) == Some(&b'-'))
            .then(|| branch[at + 1..at + 1 + digits].parse().ok())?
    })
}

/// How many files are changed, new or deleted in the working tree.
pub fn changed_count(dir: &Path) -> usize {
    // Files, not lines: a new folder is each of its files (DIFF's `N uncommitted`, actions' `N uncommitted files`).
    git(dir, &["status", "--porcelain", "--untracked-files=all"])
        .lines()
        .count()
}

/// The default branch of the repo of `dir` (breadboard.toml, #342):
/// `develop`, `main`. The base of every worktree without another.
pub fn branch(dir: &Path) -> String {
    crate::config::of(dir).branch.clone()
}

/// The default branch of the repo of the working directory, for what names
/// it as text that lives as long as the process (an MR's target, a diff's base).
pub fn default_branch() -> &'static str {
    static BRANCH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BRANCH.get_or_init(|| crate::config::here().branch.clone())
}

/// `origin/<the base of the worktree dir>`: its epic's or the default branch's.
pub fn base(dir: &Path) -> String {
    base_of(dir).remote
}

/// The branch a worktree branches off, is compared against, merges in and
/// opens its MR into (#349): the default branch, or the epic of its orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Base {
    /// The branch: `develop`, `epic/12-name`.
    pub local: String,
    /// Its ref on origin: `origin/develop`.
    pub remote: String,
}

impl Base {
    pub fn of(branch: &str) -> Base {
        Base {
            local: branch.to_string(),
            remote: format!("origin/{branch}"),
        }
    }

    /// The default branch of the repo of the working directory (breadboard.toml).
    pub fn develop() -> Base {
        Base::of(&crate::config::here().branch)
    }

    /// Whether it is the default branch, no epic.
    pub fn is_develop(&self) -> bool {
        self.local == crate::config::here().branch
    }
}

impl Default for Base {
    fn default() -> Base {
        Base::develop()
    }
}

/// The base of the worktree `dir`: `base` of its `.claude/origin`, the
/// repo's default branch without one (every worktree from before #349).
pub fn base_of(dir: &Path) -> Base {
    crate::record::base_in(dir).map_or_else(|| Base::of(&branch(dir)), |base| Base::of(&base))
}

/// Commits of HEAD its base doesn't have.
pub fn ahead_of_base(dir: &Path) -> u64 {
    count(dir, &format!("{}..HEAD", base_of(dir).remote)).unwrap_or(0)
}

/// Commits of its base HEAD doesn't have.
pub fn behind_base(dir: &Path) -> u64 {
    count(dir, &format!("HEAD..{}", base_of(dir).remote)).unwrap_or(0)
}

/// Commits of `branch` its base doesn't have by patch (`git cherry`), what
/// keeps a merged worktree from wt:remove (prune.mjs `hasOwnCommits`,
/// counted): a task merged into its epic has none.
pub fn own_commits(dir: &Path, branch: &str) -> usize {
    own_commits_on(dir, &base_of(dir), branch)
}

/// Commits of `branch` `base` doesn't have by patch.
pub fn own_commits_on(dir: &Path, base: &Base, branch: &str) -> usize {
    if branch == "(detached)" {
        return 0;
    }
    git(dir, &["cherry", &base.remote, branch])
        .lines()
        .filter(|line| line.starts_with('+'))
        .count()
}

/// The bases there are on origin (as of the last fetch): develop and every
/// epic branch, for a branch whose worktree is gone and with it its base.
pub fn bases(dir: &Path) -> Vec<Base> {
    let epics = git(
        dir,
        &[
            "for-each-ref",
            "--format=%(refname:lstrip=3)",
            "refs/remotes/origin/epic/",
        ],
    );
    std::iter::once(Base::of(&branch(dir)))
        .chain(epics.lines().map(Base::of))
        .collect()
}

/// Whether `branch` has commits its base lacks (`dir` its worktree), or,
/// asked in a checkout based on develop (the main one), that every base lacks.
pub fn has_own_commits(dir: &Path, branch: &str) -> bool {
    if base_of(dir).is_develop() {
        own_commits_anywhere(dir, branch) > 0
    } else {
        own_commits(dir, branch) > 0
    }
}

/// Commits of `branch` no base has: a branch without its worktree is done
/// once develop or any epic holds it (#349).
pub fn own_commits_anywhere(dir: &Path, branch: &str) -> usize {
    bases(dir)
        .iter()
        .map(|base| own_commits_on(dir, base, branch))
        .min()
        .unwrap_or(0)
}

/// The commit HEAD and its base started from, None without one.
pub fn merge_base(dir: &Path) -> Option<String> {
    let out = git(dir, &["merge-base", &base_of(dir).remote, "HEAD"]);
    Some(out.out().to_string()).filter(|sha| out.is_ok && !sha.is_empty())
}

/// How much of a worktree's branch is on origin (change.mjs `pushStateOf`):
/// from the refs git has, no fetch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushState {
    /// The commit of `origin/<branch>`, None without one.
    pub pushed: Option<String>,
    /// Commits of HEAD origin doesn't have (without a remote branch: the ones its base doesn't have).
    pub unpushed: u64,
    /// When the oldest of them was made, ms since the epoch.
    pub unpushed_at: Option<i64>,
    pub has_remote_branch: bool,
}

pub fn push_state(dir: &Path, branch: Option<&str>) -> PushState {
    let remote = branch.map(|branch| format!("refs/remotes/origin/{branch}"));
    let pushed = match (&remote, branch) {
        (Some(remote), Some(branch)) if branch != "(detached)" => {
            let out = git(dir, &["rev-parse", "--verify", "--quiet", remote]);
            Some(out.out().to_string()).filter(|sha| out.is_ok && !sha.is_empty())
        }
        _ => None,
    };
    let base = if pushed.is_some() {
        remote.clone().unwrap_or_default()
    } else {
        base_of(dir).remote
    };
    let log = git(dir, &["log", "--format=%ct", &format!("{base}..HEAD")]);
    let times: Vec<i64> = log.lines().filter_map(|line| line.trim().parse().ok()).collect();
    PushState {
        has_remote_branch: pushed.is_some(),
        pushed,
        unpushed: times.len() as u64,
        unpushed_at: times.last().map(|seconds| seconds * 1000),
    }
}

/// The heads on origin: `(sha, branch)` of `git ls-remote --heads origin`.
pub fn remote_heads(dir: &Path) -> Vec<(String, String)> {
    git(dir, &["ls-remote", "--heads", "origin"])
        .lines()
        .filter_map(|line| {
            let (sha, reference) = line.split_once("\trefs/heads/")?;
            Some((sha.to_string(), reference.to_string()))
        })
        .collect()
}

/// A path of `status --porcelain`: the new name of a rename, without quotes.
pub fn path_of_status(line: &str) -> String {
    let path = line.get(3..).unwrap_or_default();
    let path = path.rsplit(" -> ").next().unwrap_or(path);
    path.trim_start_matches('"').trim_end_matches('"').to_string()
}

/// The files a worktree's work touches: committed since the merge base with
/// its base, changed or new (diff.mjs `filesOf`).
pub fn work_files(dir: &Path) -> BTreeSet<String> {
    let Some(base) = merge_base(dir) else {
        return BTreeSet::new();
    };
    let mut files: BTreeSet<String> = git(dir, &["diff", "--name-only", &base])
        .lines()
        .map(String::from)
        .collect();
    files.extend(
        git(dir, &["status", "--porcelain", "-uall"])
            .lines()
            .map(path_of_status),
    );
    files
}

/// Of `files`, the ones its base changed too since the merge base: where the
/// work collides with it (diff.mjs, DIFF's `⚠ also changed on develop`).
pub fn on_base(dir: &Path, files: &BTreeSet<String>) -> Vec<String> {
    if files.is_empty() {
        return Vec::new();
    }
    let Some(base) = merge_base(dir) else {
        return Vec::new();
    };
    git(dir, &["diff", "--name-only", &base, &base_of(dir).remote])
        .lines()
        .filter(|file| files.contains(*file))
        .map(String::from)
        .collect()
}

/// The files the commits of a worktree's branch touched, from where wt:new
/// made the branch (its reflog's oldest entry) along the first parent and
/// without merges: what its base brought in doesn't count, and it stays the
/// same after the merge (worktree/workers.mjs `filesOf`).
pub fn branch_files(dir: &Path) -> Vec<String> {
    let branch = git(dir, &["symbolic-ref", "--quiet", "HEAD"]);
    if !branch.is_ok || branch.out().is_empty() {
        return Vec::new();
    }
    let reflog = git(dir, &["reflog", "show", "--format=%H", branch.out()]);
    let Some(created) = reflog.lines().last().map(str::to_string) else {
        return Vec::new();
    };
    let log = git(
        dir,
        &[
            "log",
            "--first-parent",
            "--no-merges",
            "--format=",
            "--name-only",
            &format!("{created}..HEAD"),
        ],
    );
    let mut seen = BTreeSet::new();
    log.lines()
        .filter(|file| seen.insert(file.to_string()))
        .map(String::from)
        .collect()
}

/// The commit HEAD points at, None in a repo without one.
pub fn head_of(dir: &Path) -> Option<String> {
    let out = git(dir, &["rev-parse", "--verify", "--quiet", "HEAD"]);
    Some(out.out().to_string()).filter(|sha| out.is_ok && !sha.is_empty())
}

/// The tree of a commit (check.mjs `treeOf`).
pub fn tree_of(dir: &Path, rev: &str) -> Option<String> {
    let out = git(dir, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{tree}}")]);
    Some(out.out().to_string()).filter(|tree| out.is_ok && !tree.is_empty())
}

/// The git directory of a worktree (`.git/worktrees/<name>` for a linked one)
/// and the common one, for watching what changes its state.
pub fn git_dirs(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let out = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir"],
    );
    let mut lines = out.lines();
    Some((PathBuf::from(lines.next()?), PathBuf::from(lines.next()?)))
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A repo with a bare origin, a develop and worktrees, for the tests.
    use super::*;
    use crate::lock::scratch::Scratch;

    pub fn run(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// `<scratch>/platform` on develop, pushed to `<scratch>/origin.git`.
    pub fn repo(scratch: &Scratch) -> PathBuf {
        let origin = scratch.join("origin.git");
        let main = scratch.join("platform");
        run(
            scratch.path(),
            &[
                "init",
                "--quiet",
                "--bare",
                "--initial-branch=develop",
                origin.to_str().unwrap(),
            ],
        );
        run(
            scratch.path(),
            &["init", "--quiet", "--initial-branch=develop", main.to_str().unwrap()],
        );
        run(&main, &["remote", "add", "origin", origin.to_str().unwrap()]);
        std::fs::write(main.join(".gitignore"), ".claude/\n").unwrap();
        std::fs::write(main.join("README.md"), "# platform\n").unwrap();
        run(&main, &["add", "."]);
        run(&main, &["commit", "--quiet", "-m", "chore: begin"]);
        run(&main, &["push", "--quiet", "origin", "develop"]);
        run(&main, &["fetch", "--quiet", "origin"]);
        // origin's HEAD names the default branch where no breadboard.toml does.
        run(&main, &["remote", "set-head", "origin", "develop"]);
        main
    }

    /// The worktree `<main>-<feature>` on `feature/<feature>` with one commit.
    pub fn worktree(main: &Path, feature: &str, is_pushed: bool) -> PathBuf {
        let dir = dir_of(main, feature);
        let branch = format!("feature/{feature}");
        run(
            main,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                &branch,
                dir.to_str().unwrap(),
                "develop",
            ],
        );
        std::fs::create_dir_all(dir.join("work")).unwrap();
        std::fs::write(dir.join(format!("work/{feature}.md")), format!("{feature}\n")).unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "--quiet", "-m", &format!("feat: {feature}")]);
        if is_pushed {
            run(&dir, &["push", "--quiet", "origin", &branch]);
        }
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::lock::scratch::Scratch;

    #[test]
    fn answers_reads_from_its_memo_and_never_a_call_to_origin() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let memo: Arc<Mutex<Memo>> = Arc::default();
        let head = remembered(&memo, || git(&main, &["rev-parse", "HEAD"]).out().to_string());
        std::fs::write(main.join("x.md"), "x\n").unwrap();
        run(&main, &["add", "."]);
        run(&main, &["commit", "--quiet", "-m", "feat: x"]);
        // Its memo holds what it answered: the commit isn't seen through it, it is without.
        assert_eq!(
            remembered(&memo, || git(&main, &["rev-parse", "HEAD"]).out().to_string()),
            head
        );
        assert_ne!(git(&main, &["rev-parse", "HEAD"]).out(), head);
        // A call to origin is never kept.
        remembered(&memo, || remote_heads(&main));
        assert_eq!(memo.lock().unwrap().len(), 1);
    }

    #[test]
    fn reads_the_porcelain_list() {
        let list = parse_worktrees(
            "worktree /r/platform\nHEAD abc\nbranch refs/heads/develop\n\nworktree /r/platform-1-x\nHEAD def\nbranch refs/heads/feature/1-x\n\nworktree /r/platform/.claude/orchestrations/o\nHEAD abc\ndetached\n",
        );
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].branch.as_deref(), Some("feature/1-x"));
        assert_eq!(list[2].branch, None);
    }

    #[test]
    fn names_features_and_issues() {
        assert_eq!(
            feature_of(Path::new("/r/platform"), Path::new("/r/platform-292-x")),
            "292-x"
        );
        assert_eq!(issue_of_branch("feature/292-every-agent"), Some(292));
        assert_eq!(issue_of_branch("fix/12-a"), Some(12));
        assert_eq!(issue_of_branch("feature/update-pnpm-12"), None);
        assert_eq!(issue_of_branch("develop"), None);
        assert!(is_inside(Path::new("/r/a/b"), Path::new("/r/a")));
        assert!(!is_inside(Path::new("/r/ab"), Path::new("/r/a")));
        assert_eq!(path_of_status("R  old.md -> new.md"), "new.md");
        assert_eq!(path_of_status("?? \"a b.md\""), "a b.md");
    }

    #[test]
    fn reads_a_worktree_against_develop_and_origin() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let pushed = worktree(&main, "1-pushed", true);
        let local = worktree(&main, "2-local", false);
        // The orchestration's checkout is no worktree with work.
        run(
            &main,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                main.join(".claude/orchestrations/o").to_str().unwrap(),
            ],
        );

        let trees = worktrees(&local);
        assert_eq!(trees.len(), 3);
        assert_eq!(trees[0].dir, main);
        assert_eq!(all_worktrees(&main).len(), 4);
        assert_eq!(
            orchestrator_checkouts(&main),
            vec![main.join(".claude/orchestrations/o")]
        );
        assert_eq!(main_dir(&pushed).as_deref(), Some(main.as_path()));

        assert_eq!(ahead_of_base(&pushed), 1);
        assert_eq!(behind_base(&pushed), 0);
        assert_eq!(own_commits(&pushed, "feature/1-pushed"), 1);
        let state = push_state(&pushed, Some("feature/1-pushed"));
        assert!(state.has_remote_branch);
        assert_eq!(state.unpushed, 0);
        let state = push_state(&local, Some("feature/2-local"));
        assert!(!state.has_remote_branch);
        assert_eq!(state.unpushed, 1);
        assert!(state.unpushed_at.is_some());

        std::fs::write(local.join("new.md"), "new\n").unwrap();
        assert_eq!(changed_count(&local), 1);
        // A new folder counts its files, not one line of `git status`.
        std::fs::create_dir(local.join("new")).unwrap();
        std::fs::write(local.join("new/a.md"), "a\n").unwrap();
        std::fs::write(local.join("new/b.md"), "b\n").unwrap();
        assert_eq!(changed_count(&local), 3);
        std::fs::remove_dir_all(local.join("new")).unwrap();
        assert_eq!(
            work_files(&local).into_iter().collect::<Vec<_>>(),
            vec!["new.md".to_string(), "work/2-local.md".to_string()]
        );
        assert_eq!(branch_files(&local), vec!["work/2-local.md".to_string()]);
        // develop moves on with a file the work changes too: they collide there.
        assert!(on_base(&local, &work_files(&local)).is_empty());
        std::fs::write(main.join("new.md"), "develop's\n").unwrap();
        run(&main, &["add", "new.md"]);
        run(&main, &["commit", "--quiet", "-m", "feat: develop's new.md"]);
        run(&main, &["push", "--quiet", "origin", "develop"]);
        run(&main, &["fetch", "--quiet", "origin"]);
        assert_eq!(on_base(&local, &work_files(&local)), vec!["new.md".to_string()]);
        assert_eq!(behind_base(&local), 1);
        assert!(
            remote_heads(&main)
                .iter()
                .any(|(_, branch)| branch == "feature/1-pushed")
        );
        assert!(git_dirs(&local).is_some());
    }
}
