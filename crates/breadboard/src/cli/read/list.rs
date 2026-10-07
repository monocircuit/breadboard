//! `wt:list` (list.mjs): which version runs where. `●` means something
//! answers on that port, and with an own stack that is this worktree.
//! Worktrees on the shared stack all point at 3000 and which of them serves
//! it can't be seen from outside, so they say that instead of claiming it.

use super::args::{Takes, cwd, main_dir, pad};
use breadboard_core::git::{self, Worktree};
use breadboard_core::local::{self, StackContainer};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::thread;

/// Where vita's app and its `.env.local` live in a checkout (constants.mjs `VITA`).
const VITA: &str = "products/vita/web";
const LEGEND: &str = "  ● answers  ○ nothing there  ⇒ shares the port of the main checkout  → you are here";

pub const TAKES: Takes = Takes {
    valued: &[],
    lists: &[],
    optional: &[],
    switches: &[],
    words: 0,
    about: "Which version runs where: every worktree with its branch, its app and its stack.",
    usage: "usage: pnpm wt:list",
};

pub fn run() -> ExitCode {
    let main = match main_dir() {
        Ok(main) => main,
        Err(code) => return code,
    };
    let here = cwd();
    let trees = git::worktrees(&main);
    // git and the ports per worktree side by side; docker is one shared read for all (#331).
    let rows: Vec<Row> = thread::scope(|scope| {
        let handles: Vec<_> = trees
            .iter()
            .enumerate()
            .map(|(index, tree)| scope.spawn(move || row_of(index == 0, tree, port_answers)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    print!("{}", table(&rows, &here));
    ExitCode::SUCCESS
}

/// What a worktree's line says.
#[derive(Debug, Default, Clone, PartialEq)]
struct Row {
    dir: PathBuf,
    branch: String,
    app: String,
    stack: String,
}

fn row_of(is_main: bool, tree: &Worktree, is_answering: impl Fn(i64) -> bool) -> Row {
    let dir = &tree.dir;
    let env_file = env_file_of(dir);
    let has_env = env_file.exists();
    let offset = if is_main { 0 } else { offset_of(&env_file) };
    let has_own_port = is_main || offset > 0;
    let port = 3000 + offset;
    let app = if !has_env {
        "○ no .env.local".to_string()
    } else if has_own_port {
        format!("{} http://localhost:{port}", if is_answering(port) { "●" } else { "○" })
    } else {
        "⇒ http://localhost:3000 (shared)".to_string()
    };
    let stack = if is_main {
        "the default stack (pg 5432)".to_string()
    } else if !has_env {
        "none".to_string()
    } else if offset == 0 {
        "shared with the main checkout — only one app at a time".to_string()
    } else {
        let state = match containers_up(&env_file) {
            0 => "stopped".to_string(),
            count => format!("{count} containers up"),
        };
        format!("own +{offset} (pg {}), {state}", 5432 + offset)
    };
    Row {
        dir: dir.clone(),
        branch: tree.branch.clone().unwrap_or_else(|| "(detached)".into()),
        app,
        stack,
    }
}

/// The lines of `wt:list`: an empty one, a row per worktree with its columns
/// padded to their longest, the legend between empty lines.
fn table(rows: &[Row], here: &Path) -> String {
    let names: Vec<String> = rows
        .iter()
        .map(|row| {
            row.dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    let width = |cells: &mut dyn Iterator<Item = &str>| cells.map(|cell| cell.chars().count()).max().unwrap_or(0);
    let name_width = width(&mut names.iter().map(String::as_str));
    let branch_width = width(&mut rows.iter().map(|row| row.branch.as_str()));
    let app_width = width(&mut rows.iter().map(|row| row.app.as_str()));
    let mut text = String::from("\n");
    for (row, name) in rows.iter().zip(&names) {
        let mark = if git::is_inside(here, &row.dir) { "→" } else { " " };
        text += &format!(
            "{mark} {}  {}  {}  {}\n",
            pad(name, name_width),
            pad(&row.branch, branch_width),
            pad(&row.app, app_width),
            row.stack
        );
    }
    text + "\n" + LEGEND + "\n\n"
}

/// The `.env.local` of a checkout, moved over from `products/vita` first when
/// it is still there from before #191 (stack.mjs `envFileOf` / `adoptEnv`):
/// it is untracked, so git left it behind when the app moved to `web/`.
fn env_file_of(dir: &Path) -> PathBuf {
    let target = dir.join(VITA).join(".env.local");
    let old = dir.join("products/vita/.env.local");
    if old.exists() && !target.exists() && dir.join(VITA).join("package.json").exists() {
        let _ = fs::rename(&old, &target);
    }
    target
}

/// The offset of a worktree's own stack: `POSTGRES_PORT - 5432` when the env
/// names its own compose project, else 0, the shared stack (stack.mjs `offsetOf`).
pub(super) fn offset_of(env_file: &Path) -> i64 {
    let Ok(text) = fs::read_to_string(env_file) else {
        return 0;
    };
    let value = |key: &str| {
        text.split('\n')
            .filter(|line| line.contains('=') && !line.trim_start().starts_with('#'))
            .filter_map(|line| line.split_once('='))
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| value)
            .next_back()
    };
    let has_project = value("COMPOSE_PROJECT_NAME").is_some_and(|name| !name.is_empty());
    match value("POSTGRES_PORT").filter(|port| !port.is_empty()) {
        Some(port) if has_project => port.trim().parse::<i64>().map(|port| port - 5432).unwrap_or(0),
        _ => 0,
    }
}

/// Whether something listens on the port: it can't be taken on 127.0.0.1 (stack.mjs `isPortFree`).
pub(super) fn port_answers(port: i64) -> bool {
    u16::try_from(port).is_ok_and(|port| TcpListener::bind(("127.0.0.1", port)).is_err())
}

/// The containers of a worktree's own stack that run (`docker compose ps -q`
/// counted them), from core's one shared read of docker (#331): one
/// `docker ps` for every stack, none while another process's read is fresh.
fn containers_up(env_file: &Path) -> usize {
    static CONTAINERS: OnceLock<Vec<StackContainer>> = OnceLock::new();
    let Some(project) = local::read_env(env_file).and_then(|env| env.get("COMPOSE_PROJECT_NAME").cloned()) else {
        return 0;
    };
    // Never the machine's docker from a test: `running_of` is tested with its own.
    let containers = CONTAINERS.get_or_init(|| {
        if cfg!(test) {
            Vec::new()
        } else {
            local::containers().unwrap_or_default()
        }
    });
    running_of(containers, &project)
}

/// How many of `containers` belong to `project` and run.
fn running_of(containers: &[StackContainer], project: &str) -> usize {
    containers
        .iter()
        .filter(|container| container.project == project && container.state == "running")
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("breadboard-list-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tree(dir: &Path) -> Worktree {
        Worktree {
            dir: dir.to_path_buf(),
            branch: Some("feature/1-x".into()),
        }
    }

    #[test]
    fn a_worktree_without_env_has_no_stack() {
        let dir = scratch("none");
        let row = row_of(false, &tree(&dir), |_| true);
        assert_eq!((row.app.as_str(), row.stack.as_str()), ("○ no .env.local", "none"));
        let main = row_of(
            true,
            &Worktree {
                branch: None,
                ..tree(&dir)
            },
            |_| true,
        );
        assert_eq!(
            (main.branch.as_str(), main.stack.as_str()),
            ("(detached)", "the default stack (pg 5432)")
        );
    }

    #[test]
    fn an_env_without_its_own_project_shares_the_main_stack() {
        let dir = scratch("shared");
        fs::create_dir_all(dir.join(VITA)).unwrap();
        fs::write(dir.join(VITA).join(".env.local"), "POSTGRES_PORT=5433\n").unwrap();
        let row = row_of(false, &tree(&dir), |_| true);
        assert_eq!(row.app, "⇒ http://localhost:3000 (shared)");
        assert_eq!(row.stack, "shared with the main checkout — only one app at a time");
    }

    #[test]
    fn an_own_stack_has_its_ports_moved_by_its_offset() {
        let dir = scratch("own");
        fs::create_dir_all(dir.join(VITA)).unwrap();
        fs::write(
            dir.join(VITA).join(".env.local"),
            "# x=1\nCOMPOSE_PROJECT_NAME=p2\nPOSTGRES_PORT=5434\n",
        )
        .unwrap();
        let row = row_of(false, &tree(&dir), |port| port == 3002);
        assert_eq!(row.app, "● http://localhost:3002");
        assert!(row.stack.starts_with("own +2 (pg 5434), "), "{}", row.stack);
    }

    #[test]
    fn counts_the_running_containers_of_its_own_stack_only() {
        let container = |project: &str, service: &str, state: &str| StackContainer {
            project: project.into(),
            service: service.into(),
            state: state.into(),
            health: String::new(),
        };
        let containers = [
            container("p2", "app", "running"),
            container("p2", "db", "running"),
            container("p2", "seed", "exited"),
            container("p20", "db", "running"),
            container("vita", "db", "running"),
        ];
        assert_eq!(running_of(&containers, "p2"), 2);
        assert_eq!(running_of(&containers, "p3"), 0);
        assert_eq!(running_of(&[], "p2"), 0);
    }

    #[test]
    fn an_old_env_moves_to_the_app_once_the_app_is_there() {
        let dir = scratch("adopt");
        fs::create_dir_all(dir.join(VITA)).unwrap();
        fs::write(dir.join("products/vita/.env.local"), "A=1\n").unwrap();
        assert!(!env_file_of(&dir).exists() || dir.join(VITA).join(".env.local").exists());
        assert!(
            dir.join("products/vita/.env.local").exists(),
            "no package.json: it stays"
        );
        fs::write(dir.join(VITA).join("package.json"), "{}").unwrap();
        assert!(env_file_of(&dir).exists());
        assert!(!dir.join("products/vita/.env.local").exists());
    }

    #[test]
    fn the_table_pads_every_column_but_the_last_and_marks_where_you_are() {
        let rows = vec![
            Row {
                dir: "/r/platform".into(),
                branch: "develop".into(),
                app: "○ no .env.local".into(),
                stack: "a".into(),
            },
            Row {
                dir: "/r/platform-4-x".into(),
                branch: "feature/4-x".into(),
                app: "○ x".into(),
                stack: "b".into(),
            },
        ];
        let text = table(&rows, Path::new("/r/platform-40-y"));
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[0], "");
        assert_eq!(lines[1], "  platform      develop      ○ no .env.local  a");
        assert_eq!(lines[2], "  platform-4-x  feature/4-x  ○ x              b");
        assert_eq!(lines[4], LEGEND);
        assert!(table(&rows, Path::new("/r/platform-4-x/src")).contains("→ platform-4-x"));
    }
}
