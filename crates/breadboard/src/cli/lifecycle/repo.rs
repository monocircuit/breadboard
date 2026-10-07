//! The repo these commands act on (worktree/worktrees.mjs): its main
//! checkout, found from where the command runs, the worktrees next to it,
//! the orchestrations that started them, and a worktree's own stack
//! (worktree/stack.mjs).

use super::proc::{Fail, fail, text};
use breadboard_core::git::{self, Worktree};
use breadboard_core::record::{self, Origin, SPACE};
use std::fs;
use std::path::{Path, PathBuf};

/// The products/vita/web of a checkout, where its stack's files are.
pub const VITA: &str = "products/vita/web";

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

/// The .env.local of a checkout's stack. One from before #191 in
/// products/vita moves to products/vita/web once that folder exists
/// (stack.mjs `adoptEnv`).
pub fn env_file_of(dir: &Path) -> PathBuf {
    let old = dir.join("products/vita/.env.local");
    let target = dir.join(VITA).join(".env.local");
    if old.exists() && !target.exists() && dir.join(VITA).join("package.json").exists() {
        let _ = fs::rename(&old, &target);
    }
    target
}

/// The variables of an env file, None without one.
pub fn read_env(file: &Path) -> Option<Vec<(String, String)>> {
    let content = fs::read_to_string(file).ok()?;
    Some(
        content
            .split('\n')
            .filter(|line| line.contains('=') && !line.trim_start().starts_with('#'))
            .filter_map(|line| {
                line.split_once('=')
                    .map(|(key, value)| (key.to_string(), value.to_string()))
            })
            .collect(),
    )
}

/// The offset of a worktree's own stack, 0 for the shared one.
pub fn offset_of(dir: &Path) -> i64 {
    let Some(env) = read_env(&env_file_of(dir)) else {
        return 0;
    };
    let get = |key: &str| {
        env.iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    match (get("COMPOSE_PROJECT_NAME"), get("POSTGRES_PORT")) {
        (Some(project), Some(port)) if !project.is_empty() && !port.is_empty() => {
            port.trim().parse::<i64>().map(|port| port - 5432).unwrap_or(0)
        }
        _ => 0,
    }
}

/// `docker compose --env-file .env.local -f docker-compose.local.yml <args>`.
pub fn compose_args<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut all = vec!["compose", "--env-file", ".env.local", "-f", "docker-compose.local.yml"];
    all.extend_from_slice(args);
    all
}

/// packages/breadboard, where the nvim Lua and the herdr plugin lie.
pub fn package_dir() -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(crate_dir)
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_own_stack_has_a_project_and_a_moved_port() {
        let dir = std::env::temp_dir().join(format!("lifecycle-repo-{}", std::process::id()));
        fs::create_dir_all(dir.join(VITA)).unwrap();
        assert_eq!(offset_of(&dir), 0);
        fs::write(dir.join(VITA).join(".env.local"), "# x\nPOSTGRES_PORT=5435\n").unwrap();
        assert_eq!(offset_of(&dir), 0);
        fs::write(
            dir.join(VITA).join(".env.local"),
            "COMPOSE_PROJECT_NAME=vita-3\nPOSTGRES_PORT=5435\n",
        )
        .unwrap();
        assert_eq!(offset_of(&dir), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_package_has_its_lua() {
        assert!(package_dir().ends_with("packages/breadboard") || package_dir().join("Cargo.toml").exists());
    }
}
