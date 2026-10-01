//! `wt:issues` (issues.mjs, prune.mjs `staleIssues`): the issues of merged
//! work nobody came back to, no comment since the merge. Catching one up is a
//! tick of what the MR did (`wt:tick`) and a comment with the MR, what is done
//! and what is open (`wt:note`). `wt:new` counts the same next to the stale
//! worktrees: `stale_issues` is the one function both use.

use super::args::{Takes, fail, main_dir, pad};
use super::gitlab;
use breadboard_core::git::issue_of_branch;
use breadboard_core::json::str_of;
use breadboard_core::time::parse_iso;
use serde_json::Value;
use std::cmp::Ordering;
use std::path::Path;
use std::process::ExitCode;

const CATCH_UP: &str = "  Catch one up: pnpm wt:tick <issue> (then the boxes the MR did), and a comment with the MR,\n  what is done and what is open: pnpm wt:note <issue> --message-file <f>";

pub const TAKES: Takes = Takes {
    valued: &[],
    lists: &[],
    optional: &[],
    switches: &[],
    words: 0,
    about: "The issues of merged work nobody came back to: still open or a box left open, no comment since the merge.",
    usage: "usage: pnpm wt:issues",
};

pub fn run() -> ExitCode {
    let main = match main_dir() {
        Ok(main) => main,
        Err(code) => return code,
    };
    match stale_issues(&main) {
        Ok(stale) => {
            print!("{}", text_of(&stale));
            ExitCode::SUCCESS
        }
        // The reason once: `GitLab: <reason> (<call>)` says GitLab already.
        Err(error) => fail(&format!(
            "GitLab could not be read: {}",
            error.strip_prefix("GitLab: ").unwrap_or(&error)
        )),
    }
}

/// An issue of merged work without a comment since the merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Stale {
    pub iid: u64,
    /// GitLab's word: `opened` / `closed`.
    pub state: String,
    pub title: String,
    /// Its boxes: (ticked, total).
    pub boxes: (usize, usize),
    /// The MR that merged last of those touching it, and when.
    pub mr: u64,
    pub merged_at: String,
}

/// The issues a merged MR touched (the number of its branch, the `Closes #n` /
/// `Refs #n` of its description) that are still open or have a box left open,
/// and no comment since the last of those merges. System notes ("mentioned
/// in …", "closed via …") don't count: GitLab writes them, nobody caught
/// anything up. The newest merge first.
pub fn stale_issues(main: &Path) -> Result<Vec<Stale>, String> {
    let mrs = gitlab::list(
        main,
        "projects/:fullpath/merge_requests?state=merged&order_by=merged_at&sort=desc&per_page=100",
    )?;
    // issue → (the MR that merged last of those touching it, when)
    let mut last_merge: Vec<(u64, u64, String)> = Vec::new();
    for mr in &mrs {
        let (Some(iid), Some(merged_at)) = (mr.get("iid").and_then(Value::as_u64), str_of(mr, "merged_at")) else {
            continue;
        };
        let mut numbers: Vec<u64> = str_of(mr, "source_branch")
            .and_then(issue_of_branch)
            .into_iter()
            .collect();
        numbers.extend(references(str_of(mr, "description").unwrap_or("")));
        for number in numbers {
            match last_merge.iter_mut().find(|(known, _, _)| *known == number) {
                Some(known) if is_later(merged_at, &known.2) => *known = (number, iid, merged_at.to_string()),
                Some(_) => {}
                None => last_merge.push((number, iid, merged_at.to_string())),
            }
        }
    }
    if last_merge.is_empty() {
        return Ok(Vec::new());
    }

    let endpoints: Vec<String> = last_merge
        .chunks(50)
        .map(|chunk| {
            let iids: Vec<String> = chunk.iter().map(|(number, _, _)| format!("iids[]={number}")).collect();
            format!("projects/:fullpath/issues?{}&per_page=50", iids.join("&"))
        })
        .collect();
    let mut issues = Vec::new();
    for found in gitlab::lists(main, &endpoints) {
        issues.extend(found?);
    }

    // A closed issue with every box ticked says all there is: the merge closed
    // it, the boxes show what it did. Open ones and ones with a box left open
    // need the word on what's done and what's open.
    let open: Vec<Stale> = issues
        .iter()
        .filter_map(|issue| {
            let iid = issue.get("iid").and_then(Value::as_u64)?;
            let (_, mr, merged_at) = last_merge.iter().find(|(number, _, _)| *number == iid)?;
            let boxes = boxes_of(str_of(issue, "description").unwrap_or(""));
            let state = str_of(issue, "state").unwrap_or("").to_string();
            let ticked = boxes.iter().filter(|is_ticked| **is_ticked).count();
            (state == "opened" || ticked < boxes.len()).then(|| Stale {
                iid,
                state,
                title: str_of(issue, "title").unwrap_or("").to_string(),
                boxes: (ticked, boxes.len()),
                mr: *mr,
                merged_at: merged_at.clone(),
            })
        })
        .collect();

    let endpoints: Vec<String> = open
        .iter()
        .map(|entry| {
            format!(
                "projects/:fullpath/issues/{}/notes?sort=desc&order_by=created_at&per_page=100",
                entry.iid
            )
        })
        .collect();
    let mut stale = Vec::new();
    for (entry, notes) in open.into_iter().zip(gitlab::lists(main, &endpoints)) {
        let has_comment = notes?.iter().any(|note| {
            note.get("system").and_then(Value::as_bool) != Some(true)
                && str_of(note, "created_at").is_some_and(|at| is_later(at, &entry.merged_at))
        });
        if !has_comment {
            stale.push(entry);
        }
    }
    stale.sort_by(|a, b| order_of(&b.merged_at, &a.merged_at));
    Ok(stale)
}

/// What `wt:issues` prints.
fn text_of(stale: &[Stale]) -> String {
    if stale.is_empty() {
        return "\n✓ every issue of a merged MR has a comment since its merge\n\n".into();
    }
    let rows: Vec<[String; 5]> = stale
        .iter()
        .map(|entry| {
            let (ticked, total) = entry.boxes;
            [
                format!("#{}", entry.iid),
                entry.state.clone(),
                if total > 0 {
                    format!("{ticked}/{total} ticked")
                } else {
                    "no boxes".into()
                },
                format!("!{} {}", entry.mr, entry.merged_at.chars().take(10).collect::<String>()),
                entry.title.clone(),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..5)
        .map(|column| rows.iter().map(|row| row[column].chars().count()).max().unwrap_or(0))
        .collect();
    let count = stale.len();
    let mut text = format!(
        "\n{count} issue{} of merged work without a comment since the merge:\n\n",
        if count == 1 { "" } else { "s" }
    );
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(column, cell)| {
                if column < 4 {
                    pad(cell, widths[column])
                } else {
                    cell.clone()
                }
            })
            .collect();
        text += &format!("  {}\n", cells.join("  "));
    }
    text + "\n" + CATCH_UP + "\n\n"
}

/// Whether the time `a` comes after `b`: by the clock when both parse, as
/// text otherwise (ISO strings of one form sort the same).
fn is_later(a: &str, b: &str) -> bool {
    order_of(a, b) == Ordering::Greater
}

fn order_of(a: &str, b: &str) -> Ordering {
    match (parse_iso(a), parse_iso(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => a.cmp(b),
    }
}

/// A checkbox of an issue description (issues.mjs `boxesOf`): its line (from
/// 0), ticked or not, its text and the heading it stands under.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Box {
    pub line: usize,
    pub is_ticked: bool,
    pub text: String,
    pub heading: Option<String>,
}

/// The checkboxes of a description in order: `- [ ]`, `* [x]`, `+ [X]` and
/// a space after; a box in fenced code is an example, not a task.
pub fn boxes_with_headings(text: &str) -> Vec<Box> {
    let mut boxes = Vec::new();
    let mut heading = None;
    let mut is_in_code = false;
    for (at, line) in text.split('\n').enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            is_in_code = !is_in_code;
            continue;
        }
        if is_in_code {
            continue;
        }
        let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(char::is_whitespace) {
            heading = Some(line[hashes..].trim().to_string());
        }
        let Some(after) = trimmed.strip_prefix(['-', '*', '+']) else {
            continue;
        };
        let rest = after.trim_start();
        if rest.len() == after.len() || !rest.starts_with('[') {
            continue;
        }
        // `[` mark `]` and at least one space after it.
        let bytes = rest.as_bytes();
        if bytes.len() >= 4
            && bytes[2] == b']'
            && matches!(bytes[1], b' ' | b'x' | b'X')
            && (bytes[3] as char).is_whitespace()
        {
            boxes.push(Box {
                line: at,
                is_ticked: bytes[1] != b' ',
                text: rest[3..].trim().to_string(),
                heading: heading.clone(),
            });
        }
    }
    boxes
}

/// Whether each checkbox of a description is ticked, in order.
pub fn boxes_of(text: &str) -> Vec<bool> {
    boxes_with_headings(text).iter().map(|found| found.is_ticked).collect()
}

/// The issues a description says it works on: `Closes #12`, `refs #12`,
/// `fixed #12` (prune.mjs `REFERENCE`, any case).
pub fn references(text: &str) -> Vec<u64> {
    const WORDS: [&str; 11] = [
        "closes", "closed", "close", "fixes", "fixed", "fix", "resolves", "resolved", "resolve", "refs", "ref",
    ];
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut numbers = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let is_start = at == 0 || !is_word(bytes[at - 1]);
        let found = is_start
            .then(|| WORDS.iter().find_map(|word| reference_at(&lower[at..], word)))
            .flatten();
        match found {
            Some((number, length)) => {
                numbers.push(number);
                at += length;
            }
            None => at += 1,
        }
        while at < bytes.len() && !lower.is_char_boundary(at) {
            at += 1;
        }
    }
    numbers
}

/// `<word><whitespace>#<digits>` at the start of `text`: the number and the length matched.
fn reference_at(text: &str, word: &str) -> Option<(u64, usize)> {
    let rest = text.strip_prefix(word)?;
    let after_space = rest.trim_start();
    if after_space.len() == rest.len() {
        return None;
    }
    let digits = after_space.strip_prefix('#')?;
    let length = digits.bytes().take_while(u8::is_ascii_digit).count();
    let number = digits[..length].parse().ok()?;
    Some((number, text.len() - digits.len() + length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_issues_a_description_works_on() {
        assert_eq!(references("Closes #12, refs  #3 and FIXED #7."), vec![12, 3, 7]);
        assert_eq!(references("Resolve #4\nresolves\t#5"), vec![4, 5]);
        assert_eq!(
            references("prefix#1 unclosed #2 closes#3 xfix #4 ref #"),
            Vec::<u64>::new()
        );
        assert_eq!(references("é close #9"), vec![9]);
        // `close` + `d` backtracks into `closed`: one match, not two.
        assert_eq!(references("closed #10"), vec![10]);
    }

    #[test]
    fn counts_boxes_outside_code() {
        let text = "## Done\n- [x] one\n  * [ ] two\n+ [X] three\n```\n- [ ] example\n```\n- [] none\n-[ ] none\n- [ ]";
        assert_eq!(boxes_of(text), vec![true, false, true]);
    }

    fn stale(iid: u64, boxes: (usize, usize)) -> Stale {
        Stale {
            iid,
            state: "closed".into(),
            title: format!("title {iid}"),
            boxes,
            mr: 302,
            merged_at: "2026-10-01T13:21:14.767Z".into(),
        }
    }

    #[test]
    fn prints_one_row_per_issue_padded_to_the_widest() {
        let text = text_of(&[stale(289, (8, 11)), stale(7, (0, 0))]);
        assert!(text.starts_with("\n2 issues of merged work without a comment since the merge:\n\n"));
        assert!(
            text.contains("\n  #289  closed  8/11 ticked  !302 2026-10-01  title 289\n"),
            "{text}"
        );
        assert!(
            text.contains("\n  #7    closed  no boxes     !302 2026-10-01  title 7\n"),
            "{text}"
        );
        assert!(text.ends_with("--message-file <f>\n\n"));
        assert!(text_of(&[stale(1, (0, 1))]).starts_with("\n1 issue of merged work"));
        assert_eq!(
            text_of(&[]),
            "\n✓ every issue of a merged MR has a comment since its merge\n\n"
        );
    }

    #[test]
    fn times_compare_by_the_clock() {
        assert!(is_later("2026-10-01T13:22:07.951Z", "2026-10-01T13:21:14.767Z"));
        assert!(is_later("2026-10-01T15:22:00+00:00", "2026-10-01T17:21:00+02:00"));
        assert!(!is_later("2026-10-01T13:21:14.767Z", "2026-10-01T13:21:14.767Z"));
    }
}
