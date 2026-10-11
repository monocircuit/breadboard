//! The repo as the setup commands see it (worktrees.mjs, stack.mjs,
//! constants.mjs): the main checkout, its worktrees, the vita stack of each
//! (its `.env.local`, the ports moved by an offset).

use super::proc::fail;
use breadboard_core::git::{self as core_git, Worktree};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use crate::cli::lifecycle::repo::{VITA, env_file_of};

/// Default port → the variable it is moved with in the `.env.local` of a
/// worktree, in the order they are written (constants.mjs `PORTS`).
pub const PORTS: [(u16, Option<&str>); 7] = [
    (3000, None),
    (4983, None),
    (5432, Some("POSTGRES_PORT")),
    (6006, None),
    (8080, Some("KEYCLOAK_PORT")),
    (8333, Some("S3_PORT")),
    (64738, Some("VISUALIZER_PORT")),
];

/// Where the commands stand: `INIT_CWD` (pnpm keeps the place it was typed
/// in) when it lies in the repo of the working directory, else the working
/// directory. An `INIT_CWD` of another repo (a pnpm script of one checkout
/// that runs this in another, a world under a checkout) is not followed.
pub fn here() -> PathBuf {
    static HERE: OnceLock<PathBuf> = OnceLock::new();
    HERE.get_or_init(|| {
        let cwd = cwd();
        std::env::var_os("INIT_CWD")
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir() && *dir != cwd)
            .filter(|dir| find_main().is_some_and(|main| core_git::main_dir(dir).as_ref() == Some(&main)))
            .unwrap_or(cwd)
    })
    .clone()
}

/// The working directory itself.
pub fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

static MAIN: OnceLock<PathBuf> = OnceLock::new();

/// The main checkout of the repo of the working directory; asked once, so a
/// worktree removed meanwhile doesn't lose it. Never from `INIT_CWD` or from
/// where the binary lies: a binary of one checkout run outside any would act
/// on that checkout (fetch, worktree add, push its claim).
pub fn main_dir() -> PathBuf {
    find_main().unwrap_or_else(|| fail("not inside the repo: run it in a checkout of monocircuit/platform"))
}

/// The main checkout, None outside any repo (for what only helps inside one).
pub fn find_main() -> Option<PathBuf> {
    if let Some(main) = MAIN.get() {
        return Some(main.clone());
    }
    let main = core_git::main_dir(&cwd())?;
    Some(MAIN.get_or_init(|| main).clone())
}

/// The repo of the working directory as the lifecycle group takes it: what
/// the setup commands share with it (the layout of herdr, the issues of an
/// orchestration) is its code, called with this.
pub fn lifecycle() -> crate::cli::lifecycle::repo::Repo {
    crate::cli::lifecycle::repo::Repo { main: main_dir() }
}

/// The worktrees, the main checkout first, without the orchestrations' checkouts.
pub fn worktrees() -> Vec<Worktree> {
    core_git::worktrees(&main_dir())
}

/// `<main>-<feature>`.
pub fn dir_of(feature: &str) -> PathBuf {
    core_git::dir_of(&main_dir(), feature)
}

/// `platform-40-x` → `40-x`.
pub fn feature_of(dir: &Path) -> String {
    core_git::feature_of(&main_dir(), dir)
}

/// The basename of a directory.
pub fn base_of(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether `here` is `dir` or lies in it, as text (`platform-40` is not in `platform-4`).
pub fn is_inside(here: &Path, dir: &Path) -> bool {
    core_git::is_inside(here, dir)
}

/// Commits of `branch` that develop lacks by patch.
pub fn has_own_commits(branch: &str, cwd: &Path) -> bool {
    core_git::own_commits(cwd, branch) > 0
}

/// The offset of a worktree's own stack, 0 for the shared one (the lifecycle group's reading).
pub fn offset_of(dir: &Path) -> u16 {
    u16::try_from(crate::cli::lifecycle::repo::offset_of(dir)).unwrap_or(0)
}

/// Whether nothing listens on `port` of 127.0.0.1.
pub fn is_port_free(port: u32) -> bool {
    !breadboard_core::local::is_port_taken(i64::from(port))
}

/// The checkout whose stack answers on `port`: the app port or one of its
/// services', offset 0 the main checkout; an orchestration's epic stack too.
pub fn worktree_on_port(port: u32) -> Option<Worktree> {
    let offset = PORTS
        .iter()
        .map(|(base, _)| i64::from(port) - i64::from(*base))
        .find(|candidate| (0..50).contains(candidate))?;
    let all = core_git::all_worktrees(&main_dir());
    if offset == 0 {
        return all.into_iter().next();
    }
    all.into_iter().find(|tree| i64::from(offset_of(&tree.dir)) == offset)
}
