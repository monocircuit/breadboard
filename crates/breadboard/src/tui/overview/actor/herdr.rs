//! What the overview asks herdr to do (answers.mjs, tabs.mjs, spaces.mjs): a
//! line typed into an agent, a tab or space in front, a tab made, a
//! notification. Outside herdr nothing is asked and nothing gets in. The
//! reads are core's (`breadboard_core::herdr`).

use breadboard_core::git::feature_of;
use breadboard_core::herdr::{self, Pane, Tab, is_in_herdr};
use breadboard_core::model::order::Target;
use breadboard_core::reader::{LocalReader, View};
use breadboard_core::record::{SPACE, space_label_of};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::process::{Command, Stdio};

/// What every line typed into an agent starts with: Claude Code takes a line
/// starting with `!` as a shell command, `/` as a slash command, `#` as a memory.
pub const PREFIX: &str = "[overview] ";

/// `line` as it is typed: with `[overview] ` in front unless it has it (`overviewLine`).
pub fn overview_line(line: &str) -> String {
    if line.starts_with(PREFIX) {
        line.to_string()
    } else {
        format!("{PREFIX}{line}")
    }
}

/// `herdr <args>` for its effect: true when herdr did it (`herdrDid`); never outside herdr.
pub fn did(args: &[&str]) -> bool {
    is_in_herdr()
        && Command::new("herdr")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

/// `herdr <args>` → its result, None outside herdr.
pub fn ask(args: &[&str]) -> Option<serde_json::Value> {
    if !is_in_herdr() {
        return None;
    }
    herdr::herdr(args)
}

/// herdr's states of its panes' agents, hashed (`pane list`): they hold no
/// file, so the local read's 2 s look asks herdr too and reads again when
/// one changed. An answer or a failed pipeline's line waiting on a blocked
/// agent goes in at the next look after it is free (Node: every local read,
/// view.mjs `retryDeliveries`); with `--herdr` (`is_sync`) the sidebar's
/// line of an agent follows its state within that look too, and a pane that
/// came or went sets off the sync (overview.mjs `keepInStep`). None for the
/// view of everything without the sync (it shows no agent) and outside herdr.
pub fn states_stamp(reader: &LocalReader, is_sync: bool) -> Option<u64> {
    if !reader.is_in_herdr || (reader.view == View::Everything && !is_sync) {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    for pane in herdr::all_panes() {
        (pane.id, pane.session, pane.status).hash(&mut hasher);
    }
    Some(hasher.finish())
}

/// The label of a checkout's space, the name of its tab group: the main space for the main checkout.
pub fn space_label(main: &Path, dir: &Path) -> String {
    if dir == main {
        SPACE.to_string()
    } else if let Ok(name) = dir.strip_prefix(breadboard_core::git::orchestrations_dir(main)) {
        // An orchestration's checkout carries its space: `⇶ <title>`, its product's (#405).
        space_label_of(main, &name.to_string_lossy())
    } else {
        feature_of(main, dir)
    }
}

/// The pane of the agent `to` addresses, None without one (`agentPaneOf`):
/// a worktree's, the pane that runs its bound conversation, wherever it is
/// (#409: a worktree has no space of its own); the orchestrator's, in the tab
/// `orchestrator` of its space.
pub fn agent_pane_of(main: &Path, to: &Target) -> Option<Pane> {
    match to {
        Target::Dir(dir) => {
            let id = breadboard_core::conversations::bound_session(dir)?;
            // Fresh, never the daemon's shared answer: what it says decides whether a line is typed in.
            herdr::all_panes_now()
                .into_iter()
                .find(|pane| pane.agent.is_some() && pane.session.as_deref() == Some(id.as_str()))
        }
        Target::Orchestration(name) => {
            let space = space_label_of(main, name);
            let workspace = herdr::spaces().into_iter().find(|found| found.label == space)?.id;
            let tab = herdr::tabs_of(&workspace)
                .into_iter()
                .find(|found| found.label == breadboard_core::local::ORCHESTRATOR_TAB)?
                .id;
            herdr::panes_of_now(&workspace)
                .into_iter()
                .find(|pane| pane.tab == tab && pane.agent.is_some())
        }
    }
}

/// The tab of the pane that runs the conversation `id`, None when none does.
pub fn agent_tab_of_session(id: &str) -> Option<String> {
    herdr::all_panes()
        .into_iter()
        .find(|pane| pane.session.as_deref() == Some(id))
        .map(|pane| pane.tab)
}

/// Whether a line can go into `pane`: there is one, and its agent isn't at a question.
pub fn can_type(pane: Option<&Pane>) -> bool {
    pane.is_some_and(|pane| pane.status.as_deref() != Some("blocked"))
}

/// `line` typed into `pane` and submitted (`herdr agent prompt`); true when it got in.
pub fn type_into(pane: Option<&Pane>, line: &str) -> bool {
    let Some(pane) = pane.filter(|pane| can_type(Some(pane))) else {
        return false;
    };
    did(&["agent", "prompt", &pane.id, &overview_line(line)])
}

/// The pane of an interactive claude that runs the conversation of the
/// worktree `dir` now (#361): one opened by a click, or a task agent still in
/// its tab from before the switch. Its queue is held while it runs, so a line
/// for it goes into that pane, as before the switch. None when no
/// interactive claude runs it, or none of herdr's panes does.
pub fn held_pane(main: &Path, dir: &Path) -> Option<Pane> {
    let id = breadboard_core::conversations::bound_session(dir)?;
    if breadboard_core::conversations::interactive_pids_of(&id).is_empty() {
        return None;
    }
    agent_pane_of(main, &Target::Dir(dir.to_path_buf())).or_else(|| {
        herdr::all_panes()
            .into_iter()
            .find(|pane| pane.session.as_deref() == Some(id.as_str()))
    })
}

/// Whether a line for the agent of `dir` goes into its run queue: a task
/// agent's (#361), unless an interactive claude of it runs in a pane.
pub fn is_queued_for(main: &Path, dir: &Path) -> bool {
    breadboard_core::runs::is_headless(main, dir) && held_pane(main, dir).is_none()
}

/// One line for the agent of the worktree `dir` (`deliverTo`); true when it
/// got in: queued for the next run of a task agent (#358, #361), typed into
/// the pane of a claude that runs it interactively (one opened by a click, a
/// task agent still in its tab, the claude of a worktree of your own).
pub fn deliver_to(main: &Path, dir: &Path, line: &str) -> bool {
    if breadboard_core::runs::is_headless(main, dir) {
        if let Some(pane) = held_pane(main, dir) {
            return type_into(Some(&pane), line);
        }
        return enqueue(main, dir, line);
    }
    type_into(agent_pane_of(main, &Target::Dir(dir.to_path_buf())).as_ref(), line)
}

/// One line queued for the next run of the headless agent of the worktree
/// `dir` (#358), the daemon woken to run it; false when no session is bound
/// there. Queued is in: the daemon runs it, now or once memory allows.
pub fn enqueue(main: &Path, dir: &Path, line: &str) -> bool {
    enqueue_allowing(main, dir, line, Vec::new())
}

/// `enqueue` with rules allowed for the one run that takes the line: the
/// user's `allow` of a `permission` action (#360).
pub fn enqueue_allowing(main: &Path, dir: &Path, line: &str, allowed: Vec<String>) -> bool {
    if breadboard_core::runs::enqueue_allowing(main, dir, &overview_line(line), allowed).is_err() {
        return false;
    }
    crate::daemon::client::wake_runs(main);
    true
}

/// The tab of an agent in front, its space first: wherever herdr has it (#409).
pub fn focus_tab(_main: &Path, _dir: &Path, tab: Option<&str>) -> bool {
    let Some(tab) = tab else { return false };
    let space = herdr::all_panes()
        .into_iter()
        .find(|pane| pane.tab == tab)
        .map(|pane| pane.workspace);
    if let Some(space) = space {
        did(&["workspace", "focus", &space]);
    }
    did(&["tab", "focus", tab])
}

/// A herdr notification with its sound (`request`, `done`).
pub fn notify(title: &str, body: &str, sound: &str) -> bool {
    did(&["notification", "show", title, "--body", body, "--sound", sound])
}

/// The one pane of a tab when its shell idles (`idlePaneOf`).
pub fn idle_pane_of(workspace: &str, label: &str, tabs: &[Tab]) -> Option<String> {
    let tab = tabs.iter().find(|tab| tab.label == label)?;
    if tab.pane_count != 1 {
        return None;
    }
    let pane = herdr::panes_of(workspace).into_iter().find(|pane| pane.tab == tab.id)?;
    // Fresh: a command is run in it next.
    herdr::is_idle_now(&pane.id).then_some(pane.id)
}

/// A tab by its label, made unfocused with `command` running in it when it is
/// missing, `command` run again when its shell idles (`ensureTab`). True when either happened.
pub fn ensure_tab(workspace: &str, label: &str, cwd: &Path, command: Option<&str>, tabs: &[Tab]) -> bool {
    if tabs.iter().any(|tab| tab.label == label) {
        return command.is_some_and(|command| {
            idle_pane_of(workspace, label, tabs).is_some_and(|pane| did(&["pane", "run", &pane, command]))
        });
    }
    let cwd = cwd.to_string_lossy();
    let created = ask(&[
        "tab",
        "create",
        "--workspace",
        workspace,
        "--cwd",
        &cwd,
        "--label",
        label,
        "--no-focus",
    ]);
    let Some(created) = created else { return false };
    if let Some(command) = command
        && let Some(pane) = created
            .get("root_pane")
            .and_then(|pane| pane.get("pane_id"))
            .and_then(|id| id.as_str())
    {
        did(&["pane", "run", pane, command]);
    }
    true
}

/// A path as one word of a shell line: `'…'`, a `'` inside closed and escaped.
pub fn quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "/._-+=:@,".contains(ch))
    {
        return text.to_string();
    }
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_a_line_once() {
        assert_eq!(overview_line("!316 merged"), "[overview] !316 merged");
        assert_eq!(overview_line("[overview] #2 look"), "[overview] #2 look");
    }

    #[test]
    fn a_blocked_agent_takes_no_line() {
        let pane = |status: &str| Pane {
            id: "p".into(),
            status: Some(status.into()),
            ..Default::default()
        };
        assert!(can_type(Some(&pane("idle"))));
        assert!(can_type(Some(&pane("working"))));
        assert!(!can_type(Some(&pane("blocked"))));
        assert!(!can_type(None));
    }

    #[test]
    fn labels_the_main_checkout_with_the_main_space() {
        let main = Path::new("/r/platform");
        assert_eq!(space_label(main, main), "Monocircuit/Platform");
        assert_eq!(space_label(main, Path::new("/r/platform-307-x")), "307-x");
        assert_eq!(
            space_label(main, Path::new("/r/platform/.claude/orchestrations/bb")),
            "⇶ bb"
        );
    }

    #[test]
    fn quotes_what_the_shell_would_split() {
        assert_eq!(quote("/r/platform"), "/r/platform");
        assert_eq!(quote("/r/it's here"), r"'/r/it'\''s here'");
    }
}
