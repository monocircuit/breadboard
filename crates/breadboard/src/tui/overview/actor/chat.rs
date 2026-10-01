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

/// The click (`restoreChat` with `isClick`): what happened, None when herdr did nothing.
fn restore(main: &Path, name: &str) -> Option<String> {
    let label = orchestrator_label(main, name);
    let Some(workspace) = spaces()
        .into_iter()
        .find(|space| space.label == label)
        .map(|space| space.id)
    else {
        return Some(format!("its space {label} is gone: pnpm wt:herdr brings it back"));
    };
    let chat = chat_of(main, name);
    let tabs = tabs_of(&workspace);
    let panes = panes_of(&workspace);
    let all = all_panes();
    let tab = tabs.iter().find(|tab| tab.label == ORCHESTRATOR_TAB);
    let running = chat
        .as_ref()
        .and_then(|(id, _)| all.iter().find(|pane| pane.session.as_deref() == Some(id)));
    let focus = |workspace: &str, tab: &str| {
        herdr::did(&["workspace", "focus", workspace]) && herdr::did(&["tab", "focus", tab])
    };
    if let Some(pane) = running {
        focus(&pane.workspace, &pane.tab);
        return Some("focused the orchestrator".into());
    }
    let Some(tab) = tab else {
        if let Some(lone) = chat_pane_of(&tabs, &panes) {
            herdr::did(&["tab", "rename", &lone.tab, ORCHESTRATOR_TAB]);
            record_chat(main, name, chat_of_pane(main, lone));
            focus(&lone.workspace, &lone.tab);
            return Some("focused the orchestrator".into());
        }
        return resume(main, &workspace, chat, true, &tabs);
    };
    let own: Vec<&Pane> = panes.iter().filter(|pane| pane.tab == tab.id).collect();
    if let Some(claude) = own.iter().find(|pane| pane.agent.is_some()) {
        focus(&workspace, &claude.tab);
        return Some("focused the orchestrator".into());
    }
    if own.len() == 1 && is_idle(&own[0].id) {
        return resume(main, &workspace, chat, false, &tabs)
            .map(|said| said.replace(" in a new tab orchestrator", " in its tab"));
    }
    focus(&workspace, &tab.id);
    Some(format!("focused the tab {ORCHESTRATOR_TAB}, something else runs in it"))
}

/// The recorded conversation resumed (a fresh claude without one): in the
/// idle tab, or in a new tab `orchestrator`; then in front.
fn resume(
    main: &Path,
    workspace: &str,
    chat: Option<(String, PathBuf)>,
    is_missing: bool,
    tabs: &[Tab],
) -> Option<String> {
    // A second claude in the same conversation would write into the same transcript.
    if let Some((id, _)) = chat.as_ref().filter(|(id, _)| is_conversation_running(id)) {
        return Some(format!("the conversation {id} runs elsewhere, outside herdr's panes"));
    }
    let command = match &chat {
        Some((id, cwd)) => format!("cd {} && claude --resume {id}", herdr::quote(&cwd.to_string_lossy())),
        None => format!("cd {} && claude", herdr::quote(&main.to_string_lossy())),
    };
    if is_missing {
        if !herdr::ensure_tab(workspace, ORCHESTRATOR_TAB, main, Some(&command), tabs) {
            return None;
        }
    } else {
        let tab = tabs.iter().find(|tab| tab.label == ORCHESTRATOR_TAB)?;
        let pane = panes_of(workspace).into_iter().find(|pane| pane.tab == tab.id)?;
        if !herdr::did(&["pane", "run", &pane.id, &command]) {
            return None;
        }
    }
    if let Some(tab) = tabs_of(workspace).into_iter().find(|tab| tab.label == ORCHESTRATOR_TAB) {
        herdr::did(&["workspace", "focus", workspace]);
        herdr::did(&["tab", "focus", &tab.id]);
    }
    let what = match &chat {
        Some((id, _)) => format!("resumed {id}"),
        None => "started a fresh claude".into(),
    };
    Some(format!("{what} in a new tab {ORCHESTRATOR_TAB}"))
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
}
