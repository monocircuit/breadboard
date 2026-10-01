//! The plugin after a move (#230): herdr keeps loading `monocircuit.platform`
//! from the directory it was linked from. When that directory is gone (a
//! worktree removed, the package moved as `packages/workflow` did in #219),
//! every plugin action fails with "plugin action not found" and the tab
//! bar's status command fails every ten seconds, without a word.
//!
//! Once per run inside herdr, a lifecycle command asks `herdr plugin list`,
//! links the plugin again from the package's current `herdr/` when its path
//! is gone, and rewrites the gone paths in the repo's `.herdr/config.toml`.
//! The user's own `~/.config/herdr/config.toml` is only read: what names a
//! gone path there is said with the command that writes it (the memory
//! ask-before-global-config; rewrite-log 2026-10-02).

use super::herdr;
use super::proc::{output, step, text};
use breadboard_core::git::Out;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;

/// The plugin's id in `herdr-plugin.toml`.
pub const PLUGIN: &str = "monocircuit.platform";
const MANIFEST: &str = "herdr-plugin.toml";

/// Checks the plugin and the configs once per process, inside herdr only.
pub fn heal(main: &Path) {
    static ONCE: Once = Once::new();
    if herdr::is_in() {
        ONCE.call_once(|| {
            for line in check(main) {
                step(&line);
            }
        });
    }
}

/// What was done or needs doing, one line each; nothing when all is well.
fn check(main: &Path) -> Vec<String> {
    let current = current_dir(main);
    let mut lines = Vec::new();
    let mut gone = Vec::new();
    if let Some(paths) = run(&["plugin", "list"])
        .filter(|out| out.is_ok)
        .and_then(|out| linked_paths(&answer(&out)))
        && !paths.is_empty()
        && paths.iter().all(|path| !path.exists())
    {
        let old = plugin_dir_of(&paths[0]);
        lines.push(match &current {
            Some(current) => relink(&old, current),
            None => format!(
                "plugin {PLUGIN} is linked from {}, which is gone, and this checkout has no packages/breadboard/herdr to link instead: pnpm herdr:install in one that has",
                shown(&old)
            ),
        });
        gone.push(old);
    }
    let Some(current) = current else {
        return lines;
    };
    let config = main.join(".herdr").join("config.toml");
    if let Ok(was) = fs::read_to_string(&config) {
        let (now, replaced) = rewritten(&was, &gone, &current);
        if now != was && fs::write(&config, &now).is_ok() {
            let reloaded = if is_session_config(&config) && herdr::did(&["server", "reload-config"]) {
                ", and reloaded"
            } else {
                ""
            };
            // The plugin's gone path was said just above; another one is named.
            let was_named = replaced.iter().all(|dir| gone.contains(dir));
            lines.push(if was_named {
                format!(".herdr/config.toml names the new path too{reloaded}")
            } else {
                format!(
                    ".herdr/config.toml names {} instead of {}, which is gone{reloaded}",
                    shown(&current),
                    list(&replaced)
                )
            });
        }
    }
    if let Some(own) = own_config()
        && let Ok(was) = fs::read_to_string(&own)
    {
        let stale = stale_dirs(&was, &gone, &current);
        if !stale.is_empty() {
            let mine = outside_block(&was);
            let is_outside = stale.iter().any(|dir| mine.contains(&text(dir)));
            lines.push(format!(
                "{} still names {}, which is gone: pnpm herdr:install writes the plugin's block again{}",
                shown(&own),
                list(&stale),
                if is_outside {
                    format!("; a line of your own there needs {}", shown(&current))
                } else {
                    String::new()
                }
            ));
        }
    }
    lines
}

fn relink(old: &Path, current: &Path) -> String {
    herdr::did(&["plugin", "unlink", PLUGIN]);
    match run(&["plugin", "link", &text(current)]) {
        Some(out) if out.is_ok => format!(
            "plugin {PLUGIN} linked again from {}: {} is gone",
            shown(current),
            shown(old)
        ),
        out => format!(
            "plugin {PLUGIN} was linked from {}, which is gone, and herdr refused {}: {}",
            shown(old),
            shown(current),
            out.map(|out| answer(&out))
                .unwrap_or_else(|| "herdr did not run".into())
        ),
    }
}

/// `herdr <args>` with its whole answer; None when herdr didn't start.
fn run(args: &[&str]) -> Option<Out> {
    herdr::forget();
    let out = output("herdr", args, Path::new("/"), None);
    (out.status != -1).then_some(out)
}

fn answer(out: &Out) -> String {
    format!("{}{}", out.stdout, out.stderr).trim().to_string()
}

/// The plugin of the main checkout, else the one next to this binary
/// (session.mjs `pluginDir`: the main checkout's once merged, else the
/// checkout this runs from).
fn current_dir(main: &Path) -> Option<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|exe| {
        exe.ancestors()
            .map(|dir| dir.join("herdr"))
            .find(|dir| dir.join(MANIFEST).is_file())
    });
    [Some(main.join("packages/breadboard/herdr")), beside]
        .into_iter()
        .flatten()
        .find(|dir| dir.join(MANIFEST).is_file())
}

/// The directory of a linked path: itself, or the one of its manifest.
fn plugin_dir_of(path: &Path) -> PathBuf {
    if path.file_name().is_some_and(|name| name == MANIFEST) {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        path.to_path_buf()
    }
}

/// The paths `herdr plugin list` names for the plugin; None when it isn't
/// listed. herdr answers JSON or a table: in JSON, the absolute paths of
/// the entry that names the plugin; in text, those on its line and the
/// indented lines under it.
fn linked_paths(listed: &str) -> Option<Vec<PathBuf>> {
    if !listed.contains(PLUGIN) {
        return None;
    }
    let mut paths = Vec::new();
    if let Ok(json) = serde_json::from_str::<Value>(listed) {
        entry_paths(&json, &mut paths);
        return Some(paths);
    }
    let mut is_ours = false;
    for line in listed.lines() {
        if line.contains(PLUGIN) {
            is_ours = true;
        } else if !line.starts_with(char::is_whitespace) {
            is_ours = false;
        }
        if is_ours {
            paths.extend(
                line.split(|c: char| c.is_whitespace() || "\"'(),[]=".contains(c))
                    .filter(|word| word.starts_with('/'))
                    .map(PathBuf::from),
            );
        }
    }
    Some(paths)
}

/// The absolute paths inside every JSON object that names the plugin.
fn entry_paths(value: &Value, paths: &mut Vec<PathBuf>) {
    match value {
        Value::Object(map) => {
            if let Some(entry) = map.get(PLUGIN) {
                strings_in(entry, paths);
            } else if map.values().any(|field| field.as_str() == Some(PLUGIN)) {
                strings_in(value, paths);
            } else {
                map.values().for_each(|field| entry_paths(field, paths));
            }
        }
        Value::Array(items) => items.iter().for_each(|item| entry_paths(item, paths)),
        _ => {}
    }
}

fn strings_in(value: &Value, paths: &mut Vec<PathBuf>) {
    match value {
        Value::String(path) if path.starts_with('/') => paths.push(PathBuf::from(path)),
        Value::Object(map) => map.values().for_each(|field| strings_in(field, paths)),
        Value::Array(items) => items.iter().for_each(|item| strings_in(item, paths)),
        _ => {}
    }
}

/// The plugin directories a config names that are gone: `gone` (the
/// plugin's old path) where the config names it, and every `<dir>/run.mjs`
/// (the `{{RUN}}` of herdr/config.toml and keys.toml) whose `…/herdr` is
/// gone; never `current`.
fn stale_dirs(config: &str, gone: &[PathBuf], current: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = gone
        .iter()
        .filter(|dir| replace_dir(config, &text(dir), "") != config)
        .cloned()
        .collect();
    for (at, _) in config.match_indices("/run.mjs") {
        let start = config[..at]
            .rfind(|c: char| c.is_whitespace() || "\"'=[,".contains(c))
            .map_or(0, |before| before + 1);
        let dir = PathBuf::from(&config[start..at]);
        if dir.is_absolute() && dir.ends_with("herdr") && dir != current && !dirs.contains(&dir) && !dir.exists() {
            dirs.push(dir);
        }
    }
    dirs
}

/// The config with every gone plugin directory replaced by `current`, and
/// the directories it replaced.
fn rewritten(config: &str, gone: &[PathBuf], current: &Path) -> (String, Vec<PathBuf>) {
    let stale = stale_dirs(config, gone, current);
    let mut now = config.to_string();
    for dir in &stale {
        now = replace_dir(&now, &text(dir), &text(current));
    }
    (now, stale)
}

/// `old` replaced where it is a whole directory: followed by `/`, a quote,
/// a space, `,`, `]` or the end, never as the start of a longer name.
fn replace_dir(config: &str, old: &str, new: &str) -> String {
    let mut result = String::with_capacity(config.len());
    let mut rest = config;
    while let Some(at) = rest.find(old) {
        let after = rest[at + old.len()..].chars().next();
        let is_whole = after.is_none_or(|c| c == '/' || c.is_whitespace() || "\"',]".contains(c));
        result.push_str(&rest[..at]);
        result.push_str(if is_whole { new } else { old });
        rest = &rest[at + old.len()..];
    }
    result.push_str(rest);
    result
}

/// The config without the block pnpm herdr:install keeps (session.mjs `withoutBlock`).
fn outside_block(config: &str) -> String {
    let mark = format!("# >>> {PLUGIN}");
    let end = format!("# <<< {PLUGIN}");
    let block = config
        .find(&mark)
        .and_then(|start| Some((start, start + config[start..].find(&end)? + end.len())));
    match block {
        Some((start, stop)) => format!("{}{}", &config[..start], &config[stop..]),
        None => config.to_string(),
    }
}

/// The user's own herdr config, read only.
fn own_config() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("herdr").join("config.toml"))
}

/// Whether the herdr this runs in reads `config`: the session `platform`
/// of pnpm herdr, which runs with the repo's `.herdr/config.toml`.
fn is_session_config(config: &Path) -> bool {
    std::env::var_os("HERDR_CONFIG_PATH").is_some_and(|path| Path::new(&path) == config)
        || std::env::var("HERDR_SOCKET_PATH").is_ok_and(|socket| socket.contains("/sessions/platform/"))
}

fn list(dirs: &[PathBuf]) -> String {
    dirs.iter().map(|dir| shown(dir)).collect::<Vec<_>>().join(", ")
}

/// A path for a message: `~/…` under the home directory.
fn shown(path: &Path) -> String {
    match std::env::var_os("HOME").and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if !rest.as_os_str().is_empty() => format!("~/{}", text(&rest)),
        _ => text(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_plugins_path_in_json() {
        let listed = r#"{"result":{"plugins":[
            {"id":"other.one","path":"/x/other"},
            {"id":"monocircuit.platform","path":"/gone/packages/workflow/herdr","enabled":true}]}}"#;
        assert_eq!(
            linked_paths(listed),
            Some(vec![PathBuf::from("/gone/packages/workflow/herdr")])
        );
        let keyed = r#"{"monocircuit.platform":{"source":"/a/herdr/herdr-plugin.toml"}}"#;
        assert_eq!(
            linked_paths(keyed),
            Some(vec![PathBuf::from("/a/herdr/herdr-plugin.toml")])
        );
        assert_eq!(linked_paths(r#"{"result":{}}"#), None);
    }

    #[test]
    fn finds_the_plugins_path_in_a_table() {
        let listed =
            "ID                    PATH\nother.one             /x/other\nmonocircuit.platform  /gone/herdr (linked)\n";
        assert_eq!(linked_paths(listed), Some(vec![PathBuf::from("/gone/herdr")]));
        let block = "monocircuit.platform 0.1.0\n  path: /gone/herdr\nother.one\n  path: /x/other\n";
        assert_eq!(linked_paths(block), Some(vec![PathBuf::from("/gone/herdr")]));
        assert_eq!(linked_paths("no plugins linked"), None);
    }

    #[test]
    fn the_dir_of_a_linked_manifest() {
        assert_eq!(
            plugin_dir_of(Path::new("/a/herdr/herdr-plugin.toml")),
            PathBuf::from("/a/herdr")
        );
        assert_eq!(plugin_dir_of(Path::new("/a/herdr")), PathBuf::from("/a/herdr"));
    }

    #[test]
    fn rewrites_only_gone_plugin_dirs() {
        let root = std::env::temp_dir().join(format!("lifecycle-plugin-{}", std::process::id()));
        let current = root.join("herdr");
        fs::create_dir_all(&current).unwrap();
        let config = concat!(
            "tab_bar_right = [{ type = \"command\", command = \"node /gone/packages/workflow/herdr/run.mjs status\" }]\n",
            "[keys]\nx = \"node /gone/packages/workflow/herdr/run.mjs open overview\"\n",
            "y = \"node /gone/packages/workflow/herdrs/run.mjs\"\n",
        );
        let (now, replaced) = rewritten(config, &[], &current);
        assert_eq!(replaced, vec![PathBuf::from("/gone/packages/workflow/herdr")]);
        let run = format!("{}/run.mjs", text(&current));
        assert_eq!(now.matches(&run).count(), 2);
        assert!(
            now.contains("/gone/packages/workflow/herdrs/run.mjs"),
            "not a plugin dir"
        );
        let (again, none) = rewritten(&now, &[], &current);
        assert!(none.is_empty());
        assert_eq!(again, now);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_listed_gone_path_counts_even_without_run_mjs() {
        let gone = vec![PathBuf::from("/old/herdr")];
        let (now, replaced) = rewritten(
            "plugin = \"/old/herdr\"\nkeep = \"/old/herdrx\"\n",
            &gone,
            Path::new("/now/herdr"),
        );
        assert_eq!(replaced, gone);
        assert_eq!(now, "plugin = \"/now/herdr\"\nkeep = \"/old/herdrx\"\n");
        let (same, none) = rewritten("keep = \"/old/herdrx\"\n", &gone, Path::new("/now/herdr"));
        assert!(none.is_empty());
        assert_eq!(same, "keep = \"/old/herdrx\"\n");
    }

    #[test]
    fn tells_the_block_from_your_own_lines() {
        let config = "a = 1\n# >>> monocircuit.platform: written\nb = \"/old/herdr/run.mjs\"\n# <<< monocircuit.platform\nc = 2\n";
        assert_eq!(outside_block(config), "a = 1\n\nc = 2\n");
    }
}
