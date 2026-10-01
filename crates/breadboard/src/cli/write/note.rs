//! `pnpm wt:note <issue> --message-file <file>` (commands/note.mjs): a
//! comment on an issue from a file, read back from the API, where a comment
//! that holds a path instead of the text shows.

use super::common::{Argv, Here, Outcome, Takes, help, number_of, say};
use super::glab;
use serde_json::Value;
use std::process::ExitCode;

const USAGE: &str = "usage: pnpm wt:note <issue> --message-file <file>";
const ABOUT: &str = "pnpm wt:note comments the text of a file on an issue and reads it back from GitLab.";
const TAKES: Takes = Takes {
    valued: &["--message-file"],
    switches: &[],
    words: Some(1),
    usage: USAGE,
};

pub fn run(argv: &Argv) -> Outcome {
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let here = Here::find();
    let number = argv.first().and_then(number_of);
    let (Some(number), Some(file)) = (number, argv.given("--message-file")) else {
        return Err(USAGE.into());
    };
    let path = here.resolve(file);
    if !path.exists() {
        return Err(format!(
            "{} does not exist — write the comment there first",
            here.shown(file)
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("{} could not be read: {error}", here.shown(file)))?;
    let message = String::from_utf8_lossy(&bytes).trim().to_string();
    if message.is_empty() {
        return Err(format!("{} is empty", here.shown(file)));
    }

    // glab finds the project through the checkout's origin.
    let Some(top) = here.top.as_deref() else {
        return Err("not in a git checkout: run it in a worktree".into());
    };
    let posted = glab::glab(top, &["issue", "note", number, "--message", &message]);
    if !posted.is_ok {
        return Err(format!("glab could not comment on #{number}: {}", posted.why()));
    }
    let url = posted
        .stdout
        .trim()
        .split('\n')
        .find(|line| line.starts_with("http"))
        // No URL printed: no line of two spaces (Node printed one).
        .map(|url| format!("\n  {url}"))
        .unwrap_or_default();

    // Read it back, as stored: `glab issue view --comments` renders the
    // markdown and leaves the newest comments of a long issue out. An answer
    // that is no JSON is a comment not seen (Node crashed on it).
    let back = glab::api(
        top,
        &format!("projects/:fullpath/issues/{number}/notes?sort=desc&order_by=created_at&per_page=10"),
    );
    let is_there = back
        .json()
        .and_then(|notes| notes.as_array().cloned())
        .is_some_and(|notes| {
            notes.iter().any(|note| {
                note.get("body")
                    .and_then(Value::as_str)
                    .is_some_and(|body| body.trim() == message)
            })
        });
    say(&format!(
        "\n{} comment on #{number}{}{url}\n",
        if is_there { "✓" } else { "⚠" },
        if is_there {
            ""
        } else {
            " — GitLab does not show it back, check it"
        }
    ));
    Ok(ExitCode::SUCCESS)
}
