//! A file of the worktree in its nvim (#257, files.mjs `openFile`,
//! nvim.mjs, spaces.mjs `openInEditor`): through the socket of the tab
//! `nvim` (`.claude/nvim.sock`, relative to the checkout: a socket's path
//! holds at most 107 bytes), else the tab started on the file, then its space
//! and tab in front. And the herdr tabs a click starts (tabs.mjs `ensureTab`):
//! the editor's, Storybook's. The herdr writes are exactly Node's (rule 6).

use breadboard_core::git::git;
use breadboard_core::herdr::{self, Tab};
use breadboard_core::model::files::{first_change, match_file};
use breadboard_core::record::SPACE;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The tab of a worktree's editor.
const EDITOR: &str = "nvim";
/// The socket of that tab, relative to the checkout.
const SOCKET: &str = ".claude/nvim.sock";
/// claudecode.nvim's directory under the main checkout, cloned by the layout (ide.mjs `pluginDir`).
const IDE_PLUGIN: &str = ".claude/nvim/claudecode.nvim-v0.3.0";
/// How long nvim gets to answer on its socket.
const ANSWER: Duration = Duration::from_secs(2);

/// Where a checkout's file is opened: the checkout, the main one, its feature.
pub struct Checkout<'a> {
    pub dir: &'a Path,
    pub main: &'a Path,
    pub is_main: bool,
    /// `292-x`: the label of its herdr space.
    pub feature: &'a str,
}

impl Checkout<'_> {
    /// Its herdr space (spaces.mjs `spaceLabel`): the main checkout's is `Monocircuit/Platform`.
    fn space(&self) -> Option<String> {
        let label = if self.is_main { SPACE } else { self.feature };
        herdr::spaces()
            .into_iter()
            .find(|space| space.label == label)
            .map(|space| space.id)
    }
}

/// `file` (relative to the checkout, or as a log names it) at `line`, else
/// at its first change; Err is why not, for the ` nvim: …` line.
pub fn open_file(checkout: &Checkout, file: &str, line: Option<u64>) -> Result<(), String> {
    let found = file_of(checkout.dir, file).ok_or_else(|| format!("{file} is no file of this checkout"))?;
    let line = line.unwrap_or_else(|| first_change(checkout.dir, &found));
    if !can_edit() {
        return Err("no nvim on the PATH".into());
    }
    if open_on_socket(checkout.dir, &found, line) {
        if let Some(space) = checkout.space() {
            focus(&space);
        }
        return Ok(());
    }
    let Some(space) = checkout.space() else {
        return Err("no herdr space to open nvim in".into());
    };
    let command = editor_command(checkout, &found, line);
    if !ensure_tab(&space, EDITOR, checkout.dir, &command) {
        return Err(format!(
            "the tab {EDITOR} runs an nvim without its socket: quit it, the next one listens"
        ));
    }
    focus(&space);
    Ok(())
}

/// The file of the checkout `file` means: itself when it is there inside the
/// checkout (a file of DIFF, a conflict, an untracked one too), else the one
/// of `git ls-files` a log's path names.
fn file_of(dir: &Path, file: &str) -> Option<String> {
    let is_inside = Path::new(file).is_relative() && !file.split('/').any(|part| part == "..");
    if is_inside && dir.join(file).is_file() {
        return Some(file.to_string());
    }
    let listed = git(dir, &["ls-files"]).stdout;
    let files: Vec<String> = listed.lines().map(String::from).collect();
    match_file(&files, file)
}

/// Whether nvim runs at all here (spaces.mjs `canEdit`), asked once.
fn can_edit() -> bool {
    static HAS_NVIM: OnceLock<bool> = OnceLock::new();
    *HAS_NVIM.get_or_init(|| {
        Command::new("nvim")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// `nvim --server <socket> <args>` in `dir`: whether it answered within ANSWER.
fn remote(dir: &Path, args: &[&str]) -> bool {
    let child = Command::new("nvim")
        .args(["--server", SOCKET])
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return false };
    let until = Instant::now() + ANSWER;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// The file at the line in the nvim listening in `dir`, out of insert or
/// command-line mode first (nvim.mjs `openOnSocket`).
fn open_on_socket(dir: &Path, file: &str, line: u64) -> bool {
    remote(dir, &["--remote-send", "<C-\\><C-N>"]);
    let path = dir.join(file);
    remote(dir, &["--remote-expr", &open_expr(&path.to_string_lossy(), line)])
}

/// The Vim script that opens `file` at `line` (its window if one shows it) with the line in the middle.
fn open_expr(file: &str, line: u64) -> String {
    let file = file.replace('\'', "''");
    format!(
        "execute('drop +{} ' .. fnameescape('{file}') .. ' | normal! zz')",
        line.max(1)
    )
}

/// A word for sh, bash, zsh and fish alike.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The shell line of the tab `nvim` (nvim.mjs `editorCommand`): a socket
/// nobody answers on goes first, then nvim listens on it with `:Changes`
/// loaded, Claude Code's IDE plugin when the layout cloned it and the
/// checkout didn't switch it off (#258), on the file at the line.
fn editor_command(checkout: &Checkout, file: &str, line: u64) -> String {
    let socket = quote(SOCKET);
    let changes = checkout.main.join("packages/breadboard/nvim/changes.lua");
    let mut commands = vec![format!("luafile {}", changes.display())];
    if let Some(plugin) = ide_plugin(checkout) {
        let lua = checkout.main.join("packages/breadboard/nvim/ide.lua");
        let json = |path: &Path| serde_json::to_string(&path.to_string_lossy()).unwrap_or_default();
        commands.push(format!("lua dofile({})({})", json(&lua), json(&plugin)));
    }
    let start: String = commands
        .iter()
        .map(|command| format!(" -c {}", quote(command)))
        .collect();
    format!(
        "cd {}; nvim --server {socket} --remote-expr 1 >/dev/null 2>&1 || rm -f {socket}; nvim --listen {socket}{start} +{} {}",
        quote(&checkout.dir.to_string_lossy()),
        line.max(1),
        quote(file)
    )
}

/// The IDE plugin for the checkout: None when `git config breadboard.ide` is off or the layout hasn't cloned it.
fn ide_plugin(checkout: &Checkout) -> Option<std::path::PathBuf> {
    let value = git(checkout.dir, &["config", "breadboard.ide"]).stdout;
    if ["false", "off", "no", "0"].contains(&value.trim().to_lowercase().as_str()) {
        return None;
    }
    let plugin = checkout.main.join(IDE_PLUGIN);
    plugin.join("lua/claudecode").is_dir().then_some(plugin)
}

/// The space in front, and its tab `nvim`.
fn focus(space: &str) {
    herdr::herdr(&["workspace", "focus", space]);
    if let Some(tab) = herdr::tabs_of(space).into_iter().find(|tab| tab.label == EDITOR) {
        herdr::herdr(&["tab", "focus", &tab.id]);
    }
}

/// The pane of the tab `label` when it is its only one and its shell idles (tabs.mjs `idlePaneOf`).
fn idle_pane_of(space: &str, label: &str, tabs: &[Tab]) -> Option<String> {
    let tab = tabs.iter().find(|tab| tab.label == label && tab.pane_count == 1)?;
    let pane = herdr::panes_of(space).into_iter().find(|pane| pane.tab == tab.id)?;
    herdr::is_idle(&pane.id).then_some(pane.id)
}

/// The tab `label` of `space` running `command`: created unfocused when it
/// is missing, `command` run again when its shell idles. Whether either happened.
pub fn ensure_tab(space: &str, label: &str, cwd: &Path, command: &str) -> bool {
    let tabs = herdr::tabs_of(space);
    if tabs.iter().any(|tab| tab.label == label) {
        return idle_pane_of(space, label, &tabs)
            .is_some_and(|pane| herdr::herdr(&["pane", "run", &pane, command]).is_some());
    }
    let cwd = cwd.to_string_lossy();
    let args = [
        "tab",
        "create",
        "--workspace",
        space,
        "--cwd",
        &cwd,
        "--label",
        label,
        "--no-focus",
    ];
    let Some(created) = herdr::herdr(&args) else {
        return false;
    };
    if let Some(pane) = created
        .get("root_pane")
        .and_then(|pane| pane.get("pane_id"))
        .and_then(|id| id.as_str())
    {
        herdr::herdr(&["pane", "run", pane, command]);
    }
    true
}

/// Storybook of the checkout in its tab `storybook` (layout.mjs `startStorybook`): false without a space.
pub fn start_storybook(checkout: &Checkout, port: i64) -> bool {
    let Some(space) = checkout.space() else {
        return false;
    };
    let cwd = checkout.dir.join("packages/monolithium");
    ensure_tab(
        &space,
        "storybook",
        &cwd,
        &format!("pnpm exec storybook dev -p {port} --ci"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_at_the_line_with_quotes_doubled() {
        assert_eq!(
            open_expr("/w/it's.ts", 12),
            "execute('drop +12 ' .. fnameescape('/w/it''s.ts') .. ' | normal! zz')"
        );
        assert_eq!(
            open_expr("/w/a.ts", 0),
            "execute('drop +1 ' .. fnameescape('/w/a.ts') .. ' | normal! zz')"
        );
    }

    #[test]
    fn the_tab_starts_nvim_listening_on_the_file() {
        let checkout = Checkout {
            dir: Path::new("/nowhere/platform-1-x"),
            main: Path::new("/nowhere/platform"),
            is_main: false,
            feature: "1-x",
        };
        assert_eq!(
            editor_command(&checkout, "src/a b.ts", 7),
            "cd '/nowhere/platform-1-x'; nvim --server '.claude/nvim.sock' --remote-expr 1 >/dev/null 2>&1 \
             || rm -f '.claude/nvim.sock'; nvim --listen '.claude/nvim.sock' \
             -c 'luafile /nowhere/platform/packages/breadboard/nvim/changes.lua' +7 'src/a b.ts'"
        );
    }

    #[test]
    fn a_file_of_the_checkout_is_itself_else_what_the_log_names() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(file_of(dir, "Cargo.toml").as_deref(), Some("Cargo.toml"));
        assert_eq!(
            file_of(dir, "/builds/x/packages/breadboard/crates/breadboard/src/main.rs").as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(file_of(dir, "src/gone.rs"), None);
    }
}
