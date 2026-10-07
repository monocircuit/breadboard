//! `breadboard <command> [args…]`: the commands keep the names of the root
//! package.json (`overview`, `wt:<cmd>`, `stack:<cmd>`, …).

mod cli;
mod daemon;
mod herdr;
mod tui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        return cli::bare();
    };
    match command.as_str() {
        "overview" => cli::overview_refused(rest).unwrap_or_else(|| tui::main(rest)),
        // The overviews' one reader, started by the first of them (daemon/).
        "serve" => daemon::main(rest),
        // pnpm herdr, herdr:install and the herdr plugin's actions (herdr/herdr-plugin.rs.toml).
        "herdr" => herdr::main(rest),
        "herdr:install" => herdr::install(rest),
        // Claude Code's Stop hook; `hook pre-push` is the setup group's.
        "hook" if rest.first().map(String::as_str) == Some("stop-answers") => herdr::hook::stop_answers(&rest[1..]),
        _ => cli::main(command, rest),
    }
}
