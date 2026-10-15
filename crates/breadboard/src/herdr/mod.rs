//! herdr for this repo, and the hooks that call breadboard (herdr/run.mjs,
//! src/herdr.mjs, src/session/*.mjs, .claude/hooks/stop-answers.mjs):
//!
//! ```text
//! breadboard herdr                     pnpm herdr: the session `platform`, in a window of its own
//! breadboard herdr --here|--no-attach  … attached here, or not at all
//! breadboard herdr --reload|--stop     new workflow code into a running herdr; stop the session
//! breadboard herdr:install             pnpm herdr:install: the plugin in your own herdr
//! breadboard herdr restore [--startup] the plugin's actions (herdr/herdr-plugin.toml):
//! breadboard herdr open|pane <pane>      herdr runs them in the plugin's directory
//! breadboard herdr status                the tab bar's line
//! breadboard herdr heal                  the overview again in its tabs that hold a bare shell
//! breadboard hook stop-answers         Claude Code's Stop hook: answers of the overview
//! ```
//!
//! What starts it on its own (the manifest, the tab bar, the hooks' shims)
//! goes through herdr/breadboard.sh (`launcher`): built first when missing
//! or older than the checkout's HEAD.
//!
//! One word for both halves: a plugin action is a word (`restore`, `open`,
//! `pane`, `status`), the session takes flags only (rewrite-log).

pub mod hook;
mod launcher;
mod own;
mod plugin;
mod reload;
mod session;
mod setup;
mod sidebar;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub use breadboard_core::record::{PLUGIN, SPACE};

const USAGE: &str = "usage: breadboard herdr [--here | --no-attach | --reload | --stop]   (pnpm herdr)
       breadboard herdr:install                              (pnpm herdr:install)
       breadboard herdr restore [--startup] | open <pane> | pane <pane> | status | heal
                                                             (the plugin's actions, run by herdr)
  panes: overview, commands, reload, stack-up, stack-down, stack-ps";

/// A failure: said on its own line, exit 1 (proc.mjs `fail`).
#[derive(Debug)]
pub struct Fail(pub String);

impl From<String> for Fail {
    fn from(message: String) -> Self {
        Fail(message)
    }
}

impl From<&str> for Fail {
    fn from(message: &str) -> Self {
        Fail(message.to_string())
    }
}

pub type Done = Result<ExitCode, Fail>;

/// Whether `stream` gets colours: a terminal, and no NO_COLOR.
fn is_coloured(is_terminal: bool) -> bool {
    is_terminal && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}

fn painted(code: &str, text: &str, is_on: bool) -> String {
    if is_on {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// `▸ <message>`: what the session does next; no blank line between them
/// (session.mjs `step`, tighter than the CLI's).
pub fn step(message: &str) {
    println!(
        "{} {}",
        painted("36", "▸", is_coloured(std::io::stdout().is_terminal())),
        breadboard_core::config::in_repo(message)
    );
}

/// `✓ <message>` after a blank line, and one after it.
pub fn done(message: &str) {
    let is_on = is_coloured(std::io::stdout().is_terminal());
    println!("\n{} {}\n", painted("1;32", "✓", is_on), message);
}

/// The exit code of an outcome, a failure said on stderr as `✗ …`.
pub fn finish(done: Done) -> ExitCode {
    let _ = std::io::stdout().flush();
    match done {
        Ok(code) => code,
        Err(Fail(reason)) => {
            let is_on = is_coloured(std::io::stderr().is_terminal());
            eprintln!(
                "\n{}",
                painted(
                    "1;31",
                    &format!("✗ {}", breadboard_core::config::in_repo(&reason)),
                    is_on
                )
            );
            ExitCode::FAILURE
        }
    }
}

/// This binary as it was started: `argv[0]` against the cwd it started in
/// (herdr/breadboard.sh execs it by its path), not the file
/// `/proc/self/exe` names. A rebuild replaces that file while a pane runs,
/// and the kernel then names the old one `… (deleted)`: every command the
/// pane started after it failed with ENOENT. A bare name (found on PATH)
/// takes current_exe, without that suffix.
pub fn started_binary() -> PathBuf {
    started_from(
        std::env::args_os().next().map(PathBuf::from),
        std::env::current_dir().ok().as_deref(),
        std::env::current_exe().ok(),
    )
}

/// `config` with every tab bar status of the plugin run as `herdr:install`
/// writes it for the plugin in `plugin_dir` of `main`, the rest byte for
/// byte: the lifecycle commands' #230 heal writes the same when it rewrites
/// a gone path (one way to say the status, rewrite rule 4).
pub fn with_status_of(config: &str, main: &Path, plugin_dir: &Path) -> String {
    let session = session::Session::at(main.to_path_buf(), plugin_dir.to_path_buf());
    setup::with_current_status(config, &session.status_command())
}

fn started_from(arg0: Option<PathBuf>, cwd: Option<&Path>, exe: Option<PathBuf>) -> PathBuf {
    if let Some(arg0) = arg0.filter(|path| path.components().count() > 1) {
        match cwd {
            _ if arg0.is_absolute() => return arg0,
            Some(cwd) => return cwd.join(arg0),
            None => {}
        }
    }
    let exe = exe.unwrap_or_else(|| PathBuf::from("breadboard"));
    match exe.to_str().and_then(|path| path.strip_suffix(" (deleted)")) {
        Some(path) => PathBuf::from(path),
        None => exe,
    }
}

fn usage_error(reason: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "\n✗ {reason}\n\n{USAGE}");
    ExitCode::FAILURE
}

/// `breadboard herdr <args>`.
pub fn main(args: &[String]) -> ExitCode {
    let first = args.first().map(String::as_str);
    match first {
        Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("restore" | "open" | "pane" | "status" | "heal") => plugin::main(args),
        Some(word) if !word.starts_with('-') => usage_error(&format!("herdr has no action {word}")),
        _ => session_flags(args),
    }
}

/// `breadboard herdr:install`: `pnpm herdr --install`.
pub fn install(args: &[String]) -> ExitCode {
    if let Some(flag) = args.first() {
        return if matches!(flag.as_str(), "--help" | "-h") {
            println!("{USAGE}");
            ExitCode::SUCCESS
        } else {
            usage_error(&format!("herdr:install takes no {flag}"))
        };
    }
    finish(session::Session::find().and_then(|session| own::install_into_own(&session).map(|()| ExitCode::SUCCESS)))
}

/// The flags of `pnpm herdr`, in Node's order: --install, --reload, --stop,
/// else the session. A flag it doesn't know is refused (Node ignored it:
/// `--relaod` started a whole session instead of reloading).
fn session_flags(args: &[String]) -> ExitCode {
    const FLAGS: [&str; 6] = ["--here", "--no-attach", "--reload", "--pulled", "--stop", "--install"];
    if let Some(unknown) = args.iter().find(|arg| !FLAGS.contains(&arg.as_str())) {
        return usage_error(&format!("herdr takes no {unknown}"));
    }
    let has_plugin_env = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if is_plugin_without_action(args, &has_plugin_env) {
        return usage_error(
            "herdr ran this as a plugin command without an action: herdr-plugin.toml names one for each (restore, open, pane, status); a session is set up from a terminal (pnpm herdr)",
        );
    }
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    finish(session::Session::find().and_then(|session| {
        if has("--install") {
            own::install_into_own(&session)?;
            return Ok(ExitCode::SUCCESS);
        }
        if has("--reload") {
            return reload::command(&session, has("--pulled"));
        }
        if has("--stop") {
            return setup::stop(&session);
        }
        setup::start(&session, has("--here"), has("--no-attach"))
    }))
}

/// Whether bare `breadboard herdr` (the whole `pnpm herdr`) was started by
/// herdr as a plugin command: its manifest entry lost the action word. herdr
/// gives a plugin's commands its id, state directory and context; a session
/// set up from there would open a window or attach in no terminal.
fn is_plugin_without_action(args: &[String], has_env: &dyn Fn(&str) -> bool) -> bool {
    args.is_empty()
        && ["HERDR_PLUGIN_ID", "HERDR_PLUGIN_STATE_DIR", "HERDR_PLUGIN_CONTEXT_JSON"]
            .into_iter()
            .any(has_env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_command_without_an_action_sets_no_session_up() {
        let plugin = |name: &str| name == "HERDR_PLUGIN_STATE_DIR";
        let terminal = |_: &str| false;
        assert!(is_plugin_without_action(&[], &plugin));
        assert!(!is_plugin_without_action(&[], &terminal));
        // The reload pane runs `herdr --reload` from a plugin command: a flag, deliberate.
        assert!(!is_plugin_without_action(&["--reload".into()], &plugin));
    }

    #[test]
    fn the_binary_is_the_path_it_was_started_by() {
        let exe = Some(PathBuf::from("/r/target/release/breadboard (deleted)"));
        let plugin = Path::new("/r/herdr");
        assert_eq!(
            started_from(Some("../target/release/breadboard".into()), Some(plugin), exe.clone()),
            PathBuf::from("/r/herdr/../target/release/breadboard")
        );
        assert_eq!(
            started_from(Some("/x/breadboard".into()), None, exe.clone()),
            PathBuf::from("/x/breadboard")
        );
        // Found on PATH: the file it is, a rebuild's suffix dropped.
        assert_eq!(
            started_from(Some("breadboard".into()), Some(plugin), exe),
            PathBuf::from("/r/target/release/breadboard")
        );
        assert_eq!(started_from(None, None, None), PathBuf::from("breadboard"));
    }
}
