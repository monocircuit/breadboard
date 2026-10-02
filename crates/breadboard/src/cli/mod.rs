//! `breadboard wt:<cmd>` / `stack:<cmd>` and the other commands of the root package.json.

pub mod layout;
mod remove;

use std::process::ExitCode;

pub fn main(command: &str, args: &[String]) -> ExitCode {
    match command {
        "wt:remove" => remove::main(args),
        "wt:herdr" => layout::main(args),
        _ => {
            eprintln!("{command}: not built yet");
            ExitCode::from(2)
        }
    }
}
