//! `breadboard <command> [args…]`: the commands keep the names of the root
//! package.json (`overview`, `wt:<cmd>`, `stack:<cmd>`, …).

mod cli;
mod daemon;
mod herdr;
mod tui;

use std::process::ExitCode;

/// The key of the sources this binary was built from (#339), as the launcher
/// passed it to the build (build.rs): empty for a build of anything else,
/// and when a build script's output from before carries none. The marker
/// `breadboard-source-key:<key>` sits in the file as one string; the launcher
/// finds it (grep) before it stores or uses a copy, without running the copy.
const SOURCE_KEY: &str = match option_env!("BREADBOARD_SOURCE_KEY_MARK") {
    Some(mark) => mark,
    None => "breadboard-source-key:",
};

/// The key of the sources this binary was built from, empty when it has none.
pub(crate) fn source_key() -> &'static str {
    SOURCE_KEY.trim_start_matches("breadboard-source-key:")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        return cli::bare();
    };
    match command.as_str() {
        "--source-key" => {
            println!("{}", source_key());
            ExitCode::SUCCESS
        }
        "overview" => cli::overview_refused(rest).unwrap_or_else(|| tui::main(rest)),
        // The overviews' one reader, started by the first of them (daemon/).
        "serve" => daemon::main(rest),
        // pnpm herdr, herdr:install and the herdr plugin's actions (herdr/herdr-plugin.toml).
        "herdr" => herdr::main(rest),
        "herdr:install" => herdr::install(rest),
        // Claude Code's Stop hook; `hook pre-push` is the setup group's.
        "hook" if rest.first().map(String::as_str) == Some("stop-answers") => herdr::hook::stop_answers(&rest[1..]),
        _ => cli::main(command, rest),
    }
}
