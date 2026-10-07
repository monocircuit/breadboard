//! The repo as the setup commands see it (worktrees.mjs, stack.mjs,
//! constants.mjs): the main checkout, its worktrees, the vita stack of each
//! (its `.env.local`, the ports moved by an offset).

use super::proc::fail;
use breadboard_core::git::{self as core_git, Worktree};
use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The vita app inside a checkout.
pub const VITA: &str = "products/vita/web";

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
/// drizzle studio's default port; a worktree's moves by its offset.
pub const STUDIO_PORT: u16 = 4983;

/// Where the commands stand: `INIT_CWD` (pnpm keeps the place it was typed
/// in), else the working directory.
pub fn here() -> PathBuf {
    std::env::var_os("INIT_CWD")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The working directory itself.
pub fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

static MAIN: OnceLock<PathBuf> = OnceLock::new();

/// The main checkout of the repo the command runs in, from the cwd or
/// `INIT_CWD` only; asked once, so a worktree removed meanwhile doesn't lose
/// it. Never from where the binary lies: a binary of one checkout run outside
/// any would act on that checkout (fetch, worktree add, push its claim).
pub fn main_dir() -> PathBuf {
    find_main().unwrap_or_else(|| fail("not inside the repo: run it in a checkout of monocircuit/platform"))
}

/// The main checkout, None outside any repo (for what only helps inside one).
pub fn find_main() -> Option<PathBuf> {
    if let Some(main) = MAIN.get() {
        return Some(main.clone());
    }
    let main = core_git::main_dir(&cwd()).or_else(|| core_git::main_dir(&here()))?;
    Some(MAIN.get_or_init(|| main).clone())
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

/// The `KEY=value` lines of an env file (`#` lines skipped), None without the file.
pub fn read_env(file: &Path) -> Option<BTreeMap<String, String>> {
    let text = fs::read_to_string(file).ok()?;
    Some(
        text.split('\n')
            .filter(|line| line.contains('=') && !line.trim_start().starts_with('#'))
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    )
}

/// The `.env.local` of a checkout from before #191 moves from products/vita
/// to products/vita/web once the checkout has that folder.
pub fn adopt_env(dir: &Path) -> bool {
    let old = dir.join("products/vita/.env.local");
    let target = dir.join(VITA).join(".env.local");
    if !old.exists() || target.exists() || !dir.join(VITA).join("package.json").exists() {
        return false;
    }
    fs::rename(old, target).is_ok()
}

/// The `.env.local` of a checkout, moved over first if it is still in the old place.
pub fn env_file_of(dir: &Path) -> PathBuf {
    adopt_env(dir);
    dir.join(VITA).join(".env.local")
}

/// The offset of a worktree's own stack, 0 for the shared one.
pub fn offset_of(dir: &Path) -> u16 {
    let Some(env) = read_env(&env_file_of(dir)) else {
        return 0;
    };
    match (env.get("COMPOSE_PROJECT_NAME"), env.get("POSTGRES_PORT")) {
        (Some(project), Some(port)) if !project.is_empty() && !port.is_empty() => port
            .trim()
            .parse::<i64>()
            .map(|port| port - 5432)
            .ok()
            .filter(|o| (0..=u16::MAX as i64).contains(o))
            .unwrap_or(0) as u16,
        _ => 0,
    }
}

/// Whether nothing listens on `port` of 127.0.0.1.
pub fn is_port_free(port: u32) -> bool {
    u16::try_from(port).is_ok_and(|port| TcpListener::bind(("127.0.0.1", port)).is_ok())
}

/// The worktree whose stack answers on `port`: the app port or one of its
/// services', offset 0 the main checkout.
pub fn worktree_on_port(port: u32) -> Option<Worktree> {
    let offset = PORTS
        .iter()
        .map(|(base, _)| i64::from(port) - i64::from(*base))
        .find(|candidate| (0..50).contains(candidate))?;
    let all = worktrees();
    if offset == 0 {
        return all.into_iter().next();
    }
    all.into_iter().find(|tree| i64::from(offset_of(&tree.dir)) == offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_env_file() {
        let dir = std::env::temp_dir().join(format!("bb-env-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("e");
        fs::write(&file, "# c\nA=1=2\nB=\n").unwrap();
        let env = read_env(&file).unwrap();
        assert_eq!(env.get("A").map(String::as_str), Some("1=2"));
        assert_eq!(env.get("B").map(String::as_str), Some(""));
        fs::remove_dir_all(dir).unwrap();
    }
}
