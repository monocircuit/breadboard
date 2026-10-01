//! Built by the CLI builders (#314).

use std::process::ExitCode;

/// The command's exit code, or None when it isn't one of this group's.
pub fn run(_command: &str, _args: &[String]) -> Option<ExitCode> {
    None
}
