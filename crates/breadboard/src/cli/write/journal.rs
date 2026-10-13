//! `pnpm wt:journal "<line>"` (#382): the orchestrator's state outside its
//! chat, one line into the journal of its orchestration; without a line the
//! journal printed. A fresh orchestrator starts from it.

use super::common::{Argv, Here, Outcome, Takes, help, say};
use breadboard_core::journal;
use breadboard_core::record::{is_name, record_of, recorded};
use breadboard_core::time::now_ms;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: pnpm wt:journal \"<line>\" [--orchestration <name>]   one line into the journal
       pnpm wt:journal [--orchestration <name>]            the journal";
const ABOUT: &str = "pnpm wt:journal keeps the orchestrator's state outside its chat: decisions taken for the
user, open threads, who waits for what. The overview shows its last lines; a fresh orchestrator
starts from it (.claude/orchestrations/<name>.journal.md).";
const TAKES: Takes = Takes {
    valued: &["--orchestration"],
    switches: &[],
    words: None,
    usage: USAGE,
    command: "wt:journal",
};

/// The orchestration meant: `--orchestration`, else the one whose checkout
/// the command runs in, else the only one recorded.
fn name_of(argv: &Argv, here: &Here, main: &Path) -> Result<String, String> {
    let run = breadboard_core::config::run_word();
    if let Some(name) = argv.given("--orchestration") {
        if !is_name(name) {
            return Err(format!(
                "{name} is no orchestration's name: names are kebab-case ({run} wt:orchestrate <name> <issue>…)"
            ));
        }
        if record_of(main, name).is_none() {
            return Err(format!("no orchestration {name} is recorded"));
        }
        return Ok(name.to_string());
    }
    let all = recorded(main);
    let checkouts = breadboard_core::git::orchestrations_dir(main);
    if let Some(name) = all
        .iter()
        .find(|name| here.cwd.starts_with(checkouts.join(name.as_str())))
    {
        return Ok(name.clone());
    }
    match all.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err("no orchestration is recorded".into()),
        _ => Err(format!(
            "which orchestration? --orchestration <name>: {}",
            all.join(", ")
        )),
    }
}

/// The words of the line: everything that is no flag and no flag's value.
fn line_of(argv: &Argv) -> String {
    let all = argv.all();
    let mut words = Vec::new();
    let mut at = 0;
    while at < all.len() {
        let arg = &all[at];
        at += 1;
        if arg == "--orchestration" {
            at += 1;
        } else if !arg.starts_with("--") {
            words.push(arg.as_str());
        }
    }
    words.join(" ")
}

pub fn run(argv: &Argv) -> Outcome {
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let here = Here::find();
    let Some(main) = here.main.clone() else {
        return Err("not in a git checkout of the repo".into());
    };
    let name = name_of(argv, &here, &main)?;
    let line = line_of(argv);
    if line.trim().is_empty() {
        let entries = journal::entries(&main, &name);
        if entries.is_empty() {
            say(&format!(
                "\nthe journal of {name} is empty: {} wt:journal \"<line>\"\n",
                breadboard_core::config::run_word()
            ));
        } else {
            let lines: Vec<String> = entries.iter().map(|entry| format!("  {entry}")).collect();
            say(&format!("\njournal of {name}\n{}\n", lines.join("\n")));
        }
        return Ok(ExitCode::SUCCESS);
    }
    let entry = journal::append(&main, &name, &line, now_ms()).map_err(|error| {
        format!(
            "couldn't write {}: {error}",
            journal::journal_file(&main, &name).display()
        )
    })?;
    say(&format!(
        "\n✓ into the journal of {name}: {}\n",
        entry.trim_start_matches("- ")
    ));
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_is_every_word_but_the_flags() {
        let args: Vec<String> = ["decided:", "#3", "--orchestration", "o", "waits"]
            .iter()
            .map(|arg| arg.to_string())
            .collect();
        assert_eq!(line_of(&Argv(&args)), "decided: #3 waits");
    }
}
