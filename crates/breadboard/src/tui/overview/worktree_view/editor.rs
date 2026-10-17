//! A file of the worktree in nvim (#257): which file and line a click
//! means, and the socket of the nvim the detail pane runs (#406,
//! `.claude/nvim.sock`, relative to the checkout: a socket's path holds at
//! most 107 bytes). And the herdr tabs a click starts (tabs.mjs
//! `ensureTab`): a tool's.

use super::super::actor::detail::At;
use breadboard_core::git::git;
use breadboard_core::herdr::{self, Tab};
use breadboard_core::model::files::{first_change, match_file};
use breadboard_core::record::SPACE;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The socket of the worktree's nvim, relative to the checkout.
const SOCKET: &str = ".claude/nvim.sock";
/// How long nvim gets to answer on its socket.
const ANSWER: Duration = Duration::from_secs(2);

/// Where a checkout's tool runs: the checkout, whether it is the main one, its feature.
pub struct Checkout<'a> {
    pub dir: &'a Path,
    pub is_main: bool,
    /// `292-x`: the label of its herdr space.
    pub feature: &'a str,
}

impl Checkout<'_> {
    /// Its herdr space (spaces.mjs `spaceLabel`): the main checkout's is `[herdr] space`.
    fn space(&self) -> Option<String> {
        let space = SPACE.get();
        let label = if self.is_main { space.as_str() } else { self.feature };
        herdr::spaces()
            .into_iter()
            .find(|space| space.label == label)
            .map(|space| space.id)
    }
}

/// `file` (relative to the checkout, or as a log names it) at `line`, else
/// at its first change: where the detail pane's nvim opens it (#406); Err is
/// why not, for the ` nvim: …` line.
pub fn at_of(dir: &Path, file: &str, line: Option<u64>) -> Result<At, String> {
    let found = file_of(dir, file).ok_or_else(|| format!("{file} is no file of this checkout"))?;
    let line = line.unwrap_or_else(|| first_change(dir, &found));
    if !can_edit() {
        return Err("no nvim on the PATH".into());
    }
    Ok(At { file: found, line })
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
pub fn remote(dir: &Path, args: &[&str]) -> bool {
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
pub fn open_on_socket(dir: &Path, file: &str, line: u64) -> bool {
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

/// The pane of the tab `label` when it is its only one and its shell idles (tabs.mjs `idlePaneOf`).
fn idle_pane_of(space: &str, label: &str, tabs: &[Tab]) -> Option<String> {
    let tab = tabs.iter().find(|tab| tab.label == label && tab.pane_count == 1)?;
    let pane = herdr::panes_of(space).into_iter().find(|pane| pane.tab == tab.id)?;
    // Fresh, never the daemon's shared answer: a command is run in it next.
    herdr::is_idle_now(&pane.id).then_some(pane.id)
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

/// A tool of the stack (Storybook) in the tab of its name (layout.mjs `startStorybook`): false without a space.
pub fn start_tool(checkout: &Checkout, tool: &breadboard_core::config::Tool, offset: u16) -> bool {
    let Some(space) = checkout.space() else {
        return false;
    };
    let cwd = match &tool.dir {
        Some(dir) => checkout.dir.join(dir),
        None => breadboard_core::stack::dir_of(checkout.dir),
    };
    ensure_tab(&space, &tool.name, &cwd, &tool.command(offset))
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
