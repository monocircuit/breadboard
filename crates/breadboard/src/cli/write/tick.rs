//! `pnpm wt:tick [<issue>] [<box>…]` (commands/tick.mjs): the checkboxes of
//! an issue, numbered; given box numbers, those are ticked in the
//! description and read back. Without any, the issue of the branch is listed.

use super::boxes::{self, Box, boxes_of, tally};
use super::common::{Argv, Here, Outcome, Takes, help, number_of, say};
use super::glab;
use breadboard_core::git::issue_of_branch;
use serde_json::Value;
use std::process::ExitCode;

const USAGE: &str = "usage: pnpm wt:tick [<issue>] [<box>…]   (the issue first when you tick)";
const ABOUT: &str = "pnpm wt:tick lists the checkboxes of an issue's description, numbered; given box numbers,
it ticks those on GitLab and reads the description back. Without an issue: the one of the branch.";
const TAKES: Takes = Takes {
    valued: &[],
    switches: &[],
    words: None,
    usage: USAGE,
    command: "wt:tick",
};

pub fn run(argv: &Argv) -> Outcome {
    // Refused before GitLab is asked: Node dropped every `--x` and ticked anyway.
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let here = Here::find();
    let Some(top) = here.top.as_deref() else {
        return Err("not in a git checkout: run it in a worktree".into());
    };
    let words: Vec<&str> = argv
        .all()
        .iter()
        .map(String::as_str)
        .filter(|arg| !arg.starts_with("--"))
        .collect();
    let given = match words.first() {
        Some(first) => first.to_string(),
        None => match issue_of_branch(&here.branch()) {
            Some(number) => number.to_string(),
            None => {
                return Err(format!(
                    "{USAGE}\n  This branch carries no issue number, so name the issue."
                ));
            }
        },
    };
    // The word given is what is wrong, not the branch (as wt:mr says it).
    let Some(number) = number_of(&given) else {
        return Err(format!("{given} is no issue number (34 or #34)\n{USAGE}"));
    };
    let wanted = words.get(1..).unwrap_or_default();
    if let Some(box_) = wanted.iter().find(|box_| number_of(box_) != Some(**box_)) {
        return Err(format!(
            "{box_} is no box number: pnpm wt:tick {number} lists them, numbered"
        ));
    }

    // Read right before writing: someone may have changed the description since.
    let read = || -> Result<Value, String> {
        glab::view(top, "issue", number)?.ok_or_else(|| format!("GitLab has no issue #{number}"))
    };
    let issue = read()?;
    let title = glab::text_of(&issue, "title").to_string();
    let description = glab::text_of(&issue, "description").to_string();
    let boxes = boxes_of(&description);
    if boxes.is_empty() {
        return Err(format!("#{number} has no checkboxes in its description"));
    }

    if wanted.is_empty() {
        say(&format!("\n#{number} {title} — {} ticked", tally(&boxes)));
        for line in boxes::print_boxes(&boxes, &[]) {
            say(&line);
        }
        say(&format!("\n  pnpm wt:tick {number} <box>… ticks them\n"));
        return Ok(ExitCode::SUCCESS);
    }

    let index_of = |box_: &str| {
        box_.parse::<usize>()
            .ok()
            .filter(|&n| n >= 1 && n <= boxes.len())
            .map(|n| n - 1)
    };
    let outside: Vec<&str> = wanted.iter().copied().filter(|box_| index_of(box_).is_none()).collect();
    if !outside.is_empty() {
        return Err(format!(
            "#{number} has boxes 1 to {}, not {}",
            boxes.len(),
            outside.join(", ")
        ));
    }
    let mut fresh: Vec<usize> = Vec::new();
    for index in wanted.iter().filter_map(|box_| index_of(box_)) {
        if !boxes[index].is_ticked && !fresh.contains(&index) {
            fresh.push(index);
        }
    }
    if fresh.is_empty() {
        say(&format!(
            "\n✓ already ticked, nothing sent — #{number}: {}\n",
            tally(&boxes)
        ));
        return Ok(ExitCode::SUCCESS);
    }

    let ticked = ticked_text(&description, &boxes, &fresh);
    let put = glab::put_description(top, &format!("projects/:fullpath/issues/{number}"), &ticked);
    if put.is_not_found() {
        return Err(format!("GitLab has no issue #{number}"));
    }
    if !put.is_ok {
        return Err(format!(
            "glab could not update the description of #{number}: {}",
            put.why()
        ));
    }

    // Read it back: the same boxes, the chosen ones ticked. A body that became
    // a path has no boxes at all, so this catches that too.
    let back = boxes_of(glab::text_of(&read()?, "description"));
    let is_there = is_ticked_back(&back, boxes.len(), &fresh);
    say(&format!(
        "\n{} #{number} {title} — {} ticked{}",
        if is_there { "✓" } else { "⚠" },
        tally(&back),
        if is_there {
            ""
        } else {
            " — what GitLab shows back differs, look at it"
        }
    ));
    for line in boxes::print_boxes(&back, &fresh) {
        say(&line);
    }
    say("");
    Ok(ExitCode::SUCCESS)
}

/// Whether GitLab shows the ticks back: as many boxes as before, the chosen
/// ones ticked. Anything else is `⚠`, never `✓` (a body that became a path
/// has no boxes; someone else's edit in between may have moved them).
fn is_ticked_back(back: &[Box], count: usize, fresh: &[usize]) -> bool {
    back.len() == count && fresh.iter().all(|&index| back[index].is_ticked)
}

/// The description with the boxes `fresh` ticked: the first `[ ]` of each one's line.
fn ticked_text(description: &str, boxes: &[Box], fresh: &[usize]) -> String {
    let mut lines: Vec<String> = description.split('\n').map(String::from).collect();
    for &index in fresh {
        let line = &mut lines[boxes[index].line];
        *line = line.replacen("[ ]", "[x]", 1);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_only_the_first_empty_box_of_each_chosen_line() {
        let text = "## A\n- [ ] one [ ]\n- [ ] two\n- [ ] three";
        let boxes = boxes_of(text);
        assert_eq!(
            ticked_text(text, &boxes, &[0, 2]),
            "## A\n- [x] one [ ]\n- [ ] two\n- [x] three"
        );
    }

    #[test]
    fn says_ticked_only_when_gitlab_shows_the_ticks_back() {
        let ticked = boxes_of("- [x] one\n- [ ] two\n- [x] three");
        assert!(is_ticked_back(&ticked, 3, &[0, 2]));
        // One of the chosen not ticked in what came back.
        assert!(!is_ticked_back(&ticked, 3, &[1]));
        // A box more or fewer: the description changed under it.
        assert!(!is_ticked_back(&ticked, 4, &[0]));
        // A body that became a path has no boxes at all.
        assert!(!is_ticked_back(&boxes_of("/tmp/wt-glab-x/description.md"), 3, &[0]));
    }
}
