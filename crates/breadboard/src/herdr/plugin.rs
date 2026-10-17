//! The herdr plugin's actions (herdr/run.mjs; the manifest
//! herdr/herdr-plugin.toml runs them):
//!
//! ```text
//! breadboard herdr restore [--startup]  the main space and a space per worktree (wt:herdr)
//! breadboard herdr open <pane>          an action: where it was triggered written down, the pane opened
//! breadboard herdr pane <pane>          inside the pane: run it in the worktree of that place
//! breadboard herdr status               one line for tab_bar_right: stacks, agents that ask
//! breadboard herdr heal                 the overview again in `overview` tabs that hold a bare shell
//! ```
//!
//! herdr runs plugin commands in the plugin's directory: the repo is the
//! checkout of that directory, never the one beside the binary. The place a
//! key was pressed in comes from the context herdr passes; an action gets
//! it, a pane it opens may not, so the action writes it down for the pane.

use super::{SPACE, USAGE};
use crate::cli::lifecycle::layout::tabs::OVERVIEW;
use breadboard_core::git::{self, git};
use breadboard_core::herdr::{plugin_context, plugin_id, save_context, where_from};
use breadboard_core::local::{self, StackContainer};
use breadboard_core::runs;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::io::{IsTerminal, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// The panes of the manifest; all but `overview` are popups.
const PANES: [&str; 6] = ["overview", "commands", "reload", "stack-up", "stack-down", "stack-ps"];

pub fn main(args: &[String]) -> ExitCode {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["restore"] => restore(false),
        ["restore", "--startup"] => restore(true),
        ["status"] => status(),
        ["heal"] => heal(),
        ["open", pane] if PANES.contains(pane) => open(pane),
        ["pane", pane] if PANES.contains(pane) => pane_of(pane),
        ["open" | "pane", pane] => refused(&format!("unknown pane {pane}")),
        _ => refused(&format!("herdr {} isn't an action", words.join(" "))),
    }
}

fn refused(reason: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "\n✗ {reason}\n\n{USAGE}");
    ExitCode::FAILURE
}

/// The herdr that started the plugin (`HERDR_BIN_PATH`), else `herdr`.
fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH")
        .ok()
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "herdr".into())
}

/// `herdr <args>` → its `result`, None when it can't be had.
fn ask(args: &[&str]) -> Option<Value> {
    let output = Command::new(herdr_bin())
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    value.get("result").cloned()
}

/// The checkout of the plugin's directory (the cwd herdr runs it in).
fn repo() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let top = git(&cwd, &["rev-parse", "--show-toplevel"]);
    top.is_ok.then(|| PathBuf::from(top.out()))
}

fn common_dir(dir: &Path) -> Option<String> {
    let out = git(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    out.is_ok.then(|| out.out().to_string())
}

/// The worktree of this repo `dir` lies in, None for a place elsewhere: a
/// pane in another repo has no stack of ours.
fn worktree_of(dir: &Path, repo: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let top = git(dir, &["rev-parse", "--show-toplevel"]);
    if !top.is_ok {
        return None;
    }
    let common = common_dir(dir)?;
    (Some(common) == common_dir(repo)).then(|| PathBuf::from(top.out()))
}

/// This binary (as it was started: a rebuild meanwhile doesn't lose it)
/// with `args` in `dir`, as pnpm runs a script there: `INIT_CWD` is where
/// the commands look for "you are here".
fn this(args: &[&str], dir: &Path) -> Command {
    let mut command = Command::new(super::started_binary());
    command
        .args(args)
        .current_dir(dir)
        .env("INIT_CWD", dir)
        .env("FORCE_COLOR", "1");
    command
}

fn code_of(status: std::io::Result<std::process::ExitStatus>) -> ExitCode {
    ExitCode::from(status.ok().and_then(|status| status.code()).unwrap_or(1).clamp(0, 255) as u8)
}

fn no_repo() -> ExitCode {
    refused(
        "the plugin runs in its directory, and this one is in no checkout of the repo (herdr runs it in packages/breadboard/herdr)",
    )
}

/// `restore [--startup]`: `wt:herdr --no-focus` in the repo. At a start only
/// in the session `platform` or one that has the space already: plugins are
/// linked for every session, and a personal one doesn't get a space it
/// never asked for. No focus: the space the user is in stays in front.
fn restore(is_startup: bool) -> ExitCode {
    if is_startup {
        let is_platform = std::env::var("HERDR_SOCKET_PATH").is_ok_and(|socket| socket.contains("/sessions/platform/"));
        if !is_platform && !ask(&["workspace", "list"]).is_some_and(|result| super::session::has_space_in(&result)) {
            return ExitCode::SUCCESS;
        }
    }
    let Some(repo) = repo() else { return no_repo() };
    code_of(this(&["wt:herdr", "--no-focus"], &repo).status())
}

/// `heal`: the overview again in every `overview` tab of the repo that
/// holds only a shell (#388), in the checkout of the cwd. Started by the
/// status when it saw one; says in which spaces.
fn heal() -> ExitCode {
    let Ok(repo) = crate::cli::lifecycle::repo::Repo::here() else {
        return no_repo();
    };
    for label in crate::cli::lifecycle::layout::heal_overviews(&repo) {
        println!("the overview runs again in {label}");
    }
    ExitCode::SUCCESS
}

/// `open <pane>`: where the key was pressed written down, the pane opened.
fn open(pane: &str) -> ExitCode {
    let _ = save_context(&plugin_context());
    let status = Command::new(herdr_bin())
        .args(["plugin", "pane", "open", "--plugin", &plugin_id(), "--entrypoint", pane])
        .status();
    code_of(status)
}

/// `pane <pane>`: the pane's command in the worktree of the place the key
/// was pressed in. The overview and the commands make sense anywhere; the
/// stack needs a worktree of ours.
fn pane_of(pane: &str) -> ExitCode {
    let Some(repo) = repo() else { return no_repo() };
    let from = where_from(&plugin_context());
    let tree = from.as_deref().and_then(|from| worktree_of(Path::new(from), &repo));
    let stack = pane.strip_prefix("stack-");
    if let Some(action) = stack
        && tree.is_none()
    {
        println!(
            "\n  {} is not in a worktree of the platform repo, so there is no stack to control.",
            from.as_deref().unwrap_or("this pane")
        );
        println!(
            "  Focus a tab of the repo, or use {run} stack:{action} --port <p>.",
            run = breadboard_core::config::run_word()
        );
        wait_for_key();
        return ExitCode::FAILURE;
    }
    let dir = match (&tree, &from) {
        (Some(tree), _) if stack.is_some() => tree.clone(),
        (Some(_), Some(from)) => PathBuf::from(from),
        _ => repo.clone(),
    };
    let code = match pane {
        // Zoomed, `q` closes it: this process becomes the overview.
        "overview" => {
            let error = this(&["overview"], &dir).exec();
            return refused(&format!("the overview didn't start: {error}"));
        }
        "commands" => return commands(&dir),
        "reload" => code_of(this(&["herdr", "--reload"], &repo).status()),
        _ => code_of(this(&[&format!("stack:{}", stack.unwrap_or("ps"))], &dir).status()),
    };
    wait_for_key();
    code
}

/// `pnpm commands` of the package the pane stands in, longer than a popup:
/// paged, with its colours (`q` closes). Without `less`: printed, then a key.
fn commands(dir: &Path) -> ExitCode {
    let list = match this(&["commands"], dir).stderr(Stdio::inherit()).output() {
        Ok(output) => output.stdout,
        Err(error) => {
            return refused(&format!(
                "{run} commands didn't run: {error}",
                run = breadboard_core::config::run_word()
            ));
        }
    };
    let pager = Command::new("less")
        .args(["-R", "-X", "+g"])
        .stdin(Stdio::piped())
        .spawn();
    let Ok(mut pager) = pager else {
        let _ = std::io::stdout().write_all(&list);
        wait_for_key();
        return ExitCode::SUCCESS;
    };
    if let Some(mut input) = pager.stdin.take() {
        let _ = input.write_all(&list);
    }
    code_of(pager.wait())
}

/// A popup closes when its command ends: its output stays until a key.
fn wait_for_key() {
    if !std::io::stdin().is_terminal() {
        return;
    }
    print!("\n\x1b[2m  any key closes this\x1b[0m");
    let _ = std::io::stdout().flush();
    let is_raw = crossterm::terminal::enable_raw_mode().is_ok();
    let mut buffer = [0u8; 8];
    let _ = std::io::stdin().read(&mut buffer);
    if is_raw {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// `status`: one line for the tab bar, every 10 s in every herdr server:
/// the running vita stacks and the agents in the repo's spaces that ask
/// something (two herdr calls, nothing to GitLab, #305). docker is read
/// through core's shared file (#331): no call while another process's read
/// is fresh, one `docker ps` by the lock holder otherwise. Hostile answers
/// count as nothing; it always exits 0.
///
/// It is also the tick that brings the overviews back (#388): nothing else
/// runs after a herdr restart until the user looks. One more read, the tabs,
/// and a process read per `overview` tab of the repo; one that holds only a
/// shell starts `heal` on its own, the line doesn't wait for it.
fn status() -> ExitCode {
    let containers = local::containers();
    let main = status_main();
    let spaces = ask(&["workspace", "list"]);
    let mut panes = None;
    let waiting = needs_you(main.as_deref(), spaces.as_ref(), || {
        panes = ask(&["pane", "list"]);
        panes.clone()
    }) + main.as_deref().map_or(0, headless_asks);
    if let Some(main) = main.as_deref() {
        let ours = ours(Some(main), spaces.as_ref());
        let tabs = (!ours.is_empty()).then(|| ask(&["tab", "list"])).flatten();
        if !bare_overviews(&ours, tabs.as_ref(), panes.as_ref(), is_bare).is_empty() {
            start_heal(main);
        }
    }
    let project = main
        .as_deref()
        .and_then(breadboard_core::stack::of)
        .map(|stack| stack.project)
        .unwrap_or_default();
    println!("{}", status_line(stacks_in(containers.as_deref(), &project), waiting));
    ExitCode::SUCCESS
}

/// The main checkout the status counts for. herdr runs the tab bar's
/// command in its server's cwd (your home, mostly), not the plugin's
/// directory: the cwd's checkout when it is one of this repo, else the
/// checkout of the binary as it was started (`argv[0]`, a link not
/// resolved), else of the file it is (Node: the script's own checkout).
fn status_main() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok();
    let started = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .filter(|path| path.components().count() > 1)
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let file = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    [cwd, started, file]
        .into_iter()
        .flatten()
        .filter(|dir| dir.is_dir())
        .filter_map(|dir| git::main_dir(&dir))
        .find(|main| crate::cli::files_dir(main).join("herdr/herdr-plugin.toml").is_file())
}

/// How many of the repo's stacks run: compose projects named `<project>…`
/// with a running container (what `compose ls` lists without `--all`, as
/// Node counted); none without a stack.
fn stacks_in(containers: Option<&[StackContainer]>, project: &str) -> usize {
    let running: BTreeSet<&str> = containers
        .unwrap_or_default()
        .iter()
        .filter(|container| {
            !project.is_empty() && container.state == "running" && container.project.starts_with(project)
        })
        .map(|container| container.project.as_str())
        .collect();
    running.len()
}

/// The blocked agents in the spaces of the repo: the main space, its
/// worktrees' and the orchestrations' (`repo_root` is the main checkout, or
/// the label starts with the main space's). No pane list without one.
fn needs_you(main: Option<&Path>, spaces: Option<&Value>, panes: impl FnOnce() -> Option<Value>) -> usize {
    let ours = ours(main, spaces);
    if ours.is_empty() {
        return 0;
    }
    panes()
        .and_then(|result| result.get("panes").and_then(Value::as_array).cloned())
        .map_or(0, |panes| {
            panes
                .iter()
                .filter(|pane| {
                    pane.get("workspace_id")
                        .and_then(Value::as_str)
                        .is_some_and(|space| ours.contains(&space))
                        && pane.get("agent_status").and_then(Value::as_str) == Some("blocked")
                })
                .count()
        })
}

/// The ids of the repo's spaces: the main space, its worktrees' and the
/// orchestrations' (`repo_root` is the main checkout, or the label starts
/// with the main space's).
fn ours<'a>(main: Option<&Path>, spaces: Option<&'a Value>) -> Vec<&'a str> {
    let main = main.map(|main| main.display().to_string());
    spaces
        .and_then(|result| result.get("workspaces"))
        .and_then(Value::as_array)
        .map(|spaces| {
            spaces
                .iter()
                .filter(|space| {
                    let root = space
                        .get("worktree")
                        .and_then(|tree| tree.get("repo_root"))
                        .and_then(Value::as_str);
                    let label = space.get("label").and_then(Value::as_str).unwrap_or_default();
                    (root.is_some() && root == main.as_deref()) || label.starts_with(&SPACE.get())
                })
                .filter_map(|space| space.get("workspace_id").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// The panes of the `overview` tabs in the spaces `ours` that hold only a
/// shell (`is_bare`): a tab of one pane, its overview not running. A tab
/// split by hand is the user's.
fn bare_overviews(
    ours: &[&str],
    tabs: Option<&Value>,
    panes: Option<&Value>,
    is_bare: impl Fn(&str) -> bool,
) -> Vec<String> {
    let listed = |result: Option<&Value>, key: &str| -> Vec<Value> {
        result
            .and_then(|result| result.get(key)?.as_array().cloned())
            .unwrap_or_default()
    };
    let field = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let panes = listed(panes, "panes");
    listed(tabs, "tabs")
        .iter()
        .filter(|tab| {
            field(tab, "label") == OVERVIEW
                && tab.get("pane_count").and_then(Value::as_i64) == Some(1)
                && ours.contains(&field(tab, "workspace_id").as_str())
        })
        .filter_map(|tab| panes.iter().find(|pane| field(pane, "tab_id") == field(tab, "tab_id")))
        // One herdr resumes an agent in, or that runs one, is no bare shell (#386).
        .filter(|pane| !crate::cli::lifecycle::layout::tabs::is_agents(pane))
        .map(|pane| field(pane, "pane_id"))
        .filter(|pane| !pane.is_empty() && is_bare(pane))
        .collect()
}

/// Whether the pane's shell is in front: nothing runs in it.
fn is_bare(pane: &str) -> bool {
    ask(&["pane", "process-info", "--pane", pane])
        .and_then(|result| result.get("process_info").cloned())
        .is_some_and(|info| {
            let shell = info.get("shell_pid").and_then(Value::as_i64).unwrap_or(0);
            shell != 0 && info.get("foreground_process_group_id").and_then(Value::as_i64) == Some(shell)
        })
}

/// `heal` in the main checkout `main`, on its own: in a process group of
/// its own (herdr's timeout of the status doesn't end it), not waited for.
/// It runs inside herdr, so the layout's lock holds for it too.
fn start_heal(main: &Path) {
    let _ = this(&["herdr", "heal"], main)
        .env("HERDR_ENV", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}

/// The headless agents of `main` that ask (#361): no pane of theirs is
/// blocked, the status table's `asks` says it (a denied tool, #360).
fn headless_asks(main: &Path) -> usize {
    let table: runs::Table = runs::read_or_default(&runs::status_file(main));
    let queue: runs::Queue = runs::read_or_default(&runs::queue_file(main));
    asks_in(&runs::states_of(&table, &queue))
}

fn asks_in(states: &HashMap<String, runs::Run>) -> usize {
    states.values().filter(|run| run.state == runs::State::Asks).count()
}

/// `⬢ 2 stacks · 1 needs you`, `⬡ no stack`.
fn status_line(stacks: usize, waiting: usize) -> String {
    let stacks = match stacks {
        0 => "⬡ no stack".to_string(),
        1 => "⬢ 1 stack".to_string(),
        n => format!("⬢ {n} stacks"),
    };
    match waiting {
        0 => stacks,
        1 => format!("{stacks} · 1 needs you"),
        n => format!("{stacks} · {n} need you"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_the_headless_agents_that_ask() {
        let run = |state| runs::Run {
            dir: PathBuf::from("/r/p-1"),
            name: "1-x".into(),
            state,
            since: 0,
            pid: None,
            lines: 0,
            denials: Vec::new(),
            code: None,
        };
        let states = [
            ("a", runs::State::Asks),
            ("b", runs::State::Idle),
            ("c", runs::State::Asks),
        ]
        .into_iter()
        .map(|(session, state)| (session.to_string(), run(state)))
        .collect();
        assert_eq!(asks_in(&states), 2);
    }

    #[test]
    fn says_the_stacks_and_who_needs_you() {
        assert_eq!(status_line(0, 0), "⬡ no stack");
        assert_eq!(status_line(1, 0), "⬢ 1 stack");
        assert_eq!(status_line(2, 1), "⬢ 2 stacks · 1 needs you");
        assert_eq!(status_line(0, 3), "⬡ no stack · 3 need you");
        let container = |project: &str, service: &str, state: &str| StackContainer {
            project: project.to_string(),
            service: service.to_string(),
            state: state.to_string(),
            health: String::new(),
        };
        let listed = [
            container("vita", "web", "running"),
            container("vita", "db", "running"),
            container("vita-12", "db", "exited"),
            container("vita-12", "web", "running"),
            container("vita-7", "db", "exited"),
            container("other", "db", "running"),
        ];
        // Two stacks run; one only stopped and another project's don't count.
        assert_eq!(stacks_in(Some(&listed), "vita"), 2);
        assert_eq!(stacks_in(Some(&listed), ""), 0);
        assert_eq!(stacks_in(Some(&[]), "vita"), 0);
        assert_eq!(stacks_in(None, "vita"), 0);
    }

    #[test]
    fn counts_the_blocked_agents_of_the_repos_spaces_only() {
        let spaces = json!({ "workspaces": [
            { "workspace_id": "wD", "label": "Monocircuit/Platform", "worktree": { "repo_root": "/r/platform" } },
            { "workspace_id": "w2", "label": "⇶ x", "worktree": { "repo_root": "/r/platform" } },
            { "workspace_id": "w3", "label": "Monocircuit/Platform-old" },
            { "workspace_id": "wM", "label": "Dash" },
        ] });
        let panes = json!({ "panes": [
            { "workspace_id": "wD", "agent_status": "blocked" },
            { "workspace_id": "w2", "agent_status": "blocked" },
            { "workspace_id": "w3", "agent_status": "blocked" },
            { "workspace_id": "w2", "agent_status": "idle" },
            { "workspace_id": "wM", "agent_status": "blocked" },
        ] });
        let main = Path::new("/r/platform");
        assert_eq!(needs_you(Some(main), Some(&spaces), || Some(panes.clone())), 3);
        // Outside a checkout: by the label only, never a crash.
        assert_eq!(needs_you(None, Some(&spaces), || Some(panes.clone())), 2);
        assert_eq!(needs_you(Some(main), None, panes::unexpected), 0);
        assert_eq!(needs_you(Some(main), Some(&json!({ "workspaces": "x" })), || None), 0);
    }

    #[test]
    fn finds_the_overview_tabs_of_the_repo_that_hold_a_bare_shell() {
        let tabs = json!({ "tabs": [
            { "workspace_id": "wD", "tab_id": "wD:t1", "label": "overview", "pane_count": 1 },
            { "workspace_id": "w2", "tab_id": "w2:t2", "label": "overview", "pane_count": 1 },
            // Split by hand, another repo's, not an overview tab.
            { "workspace_id": "w3", "tab_id": "w3:t2", "label": "overview", "pane_count": 2 },
            { "workspace_id": "wM", "tab_id": "wM:t1", "label": "overview", "pane_count": 1 },
            { "workspace_id": "wD", "tab_id": "wD:t2", "label": "root", "pane_count": 1 },
        ] });
        let panes = json!({ "panes": [
            { "pane_id": "wD:p1", "tab_id": "wD:t1" },
            { "pane_id": "w2:p2", "tab_id": "w2:t2" },
            { "pane_id": "w3:p2", "tab_id": "w3:t2" },
            { "pane_id": "wM:p1", "tab_id": "wM:t1" },
            { "pane_id": "wD:p2", "tab_id": "wD:t2" },
        ] });
        let ours = ["wD", "w2", "w3"];
        // The main space's overview runs, the orchestration's is a bare shell.
        let bare = bare_overviews(&ours, Some(&tabs), Some(&panes), |pane| pane != "wD:p1");
        assert_eq!(bare, ["w2:p2"]);
        assert!(bare_overviews(&ours, Some(&tabs), Some(&panes), |_| false).is_empty());
        assert!(bare_overviews(&ours, None, Some(&panes), |_| true).is_empty());
        assert!(bare_overviews(&[], Some(&tabs), Some(&panes), |_| true).is_empty());
        // After a restart herdr resumes a recorded agent there: its shell is in
        // front a moment, but a command typed now lands in that claude (#386).
        let resuming = json!({ "panes": [
            { "pane_id": "w2:p2", "tab_id": "w2:t2", "agent_session": { "value": "701ab84e" } },
        ] });
        assert!(bare_overviews(&ours, Some(&tabs), Some(&resuming), |_| true).is_empty());
    }

    mod panes {
        pub fn unexpected() -> Option<serde_json::Value> {
            panic!("no pane list without a space of ours")
        }
    }
}
