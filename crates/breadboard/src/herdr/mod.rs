//! herdr for this repo, and the hooks that call breadboard (herdr/run.mjs,
//! src/herdr.mjs, src/session/*.mjs, .claude/hooks/stop-answers.mjs):
//!
//! ```text
//! breadboard herdr                     pnpm herdr: the session `platform`, in a window of its own
//! breadboard herdr --here|--no-attach  … attached here, or not at all
//! breadboard herdr --reload|--stop     new workflow code into a running herdr; stop the session
//! breadboard herdr:install             pnpm herdr:install: the plugin in your own herdr
//! breadboard herdr restore [--startup] the plugin's actions (herdr/herdr-plugin.rs.toml):
//! breadboard herdr open|pane <pane>      herdr runs them in the plugin's directory
//! breadboard herdr status                the tab bar's line
//! breadboard hook stop-answers         Claude Code's Stop hook: answers of the overview
//! ```
//!
//! One word for both halves: a plugin action is a word (`restore`, `open`,
//! `pane`, `status`), the session takes flags only (rewrite-log).

pub mod hook;
mod own;
mod plugin;
mod reload;
mod session;
mod setup;
mod sidebar;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

/// The plugin's id: the manifest's, every sidebar token's source, the block
/// marker in configs (rule 5: never changes).
pub const PLUGIN: &str = "monocircuit.platform";
/// The herdr space of the repo's main checkout (constants.mjs `SPACE`).
pub const SPACE: &str = "Monocircuit/Platform";

const USAGE: &str = "usage: breadboard herdr [--here | --no-attach | --reload | --stop]   (pnpm herdr)
       breadboard herdr:install                              (pnpm herdr:install)
       breadboard herdr restore [--startup] | open <pane> | pane <pane> | status
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
        "{} {message}",
        painted("36", "▸", is_coloured(std::io::stdout().is_terminal()))
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
            eprintln!("\n{}", painted("1;31", &format!("✗ {reason}"), is_on));
            ExitCode::FAILURE
        }
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
        Some("restore" | "open" | "pane" | "status") => plugin::main(args),
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
