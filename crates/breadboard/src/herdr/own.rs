//! Your own herdr (`pnpm herdr:install`, e.g. Omarchy's; own.mjs): whether
//! it can load the plugin, how to get one that can, and the marked block in
//! its config.

use super::session::{self, Session, Target, shown};
use super::setup::{fetch, link_plugin, write_linked};
use super::sidebar::sidebar_config;
use super::{Fail, PLUGIN, SPACE, step};
use regex::Regex;
use std::fs;
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The minor version the manifest asks for (`min_herdr_version = "0.9.0"`):
/// older herdrs list the plugin but don't load it.
const MIN_MINOR: u64 = 9;

/// The first `x.y.z` of a text.
pub fn version_of(text: &str) -> Option<String> {
    let pattern = Regex::new(r"(\d+)\.(\d+)\.(\d+)").expect("a valid pattern");
    pattern.find(text).map(|found| found.as_str().to_string())
}

pub fn can_load_plugin(version: &str) -> bool {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    major > 0 || minor >= MIN_MINOR
}

/// Your herdr: the first on PATH that is not the pinned one in .herdr/bin,
/// which panes of `platform` see first.
pub fn own_herdr(session: &Session) -> Option<PathBuf> {
    let pinned_dir = session.bin.parent().map(Path::to_path_buf);
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|dir| !dir.as_os_str().is_empty())
        .filter(|dir| {
            Some(dir.canonicalize().unwrap_or_else(|_| dir.clone())) != pinned_dir && Some(dir.clone()) != pinned_dir
        })
        .map(|dir| dir.join("herdr"))
        .find(|file| file.exists())
}

/// Omarchy installs herdr as a pacman package of its own repo.
fn is_omarchy_package(binary: &Path) -> bool {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    binary.starts_with("/usr/") && home.join(".local/share/omarchy").exists()
}

/// The herdr package of Omarchy's edge channel, from its repo database,
/// when it can load the plugin. The database is zstd: `zstd -dc` decodes it
/// (Arch's pacman needs zstd, so it is there where this matters); without
/// it, or offline, None, as Node's failed fetch.
fn omarchy_edge_package() -> Option<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    let compressed = fetch(&format!("https://pkgs.omarchy.org/edge/{arch}/omarchy.db")).ok()?;
    let mut child = Command::new("zstd")
        .args(["-dc"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut input = child.stdin.take()?;
    let writer = std::thread::spawn(move || input.write_all(&compressed));
    let mut db = Vec::new();
    child.stdout.take()?.read_to_end(&mut db).ok()?;
    let _ = writer.join();
    child.wait().ok().filter(|status| status.success())?;
    edge_file_in(&String::from_utf8_lossy(&db)).map(|file| format!("https://pkgs.omarchy.org/edge/{arch}/{file}"))
}

/// The herdr package file a pacman database names, when it can load the plugin.
fn edge_file_in(db: &str) -> Option<String> {
    let pattern = Regex::new(r"%FILENAME%\n(herdr-\d[^\n]*)").expect("a valid pattern");
    let file = pattern.captures(db)?.get(1)?.as_str().to_string();
    version_of(&file)
        .filter(|version| can_load_plugin(version))
        .map(|_| file)
}

/// What to do about a herdr too old for the plugin.
fn upgrade_hint(binary: &Path, version: &str) -> String {
    let intro = format!(
        "herdr {version} ({}) doesn't load the plugin, it needs 0.{MIN_MINOR} or newer.",
        binary.display()
    );
    if !is_omarchy_package(binary) {
        return format!(
            "{intro} Update it (herdr update, or the package manager it came from), or use plain pnpm herdr"
        );
    }
    let install = match omarchy_edge_package() {
        Some(edge) => format!(
            "Omarchy's edge channel has {}. Install that package through pacman, so the next omarchy-update carries on from it:\n\n    sudo pacman -U {edge}\n\n",
            version_of(edge.rsplit('/').next().unwrap_or_default()).unwrap_or_default()
        ),
        None => "Omarchy has no newer herdr package yet: omarchy-update brings it once it has. ".into(),
    };
    format!(
        "{intro} {install}herdr update doesn't work here, pacman owns {}. Then restart your herdr (herdr server stop, it ends its panes) and run pnpm herdr:install again",
        binary.display()
    )
}

/// The block the session adds to your config: the plugin's keys, and the
/// sidebar rows unless your config has its own (`text` without the block).
fn block_for(session: &Session, text: &str) -> Result<String, Fail> {
    let keys_file = session.plugin_dir.join("keys.toml");
    let keys = fs::read_to_string(&keys_file).map_err(|error| format!("{}: {error}", keys_file.display()))?;
    let mut lines = vec![session::block_begin(), keys.trim_end().to_string()];
    if let Some(rows) = sidebar_config(&session::without_block(text)) {
        lines.push(String::new());
        lines.push(rows);
    }
    lines.push(session::block_end());
    Ok(lines.join("\n"))
}

/// The marked block at the end of your config.toml, replaced on every run so
/// new keys arrive by themselves. The first time it asks; with no terminal
/// to ask in, or a no, it prints the block to paste instead. `is_renew_only`
/// (--reload): only a block that is there, no question. Whether it changed.
pub fn write_own_block(session: &Session, file: &Path, is_renew_only: bool) -> Result<bool, Fail> {
    write_own_block_with(session, file, is_renew_only, &mut || {
        if !std::io::stdin().is_terminal() {
            return None;
        }
        print!(
            "? add the plugin's keys and sidebar rows to {} as one marked block [Y/n] ",
            shown(file)
        );
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer).ok()?;
        Some(answer)
    })
}

/// `write_own_block` with the question asked by `ask` (None: nobody to ask).
fn write_own_block_with(
    session: &Session,
    file: &Path,
    is_renew_only: bool,
    ask: &mut dyn FnMut() -> Option<String>,
) -> Result<bool, Fail> {
    let text = fs::read_to_string(file).unwrap_or_default();
    let block = block_for(session, &text)?;
    let shown = shown(file);
    let Some((start, stop)) = session::block_range(&text) else {
        if is_renew_only {
            return Ok(false);
        }
        let is_wanted = ask().is_some_and(|answer| !answer.trim_start().to_lowercase().starts_with('n'));
        if !is_wanted {
            println!("\nFor {shown} (then herdr server reload-config):\n\n{block}\n");
            return Ok(false);
        }
        let joined = format!(
            "{}{}{block}\n",
            text.trim_end(),
            if text.trim().is_empty() { "" } else { "\n\n" }
        );
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        fs::write(file, joined).map_err(|error| format!("{shown}: {error}"))?;
        step(&format!("the plugin's block appended to {shown}"));
        return Ok(true);
    };
    let next = format!("{}{block}{}", &text[..start], &text[stop..]);
    if next == text {
        return Ok(false);
    }
    fs::write(file, next).map_err(|error| format!("{shown}: {error}"))?;
    step(&format!("the plugin's block in {shown} renewed"));
    Ok(true)
}

/// The plugin in your own herdr instead of the session `platform`. Your
/// herdr and its running server have to load it (>= 0.9); then it is linked,
/// its keys go into your config, and the space of the repo is built once.
pub fn install_into_own(session: &Session) -> Result<(), Fail> {
    let own = own_herdr(session).ok_or("no herdr of your own on PATH: install one, or use plain pnpm herdr")?;
    let target = Target::Own(own.clone());
    let version = version_of(&session.herdr(&target, &["--version"]).out)
        .ok_or_else(|| format!("{} --version names no version", own.display()))?;
    if !can_load_plugin(&version) {
        return Err(upgrade_hint(&own, &version).into());
    }
    let server = session.herdr(&target, &["status", "server"]).out;
    let is_serving = server.contains("status: running");
    let serving = session::version_after(&server).and_then(|version| version_of(&version));
    if is_serving && let Some(serving) = serving.filter(|serving| !can_load_plugin(serving)) {
        return Err(format!(
            "your herdr is {version}, its server still runs {serving} and doesn't load the plugin. herdr server stop (it ends its panes), start herdr again, then pnpm herdr:install"
        )
        .into());
    }
    let dir = session.plugin_dir.display();
    if link_plugin(session, &target)? {
        step(&format!("plugin {PLUGIN} linked from {dir}"));
    } else {
        step(&format!("plugin {PLUGIN} is linked from {dir}"));
    }
    write_linked(session, &target);
    let is_changed = write_own_block(session, &session::own_config(), false)?;
    if !is_serving {
        println!(
            "\n✓ {PLUGIN} is ready. Your herdr doesn't run: start it (Omarchy: super+ctrl+enter), then pnpm herdr:install again for the space {SPACE}"
        );
        return Ok(());
    }
    if is_changed && session.herdr(&target, &["server", "reload-config"]).is_ok {
        step("config reloaded");
    }
    let restored = session.herdr(&target, &["plugin", "action", "invoke", &format!("{PLUGIN}.restore")]);
    if !restored.is_ok {
        return Err(format!("the layout failed: {}", restored.out).into());
    }
    println!("\n✓ {PLUGIN} runs in your herdr {version}: the space {SPACE}, a space per worktree, the plugin's keys");
    // A block can't add to your [ui]: the tab bar's line is yours to add (README).
    let config = fs::read_to_string(session::own_config()).unwrap_or_default();
    let status = session.status_command();
    if !config.contains(&status) {
        println!(
            "  the tab bar's ⬢ stacks · needs you: add to tab_bar_right of your [ui]\n  {{ type = \"command\", command = \"{status}\", interval_seconds = 10, timeout_seconds = 3 }}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> (Scratch, Session) {
        let root = std::env::temp_dir().join(format!("herdr-own-{name}-{}", std::process::id()));
        let plugin = root.join("platform/packages/breadboard/herdr");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("keys.toml"),
            "[[keys.command]]\nkey = \"prefix+o\"\ncommand = \"monocircuit.platform.overview\"\n",
        )
        .unwrap();
        let session = Session::at(root.join("platform"), plugin);
        (Scratch(root), session)
    }

    fn config_with(root: &Path, text: &str) -> PathBuf {
        let file = root.join(format!("{}.toml", text.len()));
        fs::write(&file, text).unwrap();
        file
    }

    // The four cases of own.test.mjs.
    #[test]
    fn renews_a_block_that_is_there_and_keeps_what_is_around_it() {
        let (dir, session) = scratch("renew");
        let file = config_with(
            &dir.0,
            &format!(
                "[ui]\nsidebar_width = 34\n\n{}\nold keys\n{}\n\n# mine after it\n",
                session::block_begin(),
                session::block_end()
            ),
        );
        assert!(write_own_block_with(&session, &file, false, &mut || None).unwrap());
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("[ui]\nsidebar_width = 34\n\n"));
        assert!(text.ends_with(&format!("{}\n\n# mine after it\n", session::block_end())));
        assert!(!text.contains("old keys"));
        assert!(text.contains("monocircuit.platform.overview"));
        assert_eq!(text.matches(&session::block_begin()).count(), 1);
    }

    #[test]
    fn changes_nothing_the_second_time() {
        let (dir, session) = scratch("twice");
        let file = config_with(
            &dir.0,
            &format!("{}\n{}\n", session::block_begin(), session::block_end()),
        );
        write_own_block_with(&session, &file, false, &mut || None).unwrap();
        assert!(!write_own_block_with(&session, &file, false, &mut || None).unwrap());
    }

    #[test]
    fn never_adds_a_block_when_it_only_renews() {
        let (dir, session) = scratch("renew-only");
        let file = config_with(&dir.0, "[ui]\n");
        let mut is_asked = false;
        let added = write_own_block_with(&session, &file, true, &mut || {
            is_asked = true;
            Some("y".into())
        });
        assert!(!added.unwrap() && !is_asked);
        assert_eq!(fs::read_to_string(&file).unwrap(), "[ui]\n");
    }

    #[test]
    fn leaves_out_the_sidebar_rows_when_the_config_has_its_own() {
        let (dir, session) = scratch("own-rows");
        let file = config_with(
            &dir.0,
            &format!(
                "[ui.sidebar.spaces]\nrows = [[\"workspace\"]]\n\n{}\n{}\n",
                session::block_begin(),
                session::block_end()
            ),
        );
        write_own_block_with(&session, &file, false, &mut || None).unwrap();
        assert_eq!(
            fs::read_to_string(&file)
                .unwrap()
                .matches("[ui.sidebar.spaces]")
                .count(),
            1
        );
    }

    #[test]
    fn appends_after_a_yes_and_prints_after_a_no() {
        let (dir, session) = scratch("ask");
        let file = config_with(&dir.0, "[ui]\nx = 1\n\n");
        assert!(!write_own_block_with(&session, &file, false, &mut || Some("n\n".into())).unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), "[ui]\nx = 1\n\n");
        assert!(write_own_block_with(&session, &file, false, &mut || Some("\n".into())).unwrap());
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.starts_with(&format!("[ui]\nx = 1\n\n{}\n", session::block_begin())));
        assert!(text.ends_with(&format!("{}\n", session::block_end())));
    }

    #[test]
    fn versions_that_load_the_plugin() {
        assert_eq!(version_of("herdr 0.9.1\n").as_deref(), Some("0.9.1"));
        assert!(can_load_plugin("0.9.0") && can_load_plugin("1.0.0"));
        assert!(!can_load_plugin("0.8.2"));
        let db = "%FILENAME%\nherdr-0.9.2-1-x86_64.pkg.tar.zst\n%NAME%\nherdr\n";
        assert_eq!(edge_file_in(db).as_deref(), Some("herdr-0.9.2-1-x86_64.pkg.tar.zst"));
        assert_eq!(edge_file_in("%FILENAME%\nherdr-0.8.9-1-x86_64.pkg.tar.zst\n"), None);
    }
}
