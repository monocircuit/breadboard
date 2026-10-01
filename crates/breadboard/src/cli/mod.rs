//! `breadboard wt:<cmd>` / `stack:<cmd>` and the other commands of the root package.json.
//! One module per group of commands; each owns its commands' parsing and output.

mod lifecycle;
mod read;
mod write;

use std::process::ExitCode;

pub fn main(command: &str, args: &[String]) -> ExitCode {
    if let Some(code) = read::run(command, args) {
        return code;
    }
    if let Some(code) = write::run(command, args) {
        return code;
    }
    if let Some(code) = lifecycle::run(command, args) {
        return code;
    }
    eprintln!("{command}: not built yet");
    ExitCode::from(2)
}
