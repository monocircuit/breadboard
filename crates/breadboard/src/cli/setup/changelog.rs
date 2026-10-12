//! The changelogs written from GitLab (changelog.mjs, overall-changelog.mjs):
//! an orchestration's (#248, `wt:orchestrate <name> --changelog`) next to its
//! record, and the repo's (#261, `pnpm changelog [--since 30d|2w|<date>]`) in
//! `.claude/changelog.md`, by product and package. Rewritten and printed, or
//! shown live with `--watch` (view.rs). The files keep today's format.

use super::fragments::{self, Entry};
use super::gitlab;
use super::proc::{Args, errln, fail, step};
use super::repo;
use super::view;
use breadboard_core::json::{int_of, str_of};
use breadboard_core::record::{self, ORCHESTRATOR};
use breadboard_core::time::{day as day_of, hm as time_of};
use breadboard_core::time::{now_ms, parse_iso, to_iso};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::LazyLock;

const GROUPS: [(&str, &str); 9] = [
    ("feat", "Features"),
    ("fix", "Fixes"),
    ("perf", "Performance"),
    ("refactor", "Refactoring"),
    ("docs", "Documentation"),
    ("test", "Tests"),
    ("build", "Build"),
    ("ci", "CI"),
    ("chore", "Chores"),
];
const OTHER: &str = "Other";

static TYPED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^(\w+)(?:\([^)]*\))?!?:\s*(.*)$").unwrap());

/// The group of an MR title `feat(breadboard): …`, and the title without its type.
pub fn group_of(title: &str) -> (&'static str, String) {
    match TYPED.captures(title) {
        Some(found) => {
            let kind = found[1].to_lowercase();
            let group = GROUPS
                .iter()
                .find(|(known, _)| *known == kind)
                .map_or(OTHER, |(_, group)| group);
            (group, found[2].to_string())
        }
        None => (OTHER, title.to_string()),
    }
}

const SUMMARY_CHARACTERS: usize = 280;

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#{1,6}\s+").unwrap());
static PARAGRAPH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\s*\n").unwrap());
static TITLE_AGAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\w+(\([^)]*\))?!?: [^\n]*$").unwrap());
static REFERENCES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(closes|fixes|resolves|part of)\b").unwrap());
static BOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[-*] \[[ x]\]").unwrap());
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// One or two sentences of what changed, from an MR description (changelog.mjs `summaryOf`).
pub fn summary_of(description: &str, title: &str) -> String {
    let text = description.replace('\r', "");
    let sections: Vec<&str> = HEADING.split(&text).collect();
    let summary = sections.iter().find(|section| {
        section.len() >= 7
            && section[..7].eq_ignore_ascii_case("summary")
            && !section[7..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    });
    let body = match summary {
        Some(summary) => summary.split_once('\n').map_or("", |(_, rest)| rest),
        None => sections.first().copied().unwrap_or_default(),
    };
    let paragraph = PARAGRAPH
        .split(body)
        .map(str::trim)
        .find(|part| {
            !part.is_empty()
                && *part != title.trim()
                && !TITLE_AGAIN.is_match(part)
                && !REFERENCES.is_match(part)
                && !BOX.is_match(part)
        })
        .unwrap_or_default();
    let flat = MD_LINK.replace_all(paragraph, "$1");
    let flat = SPACES.replace_all(&flat, " ");
    let flat = flat.trim();
    let two = first_sentences(flat, 2);
    if two.chars().count() > SUMMARY_CHARACTERS {
        let cut: String = two.chars().take(SUMMARY_CHARACTERS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        two
    }
}

/// The first `count` sentences: one ends at `.!?` before a space and a capital.
fn first_sentences(text: &str, count: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut sentences: Vec<String> = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < chars.len() {
        if matches!(chars[at], '.' | '!' | '?') {
            let mut next = at + 1;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            if next > at + 1 && next < chars.len() && chars[next].is_uppercase() {
                sentences.push(chars[start..=at].iter().collect());
                start = next;
                at = next;
                continue;
            }
        }
        at += 1;
    }
    sentences.push(chars[start..].iter().collect());
    sentences
        .into_iter()
        .take(count)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn ms_of(iso: &str) -> i64 {
    parse_iso(iso).unwrap_or(0)
}

/// A merged MR as the changelogs list it.
#[derive(Debug, Clone, Default)]
pub struct Merged {
    pub iid: i64,
    pub url: String,
    pub title: String,
    pub issue: Option<(i64, String)>,
    pub merged_at: String,
    pub summary: String,
    pub groups: Vec<String>,
    pub changesets: Vec<Changeset>,
    pub orchestration: Option<String>,
}

/// An issue closed without a merge request.
#[derive(Debug, Clone, Default)]
pub struct Closed {
    pub number: i64,
    pub url: String,
    pub title: String,
    pub closed_at: Option<String>,
}

fn refs_of(mr: &Merged) -> String {
    let mut refs = vec![format!("[!{}]({})", mr.iid, mr.url)];
    if let Some((number, url)) = &mr.issue {
        refs.push(format!("[#{number}]({url})"));
    }
    refs.join(" ")
}

/// The changelog of an orchestration as Markdown (changelog.mjs `changelogText`).
pub fn changelog_text(title: &str, merged: &[Merged], closed: &[Closed]) -> String {
    let mut lines = vec![format!("# {ORCHESTRATOR} {title}: changelog"), String::new()];
    if merged.is_empty() && closed.is_empty() {
        lines.extend(["Nothing merged yet.".to_string(), String::new()]);
    }
    let mut entries: Vec<&Merged> = merged.iter().collect();
    entries.sort_by(|a, b| b.merged_at.cmp(&a.merged_at));
    let mut days: Vec<String> = Vec::new();
    for entry in &entries {
        let day = day_of(ms_of(&entry.merged_at));
        if !days.contains(&day) {
            days.push(day);
        }
    }
    let order: Vec<&str> = GROUPS.iter().map(|(_, group)| *group).chain([OTHER]).collect();
    for day in days {
        lines.extend([format!("## {day}"), String::new()]);
        let of_day: Vec<&&Merged> = entries
            .iter()
            .filter(|entry| day_of(ms_of(&entry.merged_at)) == day)
            .collect();
        for group in &order {
            let in_group: Vec<&&&Merged> = of_day
                .iter()
                .filter(|entry| group_of(&entry.title).0 == *group)
                .collect();
            if in_group.is_empty() {
                continue;
            }
            lines.extend([format!("### {group}"), String::new()]);
            for entry in in_group {
                lines.push(format!(
                    "- {} {} · {}",
                    refs_of(entry),
                    group_of(&entry.title).1,
                    time_of(ms_of(&entry.merged_at))
                ));
                if !entry.summary.is_empty() {
                    lines.push(format!("  {}", entry.summary));
                }
            }
            lines.push(String::new());
        }
    }
    if !closed.is_empty() {
        lines.extend(["## Closed without a merge request".to_string(), String::new()]);
        let mut sorted: Vec<&Closed> = closed.iter().collect();
        sorted.sort_by(|a, b| {
            b.closed_at
                .clone()
                .unwrap_or_default()
                .cmp(&a.closed_at.clone().unwrap_or_default())
        });
        for issue in sorted {
            let at = issue
                .closed_at
                .as_ref()
                .map(|at| format!(" · {} {}", day_of(ms_of(at)), time_of(ms_of(at))))
                .unwrap_or_default();
            lines.push(format!("- [#{}]({}) {}{at}", issue.number, issue.url, issue.title));
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

fn untitled(title: &str) -> String {
    title
        .strip_prefix("Draft:")
        .map(str::trim_start)
        .unwrap_or(title)
        .to_string()
}

/// Through a temporary file: the tab reading it never sees half of it.
fn write_atomic(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = format!("{}.{}.tmp", file.display(), std::process::id());
    fs::write(&temporary, text)?;
    fs::rename(&temporary, file)
}

/// How a `--changelog` that GitLab didn't answer for starts its `✗ ` line:
/// the file stays as it was (#306). wt:remove reads it from its child.
pub const UNANSWERED: &str = "GitLab did not answer";

/// A rewrite of an orchestration's changelog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewritten {
    /// Written: the Markdown.
    Written(String),
    /// Not written: GitLab didn't answer for this issue (`#12`, or its
    /// branch), and why.
    Unanswered(String, String),
}

/// What GitLab said for one issue of the record: its latest merged MR, else
/// the issue (when it is closed without one).
type Read = (Option<i64>, Option<Value>, Option<Value>);

/// One issue's MR, and its issue only when it has none: the link of an MR's
/// issue is the MR's project's `issues/<n>`, as GitLab's `web_url` says it
/// (#305: half the calls). Err: GitLab didn't answer, after its tries: for
/// what, and why.
fn read_issue(number: Option<i64>, branch: &str) -> Result<Read, (String, String)> {
    let unanswered = |failure: gitlab::Failure| {
        let what = number.map_or_else(|| branch.to_string(), |number| format!("#{number}"));
        (what, failure.reason)
    };
    let mut mrs = gitlab::list(&format!(
        "projects/:fullpath/merge_requests?state=merged&source_branch={}",
        gitlab::encode(branch)
    ))
    .map_err(unanswered)?;
    mrs.sort_by(|a, b| str_of(b, "merged_at").cmp(&str_of(a, "merged_at")));
    if let Some(mr) = mrs.into_iter().next() {
        return Ok((number, Some(mr), None));
    }
    let Some(number) = number else {
        return Ok((None, None, None));
    };
    match gitlab::api(&format!("projects/:fullpath/issues/{number}")) {
        Ok(issue) => Ok((Some(number), None, Some(issue))),
        // An issue that is gone is no issue closed without a merge request.
        Err(failure) if failure.reason.starts_with("HTTP 404") => Ok((Some(number), None, None)),
        Err(failure) => Err(unanswered(failure)),
    }
}

/// The changelog of `name` rewritten from its record and GitLab; None without
/// a record. A merge never becomes unmerged: when GitLab didn't answer for an
/// issue the file stays as it was, never "Nothing merged yet." (#306).
pub fn write_changelog(name: &str) -> Option<Rewritten> {
    let main = repo::main_dir();
    let found = record::record_of(&main, name)?;
    let issues: Vec<(Option<i64>, String)> = found
        .issues
        .iter()
        .filter_map(|issue| issue.branch.clone().map(|branch| (issue.number, branch)))
        .collect();
    let read = match gitlab::all_in_turns(&issues, 12, |(number, branch)| read_issue(*number, branch)) {
        Ok(read) => read,
        Err((what, why)) => return Some(Rewritten::Unanswered(what, why)),
    };
    let mut merged = Vec::new();
    let mut closed = Vec::new();
    for (number, mr, issue) in read {
        match mr {
            Some(mr) => {
                let url = str_of(&mr, "web_url").unwrap_or_default().to_string();
                let title = str_of(&mr, "title").unwrap_or_default();
                merged.push(Merged {
                    iid: int_of(&mr, "iid").unwrap_or_default(),
                    issue: number.map(|number| {
                        let base = url
                            .rsplit_once("merge_requests/")
                            .map_or(url.as_str(), |(base, _)| base);
                        (number, format!("{base}issues/{number}"))
                    }),
                    summary: summary_of(str_of(&mr, "description").unwrap_or_default(), title),
                    title: untitled(title),
                    merged_at: str_of(&mr, "merged_at").unwrap_or_default().to_string(),
                    url,
                    ..Merged::default()
                });
            }
            None => {
                if let Some(issue) = issue.filter(|issue| str_of(issue, "state") == Some("closed")) {
                    closed.push(Closed {
                        number: int_of(&issue, "iid").unwrap_or_default(),
                        url: str_of(&issue, "web_url").unwrap_or_default().to_string(),
                        title: str_of(&issue, "title").unwrap_or_default().to_string(),
                        closed_at: str_of(&issue, "closed_at").map(String::from),
                    });
                }
            }
        }
    }
    let text = changelog_text(&record::title_of(&main, name), &merged, &closed);
    let _ = write_atomic(&record::changelog_file(&main, name), &text);
    Some(Rewritten::Written(text))
}

/// `wt:orchestrate <name> --changelog [--watch]`.
pub fn orchestration_command(name: &str, is_live: bool) -> ExitCode {
    let main = repo::main_dir();
    let file = record::changelog_file(&main, name);
    let relative = file
        .strip_prefix(&main)
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    if is_live {
        let empty = format!(
            "# {}: changelog\n\nNothing merged yet: the first merge writes {relative}.\n",
            record::orchestrator_label(&main, name)
        );
        return view::watch(&file, &empty);
    }
    let text = match write_changelog(name) {
        Some(Rewritten::Written(text)) => {
            step(&format!("wrote {relative} from the record and GitLab"));
            text
        }
        Some(Rewritten::Unanswered(what, why)) => fail(&format!(
            "{UNANSWERED} for {what} ({why}): {relative} stays as it was\n  pnpm wt:orchestrate {name} --changelog tries again"
        )),
        None if file.exists() => {
            step(&format!("{name} has no record any more: {relative} as it is"));
            fs::read_to_string(&file).unwrap_or_default()
        }
        None => fail(&format!("{name} has no record and no changelog ({relative})")),
    };
    view::print(&text);
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// The repo's changelog (overall-changelog.mjs)
// ---------------------------------------------------------------------------

const DEFAULT_SINCE: &str = "30d";
const REPO: &str = "repo";

/// The start of the window `since` (`30d`, `2w`, `2026-09-01`) at `now`, ms; None when it is neither.
pub fn since_of(since: &str, now: i64) -> Option<i64> {
    if let Some(unit) = since.chars().last().filter(|unit| *unit == 'd' || *unit == 'w') {
        let digits = &since[..since.len() - 1];
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            let days: i64 = digits.parse().ok()?;
            return Some(now - days * if unit == 'w' { 7 } else { 1 } * 86_400_000);
        }
    }
    let bytes = since.as_bytes();
    let is_date = bytes.len() >= 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit);
    if is_date { parse_iso(since) } else { None }
}

/// The group of a changed path: `products/<x>`, `packages/<x>`, else `repo`.
pub fn group_of_path(file: &str) -> String {
    for prefix in ["products/", "packages/"] {
        if let Some(rest) = file.strip_prefix(prefix)
            && let Some((name, _)) = rest.split_once('/')
        {
            return format!("{prefix}{name}");
        }
    }
    REPO.into()
}

fn group_of_package(name: &str) -> String {
    format!("packages/{}", name.rsplit('/').next().unwrap_or(name))
}

/// A changeset an MR adds: its packages, bump and text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changeset {
    pub packages: Vec<String>,
    pub bump: Option<String>,
    pub text: String,
}

static CHANGESET_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\.changeset/[^/]+\.md$").unwrap());
static CHANGESET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^---\n(.*?)\n---\n?(.*)$").unwrap());
static PACKAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^\s*["']?([^"':]+)["']?\s*:\s*(\w+)"#).unwrap());

pub fn changeset_of(text: &str) -> Option<Changeset> {
    let text = text.replace('\r', "");
    let found = CHANGESET.captures(&text)?;
    let packages: Vec<(String, String)> = PACKAGE
        .captures_iter(&found[1])
        .map(|package| (package[1].trim().to_string(), package[2].to_string()))
        .collect();
    Some(Changeset {
        bump: packages.first().map(|(_, bump)| bump.clone()),
        packages: packages.into_iter().map(|(name, _)| name).collect(),
        text: found[2].trim().to_string(),
    })
}

/// The groups an MR belongs to: its paths' (a changeset counts for the packages it names).
pub fn groups_of(paths: &[String], changesets: &[Changeset]) -> Vec<String> {
    let mut groups: Vec<String> = Vec::new();
    for file in paths.iter().filter(|file| !CHANGESET_FILE.is_match(file)) {
        let group = group_of_path(file);
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    for changeset in changesets {
        for name in &changeset.packages {
            let group = group_of_package(name);
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
    }
    groups
}

/// The groups a merged MR is listed in: those of what it changed, `repo`
/// when GitLab named no file of it (an empty diff). Never none: a merged
/// MR left out of every group made the changelog say `Nothing merged in
/// this window.` over 42 merges (the critic, round 7).
fn listed_groups(paths: &[String], changesets: &[Changeset]) -> Vec<String> {
    let groups = groups_of(paths, changesets);
    if groups.is_empty() { vec![REPO.into()] } else { groups }
}

/// The issue of an MR: its branch's number, else one it closes.
pub fn issue_of(branch: &str, description: &str) -> Option<i64> {
    static BRANCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:feature|fix)/(\d+)-").unwrap());
    static CLOSES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:closes|fixes|resolves) #(\d+)").unwrap());
    BRANCH
        .captures(branch)
        .or_else(|| CLOSES.captures(description))
        .and_then(|found| found[1].parse().ok())
}

/// A release tag: vita `vX.Y.Z` in products/vita, `monolithium-vX.Y.Z` in monolithium and utilities.
pub fn release_of(tag: &str) -> Option<(String, &'static str, Vec<&'static str>)> {
    static VITA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^v(\d+\.\d+\.\d+.*)$").unwrap());
    static LIBRARY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^monolithium-v(\d+\.\d+\.\d+.*)$").unwrap());
    if let Some(found) = VITA.captures(tag) {
        return Some((found[1].to_string(), "vita", vec!["products/vita"]));
    }
    LIBRARY.captures(tag).map(|found| {
        (
            found[1].to_string(),
            "monolithium",
            vec!["packages/monolithium", "packages/utilities"],
        )
    })
}

/// The list lines of `version`'s entry in a package CHANGELOG.md, six at most.
pub fn release_notes_of(text: &str, version: &str) -> Vec<String> {
    static SHA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{7,}: ").unwrap());
    static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*] (.*)$").unwrap());
    let text = text.replace('\r', "");
    let sections: Vec<&str> = text.split("\n## ").collect();
    let section = sections.iter().enumerate().find_map(|(i, part)| {
        let part = if i == 0 { part.strip_prefix("## ")? } else { part };
        (part.starts_with(&format!("{version}\n")) || part.starts_with(&format!("{version} "))).then_some(part)
    });
    let Some(section) = section else { return Vec::new() };
    section
        .split('\n')
        .skip(1)
        .filter_map(|line| ITEM.captures(line).map(|found| SHA.replace(&found[1], "").into_owned()))
        .take(6)
        .collect()
}

fn group_order(group: &str) -> String {
    let rank = if group.starts_with("products/") {
        0
    } else if group.starts_with("packages/") {
        1
    } else {
        2
    };
    format!("{rank}{group}")
}

/// A release in the window.
#[derive(Debug, Clone, Default)]
pub struct Release {
    pub tag: String,
    pub at: String,
    pub url: Option<String>,
    pub groups: Vec<String>,
    pub notes: Vec<String>,
}

const TYPES: [&str; 10] = [
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

/// The repo's changelog as Markdown (overall-changelog.mjs `overallText`).
pub fn overall_text(
    since: &str,
    merged: &[Merged],
    releases: &[Release],
    unreleased: Option<(&[Entry], &[String], Option<&str>)>,
) -> String {
    let mut lines = vec![
        "# Monocircuit/Platform: changelog".to_string(),
        String::new(),
        format!(
            "Merged into develop since {}, by product and package.",
            day_of(ms_of(since))
        ),
        String::new(),
    ];
    if let Some((entries, areas, url)) = unreleased.filter(|(entries, _, _)| !entries.is_empty()) {
        let count = if entries.len() == 1 {
            "1 entry".to_string()
        } else {
            format!("{} entries", entries.len())
        };
        lines.extend([
            "## Unreleased".to_string(),
            String::new(),
            format!("`CHANGELOG.md` · {count} waiting in `.changelog/` for the next release"),
            String::new(),
            fragments::section_body(entries, areas, url),
            String::new(),
        ]);
    }
    let mut groups: Vec<String> = Vec::new();
    for group in merged
        .iter()
        .flat_map(|mr| mr.groups.iter())
        .chain(releases.iter().flat_map(|release| release.groups.iter()))
    {
        if !groups.contains(group) {
            groups.push(group.clone());
        }
    }
    groups.sort_by_key(|group| group_order(group));
    if groups.is_empty() {
        lines.extend(["Nothing merged in this window.".to_string(), String::new()]);
    }
    enum Item<'a> {
        Mr(&'a Merged),
        Release(&'a Release),
    }
    for group in &groups {
        let of_group: Vec<&Merged> = merged.iter().filter(|mr| mr.groups.contains(group)).collect();
        let marks: Vec<&Release> = releases
            .iter()
            .filter(|release| release.groups.contains(group))
            .collect();
        let count = format!(
            "{} merge request{}",
            of_group.len(),
            if of_group.len() == 1 { "" } else { "s" }
        );
        let name = group.rsplit('/').next().unwrap_or(group);
        lines.extend([
            format!("## {name}"),
            String::new(),
            format!("`{group}` · {count}"),
            String::new(),
        ]);
        let mut all: Vec<(String, Item)> = of_group
            .iter()
            .map(|mr| (mr.merged_at.clone(), Item::Mr(mr)))
            .chain(marks.iter().map(|release| (release.at.clone(), Item::Release(release))))
            .collect();
        all.sort_by(|a, b| b.0.cmp(&a.0));
        let mut days: Vec<String> = Vec::new();
        for (at, _) in &all {
            let day = day_of(ms_of(at));
            if !days.contains(&day) {
                days.push(day);
            }
        }
        for day in days {
            lines.extend([format!("### {day}"), String::new()]);
            let of_day: Vec<&(String, Item)> = all.iter().filter(|(at, _)| day_of(ms_of(at)) == day).collect();
            for kind in std::iter::once("Releases").chain(TYPES) {
                let in_type: Vec<&&(String, Item)> = of_day
                    .iter()
                    .filter(|(_, item)| match item {
                        Item::Release(_) => kind == "Releases",
                        Item::Mr(mr) => group_of(&mr.title).0 == kind,
                    })
                    .collect();
                if in_type.is_empty() {
                    continue;
                }
                lines.extend([format!("#### {kind}"), String::new()]);
                for (at, item) in in_type {
                    match item {
                        Item::Release(release) => {
                            let name = release
                                .url
                                .as_ref()
                                .map_or(release.tag.clone(), |url| format!("[{}]({url})", release.tag));
                            lines.push(format!("- 🏷 {name} · {}", time_of(ms_of(at))));
                            lines.extend(release.notes.iter().map(|note| format!("  {note}")));
                        }
                        Item::Mr(mr) => {
                            let from = mr
                                .orchestration
                                .as_ref()
                                .map(|title| format!(" · {ORCHESTRATOR} {title}"))
                                .unwrap_or_default();
                            lines.push(format!(
                                "- {} {}{from} · {}",
                                refs_of(mr),
                                group_of(&mr.title).1,
                                time_of(ms_of(at))
                            ));
                            if !mr.summary.is_empty() {
                                lines.push(format!("  {}", mr.summary));
                            }
                            for changeset in mr.changesets.iter().filter(|changeset| {
                                changeset.packages.iter().any(|name| group_of_package(name) == *group)
                            }) {
                                static BULLET: LazyLock<Regex> =
                                    LazyLock::new(|| Regex::new(r"(?m)^\s*[-*] ").unwrap());
                                let bump = changeset
                                    .bump
                                    .as_ref()
                                    .map(|bump| format!(" ({bump})"))
                                    .unwrap_or_default();
                                lines.push(format!(
                                    "  changeset{bump}: {}",
                                    summary_of(&BULLET.replace_all(&changeset.text, ""), "")
                                ));
                            }
                        }
                    }
                }
                lines.push(String::new());
            }
        }
    }
    lines.join("\n")
}

/// The orchestration each MR came from: its changelog lists it, or a record names its branch.
fn orchestrations_of(mrs: &[Value]) -> Vec<(i64, String)> {
    let main = repo::main_dir();
    let dir = record::records_dir(&main);
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let mut found: Vec<(i64, String)> = Vec::new();
    static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^# ⇶ (.*): changelog$").unwrap());
    static IID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[!(\d+)\]").unwrap());
    for file in names.iter().filter(|file| file.ends_with(".changelog.md")) {
        let Ok(text) = fs::read_to_string(dir.join(file)) else {
            continue;
        };
        let title = TITLE
            .captures(&text)
            .map(|found| found[1].to_string())
            .unwrap_or_else(|| file.trim_end_matches(".changelog.md").to_string());
        for iid in IID
            .captures_iter(&text)
            .filter_map(|found| found[1].parse::<i64>().ok())
        {
            found.retain(|(known, _)| *known != iid);
            found.push((iid, title.clone()));
        }
    }
    for file in names.iter().filter(|file| file.ends_with(".json")) {
        let name = file.trim_end_matches(".json");
        let branches: Vec<String> = breadboard_core::json::read(&dir.join(file))
            .and_then(|raw| raw.get("issues").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|issue| str_of(issue, "branch").map(String::from))
            .collect();
        for mr in mrs {
            let iid = int_of(mr, "iid").unwrap_or_default();
            if branches
                .iter()
                .any(|branch| Some(branch.as_str()) == str_of(mr, "source_branch"))
                && !found.iter().any(|(known, _)| *known == iid)
            {
                found.push((iid, record::title_of(&main, name)));
            }
        }
    }
    found
}

/// Pages of a GitLab list until one comes back short, twenty at most; the
/// pages read before a call that failed for good, with that failure.
fn pages_of(endpoint: &str) -> (Vec<Value>, Option<gitlab::Failure>) {
    let mut all = Vec::new();
    for page in 1..=20 {
        let joint = if endpoint.contains('?') { '&' } else { '?' };
        match gitlab::list(&format!("{endpoint}{joint}per_page=100&page={page}")) {
            Ok(found) => {
                let is_short = found.len() < 100;
                all.extend(found);
                if is_short {
                    break;
                }
            }
            Err(failure) => return (all, Some(failure)),
        }
    }
    (all, None)
}

/// Every page of a GitLab list, or the failure of one of them.
fn all_of(endpoint: &str) -> Result<Vec<Value>, gitlab::Failure> {
    match pages_of(endpoint) {
        (_, Some(failure)) => Err(failure),
        (all, None) => Ok(all),
    }
}

fn state_file() -> std::path::PathBuf {
    repo::main_dir().join(".claude").join("changelog.json")
}

fn overall_file() -> std::path::PathBuf {
    repo::main_dir().join(".claude").join("changelog.md")
}

/// The changes of a merged MR: from what the last rewrite kept, else from its
/// diffs; a failed read of them is no "changed nothing" (it would be kept).
fn changes_of(iid: i64, known: &Map<String, Value>) -> Result<Value, gitlab::Failure> {
    if let Some(found) = known.get(&iid.to_string()) {
        return Ok(found.clone());
    }
    let diffs = all_of(&format!("projects/:fullpath/merge_requests/{iid}/diffs"))?;
    let mut paths: Vec<String> = Vec::new();
    for diff in &diffs {
        let is_deleted = diff.get("deleted_file").and_then(Value::as_bool).unwrap_or(false);
        let path = if is_deleted {
            str_of(diff, "old_path")
        } else {
            str_of(diff, "new_path")
        };
        if let Some(path) = path.filter(|path| !path.is_empty())
            && !paths.iter().any(|known| known == path)
        {
            paths.push(path.to_string());
        }
    }
    let changesets: Vec<Value> = diffs
        .iter()
        .filter(|diff| {
            diff.get("new_file").and_then(Value::as_bool).unwrap_or(false)
                && CHANGESET_FILE.is_match(str_of(diff, "new_path").unwrap_or_default())
        })
        .filter_map(|diff| {
            let added: Vec<&str> = str_of(diff, "diff")
                .unwrap_or_default()
                .split('\n')
                .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
                .map(|line| &line[1..])
                .collect();
            changeset_of(&added.join("\n"))
        })
        .map(|changeset| json!({ "packages": changeset.packages, "bump": changeset.bump, "text": changeset.text }))
        .collect();
    Ok(json!({ "paths": paths, "changesets": changesets }))
}

fn changeset_from(value: &Value) -> Changeset {
    Changeset {
        packages: breadboard_core::json::strs_of(value, "packages"),
        bump: str_of(value, "bump").map(String::from),
        text: str_of(value, "text").unwrap_or_default().to_string(),
    }
}

/// What `write_overall` wrote: the text, the window, and what GitLab didn't
/// answer (`GitLab: HTTP 502 Bad Gateway (!310's diffs): read again next time`).
#[derive(Debug, Default)]
pub struct Written {
    pub text: String,
    pub window: String,
    pub warnings: Vec<String>,
}

/// The changes kept for the next rewrite: only those read, by iid ascending
/// (as `Object.fromEntries` of Node writes integer keys). A merged MR whose
/// diffs failed stays out, so the next rewrite reads it again.
fn kept_of(read: &[(i64, Result<Value, gitlab::Failure>)]) -> Map<String, Value> {
    let mut kept: Vec<(i64, Value)> = read
        .iter()
        .filter_map(|(iid, change)| change.as_ref().ok().map(|change| (*iid, change.clone())))
        .collect();
    kept.sort_by_key(|(iid, _)| *iid);
    kept.into_iter()
        .map(|(iid, change)| (iid.to_string(), change))
        .collect()
}

/// The repo's changelog rewritten from GitLab in the window `since` (else the last chosen).
pub fn write_overall(since: Option<&str>) -> Result<Written, String> {
    let main = repo::main_dir();
    let state = breadboard_core::json::read(&state_file()).unwrap_or(json!({}));
    let known = state.get("mrs").and_then(Value::as_object).cloned().unwrap_or_default();
    let now = now_ms();
    let window = since
        .filter(|since| since_of(since, now).is_some())
        .map(String::from)
        .or_else(|| str_of(&state, "since").map(String::from))
        .unwrap_or_else(|| DEFAULT_SINCE.to_string());
    let start = since_of(&window, now).unwrap_or_else(|| since_of(DEFAULT_SINCE, now).unwrap_or(now));
    let start_iso = to_iso(start);
    // Into develop and into every epic (#349): a task merged into its epic
    // counts here, the epic's own MR into develop is only its tasks again.
    let bases = breadboard_core::git::bases(&main);
    let mut found: Vec<Value> = Vec::new();
    for base in &bases {
        found.extend(all_of(&format!(
            "projects/:fullpath/merge_requests?state=merged&target_branch={}&updated_after={}",
            gitlab::encode(&base.local),
            gitlab::encode(&start_iso)
        ))?);
    }
    let found: Vec<Value> = found
        .into_iter()
        .filter(|mr| str_of(mr, "merged_at").is_some_and(|at| ms_of(at) >= start))
        .filter(|mr| {
            !bases
                .iter()
                .any(|base| str_of(mr, "source_branch") == Some(base.local.as_str()))
        })
        .collect();
    let read: Vec<(i64, Result<Value, gitlab::Failure>)> = gitlab::in_turns(&found, 12, |mr| {
        let iid = int_of(mr, "iid").unwrap_or_default();
        (iid, changes_of(iid, &known))
    });
    let mut warnings: Vec<String> = read
        .iter()
        .filter_map(|(iid, change)| change.as_ref().err().map(|failure| (iid, failure)))
        .map(|(iid, failure)| format!("GitLab: {} (!{iid}'s diffs): read again next time", failure.reason))
        .collect();
    let orchestrations = orchestrations_of(&found);
    let nothing = json!({});
    let merged: Vec<Merged> = found
        .iter()
        .zip(&read)
        .map(|(mr, (iid, change))| {
            let is_read = change.is_ok();
            let change = change.as_ref().unwrap_or(&nothing);
            let url = str_of(mr, "web_url").unwrap_or_default().to_string();
            let title = str_of(mr, "title").unwrap_or_default();
            let changesets: Vec<Changeset> = change
                .get("changesets")
                .and_then(Value::as_array)
                .map(|list| list.iter().map(changeset_from).collect())
                .unwrap_or_default();
            let paths = breadboard_core::json::strs_of(change, "paths");
            let number = issue_of(
                str_of(mr, "source_branch").unwrap_or_default(),
                str_of(mr, "description").unwrap_or_default(),
            );
            let base = url
                .rsplit_once("merge_requests/")
                .map_or(url.clone(), |(base, _)| base.to_string());
            Merged {
                iid: *iid,
                issue: number.map(|number| (number, format!("{base}issues/{number}"))),
                summary: summary_of(str_of(mr, "description").unwrap_or_default(), title),
                title: untitled(title),
                merged_at: str_of(mr, "merged_at").unwrap_or_default().to_string(),
                groups: if is_read {
                    listed_groups(&paths, &changesets)
                } else {
                    Vec::new()
                },
                changesets,
                orchestration: orchestrations
                    .iter()
                    .find(|(known, _)| known == iid)
                    .map(|(_, title)| title.clone()),
                url,
            }
        })
        .collect();
    let (tags, failed) = pages_of("projects/:fullpath/repository/tags");
    if let Some(failure) = failed {
        let left = if tags.is_empty() {
            "releases left out"
        } else {
            "some releases left out"
        };
        warnings.push(format!(
            "GitLab: {} (the release tags): {left}, read again next time",
            failure.reason
        ));
    }
    let base = found
        .first()
        .and_then(|mr| str_of(mr, "web_url"))
        .and_then(|url| url.rsplit_once("/merge_requests/").map(|(base, _)| base.to_string()));
    let releases: Vec<Release> = tags
        .iter()
        .filter_map(|tag| {
            let name = str_of(tag, "name")?;
            let (version, _, groups) = release_of(name)?;
            let at = str_of(tag, "created_at")
                .or_else(|| {
                    tag.get("commit")
                        .and_then(|commit| str_of(commit, "committed_date").or_else(|| str_of(commit, "created_at")))
                })?
                .to_string();
            if ms_of(&at) < start {
                return None;
            }
            let notes = release_notes_of(
                &fs::read_to_string(main.join(groups[0]).join("CHANGELOG.md")).unwrap_or_default(),
                &version,
            );
            Some(Release {
                url: base.as_ref().map(|base| format!("{base}/tags/{name}")),
                tag: name.to_string(),
                at,
                groups: groups.into_iter().map(String::from).collect(),
                notes,
            })
        })
        .collect();
    let entries = fragments::fragments_at(&main, "origin/develop");
    let areas = fragments::areas_of(&main);
    let url = fragments::project_url(&main);
    let text = overall_text(&start_iso, &merged, &releases, Some((&entries, &areas, url.as_deref())));
    write_atomic(&overall_file(), &text).map_err(|error| error.to_string())?;
    let develop = state.get("develop").cloned().unwrap_or(Value::Null);
    let saved = json!({ "since": window, "develop": develop, "mrs": kept_of(&read) });
    write_atomic(
        &state_file(),
        &format!("{}\n", serde_json::to_string_pretty(&saved).unwrap_or_default()),
    )
    .map_err(|error| error.to_string())?;
    Ok(Written { text, window, warnings })
}

/// `pnpm changelog [--since 30d|2w|<date>] [--watch]`.
pub fn overall_command(args: &Args) -> ExitCode {
    let main = repo::main_dir();
    let file = overall_file();
    let relative = file
        .strip_prefix(&main)
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    if args.has("--watch") {
        let empty = format!(
            "# Monocircuit/Platform: changelog\n\nNot written yet: the overview of the main space writes {relative}, or `pnpm changelog`.\n"
        );
        return view::watch(&file, &empty);
    }
    let since = args.value("--since");
    if let Some(since) = &since
        && since_of(since, now_ms()).is_none()
    {
        fail(&format!(
            "--since takes days (30d), weeks (2w) or a date (2026-09-01), not {since}"
        ));
    }
    let written = write_overall(since.as_deref()).unwrap_or_else(|error| fail(&error));
    step(&format!("wrote {relative} from GitLab, since {}", written.window));
    view::print(&written.text);
    // Last, where they are seen: what this rewrite is missing and why.
    for warning in &written.warnings {
        errln!("! {warning}");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_an_mr_by_its_type() {
        assert_eq!(
            group_of("feat(breadboard): a thing"),
            ("Features", "a thing".to_string())
        );
        assert_eq!(group_of("Fix!: urgent"), ("Fixes", "urgent".to_string()));
        assert_eq!(group_of("wip stuff"), ("Other", "wip stuff".to_string()));
        assert_eq!(group_of("style: x"), ("Other", "x".to_string()));
    }

    #[test]
    fn summarises_a_description() {
        let description = "feat(x): the title\n\nCloses #3\n\n## Summary\nThe [view](http://u) shows it. It wraps e.g. `a.md` too. A third one.\n";
        assert_eq!(
            summary_of(description, "feat(x): the title"),
            "The view shows it. It wraps e.g. `a.md` too."
        );
        assert_eq!(summary_of("", "t"), "");
        let long = "a ".repeat(200);
        let cut = summary_of(&long, "");
        assert_eq!(cut.chars().count(), 280);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn reads_the_window() {
        let now = 30 * 86_400_000;
        assert_eq!(since_of("2w", now), Some(now - 14 * 86_400_000));
        assert_eq!(since_of("1970-01-02", now), Some(86_400_000));
        assert_eq!(since_of("soon", now), None);
        assert_eq!(since_of("3m", now), None);
    }

    #[test]
    fn lists_an_mr_without_files_under_repo() {
        assert_eq!(listed_groups(&[], &[]), vec![REPO]);
        assert_eq!(listed_groups(&[".changeset/a.md".into()], &[]), vec![REPO]);
        assert_eq!(listed_groups(&["packages/x/a.ts".into()], &[]), vec!["packages/x"]);
    }

    #[test]
    fn finds_the_groups_of_an_mr() {
        let changesets = vec![Changeset {
            packages: vec!["@monocircuit/utilities".into()],
            bump: Some("minor".into()),
            text: "x".into(),
        }];
        assert_eq!(
            groups_of(
                &[
                    "products/vita/web/a.ts".into(),
                    ".changeset/a.md".into(),
                    "README.md".into()
                ],
                &changesets
            ),
            vec!["products/vita", "repo", "packages/utilities"]
        );
        assert_eq!(
            changeset_of("---\n\"@monocircuit/monolithium\": patch\n---\n\nFixed it.\n")
                .unwrap()
                .bump
                .as_deref(),
            Some("patch")
        );
        assert_eq!(issue_of("feature/261-x", ""), Some(261));
        assert_eq!(issue_of("main", "Closes #9"), Some(9));
        assert_eq!(
            release_notes_of(
                "# c\n\n## 1.2.0\n\n### Minor\n\n- abcdef0: did it\n\n## 1.1.0\n- old\n",
                "1.2.0"
            ),
            vec!["did it"]
        );
    }

    #[test]
    fn keeps_only_the_changes_read_in_ascending_iids() {
        let failed = gitlab::Failure {
            reason: "HTTP 502 Bad Gateway".into(),
            endpoint: "projects/:fullpath/merge_requests/310/diffs".into(),
            is_passing: true,
        };
        let read = vec![
            (315, Ok(json!({ "paths": ["a"], "changesets": [] }))),
            (310, Err(failed)),
            (99, Ok(json!({ "paths": [], "changesets": [] }))),
        ];
        let kept = kept_of(&read);
        assert_eq!(kept.keys().collect::<Vec<_>>(), vec!["99", "315"]);
    }

    #[test]
    fn writes_an_orchestrations_changelog() {
        let merged = [Merged {
            iid: 5,
            url: "u/5".into(),
            title: "fix: it".into(),
            merged_at: "2026-09-30T10:00:00.000Z".into(),
            issue: Some((7, "u/7".into())),
            ..Merged::default()
        }];
        let text = changelog_text("B", &merged, &[]);
        assert!(text.starts_with("# ⇶ B: changelog\n\n## "));
        assert!(text.contains("### Fixes\n\n- [!5](u/5) [#7](u/7) it · "));
        assert_eq!(
            changelog_text("B", &[], &[]),
            "# ⇶ B: changelog\n\nNothing merged yet.\n"
        );
    }
}
