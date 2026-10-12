//! `open orchestrator chat` (#287, chat.mjs, orchestrator-chat.mjs
//! `restoreChat`): one click brings the orchestrator's conversation back in
//! the tab `orchestrator` of its space, or focuses it while it runs; never
//! a second tab. What the click did replaces the row's text for 8 s.

use super::{Effect, herdr};
use crate::tui::overview::Overview;
use breadboard_core::conversations::running_claudes;
use breadboard_core::herdr::{Pane, Tab, all_panes, is_idle, panes_of, spaces, tabs_of};
use breadboard_core::local::ORCHESTRATOR_TAB;
use breadboard_core::model::Paint;
use breadboard_core::record::{is_session_id, orchestrator_label, record_of, update_record};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the row shows what a click did.
const SAID: Duration = Duration::from_millis(8000);

/// The conversation recorded for the orchestration: (session, cwd).
fn chat_of(main: &Path, name: &str) -> Option<(String, PathBuf)> {
    let chat = record_of(main, name)?.chat?;
    is_session_id(&chat.session_id).then(|| {
        let cwd = chat
            .cwd
            .filter(|cwd| !cwd.is_empty())
            .map_or_else(|| main.to_path_buf(), PathBuf::from);
        (chat.session_id.clone(), cwd)
    })
}

/// The pane of the orchestrator's claude in its space: the claude of the tab
/// `orchestrator`; without that tab the one claude in the space (not a free agent's).
fn chat_pane_of<'a>(tabs: &[Tab], panes: &'a [Pane]) -> Option<&'a Pane> {
    let free: Vec<&str> = tabs
        .iter()
        .filter(|tab| tab.label.starts_with("free "))
        .map(|tab| tab.id.as_str())
        .collect();
    let claudes: Vec<&Pane> = panes
        .iter()
        .filter(|pane| {
            pane.agent.as_deref() == Some("claude") && pane.session.is_some() && !free.contains(&pane.tab.as_str())
        })
        .collect();
    match tabs.iter().find(|tab| tab.label == ORCHESTRATOR_TAB) {
        Some(tab) => claudes.into_iter().find(|pane| pane.tab == tab.id),
        None => (claudes.len() == 1).then(|| claudes[0]),
    }
}

/// The conversation a pane runs, (session, cwd): where its claude stands now, else herdr's.
fn chat_of_pane(main: &Path, pane: &Pane) -> Option<(String, PathBuf)> {
    let session = pane.session.clone().filter(|id| is_session_id(id))?;
    let cwd = running_claudes()
        .into_iter()
        .find(|claude| claude.id == session)
        .map(|claude| claude.cwd)
        .or_else(|| pane.cwd.clone().map(PathBuf::from))
        .unwrap_or_else(|| main.to_path_buf());
    Some((session, cwd))
}

/// `chat` into the record, only on a change (`recordChat`).
pub fn record_chat(main: &Path, name: &str, chat: Option<(String, PathBuf)>) {
    let Some((session, cwd)) = chat else { return };
    if chat_of(main, name).is_some_and(|known| known == (session.clone(), cwd.clone())) {
        return;
    }
    let fields = json!({ "orchestrator": { "sessionId": session, "cwd": cwd } });
    let _ = update_record(main, name, fields.as_object().cloned().unwrap_or_default());
}

/// Whether a conversation runs in some claude on this machine (`isConversationRunning`).
fn is_conversation_running(id: &str) -> bool {
    running_claudes().iter().any(|claude| claude.id == id)
}

/// Where the chat of a space stands (`chatStateOf`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// The recorded conversation runs in a pane, or a claude in the tab `orchestrator`, or the lone claude.
    Alive,
    /// The tab is there, a shell alone idles in it: its claude died.
    Idle,
    /// The tab is there, something else runs in it.
    Busy,
    /// No tab `orchestrator`.
    Missing,
}

/// The state of a space's chat, its pane, and whether that is a lone claude
/// still to be named (`chatStateOf`). `idle`: whether a pane's shell idles.
fn state_of<'a>(
    tabs: &[Tab],
    panes: &'a [Pane],
    all: &'a [Pane],
    session: Option<&str>,
    idle: impl Fn(&str) -> bool,
) -> (State, Option<&'a Pane>, bool) {
    if let Some(running) = session.and_then(|id| all.iter().find(|pane| pane.session.as_deref() == Some(id))) {
        return (State::Alive, Some(running), false);
    }
    let Some(tab) = tabs.iter().find(|tab| tab.label == ORCHESTRATOR_TAB) else {
        return match chat_pane_of(tabs, panes) {
            Some(lone) => (State::Alive, Some(lone), true),
            None => (State::Missing, None, false),
        };
    };
    let own: Vec<&Pane> = panes.iter().filter(|pane| pane.tab == tab.id).collect();
    if let Some(claude) = own.iter().find(|pane| pane.agent.is_some()) {
        return (State::Alive, Some(claude), false);
    }
    if own.len() == 1 && idle(&own[0].id) {
        return (State::Idle, Some(own[0]), false);
    }
    (State::Busy, own.first().copied(), false)
}

/// What the tab `orchestrator` runs (`chatCommand`): the recorded
/// conversation, else a fresh claude in the main checkout; `--ide` where the
/// nvim integration is on.
fn chat_command(main: &Path, chat: Option<&(String, PathBuf)>) -> String {
    use crate::cli::lifecycle::layout::nvim::ide_flag;
    match chat {
        Some((id, cwd)) => format!(
            "cd {} && claude --resume {id}{}",
            herdr::quote(&cwd.to_string_lossy()),
            ide_flag(cwd)
        ),
        None => format!(
            "cd {} && claude{}",
            herdr::quote(&main.to_string_lossy()),
            ide_flag(main)
        ),
    }
}

/// The click (`restoreChat` with `isClick`): the space made if it is gone, a
/// live chat focused, a dead one resumed in its tab, a missing tab made with
/// it running, then in front. What happened, None when herdr did nothing.
fn restore(main: &Path, name: &str) -> Option<String> {
    use crate::cli::lifecycle::layout::{orchestrations, tabs::keep_order};
    let label = orchestrator_label(main, name);
    let repo = crate::cli::lifecycle::repo::Repo {
        main: main.to_path_buf(),
    };
    let workspace = match spaces().into_iter().find(|space| space.label == label) {
        Some(space) => space.id,
        None => orchestrations::orchestrator_space(&repo, name)?,
    };
    let chat = chat_of(main, name);
    let tabs = tabs_of(&workspace);
    let panes = panes_of(&workspace);
    let all = all_panes();
    let (state, pane, is_lone) = state_of(&tabs, &panes, &all, chat.as_ref().map(|(id, _)| id.as_str()), is_idle);
    let focus = |workspace: &str, tab: &str| {
        herdr::did(&["workspace", "focus", workspace]) && herdr::did(&["tab", "focus", tab])
    };
    if let (true, Some(lone)) = (is_lone, pane) {
        herdr::did(&["tab", "rename", &lone.tab, ORCHESTRATOR_TAB]);
        record_chat(main, name, chat_of_pane(main, lone));
    }
    match (state, pane) {
        (State::Alive | State::Busy, None) => return None,
        (State::Alive, Some(pane)) => {
            let space = if pane.workspace.is_empty() {
                &workspace
            } else {
                &pane.workspace
            };
            focus(space, &pane.tab);
            return Some("focused the orchestrator".into());
        }
        (State::Busy, Some(pane)) => {
            focus(&workspace, &pane.tab);
            return Some(format!("focused the tab {ORCHESTRATOR_TAB}, something else runs in it"));
        }
        _ => {}
    }
    // A second claude in the same conversation would write into the same transcript.
    if let Some((id, _)) = chat.as_ref().filter(|(id, _)| is_conversation_running(id)) {
        return Some(format!("the conversation {id} runs elsewhere, outside herdr's panes"));
    }
    let command = chat_command(main, chat.as_ref());
    match (state, pane) {
        (State::Idle, Some(pane)) => {
            if !herdr::did(&["pane", "run", &pane.id, &command]) {
                return None;
            }
        }
        _ => {
            if !herdr::ensure_tab(&workspace, ORCHESTRATOR_TAB, main, Some(&command), &tabs) {
                return None;
            }
            keep_order(&workspace, &orchestrations::ORDER);
        }
    }
    if let Some(tab) = tabs_of(&workspace)
        .into_iter()
        .find(|tab| tab.label == ORCHESTRATOR_TAB)
    {
        focus(&workspace, &tab.id);
    }
    let what = match &chat {
        Some((id, _)) => format!("resumed {id}"),
        None => "started a fresh claude".into(),
    };
    Some(if state == State::Idle {
        format!("{what} in its tab")
    } else {
        format!("{what} in a new tab {ORCHESTRATOR_TAB}")
    })
}

/// A click on the chat row (`clickChat`): its answer shown for a while.
pub fn click(app: &mut Overview) {
    let Some(name) = app.orchestration_name() else { return };
    let main = app.actor.main.clone();
    app.actor.queue(move || {
        let said = restore(&main, &name).unwrap_or_else(|| "herdr did nothing".into());
        Some(Box::new(move |app: &mut Overview| {
            let paint = if said.starts_with("the conversation") || said.starts_with("herdr did nothing") {
                Paint::Red
            } else {
                Paint::Green
            };
            app.actor.log(paint, format!("chat: {said}"));
            app.actor.chat = Some(said.clone());
            app.actor.later(SAID, move |app: &mut Overview| {
                if app.actor.chat.as_deref() == Some(said.as_str()) {
                    app.actor.chat = None;
                    app.redraw();
                }
            });
            app.redraw();
        }) as Effect)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, tab: &str, session: Option<&str>) -> Pane {
        Pane {
            id: id.into(),
            tab: tab.into(),
            agent: session.map(|_| "claude".into()),
            session: session.map(String::from),
            ..Default::default()
        }
    }
    fn tab(id: &str, label: &str) -> Tab {
        Tab {
            id: id.into(),
            label: label.into(),
            pane_count: 1,
        }
    }

    #[test]
    fn finds_the_orchestrators_pane_never_a_free_agents() {
        let tabs = [tab("t1", "root"), tab("t2", "2"), tab("t3", "free mira")];
        let panes = [
            pane("p1", "t1", None),
            pane("p2", "t2", Some("a")),
            pane("p3", "t3", Some("b")),
        ];
        assert_eq!(chat_pane_of(&tabs, &panes).map(|pane| pane.id.as_str()), Some("p2"));
        let tabs = [tab("t1", ORCHESTRATOR_TAB), tab("t2", "2")];
        assert_eq!(chat_pane_of(&tabs, &panes), None);
    }

    #[test]
    fn tells_where_the_chat_stands_as_node_does() {
        let idle = |_: &str| true;
        let busy = |_: &str| false;
        // The recorded conversation runs somewhere: alive, wherever it is.
        let elsewhere = [pane("p9", "t9", Some("s1"))];
        let (state, found, is_lone) = state_of(&[], &[], &elsewhere, Some("s1"), idle);
        assert_eq!(
            (state, found.map(|p| p.id.as_str()), is_lone),
            (State::Alive, Some("p9"), false)
        );
        // No tab orchestrator: the lone claude is it, to be named; none, missing.
        let tabs = [tab("t1", "root"), tab("t2", "2")];
        let panes = [pane("p1", "t1", None), pane("p2", "t2", Some("s2"))];
        let (state, found, is_lone) = state_of(&tabs, &panes, &panes, Some("s1"), idle);
        assert_eq!(
            (state, found.map(|p| p.id.as_str()), is_lone),
            (State::Alive, Some("p2"), true)
        );
        assert_eq!(state_of(&tabs, &panes[..1], &panes, None, idle).0, State::Missing);
        // The tab: its shell idles (its claude died) or something else runs.
        let tabs = [tab("t1", ORCHESTRATOR_TAB)];
        let shell = [pane("p1", "t1", None)];
        assert_eq!(state_of(&tabs, &shell, &shell, None, idle).0, State::Idle);
        assert_eq!(state_of(&tabs, &shell, &shell, None, busy).0, State::Busy);
        let claude = [pane("p1", "t1", Some("s3"))];
        assert_eq!(state_of(&tabs, &claude, &claude, None, busy).0, State::Alive);
    }

    #[test]
    fn resumes_the_recorded_conversation_where_it_ran() {
        let chat = ("0123abcd".to_string(), PathBuf::from("/r/it's"));
        let command = chat_command(Path::new("/r"), Some(&chat));
        assert!(
            command.starts_with(r"cd '/r/it'\''s' && claude --resume 0123abcd"),
            "{command}"
        );
        assert!(chat_command(Path::new("/r"), None).starts_with("cd /r && claude"));
    }
}
