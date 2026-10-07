//! The herdr session `platform` of `pnpm herdr` (session.mjs): the pinned
//! version, where its binary and config live, and its CLI; and your own
//! herdr next to it (`pnpm herdr:install`).

use super::{Fail, PLUGIN};
use breadboard_core::git;
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The herdr the plugin is tested with. A bump is a commit: the version and
/// the sha256 of each asset of the release
/// (`gh api repos/herdrdev/herdr/releases/tags/v<version>`).
pub const VERSION: &str = "0.9.1";
/// `<os>-<arch>` → the release's asset and its sha256.
pub const ASSETS: [(&str, &str, &str); 4] = [
    (
        "linux-x86_64",
        "herdr-linux-x86_64",
        "2a02fed16beb651ef006e1d43f048f652ca4dc58ad053cd2d44450563d5c54b7",
    ),
    (
        "linux-aarch64",
        "herdr-linux-aarch64",
        "f4ccf4de745f2cb9a39a983e9ba3703dad50ec2a58dea83026ceab721bbd8d9e",
    ),
    (
        "macos-aarch64",
        "herdr-macos-aarch64",
        "5fc7a7e7adfaca56fa80aa89dcb025693357268dab8285b9ce2d08a2313c89de",
    ),
    (
        "macos-x86_64",
        "herdr-macos-x86_64",
        "053be0639935fe54ab5efbdb46651054e4f6a753a5b43153c88bd6912bce1e94",
    ),
];
pub const SESSION: &str = "platform";
const MANIFEST: &str = "herdr-plugin.toml";

/// The block `pnpm herdr:install` keeps in your config.toml; a line starting
/// with the mark counts, whatever follows it.
pub fn block_mark() -> String {
    format!("# >>> {PLUGIN}")
}
pub fn block_begin() -> String {
    format!(
        "{}: written by pnpm herdr:install on every run, changes here are overwritten",
        block_mark()
    )
}
pub fn block_end() -> String {
    format!("# <<< {PLUGIN}")
}

/// Where the block is in `text`: from its mark to the end of its end line's marker.
pub fn block_range(text: &str) -> Option<(usize, usize)> {
    let start = text.find(&block_mark())?;
    let end = block_end();
    let stop = start + text[start..].find(&end)? + end.len();
    Some((start, stop))
}

/// A config text without the marked block: what is yours in it.
pub fn without_block(text: &str) -> String {
    match block_range(text) {
        Some((start, stop)) => format!("{}{}", &text[..start], &text[stop..]),
        None => text.to_string(),
    }
}

/// Which herdr a call goes to: the pinned one of the session `platform`, or
/// your own (its default session).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Pinned,
    Own(PathBuf),
}

/// What a herdr call answered: exit 0, and stdout + stderr trimmed.
pub struct Answer {
    pub is_ok: bool,
    pub out: String,
}

/// The paths of the session, from the checkout this runs in.
pub struct Session {
    pub main: PathBuf,
    /// `<main>/.herdr`
    pub home: PathBuf,
    /// `<main>/.herdr/bin/herdr`
    pub bin: PathBuf,
    /// `<main>/.herdr/config.toml`
    pub config_file: PathBuf,
    /// The plugin of the main checkout once merged, else the one of the
    /// checkout this runs in (never the one beside the binary: a binary of
    /// one checkout run in another acts on the one it runs in).
    pub plugin_dir: PathBuf,
}

impl Session {
    pub fn find() -> Result<Session, Fail> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let init = std::env::var_os("INIT_CWD").map(PathBuf::from);
        let main = git::main_dir(&cwd)
            .or_else(|| init.as_deref().and_then(git::main_dir))
            .ok_or("not inside the repo: run it in a checkout of monocircuit/platform")?;
        let top = git::git(&cwd, &["rev-parse", "--show-toplevel"]);
        let here = top.is_ok.then(|| PathBuf::from(top.out()));
        let plugin_dir = [Some(main.as_path()), here.as_deref()]
            .into_iter()
            .flatten()
            .map(|checkout| checkout.join("packages/breadboard/herdr"))
            .find(|dir| dir.join(MANIFEST).is_file())
            .ok_or("packages/breadboard/herdr is missing in this checkout")?;
        Ok(Session::at(main, plugin_dir))
    }

    pub fn at(main: PathBuf, plugin_dir: PathBuf) -> Session {
        let home = main.join(".herdr");
        Session {
            bin: home.join("bin").join("herdr"),
            config_file: home.join("config.toml"),
            home,
            main,
            plugin_dir,
        }
    }

    /// The breadboard binary of the plugin's checkout, what the plugin's
    /// manifest (`../target/release/breadboard` from the plugin's directory)
    /// and the tab bar's status run.
    pub fn breadboard(&self) -> PathBuf {
        self.plugin_dir
            .parent()
            .unwrap_or(&self.plugin_dir)
            .join("target/release/breadboard")
    }

    /// The tab bar's status command, as the configs carry it.
    pub fn status_command(&self) -> String {
        format!("{} herdr status", self.breadboard().display())
    }

    /// The environment of the session's server, and so of every pane in it:
    /// the pinned herdr first on PATH, the repo's config, nothing of a herdr
    /// or an agent this was started from.
    pub fn session_env(&self) -> Vec<(OsString, OsString)> {
        let mut env: Vec<(OsString, OsString)> = clean_env().into_iter().filter(|(key, _)| key != "PATH").collect();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![self.bin.parent().map(Path::to_path_buf).unwrap_or_default()];
        dirs.extend(std::env::split_paths(&path));
        env.push(("PATH".into(), std::env::join_paths(dirs).unwrap_or(path)));
        env.push(("HERDR_CONFIG_PATH".into(), self.config_file.clone().into_os_string()));
        env
    }

    pub fn binary_of(&self, target: &Target) -> PathBuf {
        match target {
            Target::Pinned => self.bin.clone(),
            Target::Own(path) => path.clone(),
        }
    }

    fn command(&self, target: &Target, args: &[&str]) -> Command {
        let mut command = Command::new(self.binary_of(target));
        command.env_clear();
        match target {
            Target::Pinned => {
                command.args(["--session", SESSION]).envs(self.session_env());
            }
            // Your own herdr talks to your default session, also from a pane of `platform`.
            Target::Own(_) => {
                command.envs(clean_env());
            }
        }
        command.args(args).stdin(Stdio::null());
        command
    }

    /// `herdr <args>` of the target, its output captured.
    pub fn herdr(&self, target: &Target, args: &[&str]) -> Answer {
        match self.command(target, args).output() {
            Ok(output) => Answer {
                is_ok: output.status.success(),
                out: format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
                .trim()
                .to_string(),
            },
            Err(error) => Answer {
                is_ok: false,
                out: error.to_string(),
            },
        }
    }

    /// `herdr <args>` of the target, its output on this terminal.
    pub fn herdr_shown(&self, target: &Target, args: &[&str]) -> bool {
        self.command(target, args).status().is_ok_and(|status| status.success())
    }

    /// `herdr <args>` → its `result`, None when it failed or said no JSON.
    pub fn ask(&self, target: &Target, args: &[&str]) -> Option<Value> {
        let answer = self.herdr(target, args);
        if !answer.is_ok {
            return None;
        }
        serde_json::from_str::<Value>(&answer.out).ok()?.get("result").cloned()
    }

    pub fn is_running(&self, target: &Target) -> bool {
        self.herdr(target, &["status", "server"])
            .out
            .contains("status: running")
    }

    pub fn server_version(&self, target: &Target) -> Option<String> {
        version_after(&self.herdr(target, &["status", "server"]).out)
    }

    /// Whether the target has a workspace labelled exactly `Monocircuit/Platform`.
    pub fn has_space(&self, target: &Target) -> bool {
        self.ask(target, &["workspace", "list"])
            .is_some_and(|result| has_space_in(&result))
    }
}

/// `version: <v>` of `herdr status server`.
pub fn version_after(text: &str) -> Option<String> {
    let at = text.find("version: ")? + "version: ".len();
    let version: String = text[at..].chars().take_while(|c| !c.is_whitespace()).collect();
    (!version.is_empty()).then_some(version)
}

/// Whether a `workspace list` result has the space of the repo.
pub fn has_space_in(result: &Value) -> bool {
    result
        .get("workspaces")
        .and_then(Value::as_array)
        .is_some_and(|spaces| {
            spaces
                .iter()
                .any(|space| space.get("label").and_then(Value::as_str) == Some(super::SPACE))
        })
}

/// This environment without what a pane must not inherit: the variables of
/// a herdr this was started from, and those of a Claude Code session. A
/// `claude` started in a pane would otherwise take itself for a child
/// session and save no transcript.
pub fn clean_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy();
            !key.starts_with("HERDR_") && !key.starts_with("CLAUDE")
        })
        .collect()
}

/// This process runs in a pane of the session `platform`.
pub fn is_in_platform() -> bool {
    std::env::var("HERDR_SOCKET_PATH").is_ok_and(|socket| socket.contains(&format!("/sessions/{SESSION}/")))
}

/// Inside a herdr pane: `HERDR_ENV=1`.
pub fn is_in_herdr() -> bool {
    std::env::var("HERDR_ENV").as_deref() == Ok("1")
}

/// A path for a message: `~/…` under the home directory.
pub fn shown(path: &Path) -> String {
    match std::env::var_os("HOME").and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// `~/.config/herdr/config.toml`: your own herdr's config (Node: the home's,
/// not XDG_CONFIG_HOME's).
pub fn own_config() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/herdr/config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cuts_the_block_out_and_leaves_a_text_without_one() {
        let text = format!("a = 1\n{}\nkeys\n{}\nb = 2\n", block_begin(), block_end());
        assert_eq!(without_block(&text), "a = 1\n\nb = 2\n");
        let half = format!("a = 1\n{}\nkeys\n", block_begin());
        assert_eq!(without_block(&half), half);
        let any = format!("{} whatever\nx\n{}", block_mark(), block_end());
        assert_eq!(without_block(&any), "");
    }

    #[test]
    fn reads_the_servers_version_and_the_space() {
        assert_eq!(
            version_after("status: running\nversion: 0.9.1\npid: 4").as_deref(),
            Some("0.9.1")
        );
        assert_eq!(version_after("status: stopped"), None);
        assert!(has_space_in(
            &json!({ "workspaces": [{ "label": "Monocircuit/Platform" }] })
        ));
        assert!(!has_space_in(
            &json!({ "workspaces": [{ "label": "Monocircuit/Platform-1" }] })
        ));
    }

    #[test]
    fn the_session_lives_in_the_main_checkout() {
        let session = Session::at(
            PathBuf::from("/r/platform"),
            PathBuf::from("/r/platform/packages/breadboard/herdr"),
        );
        assert_eq!(session.bin, PathBuf::from("/r/platform/.herdr/bin/herdr"));
        assert_eq!(
            session.status_command(),
            "/r/platform/packages/breadboard/target/release/breadboard herdr status"
        );
        let env = session.session_env();
        assert!(
            env.iter()
                .any(|(key, value)| key == "PATH" && value.to_string_lossy().starts_with("/r/platform/.herdr/bin"))
        );
        assert!(env.iter().all(|(key, _)| !key.to_string_lossy().starts_with("CLAUDE")));
    }
}
