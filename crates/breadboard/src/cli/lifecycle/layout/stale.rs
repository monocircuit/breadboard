//! herdr's records of conversations it must never resume (#425). Once
//! claude's hook reported a pane's `agent_session`, herdr keeps it and runs
//! `claude --resume <id>` in that pane after every restart; only the end of
//! that claude clears it, `pane release-agent` doesn't (herdr 0.9.1, tried on
//! a probe session). A record is stale in an `overview` tab (no agent's tab)
//! and wherever it names a headless agent's conversation: a worker of an
//! orchestration's record, a retired one too. A pane that holds one without
//! its claude is replaced by a fresh pane (split, the old one closed; an
//! overview tab gets its overview in it); a claude herdr resumed from one (a
//! bare `claude --resume`, never a click's `--name`) is ended once herdr says
//! it idles, its worktree's work kept as a snapshot first, and its record
//! goes with it.

use super::super::herdr::{self, call, did, id_at, s};
use super::super::repo::Repo;
use super::tabs::OVERVIEW;
use breadboard_core::conversations::unnamed_resumes;
use breadboard_core::record::{self, SPACE};
use breadboard_core::time::now_ms;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What one pane needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    /// A claude herdr resumed from a stale record, idle now: ended.
    End { pane: String, pid: i64 },
    /// A stale record on a pane without its claude: the pane replaced.
    Replace {
        pane: String,
        workspace: String,
        cwd: String,
        is_overview: bool,
    },
}

/// The conversations of every agent of the orchestrations of `main`, the
/// retired ones too: headless, never a pane's (#358).
pub fn agent_sessions(main: &Path) -> HashSet<String> {
    record::recorded(main)
        .iter()
        .filter_map(|name| record::record_of(main, name))
        .flat_map(|record| record.workers.into_iter().map(|worker| worker.id))
        .collect()
}

/// What the panes of the spaces `ours` need. `tabs` and `panes` as herdr
/// lists them; `resumes` (from /proc) and `is_bare` (a process read) are
/// asked only for a pane that has a stale record.
pub fn plan(
    ours: &[&str],
    tabs: &[Value],
    panes: &[Value],
    agents: &HashSet<String>,
    resumes: impl FnOnce() -> Vec<(i64, String)>,
    is_bare: impl Fn(&str) -> bool,
) -> Vec<Stale> {
    // An overview tab of one pane: one the overview split (its detail, #406) waits.
    let overview_tabs: HashSet<&str> = tabs
        .iter()
        .filter(|tab| {
            s(tab, "label") == OVERVIEW
                && tab.get("pane_count").and_then(Value::as_i64) == Some(1)
                && ours.contains(&s(tab, "workspace_id"))
        })
        .map(|tab| s(tab, "tab_id"))
        .collect();
    let mut resumes = Some(resumes);
    let mut resumed: Vec<(i64, String)> = Vec::new();
    let mut stale = Vec::new();
    for pane in panes.iter().filter(|pane| ours.contains(&s(pane, "workspace_id"))) {
        let session = session_of(pane);
        let is_overview = overview_tabs.contains(s(pane, "tab_id"));
        if session.is_empty() || !(is_overview || agents.contains(session)) {
            continue;
        }
        let id = s(pane, "pane_id").to_string();
        if !herdr::has_agent(pane) {
            if is_overview || is_bare(&id) {
                stale.push(Stale::Replace {
                    pane: id,
                    workspace: s(pane, "workspace_id").to_string(),
                    cwd: s(pane, "cwd").to_string(),
                    is_overview,
                });
            }
            continue;
        }
        // Its claude runs: ended once it does nothing, so no answer is cut off.
        if !matches!(s(pane, "agent_status"), "idle" | "done") {
            continue;
        }
        if let Some(resumes) = resumes.take() {
            resumed = resumes();
        }
        if let Some((pid, _)) = resumed.iter().find(|(_, resumed)| resumed == session) {
            stale.push(Stale::End { pane: id, pid: *pid });
        }
    }
    stale
}

fn session_of(pane: &Value) -> &str {
    pane.get("agent_session")
        .map(|session| s(session, "value"))
        .unwrap_or_default()
}

/// The stale records of the repo's spaces gone (under the layout's lock):
/// what it did, a line each.
pub fn sweep(repo: &Repo) -> Vec<String> {
    super::exclusive(|| {
        let listed = call(&["workspace", "list"]);
        let ours = ours(Some(&repo.main), listed.as_ref());
        if ours.is_empty() {
            return Vec::new();
        }
        let spaces = listed
            .as_ref()
            .and_then(|result| result.get("workspaces")?.as_array().cloned())
            .unwrap_or_default();
        let tabs = list(&["tab", "list"], "tabs");
        let panes = herdr::all_panes();
        let agents = agent_sessions(&repo.main);
        let label_of = |workspace: &str| {
            spaces
                .iter()
                .find(|space| s(space, "workspace_id") == workspace)
                .map(|space| s(space, "label").to_string())
                .unwrap_or_default()
        };
        let trees = repo.trees();
        let mut said = Vec::new();
        for stale in plan(&ours, &tabs, &panes, &agents, unnamed_resumes, herdr::is_idle) {
            match stale {
                Stale::End { pane, pid } => {
                    if let Some(line) = end(pid) {
                        said.push(format!("{line} (pane {pane})"));
                    }
                }
                Stale::Replace {
                    pane,
                    workspace,
                    cwd,
                    is_overview,
                } => {
                    let label = label_of(&workspace);
                    let command = is_overview
                        .then(|| super::overview_command_of(repo, &trees, &label))
                        .flatten();
                    if replace(&pane, &cwd, command.as_deref()) {
                        said.push(format!(
                            "replaced pane {pane} of {label}: herdr's record of a conversation it must not resume went with it"
                        ));
                    }
                }
            }
        }
        said
    })
}

/// The ids of the repo's spaces: the main space, its worktrees' and the
/// orchestrations' (`repo_root` is the main checkout, or the label starts
/// with the main space's).
pub fn ours<'a>(main: Option<&Path>, spaces: Option<&'a Value>) -> Vec<&'a str> {
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

fn list(args: &[&str], key: &str) -> Vec<Value> {
    call(args)
        .and_then(|result| result.get(key)?.as_array().cloned())
        .unwrap_or_default()
}

/// The claude `pid` ended (SIGTERM lets it save), its worktree's work kept first (#395).
fn end(pid: i64) -> Option<String> {
    let cwd: PathBuf = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    let kept = breadboard_core::snapshot::before_ending(&cwd, now_ms());
    // SAFETY: a signal to a pid read from /proc a moment ago.
    let is_sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0;
    is_sent.then(|| {
        let kept = kept.map(|kept| format!(", {kept}")).unwrap_or_default();
        format!(
            "ended the claude herdr resumed from a stale record in {}{kept}",
            cwd.display()
        )
    })
}

/// How long a new pane's shell is waited for before the overview is typed.
const SHELL_WAIT: Duration = Duration::from_secs(5);

/// `pane` replaced by a fresh one in `cwd` (split beside it, it closed),
/// `command` run there once its shell idles: never typed into anything
/// else. Whether the old pane went.
fn replace(pane: &str, cwd: &str, command: Option<&str>) -> bool {
    let mut split = vec!["pane", "split", pane, "--direction", "right", "--no-focus"];
    if !cwd.is_empty() {
        split.extend(["--cwd", cwd]);
    }
    let Some(new) = call(&split).and_then(|made| id_at(&made, &["pane", "pane_id"])) else {
        return false;
    };
    if !did(&["pane", "close", pane]) {
        did(&["pane", "close", &new]);
        return false;
    }
    if let Some(command) = command {
        let started = std::time::Instant::now();
        while !herdr::is_idle(&new) {
            if started.elapsed() >= SHELL_WAIT {
                // The status's heal types it at its next tick.
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        did(&["pane", "run", &new, command]);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const AGENT: &str = "7275642a-0000-4000-8000-000000000000";
    const OTHER: &str = "11111111-0000-4000-8000-000000000000";

    fn pane(id: &str, tab: &str, agent: Option<&str>, status: &str, session: Option<&str>) -> Value {
        json!({
            "pane_id": id,
            "tab_id": tab,
            "workspace_id": "w2",
            "agent": agent,
            "agent_status": status,
            "agent_session": session.map(|value| json!({ "source": "herdr:claude", "value": value })),
            "cwd": "/r/p",
        })
    }

    fn tabs() -> Vec<Value> {
        vec![
            json!({ "tab_id": "t1", "workspace_id": "w2", "label": "overview", "pane_count": 1 }),
            json!({ "tab_id": "t2", "workspace_id": "w2", "label": "orchestrator", "pane_count": 1 }),
            json!({ "tab_id": "t3", "workspace_id": "w2", "label": "overview", "pane_count": 2 }),
        ]
    }

    fn agents() -> HashSet<String> {
        HashSet::from([AGENT.to_string()])
    }

    fn planned(panes: &[Value], resumes: Vec<(i64, String)>, is_bare: bool) -> Vec<Stale> {
        plan(&["w2"], &tabs(), panes, &agents(), || resumes, |_| is_bare)
    }

    #[test]
    fn replaces_an_overview_pane_with_any_record_whatever_runs_in_it() {
        let panes = [pane("p1", "t1", None, "unknown", Some(OTHER))];
        assert_eq!(
            planned(&panes, Vec::new(), false),
            [Stale::Replace {
                pane: "p1".into(),
                workspace: "w2".into(),
                cwd: "/r/p".into(),
                is_overview: true,
            }]
        );
    }

    #[test]
    fn replaces_another_tab_only_for_an_agents_record_and_while_its_shell_idles() {
        let panes = [
            pane("p2", "t2", None, "unknown", Some(AGENT)),
            pane("p4", "t4", None, "unknown", Some(OTHER)),
        ];
        assert!(planned(&panes, Vec::new(), false).is_empty());
        assert_eq!(
            planned(&panes, Vec::new(), true),
            [Stale::Replace {
                pane: "p2".into(),
                workspace: "w2".into(),
                cwd: "/r/p".into(),
                is_overview: false,
            }]
        );
    }

    #[test]
    fn ends_herdrs_resume_once_idle_never_a_click_or_another_claude() {
        let resumed = vec![(42, AGENT.to_string())];
        let working = [pane("p2", "t2", Some("claude"), "working", Some(AGENT))];
        assert!(planned(&working, resumed.clone(), true).is_empty());
        let idle = [pane("p2", "t2", Some("claude"), "idle", Some(AGENT))];
        assert_eq!(
            planned(&idle, resumed, true),
            [Stale::End {
                pane: "p2".into(),
                pid: 42
            }]
        );
        // No bare resume of it runs: a click's (`--name`) or the user's own claude.
        assert!(planned(&idle, Vec::new(), true).is_empty());
        // The orchestrator's own conversation in its tab: no agent's.
        let own = [pane("p2", "t2", Some("claude"), "idle", Some(OTHER))];
        assert!(planned(&own, vec![(43, OTHER.to_string())], true).is_empty());
    }

    #[test]
    fn leaves_other_spaces_split_overview_tabs_and_panes_without_a_record() {
        let mut elsewhere = pane("p1", "t1", None, "unknown", Some(AGENT));
        elsewhere["workspace_id"] = json!("w9");
        let panes = [
            elsewhere,
            pane("p3", "t3", None, "unknown", Some(OTHER)),
            pane("p5", "t1", None, "unknown", None),
        ];
        assert!(planned(&panes, Vec::new(), true).is_empty());
    }

    #[test]
    fn reads_proc_only_for_a_running_claude_with_a_stale_record() {
        let panes = [pane("p1", "t1", None, "unknown", Some(OTHER))];
        let stale = plan(&["w2"], &tabs(), &panes, &agents(), || panic!("read /proc"), |_| true);
        assert_eq!(stale.len(), 1);
    }
}
