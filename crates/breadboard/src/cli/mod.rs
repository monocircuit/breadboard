//! `breadboard wt:<cmd>` / `stack:<cmd>` and the other commands of the root package.json.

use std::process::ExitCode;

pub fn main(command: &str, _args: &[String]) -> ExitCode {
    eprintln!("{command}: not built yet");
    ExitCode::from(2)
}
