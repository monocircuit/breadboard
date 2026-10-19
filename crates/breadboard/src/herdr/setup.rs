//! Setting the session `platform` up (setup.mjs, herdr.mjs): the pinned
//! binary, its config, the server, a window of its own, the plugin link.

use super::session::{self, ASSETS, Answer, SESSION, Session, Target, VERSION, clean_env, shown};
use super::sidebar::sidebar_config;
use super::{Done, Fail, PLUGIN, SPACE, done, step};
use regex::Regex;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

/// `pnpm herdr [--here] [--no-attach]`.
pub fn start(session: &Session, is_here: bool, is_no_attach: bool) -> Done {
    let exe = super::started_binary();
    let window = terminal_window(
        &session.main,
        &[exe.display().to_string(), "herdr".into(), "--here".into()],
    );
    let is_attaching_here = is_here || window.is_none();
    // Inside a herdr pane, attaching here would nest a herdr in it; a window of its own doesn't.
    if session::is_in_herdr() {
        if session::is_in_platform() {
            return Err(format!("you are in the session {SESSION} already").into());
        }
        if is_attaching_here && !is_no_attach {
            return Err("this is a herdr pane already, and no terminal was found to open a window of its own. In a plain terminal: pnpm herdr — or pnpm herdr:install to add the plugin to this herdr".into());
        }
    }
    ensure_binary(session)?;
    ensure_breadboard(session);
    let written = write_config(session)?;
    let base = match written.base.strip_prefix(&session.main) {
        Ok(rest) => rest.display().to_string(),
        Err(_) => shown(&written.base),
    };
    step(&format!(
        "config {} from {base}",
        session
            .config_file
            .strip_prefix(&session.main)
            .unwrap_or(&session.config_file)
            .display()
    ));
    if written.has_own_rows {
        step(
            "your [ui.sidebar.spaces] stays: add $orch1..$orch3 to its rows to see the orchestrations (packages/breadboard/herdr/README.md)",
        );
    }
    let pinned = Target::Pinned;
    if !session.is_running(&pinned) {
        start_server(session)?;
    } else if let Some(running) = session.server_version(&pinned).filter(|version| version != VERSION) {
        println!(
            "! the session runs herdr {running}, pinned is {VERSION}: {run} herdr --stop (ends its panes), then {run} herdr",
            run = breadboard_core::config::run_word()
        );
    } else if written.is_changed {
        session.herdr(&pinned, &["server", "reload-config"]);
    }
    if link_plugin(session, &pinned)? {
        step(&format!("plugin {PLUGIN} linked from {}", session.plugin_dir.display()));
    }
    write_linked(session, &pinned);
    // The startup hook only runs at a server start; a new or moved link gets the space now.
    session.herdr(&pinned, &["plugin", "action", "invoke", &format!("{PLUGIN}.restore")]);
    if is_no_attach {
        done(&format!(
            "{SPACE} in the herdr session {SESSION} is ready: {run} herdr attaches",
            run = breadboard_core::config::run_word()
        ));
        return Ok(ExitCode::SUCCESS);
    }
    if !is_attaching_here && let Some((program, args)) = window {
        let opened = Command::new(&program)
            .args(&args)
            .env_clear()
            .envs(clean_env())
            .current_dir(&session.main)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        if let Err(error) = opened {
            return Err(format!(
                "{program} didn't open a window ({error}): {run} herdr --here",
                run = breadboard_core::config::run_word()
            )
            .into());
        }
        done(&format!(
            "{SPACE} in the herdr session {SESSION}: opened in a window of its own ({program}, no padding)\n  here instead: {run} herdr --here",
            run = breadboard_core::config::run_word()
        ));
        return Ok(ExitCode::SUCCESS);
    }
    done(&format!(
        "{SPACE} in the herdr session {SESSION}, attaching (detach: prefix+d, back: {run} herdr)",
        run = breadboard_core::config::run_word()
    ));
    let status = Command::new(&session.bin)
        .args(["--session", &SESSION.get()])
        .env_clear()
        .envs(session.session_env())
        .status()
        .map_err(|error| format!("{} didn't start: {error}", session.bin.display()))?;
    Ok(ExitCode::from(status.code().unwrap_or(0).clamp(0, 255) as u8))
}

/// `pnpm herdr --stop`: the session `platform` ends, its panes with it.
pub fn stop(session: &Session) -> Done {
    if !session.bin.exists() || !session.is_running(&Target::Pinned) {
        println!("\n✓ the session {SESSION} is not running");
        return Ok(ExitCode::SUCCESS);
    }
    step(&format!("stop the session {SESSION} (its panes end)"));
    session.herdr_shown(&Target::Pinned, &["server", "stop"]);
    Ok(ExitCode::SUCCESS)
}

/// The asset of this machine: `<os>-<arch>` → (name, sha256).
fn asset() -> Result<(&'static str, &'static str), Fail> {
    let key = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    ASSETS
        .iter()
        .find(|(platform, _, _)| *platform == key)
        .map(|(_, name, sha)| (*name, *sha))
        .ok_or_else(|| {
            format!(
                "no herdr release for {key}; install it yourself and use {run} herdr:install",
                run = breadboard_core::config::run_word()
            )
            .into()
        })
}

/// The pinned binary in .herdr/bin, downloaded and checked when it is
/// missing or another version; written beside it first, then renamed.
pub fn ensure_binary(session: &Session) -> Result<(), Fail> {
    if session.bin.exists() && session.herdr(&Target::Pinned, &["--version"]).out.contains(VERSION) {
        return Ok(());
    }
    let (name, sha) = asset()?;
    step(&format!("download herdr {VERSION} ({name})"));
    let url = format!("https://github.com/herdrdev/herdr/releases/download/v{VERSION}/{name}");
    install_checked(&session.bin, &fetch(&url)?, name, sha)
}

/// `data` as the executable `bin` when its sha256 is the pinned `sha`, else
/// nothing written: a download that isn't the release never runs.
fn install_checked(bin: &Path, data: &[u8], name: &str, sha: &str) -> Result<(), Fail> {
    let actual = sha256_hex(data);
    if actual != sha {
        return Err(format!("{name} has sha256 {actual}, pinned is {sha}: not installed").into());
    }
    let dir = bin.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let partial = bin.with_extension("download");
    fs::write(&partial, data)
        .and_then(|()| fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)))
        .and_then(|()| fs::rename(&partial, bin))
        .map_err(|error| format!("{}: {error}", bin.display()).into())
}

/// The body of a GET, Err when it can't be had or answers no 2xx.
pub fn fetch(url: &str) -> Result<Vec<u8>, Fail> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|error| error.to_string())?
            .get(url)
            .send()
            .await
            .map_err(|error| format!("the download failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("the download answered {}", response.status().as_u16()).into());
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("the download broke off: {error}"))?;
        Ok(bytes.to_vec())
    })
}

pub fn sha256_hex(data: &[u8]) -> String {
    aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A tab bar status command as a config's `command = "…"` holds it:
/// Node's (`node <…>/run.mjs status`, a config written before the binary,
/// a line you added by hand, a moved package's), the launcher's (`sh <…>/
/// breadboard.sh --no-wait herdr status`) or a binary's run directly (`<…>/
/// breadboard herdr status`, the rewrite's first form). Group 2 is the
/// command, 1 and 3 what is around it.
fn status_value() -> &'static Regex {
    static VALUE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(
            r#"(command\s*=\s*["'])((?:node\s+[^\s"']*/run\.mjs|sh\s+[^\s"']*/breadboard\.sh(?:\s+--no-wait)?\s+herdr|(?:[^\s"']*/)?breadboard\s+herdr)\s+status)(["'])"#,
        )
        .expect("a valid pattern")
    })
}

/// Whether a config's text runs a status command of the plugin, any form.
pub fn has_status(text: &str) -> bool {
    status_value().is_match(text)
}

/// `text` with every status command that isn't `command` replaced by it in
/// place, the rest byte for byte (#230: a move, or the switch to the binary,
/// needs no hand work).
pub fn with_current_status(text: &str, command: &str) -> String {
    status_value()
        .replace_all(text, |caps: &regex::Captures| {
            format!("{}{command}{}", &caps[1], &caps[3])
        })
        .into_owned()
}

/// A config with the plugin's status first in the tab bar (`⬢ 2 stacks · 1
/// needs you`): put in front of the base's own `tab_bar_right`, else given
/// one of its own with herdr's zoom; unchanged when it runs `command`
/// already. An old status command is `command` now (`with_current_status`:
/// no second entry). Node's form of `command` keeps Node's cases exact.
pub fn with_status_entry(text: &str, command: &str) -> String {
    let text = if command.contains("/run.mjs ") {
        text.to_string()
    } else {
        with_current_status(text, command)
    };
    if text.contains(command) {
        return text;
    }
    let entry =
        format!("{{ type = \"command\", command = \"{command}\", interval_seconds = 10, timeout_seconds = 3 }}");
    let ui_head = Regex::new(r"(?m)^\[ui\]\s*$").expect("a valid pattern");
    let Some(ui) = ui_head.find(&text).map(|found| found.start()) else {
        return format!("{text}\n\n[ui]\ntab_bar_right = [{entry}, {{ type = \"zoom\" }}]");
    };
    let next_table = Regex::new(r"(?m)^\[").expect("a valid pattern");
    let end = next_table
        .find(&text[ui + 1..])
        .map_or(text.len(), |found| ui + 1 + found.start());
    let section = &text[ui..end];
    let bar = Regex::new(r"(?m)^(tab_bar_right\s*=\s*\[)").expect("a valid pattern");
    let with_entry = if bar.is_match(section) {
        bar.replacen(section, 1, |caps: &regex::Captures| format!("{}{entry}, ", &caps[1]))
            .into_owned()
    } else {
        ui_head
            .replacen(
                section,
                1,
                regex::NoExpand(&format!("[ui]\ntab_bar_right = [{entry}, {{ type = \"zoom\" }}]")),
            )
            .into_owned()
    };
    format!("{}{with_entry}{}", &text[..ui], &text[end..])
}

/// The breadboard the plugin's manifest and the tab bar's status run (the
/// plugin's checkout's) built from its HEAD: herdr/breadboard.sh `--built`
/// builds it when it is missing or older, its progress on this terminal.
/// Never a failure: the session itself runs without it, but without a word
/// its actions and the tab bar would wait for a build or fail.
pub fn ensure_breadboard(session: &Session) {
    if !super::launcher::built(&session.launcher()) && !session.breadboard().is_file() {
        let package = session.plugin_dir.parent().unwrap_or(&session.plugin_dir);
        println!(
            "! {} is missing: the plugin's keys and the tab bar's status work once it is built ({run} build:rs in {})",
            shown(&session.breadboard()),
            shown(package),
            run = breadboard_core::config::run_word()
        );
    }
}

/// What `write_config` wrote.
pub struct Written {
    pub base: PathBuf,
    pub is_changed: bool,
    pub has_own_rows: bool,
}

/// `{{RUN}}` of herdr/config.toml filled: the status command is the
/// binary's, any other `{{RUN}}` the path run.mjs had.
fn fill(session: &Session, text: &str) -> String {
    let run = session.plugin_dir.join("run.mjs").display().to_string();
    text.replace("node {{RUN}} status", &session.status_command())
        .replace("{{RUN}}", &run)
}

/// .herdr/config.toml: a base (yours if you have one: .herdr/config.local.toml,
/// else ~/.config/herdr/config.toml, which is only read; else the repo's),
/// the plugin's keys and the sidebar rows appended. Written only when it
/// changed; a base with its own `[ui.sidebar.spaces]` keeps it.
pub fn write_config(session: &Session) -> Result<Written, Fail> {
    write_config_over(session, &session::own_config())
}

/// `write_config` with your own herdr config at `own` (a test passes one that isn't there).
fn write_config_over(session: &Session, own: &Path) -> Result<Written, Fail> {
    let bases = [
        session.home.join("config.local.toml"),
        own.to_path_buf(),
        session.plugin_dir.join("config.toml"),
    ];
    let base = bases
        .into_iter()
        .find(|file| file.exists())
        .ok_or("packages/breadboard/herdr/config.toml is missing")?;
    let read = |file: &Path| fs::read_to_string(file).map_err(|error| Fail(format!("{}: {error}", file.display())));
    // Without the block of pnpm herdr:install: its keys and rows follow anyway.
    let filled = fill(session, &session::without_block(&read(&base)?));
    let base_text = with_status_entry(filled.trim_end(), &session.status_command());
    let keys = fill(session, &read(&session.plugin_dir.join("keys.toml"))?);
    let mut lines = vec![
        format!(
            "# Written by {run} herdr from {} and packages/breadboard/herdr/keys.toml. Changes here are",
            base.display(),
            run = breadboard_core::config::run_word()
        ),
        "# overwritten: put your own into .herdr/config.local.toml.".into(),
        String::new(),
        base_text.clone(),
        String::new(),
        keys.trim_end().to_string(),
        String::new(),
    ];
    if let Some(rows) = sidebar_config(&base_text) {
        lines.push(rows);
        lines.push(String::new());
    }
    let text = lines.join("\n");
    let is_changed = fs::read_to_string(&session.config_file).ok().as_deref() != Some(text.as_str());
    if is_changed {
        fs::create_dir_all(&session.home)
            .and_then(|()| fs::write(&session.config_file, &text))
            .map_err(|error| format!("{}: {error}", session.config_file.display()))?;
    }
    Ok(Written {
        base,
        is_changed,
        has_own_rows: base_text
            .split('\n')
            .any(|line| line.starts_with("[ui.sidebar.spaces]")),
    })
}

/// A new window of the terminal this runs in, running `command`, with no
/// padding (the terminal's padding would frame herdr's chrome, and only a
/// window's start can drop it). None when the terminal is unknown or there
/// is no display: then herdr attaches right here.
pub fn terminal_window(main: &Path, command: &[String]) -> Option<(String, Vec<String>)> {
    let has = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    terminal_window_of(main, command, &has, std::env::var("TERM_PROGRAM").ok().as_deref())
}

fn terminal_window_of(
    main: &Path,
    command: &[String],
    has: &dyn Fn(&str) -> bool,
    term_program: Option<&str>,
) -> Option<(String, Vec<String>)> {
    if !has("WAYLAND_DISPLAY") && !has("DISPLAY") && std::env::consts::OS != "macos" {
        return None;
    }
    let main = main.display().to_string();
    let with = |head: Vec<String>| head.into_iter().chain(command.iter().cloned()).collect::<Vec<_>>();
    let strings = |words: &[&str]| words.iter().map(|word| word.to_string()).collect::<Vec<_>>();
    if has("KITTY_WINDOW_ID") || has("KITTY_PID") {
        let mut head = strings(&["-o", "window_padding_width=0", "--directory"]);
        head.push(main);
        return Some(("kitty".into(), with(head)));
    }
    if has("ALACRITTY_WINDOW_ID") || has("ALACRITTY_SOCKET") {
        let mut head = strings(&[
            "-o",
            "window.padding.x=0",
            "-o",
            "window.padding.y=0",
            "--working-directory",
        ]);
        head.push(main);
        head.push("-e".into());
        return Some(("alacritty".into(), with(head)));
    }
    if term_program == Some("ghostty") || has("GHOSTTY_RESOURCES_DIR") {
        let mut head = strings(&["--window-padding-x=0", "--window-padding-y=0"]);
        head.push(format!("--working-directory={main}"));
        head.push("-e".into());
        return Some(("ghostty".into(), with(head)));
    }
    None
}

/// The session's server, headless and in a session of its own: closing
/// this terminal keeps it. Waits up to 5 s for it to answer.
fn start_server(session: &Session) -> Result<(), Fail> {
    step(&format!("start the herdr session {SESSION}"));
    let mut command = Command::new(&session.bin);
    command
        .args(["--session", &SESSION.get(), "server"])
        .env_clear()
        .envs(session.session_env())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid in the child before exec, async-signal-safe (Node's `detached`).
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = command.spawn();
    for _ in 0..50 {
        if session.is_running(&Target::Pinned) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if session.is_running(&Target::Pinned) {
        return Ok(());
    }
    Err(format!(
        "the server of the session {SESSION} did not come up: {} --session {SESSION} status",
        session.bin.display()
    )
    .into())
}

/// Links the plugin unless this very directory is linked already; one
/// linked from elsewhere is unlinked first. Whether it linked.
pub fn link_plugin(session: &Session, target: &Target) -> Result<bool, Fail> {
    let listed = session.herdr(target, &["plugin", "list"]).out;
    let dir = session.plugin_dir.display().to_string();
    if listed.contains(&PLUGIN.get()) && listed.contains(&dir) {
        return Ok(false);
    }
    if listed.contains(&PLUGIN.get()) {
        session.herdr(target, &["plugin", "unlink", &PLUGIN.get()]);
    }
    let Answer { is_ok, out } = session.herdr(target, &["plugin", "link", &dir]);
    if !is_ok {
        return Err(format!("herdr refused the plugin:\n{out}").into());
    }
    Ok(true)
}

/// Where the manifest a herdr has linked is written down, to relink only
/// when it changed: one file for the session `platform`, one for your own.
pub fn linked_file_of(session: &Session, target: &Target) -> PathBuf {
    session.home.join(match target {
        Target::Pinned => "plugin-manifest.sha256",
        Target::Own(_) => "plugin-manifest.own.sha256",
    })
}

pub fn manifest_hash(session: &Session) -> String {
    sha256_hex(&fs::read(session.plugin_dir.join("herdr-plugin.toml")).unwrap_or_default())
}

pub fn write_linked(session: &Session, target: &Target) {
    let _ = fs::create_dir_all(&session.home);
    let _ = fs::write(linked_file_of(session, target), format!("{}\n", manifest_hash(session)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = "/repo/packages/breadboard/herdr/run.mjs";

    fn entry(command: &str) -> String {
        format!("{{ type = \"command\", command = \"{command}\", interval_seconds = 10, timeout_seconds = 3 }}")
    }

    // The four cases of setup.test.mjs, byte for byte.
    #[test]
    fn puts_the_status_in_front_of_the_configs_own_tab_bar() {
        let command = format!("node {RUN} status");
        let text =
            "[ui]\nsidebar_width = 34\ntab_bar_right = [{ type = \"zoom\" }, { type = \"hostname\" }]\n\n[keys]\n";
        assert_eq!(
            with_status_entry(text, &command),
            format!(
                "[ui]\nsidebar_width = 34\ntab_bar_right = [{}, {{ type = \"zoom\" }}, {{ type = \"hostname\" }}]\n\n[keys]\n",
                entry(&command)
            )
        );
    }

    #[test]
    fn gives_ui_a_tab_bar_and_leaves_one_elsewhere_alone() {
        let command = format!("node {RUN} status");
        let result = with_status_entry("[ui]\naccent = \"cyan\"\n\n[other]\ntab_bar_right = []\n", &command);
        assert!(result.starts_with(&format!(
            "[ui]\ntab_bar_right = [{}, {{ type = \"zoom\" }}]\naccent = \"cyan\"",
            entry(&command)
        )));
        assert!(result.ends_with("[other]\ntab_bar_right = []\n"));
    }

    #[test]
    fn adds_ui_to_a_config_without_one() {
        let command = format!("node {RUN} status");
        assert_eq!(
            with_status_entry("[keys]", &command),
            format!(
                "[keys]\n\n[ui]\ntab_bar_right = [{}, {{ type = \"zoom\" }}]",
                entry(&command)
            )
        );
    }

    #[test]
    fn leaves_a_config_that_runs_the_status_already() {
        let command = format!("node {RUN} status");
        let text = format!("[ui]\ntab_bar_right = [{{ type = \"command\", command = \"{command}\" }}]");
        assert_eq!(with_status_entry(&text, &command), text);
    }

    #[test]
    fn the_old_status_becomes_the_launchers_once() {
        let command = "sh /repo/packages/breadboard/herdr/breadboard.sh --no-wait herdr status";
        let text = format!(
            "[ui]\ntab_bar_right = [{}, {{ type = \"zoom\" }}]",
            entry(&format!("node {RUN} status"))
        );
        let now = with_status_entry(&text, command);
        assert_eq!(
            now,
            format!("[ui]\ntab_bar_right = [{}, {{ type = \"zoom\" }}]", entry(command))
        );
        assert_eq!(with_status_entry(&now, command), now);
        // A moved package's old path too (#230).
        let moved = "[ui]\ntab_bar_right = [{ type = \"command\", command = \"node /gone/packages/workflow/herdr/run.mjs status\" }]";
        assert_eq!(with_status_entry(moved, command).matches(command).count(), 1);
    }

    #[test]
    fn finds_every_form_of_the_status_and_nothing_else() {
        let command = "sh /r/packages/breadboard/herdr/breadboard.sh --no-wait herdr status";
        for old in [
            "command = \"node /a b/run.mjs status\"",
            "command='node /x/herdr/run.mjs status'",
            "command = \"/y/target/release/breadboard herdr status\"",
            "command = \"breadboard herdr status\"",
            "command = \"sh /moved/packages/breadboard/herdr/breadboard.sh --no-wait herdr status\"",
            "command = \"sh /moved/herdr/breadboard.sh herdr status\"",
        ] {
            let text = format!("[ui]\ntab_bar_right = [{{ type = \"command\", {old} }}]\n");
            let next = with_current_status(&text, command);
            if old.contains("/a b/") {
                // A path with a space isn't a command herdr could run either.
                assert_eq!(next, text);
                continue;
            }
            assert!(has_status(&text), "{old}");
            assert_eq!(next.matches(command).count(), 1, "{old}");
            assert_eq!(with_current_status(&next, command), next);
        }
        for other in [
            "command = \"date\"",
            "command = \"node run.mjs status\"",
            "# node /x/run.mjs status",
            "command = \"/y/breadboard herdr statusbar\"",
            "command = \"sh /y/breadboard.sh --no-wait herdr restore\"",
        ] {
            assert!(!has_status(other), "{other}");
        }
    }

    #[test]
    fn installs_only_a_download_with_the_pinned_sha256() {
        let root = std::env::temp_dir().join(format!("herdr-install-{}", std::process::id()));
        let bin = root.join(".herdr/bin/herdr");
        let data = b"#!/bin/sh\necho herdr\n";
        // Not the release: refused, nothing written, nothing half written.
        let refused = install_checked(&bin, data, "herdr-linux-x86_64", &"0".repeat(64)).unwrap_err();
        assert!(
            refused.0.starts_with(&format!(
                "herdr-linux-x86_64 has sha256 {}, pinned is 000",
                sha256_hex(data)
            )),
            "{}",
            refused.0
        );
        assert!(!bin.exists() && !bin.with_extension("download").exists());
        // The release: executable where the session runs it.
        install_checked(&bin, data, "herdr-linux-x86_64", &sha256_hex(data)).unwrap();
        assert_eq!(fs::read(&bin).unwrap(), data);
        assert_eq!(fs::metadata(&bin).unwrap().permissions().mode() & 0o111, 0o111);
        assert!(!bin.with_extension("download").exists());
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_window_of_the_terminal_it_runs_in() {
        let command = vec!["/b/breadboard".to_string(), "herdr".into(), "--here".into()];
        let main = Path::new("/r/platform");
        let only = |names: &'static [&'static str]| move |name: &str| names.contains(&name);
        if std::env::consts::OS != "macos" {
            assert_eq!(terminal_window_of(main, &command, &only(&[]), None), None);
        }
        assert_eq!(terminal_window_of(main, &command, &only(&["DISPLAY"]), None), None);
        let (program, args) = terminal_window_of(
            main,
            &command,
            &only(&["DISPLAY", "KITTY_PID", "ALACRITTY_SOCKET"]),
            None,
        )
        .unwrap();
        assert_eq!(program, "kitty");
        assert_eq!(
            args,
            [
                "-o",
                "window_padding_width=0",
                "--directory",
                "/r/platform",
                "/b/breadboard",
                "herdr",
                "--here"
            ]
        );
        let (program, args) = terminal_window_of(main, &command, &only(&["WAYLAND_DISPLAY"]), Some("ghostty")).unwrap();
        assert_eq!(program, "ghostty");
        assert_eq!(
            &args[..4],
            [
                "--window-padding-x=0",
                "--window-padding-y=0",
                "--working-directory=/r/platform",
                "-e"
            ]
        );
    }

    #[test]
    fn writes_the_config_from_the_repos_base() {
        let root = std::env::temp_dir().join(format!("herdr-setup-{}", std::process::id()));
        let plugin = root.join("platform/packages/breadboard/herdr");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("config.toml"),
            "# {{RUN}} becomes it\n[ui]\ntab_bar_right = [\n  { type = \"command\", command = \"node {{RUN}} status\", interval_seconds = 10, timeout_seconds = 3 },\n  { type = \"zoom\" },\n]\n",
        )
        .unwrap();
        fs::write(plugin.join("keys.toml"), "[[keys.command]]\nkey = \"prefix+o\"\n\n").unwrap();
        let session = Session::at(root.join("platform"), plugin.clone());
        // Only the repo's base: no own config in the way, whoever runs the test.
        let own = root.join("no-own-config.toml");
        let written = write_config_over(&session, &own).unwrap();
        assert!(written.is_changed && !written.has_own_rows);
        let text = fs::read_to_string(&session.config_file).unwrap();
        let status = session.status_command();
        assert!(text.starts_with(&format!(
            "# Written by pnpm herdr from {} and packages/breadboard/herdr/keys.toml. Changes here are\n# overwritten: put your own into .herdr/config.local.toml.\n\n# {}/run.mjs becomes it\n",
            plugin.join("config.toml").display(),
            plugin.display()
        )));
        assert_eq!(text.matches(&status).count(), 1);
        assert!(text.contains("\n\n[[keys.command]]\nkey = \"prefix+o\"\n\n# The spaces of an orchestration"));
        // The agents' rows end the file.
        assert!(text.ends_with("\"agent\"]]\n"), "{text}");
        assert!(!write_config_over(&session, &own).unwrap().is_changed);
        fs::remove_dir_all(&root).unwrap();
    }
}
