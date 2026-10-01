//! The nvim tabs breadboard starts (worktree/nvim.mjs, ide.mjs): nvim
//! listening on a socket in the checkout's `.claude/`, with `:Changes`, and
//! Claude Code's IDE integration (coder/claudecode.nvim, pinned) unless
//! `git config breadboard.ide` switches it off (#258).

use super::super::proc::{git, quote, text};
use super::super::repo::package_dir;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// The plugin, its tag and the commit the tag must point at.
const PLUGIN_URL: &str = "https://github.com/coder/claudecode.nvim";
const PLUGIN_TAG: &str = "v0.3.0";
const PLUGIN_COMMIT: &str = "2e6ea6f2a63cdf4fd3c05e6a054151d46848d319";

/// A file of packages/breadboard/nvim.
pub fn lua(name: &str) -> String {
    text(&package_dir().join("nvim").join(name))
}

/// Whether nvim is on the PATH, asked once: without it no nvim tab.
pub fn can_edit() -> bool {
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

/// The socket of an nvim tab, relative to its checkout.
pub fn socket_of(name: Option<&str>) -> String {
    match name {
        Some(name) => format!(".claude/nvim-{name}.sock"),
        None => ".claude/nvim.sock".into(),
    }
}

/// The nvim command that starts nvim/ide.lua with the plugin in `plugin`.
pub fn ide_command(plugin: &Path) -> String {
    let string = |value: &str| serde_json::to_string(value).unwrap_or_default();
    format!("lua dofile({})({})", string(&lua("ide.lua")), string(&text(plugin)))
}

/// The shell line of an nvim tab in `dir`, listening on the socket of
/// `name`; a socket nobody answers on is left over and goes first.
/// `commands`: what nvim runs on start, by default `:Changes` loaded.
pub fn editor_command(dir: &Path, name: Option<&str>, commands: Option<Vec<String>>, ide: Option<&Path>) -> String {
    let socket = quote(&socket_of(name));
    let mut start = commands.unwrap_or_else(|| vec![format!("luafile {}", lua("changes.lua"))]);
    start.extend(ide.map(ide_command));
    let start: String = start.iter().map(|command| format!(" -c {}", quote(command))).collect();
    format!(
        "cd {}; nvim --server {socket} --remote-expr 1 >/dev/null 2>&1 || rm -f {socket}; nvim --listen {socket}{start}",
        quote(&text(dir))
    )
}

/// Whether a value of `git config breadboard.ide` switches it off.
fn is_off(value: &str) -> bool {
    ["false", "off", "no", "0"].contains(&value.trim().to_lowercase().as_str())
}

/// Whether the integration is on for the checkout `dir`.
pub fn is_ide_on(dir: &Path) -> bool {
    !is_off(git(dir, &["config", "breadboard.ide"], None).out())
}

/// ` --ide` for the claude of a checkout with the integration on.
pub fn ide_flag(dir: &Path) -> &'static str {
    if is_ide_on(dir) { " --ide" } else { "" }
}

/// The plugin's directory under the main checkout, cloned first if missing;
/// None when the clone fails or its tag points elsewhere (then nothing of it
/// stays): nvim starts without it rather than not at all.
fn ensure_plugin(main: &Path) -> Option<PathBuf> {
    let dir = main.join(".claude/nvim").join(format!("claudecode.nvim-{PLUGIN_TAG}"));
    if dir.join("lua/claudecode").exists() {
        return Some(dir);
    }
    fs::create_dir_all(dir.parent()?).ok()?;
    let partial = PathBuf::from(format!("{}.partial", dir.display()));
    let _ = fs::remove_dir_all(&partial);
    let partial_text = text(&partial);
    let clone = [
        "-c",
        "advice.detachedHead=false",
        "clone",
        "--quiet",
        "--depth",
        "1",
        "--branch",
        PLUGIN_TAG,
        PLUGIN_URL,
        &partial_text,
    ];
    let is_cloned = git(main, &clone, None).is_ok;
    if !is_cloned || git(&partial, &["rev-parse", "HEAD"], None).out() != PLUGIN_COMMIT {
        let _ = fs::remove_dir_all(&partial);
        return None;
    }
    fs::rename(&partial, &dir).ok()?;
    Some(dir)
}

/// The plugin for the nvim tab of the checkout `dir`: None when the
/// integration is off or the plugin can't be had.
pub fn ide_for(dir: &Path, main: &Path) -> Option<PathBuf> {
    if is_ide_on(dir) { ensure_plugin(main) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_editor_listens_on_its_socket() {
        let line = editor_command(Path::new("/r/p-1"), None, Some(vec!["luafile /c.lua".into()]), None);
        assert_eq!(
            line,
            "cd '/r/p-1'; nvim --server '.claude/nvim.sock' --remote-expr 1 >/dev/null 2>&1 || rm -f '.claude/nvim.sock'; nvim --listen '.claude/nvim.sock' -c 'luafile /c.lua'"
        );
        let orchestration = editor_command(Path::new("/r/p"), Some("o"), Some(vec!["A".into(), "B o".into()]), None);
        assert!(
            orchestration.ends_with("nvim --listen '.claude/nvim-o.sock' -c 'A' -c 'B o'"),
            "{orchestration}"
        );
    }

    #[test]
    fn the_ide_starts_from_its_lua() {
        let command = ide_command(Path::new("/m/.claude/nvim/claudecode.nvim-v0.3.0"));
        assert!(command.starts_with("lua dofile(\""));
        assert!(
            command.ends_with("ide.lua\")(\"/m/.claude/nvim/claudecode.nvim-v0.3.0\")"),
            "{command}"
        );
    }

    #[test]
    fn off_values() {
        assert!(is_off(" False\n") && is_off("0") && is_off("no") && is_off("off"));
        assert!(!is_off("") && !is_off("true"));
    }
}
