//! A checkout's dev stack as breadboard.toml's `[stack]` describes it (#342):
//! its env file, the offset its ports are moved by, its compose project, the
//! app's port. Without a `[stack]` a checkout has none: no env file, offset 0.

use crate::config::{self, Stack};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The variable an own stack's compose project is written to.
pub const PROJECT: &str = "COMPOSE_PROJECT_NAME";

/// The stack of the repo `dir` lies in, None without one.
pub fn of(dir: &Path) -> Option<Stack> {
    config::of(dir).config.stack.clone()
}

/// The env file of a checkout's stack, None without a stack.
pub fn env_file(dir: &Path) -> Option<PathBuf> {
    config::of(dir).env_file(dir)
}

/// The folder of a checkout the stack's commands run in; the checkout without a stack.
pub fn dir_of(dir: &Path) -> PathBuf {
    config::of(dir).stack_dir(dir).unwrap_or_else(|| dir.to_path_buf())
}

/// `KEY=value` lines of an env file, None without one; a key named twice keeps its last.
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

/// The env of a checkout's stack, None without one.
pub fn env_of(dir: &Path) -> Option<BTreeMap<String, String>> {
    read_env(&env_file(dir)?)
}

/// The offset of an env's own stack: its offset port's variable moved from
/// the port, when it names a compose project of its own; else 0, the shared stack.
pub fn offset_in(stack: Option<&Stack>, env: Option<&BTreeMap<String, String>>) -> i64 {
    let (Some(stack), Some(env)) = (stack, env) else {
        return 0;
    };
    let Some(port) = stack.offset_port() else { return 0 };
    let variable = port.env.unwrap_or_default();
    let has_project = env.get(PROJECT).is_some_and(|name| !name.trim().is_empty());
    match env.get(&variable).and_then(|value| value.trim().parse::<i64>().ok()) {
        Some(moved) if has_project => moved - i64::from(port.port),
        _ => 0,
    }
}

/// The offset of a checkout's own stack, 0 for the shared one or none.
pub fn offset(dir: &Path) -> i64 {
    offset_in(of(dir).as_ref(), env_of(dir).as_ref())
}

/// The compose project of an env: its own, else the shared one's (`[stack] project`).
pub fn project_in(stack: Option<&Stack>, env: Option<&BTreeMap<String, String>>) -> String {
    env.and_then(|env| env.get(PROJECT).cloned())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| stack.map(|stack| stack.project.clone()))
        .unwrap_or_default()
}

/// The app's port without an offset, None without a stack.
pub fn app_port(dir: &Path) -> Option<i64> {
    of(dir).map(|stack| i64::from(stack.app.port))
}

/// `compose --env-file <env> -f <compose file> <args>`, docker's arguments in the stack's folder.
pub fn compose_args(stack: &Stack, args: &[&str]) -> Vec<String> {
    let mut all: Vec<String> = ["compose", "--env-file", &stack.env_file, "-f", &stack.compose_file]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
    all.extend(args.iter().map(|arg| arg.to_string()));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn stack() -> Stack {
        Config::parse(
            "[stack]\nproject = \"app\"\n[stack.app]\nport = 3000\n[[stack.services]]\nname = \"db\"\nport = 5432\nenv = \"DB_PORT\"\n",
        )
        .unwrap()
        .stack
        .unwrap()
    }

    fn env(lines: &[(&str, &str)]) -> BTreeMap<String, String> {
        lines
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn an_own_stack_has_a_project_and_a_moved_port() {
        let stack = stack();
        assert_eq!(offset_in(Some(&stack), None), 0);
        assert_eq!(offset_in(Some(&stack), Some(&env(&[("DB_PORT", "5435")]))), 0);
        let own = env(&[(PROJECT, "app-3"), ("DB_PORT", "5435")]);
        assert_eq!(offset_in(Some(&stack), Some(&own)), 3);
        assert_eq!(offset_in(None, Some(&own)), 0);
        assert_eq!(project_in(Some(&stack), Some(&own)), "app-3");
        assert_eq!(project_in(Some(&stack), None), "app");
        assert_eq!(
            compose_args(&stack, &["ps"]),
            ["compose", "--env-file", ".env.local", "-f", "docker-compose.yml", "ps"]
        );
    }

    #[test]
    fn reads_an_env_file() {
        let dir = std::env::temp_dir().join(format!("stack-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env"), "# a comment\nURL=a=b\nX=1\nX=2\n").unwrap();
        let env = read_env(&dir.join(".env")).unwrap();
        assert_eq!((env["URL"].as_str(), env["X"].as_str()), ("a=b", "2"));
        assert!(read_env(&dir.join("none")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
