//! The repo these commands act on (worktree/worktrees.mjs): its main
//! checkout, found from where the command runs, the worktrees next to it,
//! the orchestrations that started them, and a worktree's own stack
//! (worktree/stack.mjs).

use super::proc::{Fail, fail, text};
use breadboard_core::git::{self, Worktree};
use breadboard_core::record::{self, Origin, SPACE};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Repo {
    pub main: PathBuf,
}

impl Repo {
    /// The repo the working directory is in. Today's found it from where its
    /// script lies; the binary lies outside the repo, so from the cwd.
    pub fn here() -> Result<Repo, Fail> {
        let cwd = std::env::current_dir().map_err(|error| Fail(format!("the working directory is gone ({error})")))?;
        match git::main_dir(&cwd) {
            Some(main) => Ok(Repo { main }),
            None => fail(format!(
                "{} is in no checkout of the platform repo: run it from the main checkout or one of its worktrees",
                text(&cwd)
            )),
        }
    }

    /// Every worktree, the main checkout first, without the orchestrations' checkouts.
    pub fn worktrees(&self) -> Vec<Worktree> {
        git::worktrees(&self.main)
    }

    /// The worktrees without the main checkout.
    pub fn trees(&self) -> Vec<Worktree> {
        self.worktrees().into_iter().skip(1).collect()
    }

    /// `<main>-<feature>`.
    pub fn dir_of(&self, feature: &str) -> PathBuf {
        git::dir_of(&self.main, feature)
    }

    /// `platform-40-x` → `40-x`, the name of its space.
    pub fn feature_of(&self, dir: &Path) -> String {
        git::feature_of(&self.main, dir)
    }

    /// The label of a checkout's space: the main space for the main checkout.
    pub fn space_label(&self, dir: &Path) -> String {
        if dir == self.main {
            SPACE.to_string()
        } else {
            self.feature_of(dir)
        }
    }

    /// The main checkout's name: `platform`.
    pub fn base(&self) -> String {
        self.main
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Every worktree an orchestration started, with its origin.
    pub fn orchestrated(&self) -> Vec<(PathBuf, Origin)> {
        self.trees()
            .into_iter()
            .filter_map(|tree| Some((tree.dir.clone(), record::origin_of(&tree.dir)?)))
            .collect()
    }

    /// The checkout an orchestration's space stands on (orchestrator.mjs `checkoutOf`).
    pub fn checkout_of(&self, name: &str) -> PathBuf {
        git::orchestrations_dir(&self.main).join(name)
    }

    /// The orchestration of a space's label `⇶ <title>`, None for any other.
    pub fn orchestration_of(&self, label: &str) -> Option<String> {
        if !label.contains(record::ORCHESTRATOR) {
            return None;
        }
        record::orchestration_of_label(label, &record::titles(&self.main))
    }

    /// `⇶ <title>`.
    pub fn orchestrator_label(&self, name: &str) -> String {
        record::orchestrator_label(&self.main, name)
    }
}

/// The env file of a checkout's stack (breadboard.toml); a path nothing is at without a stack.
pub fn env_file_of(dir: &Path) -> PathBuf {
    breadboard_core::local::env_file_of(dir)
}

/// The offset of a worktree's own stack, 0 for the shared one or none.
pub fn offset_of(dir: &Path) -> i64 {
    breadboard_core::stack::offset(dir)
}

/// The folder of a checkout its stack's commands run in.
pub fn stack_dir(dir: &Path) -> PathBuf {
    breadboard_core::stack::dir_of(dir)
}

/// `docker compose --env-file <env> -f <compose file> <args>` of the repo's stack, None without one.
pub fn compose_args(dir: &Path, args: &[&str]) -> Option<Vec<String>> {
    Some(breadboard_core::stack::compose_args(
        &breadboard_core::stack::of(dir)?,
        args,
    ))
}

/// packages/breadboard, where the nvim Lua and the Chromium extension lie:
/// the one above this binary (`target/release/` of a checkout), else the
/// cwd's main checkout's, else the one it was built in. One for the setup
/// and the lifecycle group.
pub fn package_dir() -> PathBuf {
    let is_package = |dir: &Path| dir.join("nvim").join("changes.lua").is_file();
    let above = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().find(|dir| is_package(dir)).map(Path::to_path_buf));
    let here = || {
        let cwd = std::env::current_dir().ok()?;
        Some(git::main_dir(&cwd)?.join("packages").join("breadboard")).filter(|dir| is_package(dir))
    };
    above.or_else(here).unwrap_or_else(|| {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        crate_dir
            .parent()
            .and_then(Path::parent)
            .unwrap_or(crate_dir)
            .to_path_buf()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_has_its_lua() {
        assert!(package_dir().ends_with("packages/breadboard") || package_dir().join("Cargo.toml").exists());
    }
}
