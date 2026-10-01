//! The nvim tabs breadboard starts (nvim.mjs, ide.mjs): listening on a socket
//! in the checkout's `.claude/`, with `:Changes` loaded and Claude Code's IDE
//! integration (coder/claudecode.nvim, pinned) unless `git config
//! breadboard.ide` switches it off (#258).

use super::proc::{git, quote};
use std::fs;
use std::path::{Path, PathBuf};

const PLUGIN_URL: &str = "https://github.com/coder/claudecode.nvim";
const PLUGIN_TAG: &str = "v0.3.0";
const PLUGIN_COMMIT: &str = "2e6ea6f2a63cdf4fd3c05e6a054151d46848d319";

/// `packages/breadboard/nvim/<file>` of the checkout that built this binary.
fn lua(main: &Path, file: &str) -> PathBuf {
    super::proc::package_dir(main).join("nvim").join(file)
}

/// What every nvim tab loads first: `luafile …/changes.lua`.
pub fn changes_lua(main: &Path) -> String {
    format!("luafile {}", lua(main, "changes.lua").display())
}

/// What the tab `changes` of an orchestration runs on start.
pub fn changes_commands(main: &Path, name: &str) -> Vec<String> {
    vec![
        format!("luafile {}", lua(main, "orchestration.lua").display()),
        format!("OrchestrationChanges {name}"),
    ]
}

/// The socket of an nvim tab, relative to its checkout.
pub fn socket_of(name: Option<&str>) -> String {
    match name {
        Some(name) => format!(".claude/nvim-{name}.sock"),
        None => ".claude/nvim.sock".into(),
    }
}

/// The shell line of an nvim tab in `dir`: a socket nobody answers on goes first.
pub fn editor_command(dir: &Path, name: Option<&str>, commands: &[String], ide: Option<&Path>) -> String {
    let socket = quote(&socket_of(name));
    let mut start: Vec<String> = commands.to_vec();
    if let Some(plugin) = ide {
        start.push(ide_command(&lua(&super::repo::main_dir(), "ide.lua"), plugin));
    }
    let start: String = start.iter().map(|command| format!(" -c {}", quote(command))).collect();
    format!(
        "cd {}; nvim --server {socket} --remote-expr 1 >/dev/null 2>&1 || rm -f {socket}; nvim --listen {socket}{start}",
        quote(&dir.to_string_lossy())
    )
}

/// `lua dofile("<ide.lua>")("<plugin>")`, the strings as JSON writes them.
pub fn ide_command(ide_lua: &Path, plugin: &Path) -> String {
    let text = |path: &Path| serde_json::to_string(&path.to_string_lossy()).unwrap_or_default();
    format!("lua dofile({})({})", text(ide_lua), text(plugin))
}

/// Whether a value of `git config breadboard.ide` switches it off.
pub fn is_off(value: &str) -> bool {
    ["false", "off", "no", "0"].contains(&value.trim().to_lowercase().as_str())
}

/// Whether the integration is on for the checkout `dir`.
pub fn is_ide_on(dir: &Path) -> bool {
    !is_off(git(&["config", "breadboard.ide"], dir).text())
}

/// ` --ide` for a claude in a checkout with the integration on.
pub fn ide_flag(dir: &Path) -> &'static str {
    if is_ide_on(dir) { " --ide" } else { "" }
}

/// The plugin under `.claude/nvim/` of the main checkout, cloned once at
/// the pinned commit; None when that fails (nvim starts without it).
pub fn ensure_plugin(main: &Path) -> Option<PathBuf> {
    let dir = main
        .join(".claude")
        .join("nvim")
        .join(format!("claudecode.nvim-{PLUGIN_TAG}"));
    if dir.join("lua").join("claudecode").exists() {
        return Some(dir);
    }
    fs::create_dir_all(dir.parent()?).ok()?;
    let partial = PathBuf::from(format!("{}.partial", dir.display()));
    let _ = fs::remove_dir_all(&partial);
    let partial_text = partial.to_string_lossy().into_owned();
    let is_cloned = git(
        &[
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
        ],
        main,
    )
    .is_ok;
    if !is_cloned || git(&["rev-parse", "HEAD"], &partial).text() != PLUGIN_COMMIT {
        let _ = fs::remove_dir_all(&partial);
        return None;
    }
    fs::rename(&partial, &dir).ok()?;
    Some(dir)
}

/// The plugin for the nvim tab of `dir`: None when the integration is off.
pub fn ide_for(dir: &Path, main: &Path) -> Option<PathBuf> {
    if is_ide_on(dir) { ensure_plugin(main) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_nvim_on_its_socket() {
        let line = editor_command(Path::new("/w/x"), None, &["luafile /c.lua".into()], None);
        assert_eq!(
            line,
            "cd '/w/x'; nvim --server '.claude/nvim.sock' --remote-expr 1 >/dev/null 2>&1 || rm -f '.claude/nvim.sock'; nvim --listen '.claude/nvim.sock' -c 'luafile /c.lua'"
        );
        assert_eq!(socket_of(Some("b")), ".claude/nvim-b.sock");
        assert!(is_off(" Off "));
        assert!(!is_off("true"));
    }
}
