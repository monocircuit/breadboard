//! The herdr CLI of the session this runs in, as JSON results (lib/herdr.mjs).
//! Outside herdr (or when herdr is old or unhappy) every read answers None,
//! never fails: the layout is comfort, the work goes on without it. Read only
//! here: what herdr does on a click is the writing side's.

use crate::json::str_of;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Whether this runs inside herdr: `HERDR_ENV=1`.
pub fn is_in_herdr() -> bool {
    std::env::var("HERDR_ENV").as_deref() == Ok("1")
}

/// What answers herdr's reads in place of a call of this process: the
/// overviews' shared reader (`breadboard serve`); its None: ask herdr here.
pub type SharedReads = fn(&[&str]) -> Option<Option<Value>>;

static SHARED: std::sync::OnceLock<SharedReads> = std::sync::OnceLock::new();

/// From now on this process's reads go to `shared` first (a live overview does this; never a command that writes).
pub fn share_reads(shared: SharedReads) {
    let _ = SHARED.set(shared);
}

/// Whether `args` only read: lists and a pane's process. Nothing else is ever shared.
pub fn is_read(args: &[&str]) -> bool {
    matches!(
        args,
        ["workspace", "list", ..] | ["tab", "list", ..] | ["pane", "list", ..] | ["pane", "process-info", ..]
    )
}

/// `herdr <args>` → its `result`, None if herdr is missing, old or unhappy.
pub fn herdr(args: &[&str]) -> Option<Value> {
    if is_read(args)
        && let Some(answer) = SHARED.get().and_then(|shared| shared(args))
    {
        return answer;
    }
    herdr_here(args)
}

/// `herdr <args>` run by this process.
pub fn herdr_here(args: &[&str]) -> Option<Value> {
    let output = Command::new("herdr")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    value.get("result").filter(|result| !result.is_null()).cloned()
}

/// The list under `key` of a result, empty without one.
fn list_of(result: Option<Value>, key: &str) -> Vec<Value> {
    result
        .and_then(|result| result.get(key).and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

/// A space of herdr: its id and label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub id: String,
    pub label: String,
}

/// A tab: its id, label and how many panes it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: String,
    pub label: String,
    pub pane_count: u64,
}

/// A pane as `pane list` answers it, what the overview reads of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pane {
    pub id: String,
    pub tab: String,
    pub workspace: String,
    /// The agent running in it (`claude`), None for a shell.
    pub agent: Option<String>,
    /// The conversation of its agent.
    pub session: Option<String>,
    /// herdr's state of its agent: `working`, `idle`, `done`, `blocked`, `unknown`.
    pub status: Option<String>,
    pub cwd: Option<String>,
}

impl Pane {
    pub fn from_value(value: &Value) -> Option<Pane> {
        Some(Pane {
            id: str_of(value, "pane_id")?.to_string(),
            tab: str_of(value, "tab_id").unwrap_or_default().to_string(),
            workspace: str_of(value, "workspace_id").unwrap_or_default().to_string(),
            agent: str_of(value, "agent")
                .filter(|agent| !agent.is_empty())
                .map(String::from),
            session: value
                .get("agent_session")
                .and_then(|session| str_of(session, "value"))
                .filter(|id| !id.is_empty())
                .map(String::from),
            status: str_of(value, "agent_status").map(String::from),
            cwd: str_of(value, "cwd").map(String::from),
        })
    }
}

pub fn spaces_in(result: Option<Value>) -> Vec<Space> {
    list_of(result, "workspaces")
        .iter()
        .filter_map(|space| {
            Some(Space {
                id: str_of(space, "workspace_id")?.to_string(),
                label: str_of(space, "label").unwrap_or_default().to_string(),
            })
        })
        .collect()
}

pub fn tabs_in(result: Option<Value>) -> Vec<Tab> {
    list_of(result, "tabs")
        .iter()
        .filter_map(|tab| {
            Some(Tab {
                id: str_of(tab, "tab_id")?.to_string(),
                label: str_of(tab, "label").unwrap_or_default().to_string(),
                pane_count: tab.get("pane_count").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect()
}

pub fn panes_in(result: Option<Value>) -> Vec<Pane> {
    list_of(result, "panes").iter().filter_map(Pane::from_value).collect()
}

/// The spaces of herdr, none outside it.
pub fn spaces() -> Vec<Space> {
    if !is_in_herdr() {
        return Vec::new();
    }
    spaces_in(herdr(&["workspace", "list"]))
}

/// `spaces` asked of herdr by this process, never a shared answer: the read
/// after a write of the layout (another overview's answer of up to a second
/// ago lacks the spaces it just opened).
pub fn spaces_now() -> Vec<Space> {
    if !is_in_herdr() {
        return Vec::new();
    }
    spaces_in(herdr_here(&["workspace", "list"]))
}

/// The tabs of a space.
pub fn tabs_of(workspace: &str) -> Vec<Tab> {
    tabs_in(herdr(&["tab", "list", "--workspace", workspace]))
}

/// The panes of a space.
pub fn panes_of(workspace: &str) -> Vec<Pane> {
    panes_in(herdr(&["pane", "list", "--workspace", workspace]))
}

/// `panes_of` asked of herdr by this process, never a shared answer: the read a write into a pane follows.
pub fn panes_of_now(workspace: &str) -> Vec<Pane> {
    panes_in(herdr_here(&["pane", "list", "--workspace", workspace]))
}

/// Every pane of every space, none outside herdr.
pub fn all_panes() -> Vec<Pane> {
    if !is_in_herdr() {
        return Vec::new();
    }
    panes_in(herdr(&["pane", "list"]))
}

/// What herdr says an agent does, by its conversation's id (auto-actions.mjs `agentsOf`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub pane: String,
    pub tab: String,
    pub status: Option<String>,
}

/// The agents in `panes`, by their conversation.
pub fn agents_in(panes: &[Pane]) -> HashMap<String, Agent> {
    panes
        .iter()
        .filter(|pane| pane.agent.is_some())
        .filter_map(|pane| {
            Some((
                pane.session.clone()?,
                Agent {
                    pane: pane.id.clone(),
                    tab: pane.tab.clone(),
                    status: pane.status.clone(),
                },
            ))
        })
        .collect()
}

/// The agents herdr runs, by conversation; empty outside herdr (nothing is known then).
pub fn agents() -> HashMap<String, Agent> {
    agents_in(&all_panes())
}

/// Whether the shell of a pane has the terminal to itself: nothing runs in it (tabs.mjs `isIdle`).
pub fn is_idle(pane: &str) -> bool {
    is_idle_in(herdr(&["pane", "process-info", "--pane", pane]))
}

/// `is_idle` asked of herdr by this process, never a shared answer: the read a write into the pane follows.
pub fn is_idle_now(pane: &str) -> bool {
    is_idle_in(herdr_here(&["pane", "process-info", "--pane", pane]))
}

fn is_idle_in(info: Option<Value>) -> bool {
    let Some(info) = info else {
        return false;
    };
    let info = info.get("process_info").cloned().unwrap_or(Value::Null);
    let shell = info.get("shell_pid").and_then(Value::as_i64);
    shell.is_some_and(|shell| {
        shell != 0 && info.get("foreground_process_group_id").and_then(Value::as_i64) == Some(shell)
    })
}

// ---------------------------------------------------------------------------
// Colours (herdr-tones.mjs, worktree/sidebar.mjs): the overview paints like
// herdr's sidebar with every theme.
// ---------------------------------------------------------------------------

/// herdr state → the name of the colour its sidebar paints it with.
pub fn state_colour(status: &str) -> Option<&'static str> {
    match status {
        "working" => Some("yellow"),
        "idle" | "done" => Some("teal"),
        "blocked" => Some("red"),
        _ => None,
    }
}

/// `name = "#rrggbb"` of the colour names `yellow`, `teal`, `red`, `blue` in a
/// herdr config's text; the last one wins, as TOML sections read in order.
pub fn colours_of(text: &str) -> HashMap<String, String> {
    let mut colours = HashMap::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !["yellow", "teal", "red", "blue"].contains(&name) {
            continue;
        }
        let rest = rest.trim_start();
        if let Some(hex) = rest.strip_prefix('"').and_then(|hex| hex.get(..7))
            && is_hex(hex)
            && rest.get(8..9) == Some("\"")
        {
            colours.insert(name.to_string(), hex.to_string());
        }
    }
    colours
}

fn is_hex(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// The section `[name]` of a TOML text, up to the next table.
fn section_of<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let head = format!("[{name}]");
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices("\n[").map(|(i, _)| i + 1))
        .collect();
    let at = starts.iter().position(|start| text[*start..].starts_with(&head))?;
    Some(&text[starts[at]..starts.get(at + 1).copied().unwrap_or(text.len())])
}

/// The hex of `colour` in the `[theme.custom]` of the config text, else `fallback` (sidebar.mjs `colourIn`).
pub fn colour_in(config: &str, colour: &str, fallback: &str) -> String {
    section_of(config, "theme.custom")
        .and_then(|section| {
            section.lines().find_map(|line| {
                let rest = line
                    .strip_prefix(colour)?
                    .trim_start()
                    .strip_prefix('=')?
                    .trim_start()
                    .strip_prefix('"')?;
                let hex = rest.get(..7)?;
                (is_hex(hex) && rest.get(7..8) == Some("\"")).then(|| hex.to_string())
            })
        })
        .unwrap_or_else(|| fallback.to_string())
}

/// The pink of herdr's sidebar for branches: a worktree's name reads the same in both (names.mjs).
pub fn branch_colour(config: &str) -> String {
    colour_in(config, "pink", "#FF79C6")
}

/// The config herdr runs with: the one it was started with, the repo's, your own (sidebar.mjs `herdrConfigText`).
pub fn config_text(main: &Path) -> String {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let candidates = [
        std::env::var_os("HERDR_CONFIG_PATH").map(PathBuf::from),
        Some(main.join(".herdr").join("config.toml")),
        Some(home.join(".config").join("herdr").join("config.toml")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|file| file.exists())
        .and_then(|file| std::fs::read_to_string(file).ok())
        .unwrap_or_default()
}

/// herdr's own config, where its theme writes the colours (herdr-tones.mjs `CONFIG`).
pub fn theme_text() -> String {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    std::fs::read_to_string(base.join("herdr").join("config.toml")).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The plugin's state (herdr/run.mjs): an action writes down where it was
// triggered, in the plugin state directory, for the pane it opens to read.
// ---------------------------------------------------------------------------

/// The id the plugin is linked under.
pub fn plugin_id() -> String {
    std::env::var("HERDR_PLUGIN_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "monocircuit.platform".into())
}

/// `$HERDR_PLUGIN_STATE_DIR/last-context.json`, None outside a plugin command.
pub fn context_file() -> Option<PathBuf> {
    std::env::var_os("HERDR_PLUGIN_STATE_DIR").map(|dir| PathBuf::from(dir).join("last-context.json"))
}

/// The directory a key was pressed in: the focused pane, else the active pane
/// herdr names, else the space's checkout, else its cwd (run.mjs `whereFrom`).
pub fn where_from(context: &Value) -> Option<String> {
    str_of(context, "focused_pane_cwd")
        .map(String::from)
        .or_else(|| {
            std::env::var("HERDR_ACTIVE_PANE_CWD")
                .ok()
                .filter(|cwd| !cwd.is_empty())
        })
        .or_else(|| {
            context
                .get("worktree")
                .and_then(|tree| str_of(tree, "checkout_path"))
                .map(String::from)
        })
        .or_else(|| str_of(context, "workspace_cwd").map(String::from))
}

/// The context herdr handed over (`HERDR_PLUGIN_CONTEXT_JSON`) when it says
/// where, else the one the action saved.
pub fn plugin_context() -> Value {
    let given: Value = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    if where_from(&given).is_some() {
        return given;
    }
    context_file()
        .and_then(|file| crate::json::read(&file))
        .unwrap_or(given)
}

/// Writes down where an action was triggered, as run.mjs does:
/// `JSON.stringify({ ...here, focused_pane_cwd })`.
pub fn save_context(context: &Value) -> std::io::Result<()> {
    let Some(file) = context_file() else { return Ok(()) };
    let mut saved = context.as_object().cloned().unwrap_or_default();
    saved.insert(
        "focused_pane_cwd".into(),
        where_from(context).map(Value::String).unwrap_or(Value::Null),
    );
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // JSON.stringify, no newline after it.
    std::fs::write(&file, Value::Object(saved).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_agents_of_a_pane_list() {
        let result = json!({ "panes": [
            { "pane_id": "wD:p1", "tab_id": "wD:t1", "workspace_id": "wD", "agent_status": "unknown" },
            { "pane_id": "wM:p1", "tab_id": "wM:t1", "workspace_id": "wM", "agent": "claude",
              "agent_session": { "value": "165e22aa-00c1-4380-869d-27a099b965bf" }, "agent_status": "idle" },
        ] });
        let panes = panes_in(Some(result));
        assert_eq!(panes.len(), 2);
        let agents = agents_in(&panes);
        assert_eq!(agents.len(), 1);
        assert_eq!(
            agents["165e22aa-00c1-4380-869d-27a099b965bf"].status.as_deref(),
            Some("idle")
        );
        assert_eq!(agents["165e22aa-00c1-4380-869d-27a099b965bf"].tab, "wM:t1");
    }

    #[test]
    fn reads_the_colours_herdrs_theme_wrote() {
        let config = "[theme.custom]\naccent = \"#EFEFEF\"\nred = \"#FF5C6C\"    # blocked\nyellow = \"#FFD84D\" # working\nteal = \"#5CE1E6\"\ngreen = \"#7CFF6B\"\n";
        let colours = colours_of(config);
        assert_eq!(colours.len(), 3);
        assert_eq!(colours["red"], "#FF5C6C");
        assert_eq!(colours["teal"], "#5CE1E6");
        assert_eq!(state_colour("working"), Some("yellow"));
        assert_eq!(state_colour("done"), Some("teal"));
        assert_eq!(state_colour("unknown"), None);
    }

    #[test]
    fn finds_where_a_key_was_pressed() {
        let context = json!({ "worktree": { "checkout_path": "/r/platform-1-x" }, "workspace_cwd": "/r" });
        assert_eq!(
            where_from(&json!({ "focused_pane_cwd": "/r/a", "workspace_cwd": "/r" })).as_deref(),
            Some("/r/a")
        );
        if std::env::var_os("HERDR_ACTIVE_PANE_CWD").is_none() {
            assert_eq!(where_from(&context).as_deref(), Some("/r/platform-1-x"));
            assert_eq!(where_from(&json!({})), None);
        }
    }

    #[test]
    fn finds_the_pink_of_the_branches() {
        assert_eq!(branch_colour(""), "#FF79C6");
        assert_eq!(
            branch_colour("[ui]\npink = \"#000000\"\n[theme.custom]\npink = \"#FF00AA\"\n"),
            "#FF00AA"
        );
        assert_eq!(
            branch_colour("[theme.custom]\nred = \"#FF00AA\"\n[other]\npink = \"#123456\"\n"),
            "#FF79C6"
        );
    }
}
