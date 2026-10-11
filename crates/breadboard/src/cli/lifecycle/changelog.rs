//! A removal's merge added to its orchestration's changelog (#305): the one
//! entry, from the MR read the removal holds already, put where a rewrite
//! would put it (its day, its group, by time), with no GitLab call. Today's
//! rewrote the whole file from GitLab, two calls per issue of the record.

use crate::cli::setup::changelog::{Merged, changelog_text, summary_of};
use breadboard_core::json::{int_of, str_of};
use serde_json::Value;

/// The heading of the issues closed without a merge request (`changelog_text`).
const CLOSED: &str = "## Closed without a merge request";

/// The groups of a day in `changelog_text`'s order.
const ORDER: [&str; 10] = [
    "Features",
    "Fixes",
    "Performance",
    "Refactoring",
    "Documentation",
    "Tests",
    "Build",
    "CI",
    "Chores",
    "Other",
];

/// One merge as a changelog lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `## 2026-10-01`
    day: String,
    /// `### Features`
    group: String,
    /// `- [!313](…) [#302](…) … · 15:29` and its summary.
    lines: Vec<String>,
    /// `15:29`
    time: String,
    issue: Option<i64>,
}

/// The `[#<n>](<url>)` link of issue `number`: as the changelog's other
/// issue links are (GitLab's `work_items/` or `issues/`), else as a rewrite
/// derives it from the MR's url.
fn issue_url(text: &str, mr_url: &str, number: i64) -> String {
    let copied = text.split("[#").skip(1).find_map(|rest| {
        let (listed, rest) = rest.split_once("](")?;
        let url = rest.split_once(')')?.0;
        let prefix = url.strip_suffix(listed)?;
        (listed.parse::<i64>().is_ok() && prefix.ends_with('/')).then(|| format!("{prefix}{number}"))
    });
    copied.unwrap_or_else(|| {
        let base = mr_url.rsplit_once("merge_requests/").map_or(mr_url, |(base, _)| base);
        format!("{base}issues/{number}")
    })
}

/// The entry of the merged `mr` of issue `number` (None: its record has no
/// number), rendered by `changelog_text` itself; `text`: the changelog it goes into.
pub fn entry_of(mr: &Value, number: Option<i64>, text: &str) -> Entry {
    let url = str_of(mr, "web_url").unwrap_or_default().to_string();
    let title = str_of(mr, "title").unwrap_or_default();
    let merged = Merged {
        iid: int_of(mr, "iid").unwrap_or_default(),
        issue: number.map(|number| (number, issue_url(text, &url, number))),
        summary: summary_of(str_of(mr, "description").unwrap_or_default(), title),
        title: title.strip_prefix("Draft:").map_or(title, str::trim_start).to_string(),
        merged_at: str_of(mr, "merged_at").unwrap_or_default().to_string(),
        url,
        ..Merged::default()
    };
    let rendered = changelog_text("", &[merged], &[]);
    let find = |prefix: &str| {
        rendered
            .lines()
            .find(|line| line.starts_with(prefix))
            .unwrap_or_default()
            .to_string()
    };
    let lines: Vec<String> = rendered
        .lines()
        .skip_while(|line| !line.starts_with("- "))
        .take_while(|line| !line.is_empty())
        .map(String::from)
        .collect();
    Entry {
        day: find("## "),
        group: find("### "),
        time: lines.first().map(|line| time_of(line).to_string()).unwrap_or_default(),
        lines,
        issue: number,
    }
}

/// The `HH:MM` an entry line ends with.
fn time_of(line: &str) -> &str {
    line.rsplit_once(" · ").map_or("", |(_, time)| time)
}

/// Where a section starting at `from` ends: the next line starting with `prefix`, or the end.
fn end_of(lines: &[String], from: usize, prefix: &str) -> usize {
    (from..lines.len())
        .find(|&at| lines[at].starts_with(prefix))
        .unwrap_or(lines.len())
}

/// The issue `number` taken out of the issues closed without a merge request
/// (it has one now), and that section with its last one.
fn drop_closed(lines: &mut Vec<String>, number: i64) {
    let Some(heading) = lines.iter().position(|line| line == CLOSED) else {
        return;
    };
    let end = end_of(lines, heading + 1, "## ");
    let link = format!("- [#{number}](");
    if let Some(at) = (heading + 1..end).find(|&at| lines[at].starts_with(&link)) {
        lines.remove(at);
    }
    let end = end_of(lines, heading + 1, "## ");
    if !lines[heading + 1..end].iter().any(|line| line.starts_with("- ")) {
        lines.drain(heading..end);
        if lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(String::new());
        }
    }
}

/// The changelog `text` with `entry` added where a rewrite would list it;
/// None for a text that is no orchestration's changelog.
pub fn insert(text: &str, entry: &Entry) -> Option<String> {
    if !text.starts_with("# ") || entry.lines.is_empty() {
        return None;
    }
    // Split at '\n', not lines(): joined again it is byte for byte the text.
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    if let Some(at) = lines.iter().position(|line| line == "Nothing merged yet.") {
        lines.remove(at);
        if lines.get(at).is_some_and(String::is_empty) {
            lines.remove(at);
        }
    }
    if let Some(number) = entry.issue {
        drop_closed(&mut lines, number);
    }
    // Days newest first, the issues closed without an MR after them.
    let day = (0..lines.len()).find(|&at| {
        let line = &lines[at];
        line.starts_with("## ") && (*line == entry.day || *line == CLOSED || line.as_str() < entry.day.as_str())
    });
    let block = |with_day: bool| {
        let mut block = Vec::new();
        if with_day {
            block.extend([entry.day.clone(), String::new()]);
        }
        block.extend([entry.group.clone(), String::new()]);
        block.extend(entry.lines.iter().cloned());
        block.push(String::new());
        block
    };
    match day {
        Some(at) if lines[at] == entry.day => {
            let end = end_of(&lines, at + 1, "## ");
            let order = |group: &str| ORDER.iter().position(|each| group == format!("### {each}"));
            let rank = order(&entry.group);
            match (at + 1..end).find(|&each| lines[each] == entry.group) {
                Some(group) => {
                    let first = group + 2;
                    let last = (first..end).find(|&each| lines[each].is_empty()).unwrap_or(end);
                    // By time, newest first; the one just merged before others of its minute.
                    let place = (first..last)
                        .find(|&each| lines[each].starts_with("- ") && time_of(&lines[each]) <= entry.time.as_str())
                        .unwrap_or(last);
                    lines.splice(place..place, entry.lines.iter().cloned());
                }
                None => {
                    let place = (at + 1..end)
                        .find(|&each| lines[each].starts_with("### ") && order(&lines[each]) > rank)
                        .unwrap_or(end);
                    lines.splice(place..place, block(false));
                }
            }
        }
        Some(at) => {
            lines.splice(at..at, block(true));
        }
        None => {
            // After the last section's blank line; `[…, ""]` is a text ending in "\n".
            if lines.last().is_some_and(String::is_empty) {
                lines.pop();
            }
            if lines.last().is_some_and(|line| !line.is_empty()) {
                lines.push(String::new());
            }
            lines.extend(block(true));
        }
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::setup::changelog::Closed;
    use serde_json::json;

    fn mr(iid: i64, title: &str, merged_at: &str) -> Value {
        json!({
            "iid": iid,
            "title": title,
            "web_url": format!("https://g/p/-/merge_requests/{iid}"),
            "merged_at": merged_at,
            "description": format!("## Summary\n\nWhat {iid} does."),
            "state": "merged",
        })
    }

    fn merged(mr: &Value, number: i64) -> Merged {
        let title = str_of(mr, "title").unwrap_or_default();
        Merged {
            iid: int_of(mr, "iid").unwrap(),
            url: str_of(mr, "web_url").unwrap().into(),
            title: title.into(),
            issue: Some((number, format!("https://g/p/-/work_items/{number}"))),
            merged_at: str_of(mr, "merged_at").unwrap().into(),
            summary: summary_of(str_of(mr, "description").unwrap(), title),
            ..Merged::default()
        }
    }

    /// Whatever a rewrite writes without one merge, with it added, is what a rewrite writes with it.
    #[test]
    fn an_added_merge_reads_as_a_rewrite_would_write_it() {
        let all = [
            mr(10, "feat(x): a", "2026-10-01T15:29:00Z"),
            mr(11, "fix(x): b", "2026-10-01T14:07:00Z"),
            mr(12, "feat(x): c", "2026-10-01T13:33:00Z"),
            mr(13, "docs: d", "2026-09-30T10:00:00Z"),
            mr(14, "chore: e", "2026-10-02T09:00:00Z"),
            mr(15, "untyped f", "2026-09-29T09:00:00Z"),
            mr(16, "perf: g", "2026-10-01T14:07:00Z"),
        ];
        let closed = [Closed {
            number: 99,
            url: "https://g/p/-/work_items/99".into(),
            title: "closed".into(),
            closed_at: Some("2026-09-01T10:00:00Z".into()),
        }];
        let entries: Vec<Merged> = all
            .iter()
            .enumerate()
            .map(|(at, mr)| merged(mr, 100 + at as i64))
            .collect();
        let full = changelog_text("t", &entries, &closed);
        for (at, mr) in all.iter().enumerate() {
            let mut without = entries.clone();
            without.remove(at);
            let text = changelog_text("t", &without, &closed);
            let entry = entry_of(mr, Some(100 + at as i64), &text);
            assert_eq!(insert(&text, &entry).as_deref(), Some(full.as_str()), "!{}", 10 + at);
        }
        // The first merge of an orchestration, and its issue closed without an MR before.
        let nothing = changelog_text("t", &[], &[]);
        let entry = entry_of(&all[0], Some(100), &nothing);
        let one = changelog_text("t", &[merged(&all[0], 100)], &[]);
        assert_eq!(
            insert(&nothing, &entry).map(|text| text.replace("issues/", "work_items/")),
            Some(one.clone())
        );
        let only_closed = changelog_text(
            "t",
            &[],
            &[Closed {
                number: 100,
                url: "https://g/p/-/work_items/100".into(),
                ..Closed::default()
            }],
        );
        let entry = entry_of(&all[0], Some(100), &only_closed);
        assert_eq!(insert(&only_closed, &entry), Some(one));
        assert_eq!(insert("no changelog", &entry), None);
    }

    #[test]
    fn copies_the_issue_links_of_the_changelog() {
        let text = "- [!1](https://g/p/-/merge_requests/1) [#5](https://g/p/-/work_items/5) a · 10:00";
        assert_eq!(
            issue_url(text, "https://g/p/-/merge_requests/9", 7),
            "https://g/p/-/work_items/7"
        );
        assert_eq!(
            issue_url("", "https://g/p/-/merge_requests/9", 7),
            "https://g/p/-/issues/7"
        );
    }
}
