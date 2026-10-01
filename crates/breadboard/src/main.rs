//! `breadboard <command> [args…]`: the commands keep the names of the root
//! package.json (`overview`, `wt:<cmd>`, `stack:<cmd>`, …).

mod cli;
mod tui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        eprintln!("usage: breadboard <command> [args…]");
        return ExitCode::from(2);
    };
    match command.as_str() {
        "overview" => tui::main(rest),
        _ => cli::main(command, rest),
    }
}
