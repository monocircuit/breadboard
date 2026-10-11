//! The platform's CHANGELOG.md (#291, platform-changelog.mjs): Keep a
//! Changelog from `.changelog/<issue>-<slug>.md` fragments, one per MR,
//! folded into a section at the release. `pnpm changelog:add|unreleased|
//! release|notes|check`. Plain git, glab only for the MR links of a release:
//! CI checks the fragments without an install and without glab.

use super::proc::{Args, fail, git, outln};
use breadboard_core::json::{int_of, str_of};
use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::LazyLock;

pub const FRAGMENTS: &str = ".changelog";
pub const CHANGELOG: &str = "CHANGELOG.md";
const NO_CHANGELOG: &str = "no-changelog";
const TYPES: [(&str, &str); 6] = [
    ("added", "Added"),
    ("changed", "Changed"),
    ("deprecated", "Deprecated"),
    ("removed", "Removed"),
    ("fixed", "Fixed"),
    ("security", "Security"),
];

pub const USAGE: &str = "usage:
  pnpm changelog:add [--type <t>] [--area <a>] \"<sentence>\"   a fragment for this branch's MR
  pnpm changelog:unreleased                                  what the next release would say
  pnpm changelog:release <X.Y.Z>                             fold the fragments into CHANGELOG.md
  pnpm changelog:notes <X.Y.Z>                               the section of a release, for GitLab
  pnpm changelog:check [--mr]                                every fragment valid; --mr: this MR adds one";

/// An entry of the changelog.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub file: Option<String>,
    pub kind: String,
    pub area: String,
    pub text: String,
    pub issue: Option<i64>,
    /// `(iid, url)` of the MR that added it.
    pub mr: Option<(i64, String)>,
}

/// The areas an entry may name: products, packages (without `monocircuit-`), then ci and repo.
pub fn areas_of(root: &Path) -> Vec<String> {
    let dirs = |parent: &str| -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root.join(parent))
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .map(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        name.strip_prefix("monocircuit-").map(String::from).unwrap_or(name)
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    };
    let mut areas = dirs("products");
    areas.extend(dirs("packages"));
    areas.extend(["ci".to_string(), "repo".to_string()]);
    areas
}

/// The area of a changed path.
pub fn area_of_path(file: &str) -> String {
    for prefix in ["products/", "packages/"] {
        if let Some(rest) = file.strip_prefix(prefix)
            && let Some((name, _)) = rest.split_once('/')
        {
            return name.strip_prefix("monocircuit-").unwrap_or(name).to_string();
        }
    }
    if file == ".gitlab-ci.yml" || file.starts_with(".gitlab/ci/") {
        "ci".into()
    } else {
        "repo".into()
    }
}

static FRONTMATTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^---\n(.*?)\n---\n?(.*)$").unwrap());
static FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\w+):\s*(.*?)\s*$").unwrap());
static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+#.*$").unwrap());

/// A fragment's fields (in order) and text; None without frontmatter.
pub fn parse_fragment(content: &str) -> Option<(Vec<(String, String)>, String)> {
    let content = content.replace('\r', "");
    let found = FRONTMATTER.captures(&content)?;
    let fields = found[1]
        .split('\n')
        .filter_map(|line| {
            let line = COMMENT.replace(line, "");
            FIELD
                .captures(&line)
                .map(|field| (field[1].to_string(), field[2].to_string()))
        })
        .collect();
    Some((fields, found[2].trim().to_string()))
}

fn field_of<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// The issue a fragment's name starts with.
pub fn issue_of_name(name: &str) -> Option<i64> {
    let base = Path::new(name).file_name()?.to_string_lossy().into_owned();
    let digits: String = base.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty() && base[digits.len()..].starts_with('-'))
        .then(|| digits.parse().ok())
        .flatten()
}

/// What is wrong with a fragment, as sentences.
pub fn problems_of(name: &str, content: &str, areas: &[String]) -> Vec<String> {
    let Some((fields, text)) = parse_fragment(content) else {
        return vec![format!(
            "{name}: starts with a frontmatter block (--- type: … area: … ---)"
        )];
    };
    let mut problems = Vec::new();
    let kind = field_of(&fields, "type");
    if !TYPES.iter().any(|(known, _)| Some(*known) == kind) {
        let types: Vec<&str> = TYPES.iter().map(|(known, _)| *known).collect();
        problems.push(format!(
            "{name}: type \"{}\" is none of {}",
            kind.unwrap_or_default(),
            types.join(", ")
        ));
    }
    let area = field_of(&fields, "area");
    if !area.is_some_and(|area| areas.iter().any(|known| known == area)) {
        problems.push(format!(
            "{name}: area \"{}\" is none of {}",
            area.unwrap_or_default(),
            areas.join(", ")
        ));
    }
    let mut unknown: Vec<&str> = Vec::new();
    for (key, _) in &fields {
        if key != "type" && key != "area" && !unknown.contains(&key.as_str()) {
            unknown.push(key);
        }
    }
    if !unknown.is_empty() {
        problems.push(format!("{name}: unknown field {}", unknown.join(", ")));
    }
    if text.is_empty() {
        problems.push(format!("{name}: says nothing below the frontmatter"));
    }
    if text.split('\n').any(|line| line.starts_with('#')) {
        problems.push(format!("{name}: has a heading; an entry is a sentence or two"));
    }
    problems
}

/// The fragment files, by name.
pub fn fragment_files(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join(FRAGMENTS))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".md") && name != "README.md")
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.into_iter().map(|name| format!("{FRAGMENTS}/{name}")).collect()
}

fn entry_of(file: &str, content: &str) -> Entry {
    let (fields, text) = parse_fragment(content).unwrap_or_default();
    Entry {
        file: Some(file.to_string()),
        kind: field_of(&fields, "type").unwrap_or_default().to_string(),
        area: field_of(&fields, "area").unwrap_or_default().to_string(),
        text,
        issue: issue_of_name(file),
        mr: None,
    }
}

/// The fragments as entries, or every problem of the invalid ones.
pub fn read_fragments(root: &Path) -> Result<Vec<Entry>, String> {
    let areas = areas_of(root);
    let files = fragment_files(root);
    let contents: Vec<String> = files
        .iter()
        .map(|file| fs::read_to_string(root.join(file)).unwrap_or_default())
        .collect();
    let problems: Vec<String> = files
        .iter()
        .zip(&contents)
        .flat_map(|(file, content)| problems_of(file, content, &areas))
        .collect();
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    Ok(files
        .iter()
        .zip(&contents)
        .map(|(file, content)| entry_of(file, content))
        .collect())
}

/// The valid fragments as `reference` has them, read through git.
pub fn fragments_at(root: &Path, reference: &str) -> Vec<Entry> {
    let areas = areas_of(root);
    git(&["ls-tree", "--name-only", reference, &format!("{FRAGMENTS}/")], root)
        .text()
        .split('\n')
        .filter(|file| file.ends_with(".md") && *file != format!("{FRAGMENTS}/README.md"))
        .filter_map(|file| {
            let content = git(&["show", &format!("{reference}:{file}")], root).text().to_string();
            problems_of(file, &content, &areas)
                .is_empty()
                .then(|| entry_of(file, &content))
        })
        .collect()
}

/// The project's web URL, from CI or the origin remote.
pub fn project_url(root: &Path) -> Option<String> {
    if let Ok(url) = std::env::var("CI_PROJECT_URL")
        && !url.is_empty()
    {
        return Some(url);
    }
    static REMOTE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:git@|ssh://git@|https?://)([^:/]+)[:/](.+?)(?:\.git)?$").unwrap());
    let remote = git(&["remote", "get-url", "origin"], root).text().to_string();
    REMOTE
        .captures(&remote)
        .map(|found| format!("https://{}/{}", &found[1], &found[2]))
}

fn links_of(entry: &Entry, url: Option<&str>) -> String {
    let mut links = Vec::new();
    if let (Some(issue), Some(url)) = (entry.issue, url) {
        links.push(format!("[#{issue}]({url}/-/issues/{issue})"));
    }
    if let Some((iid, link)) = &entry.mr {
        links.push(format!("[!{iid}]({link})"));
    }
    if links.is_empty() {
        String::new()
    } else {
        format!(" ({})", links.join(", "))
    }
}

fn item_of(entry: &Entry, url: Option<&str>) -> Vec<String> {
    let mut lines = entry.text.split('\n');
    let first = lines.next().unwrap_or_default();
    let mut more: Vec<String> = lines
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("  {}", line.trim())
            }
        })
        .collect();
    let links = links_of(entry, url);
    match more.last_mut() {
        None => vec![format!("- **{}:** {first}{links}", entry.area)],
        Some(last) => {
            last.push_str(&links);
            let mut all = vec![format!("- **{}:** {first}", entry.area)];
            all.extend(more);
            all
        }
    }
}

/// The body of a section: per type a `### Added` … sorted by area, then file.
pub fn section_body(entries: &[Entry], areas: &[String], url: Option<&str>) -> String {
    let rank = |area: &str| areas.iter().position(|known| known == area).unwrap_or(areas.len());
    let mut lines: Vec<String> = Vec::new();
    for (kind, heading) in TYPES {
        let mut of_type: Vec<&Entry> = entries.iter().filter(|entry| entry.kind == kind).collect();
        if of_type.is_empty() {
            continue;
        }
        of_type.sort_by(|a, b| {
            rank(&a.area).cmp(&rank(&b.area)).then(
                a.file
                    .clone()
                    .unwrap_or_default()
                    .cmp(&b.file.clone().unwrap_or_default()),
            )
        });
        lines.push(format!("### {heading}"));
        lines.push(String::new());
        for entry in of_type {
            lines.extend(item_of(entry, url));
        }
        lines.push(String::new());
    }
    lines.join("\n").trim_end().to_string()
}

/// The file a first release starts from.
pub fn empty_changelog(url: Option<&str>) -> String {
    let references = url
        .map(|url| format!("\n[Unreleased]: {url}/-/commits/develop\n"))
        .unwrap_or_default();
    format!(
        "# Changelog

All notable changes to Monocircuit/Platform are documented in this file. The
format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the versions are those of vita's releases (`vX.Y.Z`, [Semantic
Versioning](https://semver.org/spec/v2.0.0.html)). Entries are collected in
[`.changelog/`](.changelog/README.md) until the release folds them in here.

## [Unreleased]
{references}"
    )
}

static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+\.\d+\.\d+$").unwrap());
static SECTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^## \[(\d+\.\d+\.\d+)\]").unwrap());
static NEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(## \[|\[[^\]]+\]: )").unwrap());

/// What is wrong with the version `changelog:<command>` got: missing, a
/// `v` too many (the tag's, not the section's), or no X.Y.Z.
fn version_refusal(version: &str, command: &str) -> String {
    let usage = format!("pnpm changelog:{command} <X.Y.Z>");
    if version.is_empty() {
        return format!("the version is missing: {usage}");
    }
    match version.strip_prefix('v').filter(|plain| VERSION.is_match(plain)) {
        Some(plain) => format!("{version} is the tag, the version is {plain}: pnpm changelog:{command} {plain}"),
        None => format!("{version} is no X.Y.Z version: {usage}"),
    }
}

/// The versions of a changelog's sections, newest first.
pub fn versions_of(text: &str) -> Vec<String> {
    SECTION.captures_iter(text).map(|found| found[1].to_string()).collect()
}

/// The changelog with the new section below `## [Unreleased]` and the link references at its end.
pub fn fold_into(text: &str, version: &str, date: &str, body: &str, url: Option<&str>) -> Result<String, String> {
    if !VERSION.is_match(version) {
        return Err(format!("\"{version}\" is no X.Y.Z"));
    }
    let versions = versions_of(text);
    if versions.iter().any(|known| known == version) {
        return Err(format!("{CHANGELOG} has a section {version} already"));
    }
    let previous = versions.first().cloned();
    static UNRELEASED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^## \[Unreleased\][^\n]*\n").unwrap());
    let Some(unreleased) = UNRELEASED.find(text) else {
        return Err(format!("{CHANGELOG} has no \"## [Unreleased]\""));
    };
    let at = unreleased.end();
    let rest = &text[at..];
    let next = NEXT.find(rest).map(|found| found.start());
    let kept = next.map_or(rest, |next| &rest[..next]).trim();
    let after = next.map_or("", |next| &rest[next..]);
    let section = format!("## [{version}] - {date}\n\n{body}\n");
    let kept = if kept.is_empty() {
        String::new()
    } else {
        format!("{kept}\n\n")
    };
    let mut result = format!("{}\n{kept}{section}\n{after}", &text[..at]);
    let Some(url) = url else {
        return Ok(format!("{}\n", result.trim_end()));
    };
    let references = [
        format!("[Unreleased]: {url}/-/compare/v{version}...develop"),
        match &previous {
            Some(previous) => format!("[{version}]: {url}/-/compare/v{previous}...v{version}"),
            None => format!("[{version}]: {url}/-/tags/v{version}"),
        },
    ];
    static OLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\[Unreleased\]: .*\n?").unwrap());
    static FIRST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\[\d+\.\d+\.\d+\]: ").unwrap());
    result = OLD.replacen(&result, 1, "").into_owned();
    result = match FIRST.find(&result).map(|found| found.start()) {
        None => format!("{}\n\n{}\n", result.trim_end(), references.join("\n")),
        Some(first) => format!("{}{}\n{}", &result[..first], references.join("\n"), &result[first..]),
    };
    Ok(format!("{}\n", result.trim_end()))
}

/// The body of `version`'s section, None without one.
pub fn notes_of(text: &str, version: &str) -> Option<String> {
    let heading = Regex::new(&format!(r"(?m)^## \[{}\]", regex::escape(version))).ok()?;
    let start = heading.find(text)?.start();
    let rest = &text[start..];
    let rest = rest.split_once('\n').map_or("", |(_, rest)| rest);
    let end = NEXT.find(rest).map(|found| found.start());
    Some(end.map_or(rest, |end| &rest[..end]).trim().to_string())
}

/// The MR that added `file` to develop, through glab; None when git or glab can't say.
fn mr_of_fragment(root: &Path, file: &str) -> Option<(i64, String)> {
    let sha = git(&["log", "--diff-filter=A", "--format=%H", "-1", "--", file], root)
        .text()
        .to_string();
    if sha.is_empty() {
        return None;
    }
    let found = super::proc::sh(
        "glab",
        &[
            "api",
            &format!("projects/:fullpath/repository/commits/{sha}/merge_requests"),
        ],
        Some(root),
    );
    if !found.is_ok {
        return None;
    }
    let list: Vec<Value> = serde_json::from_str(found.text()).ok()?;
    let mr = list.into_iter().find(|mr| str_of(mr, "state") == Some("merged"))?;
    Some((
        int_of(&mr, "iid")?,
        str_of(&mr, "web_url").unwrap_or_default().to_string(),
    ))
}

const LIBRARY: &str = "packages/monolithium";

fn version_at(root: &Path, reference: &str, file: &str) -> Option<String> {
    let shown = git(&["show", &format!("{reference}:{file}")], root);
    if !shown.is_ok {
        return None;
    }
    let value: Value = serde_json::from_str(shown.text()).ok()?;
    str_of(&value, "version").map(String::from)
}

/// The entry of a monolithium release since the vita release `previous`.
fn library_entry_of(root: &Path, previous: Option<&str>, url: Option<&str>) -> Option<Entry> {
    let previous = previous?;
    let file = format!("{LIBRARY}/package.json");
    let before = version_at(root, &format!("v{previous}"), &file)?;
    let now: Value = serde_json::from_str(&fs::read_to_string(root.join(&file)).ok()?).ok()?;
    let now = str_of(&now, "version").unwrap_or_default().to_string();
    if before == now {
        return None;
    }
    let changelog = format!("{LIBRARY}/CHANGELOG.md");
    let link = match url {
        Some(url) => format!("[{changelog}]({url}/-/blob/monolithium-v{now}/{changelog})"),
        None => changelog,
    };
    Some(Entry {
        kind: "changed".into(),
        area: "monolithium".into(),
        text: format!("released {now} (with utilities), see {link}"),
        ..Entry::default()
    })
}

fn is_fragment(file: &str) -> bool {
    file.starts_with(&format!("{FRAGMENTS}/")) && file.ends_with(".md") && file != format!("{FRAGMENTS}/README.md")
}

/// Whether an MR may go without a new fragment: `(is_ok, reason)`.
pub fn verdict_of(changed: &[(String, String)], labels: &[&str]) -> (bool, String) {
    let added: Vec<&str> = changed
        .iter()
        .filter(|(status, file)| status == "A" && is_fragment(file))
        .map(|(_, file)| file.as_str())
        .collect();
    if !added.is_empty() {
        return (true, format!("adds {}", added.join(", ")));
    }
    if labels.contains(&NO_CHANGELOG) {
        return (true, format!("labelled {NO_CHANGELOG}: no entry needed"));
    }
    let is_release = changed.iter().any(|(_, file)| file == CHANGELOG)
        && changed.iter().any(|(status, file)| status == "D" && is_fragment(file));
    if is_release {
        return (true, format!("folds the fragments into {CHANGELOG}"));
    }
    (false, format!("this MR adds no {FRAGMENTS}/*.md"))
}

fn root_of() -> std::path::PathBuf {
    let top = git(&["rev-parse", "--show-toplevel"], &super::repo::cwd())
        .text()
        .to_string();
    if top.is_empty() {
        fail("not inside the repo");
    }
    top.into()
}

fn entries_of(root: &Path) -> Vec<Entry> {
    read_fragments(root).unwrap_or_else(|problems| fail(&format!("{problems}\n  Format: {FRAGMENTS}/README.md")))
}

fn base_of(root: &Path) -> Option<String> {
    std::env::var("CI_MERGE_REQUEST_DIFF_BASE_SHA")
        .ok()
        .filter(|sha| !sha.is_empty())
        .or_else(|| {
            let base = breadboard_core::git::base_of(root).remote;
            Some(git(&["merge-base", &base, "HEAD"], root).text().to_string()).filter(|sha| !sha.is_empty())
        })
}

fn add(root: &Path, args: &Args) {
    let text = args.words(&["--type", "--area"]).join(" ").trim().to_string();
    if text.is_empty() {
        fail("the sentence is missing: pnpm changelog:add \"<what changed, for a user>\"");
    }
    let branch = git(&["branch", "--show-current"], root).text().to_string();
    let Some(name) = branch
        .strip_prefix("feature/")
        .or_else(|| branch.strip_prefix("fix/"))
        .filter(|name| !name.is_empty())
    else {
        fail(&format!(
            "{} is no feature/… or fix/… branch: write {FRAGMENTS}/<issue>-<slug>.md yourself",
            if branch.is_empty() { "HEAD" } else { &branch }
        ))
    };
    let base = base_of(root);
    let mut paths: Vec<String> = Vec::new();
    if let Some(base) = &base {
        paths.extend(
            git(&["diff", "--name-only", base, "HEAD"], root)
                .text()
                .split('\n')
                .map(String::from),
        );
    }
    paths.extend(
        git(&["diff", "--name-only", "HEAD"], root)
            .text()
            .split('\n')
            .map(String::from),
    );
    let mut areas: Vec<String> = Vec::new();
    for file in paths
        .iter()
        .filter(|file| !file.is_empty() && !file.starts_with(&format!("{FRAGMENTS}/")))
    {
        let area = area_of_path(file);
        if !areas.contains(&area) {
            areas.push(area);
        }
    }
    let named: Vec<&String> = areas.iter().filter(|area| *area != "repo").collect();
    let guessed: Vec<&String> = if named.is_empty() {
        areas.iter().collect()
    } else {
        named
    };
    let area = args
        .value("--area")
        .or_else(|| (guessed.len() == 1).then(|| guessed[0].clone()));
    let all = areas_of(root);
    let Some(area) = area else {
        fail(&format!(
            "which area? The branch changes {}: --area <{}>",
            if areas.is_empty() {
                "nothing yet".into()
            } else {
                areas.join(", ")
            },
            all.join("|")
        ))
    };
    let subject = base
        .map(|base| {
            git(&["log", "--reverse", "--format=%s", &format!("{base}..HEAD")], root)
                .text()
                .split('\n')
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .unwrap_or_default();
    let commit_type: String = subject
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let kind = args.value("--type").unwrap_or_else(|| match commit_type.as_str() {
        "feat" => "added".into(),
        "fix" => "fixed".into(),
        _ => "changed".into(),
    });
    let content = format!("---\ntype: {kind}\narea: {area}\n---\n\n{text}\n");
    let problems = problems_of(name, &content, &all);
    if !problems.is_empty() {
        fail(&problems.join("\n"));
    }
    let _ = fs::create_dir_all(root.join(FRAGMENTS));
    let mut file = format!("{FRAGMENTS}/{name}.md");
    let mut n = 2;
    while root.join(&file).exists() {
        file = format!("{FRAGMENTS}/{name}-{n}.md");
        n += 1;
    }
    if let Err(error) = fs::write(root.join(&file), &content) {
        fail(&format!("couldn't write {file}: {error}"));
    }
    outln!("✓ {file}\n\n{content}\n  type and area wrong? Edit the file; then commit it with the change.");
}

fn unreleased(root: &Path) {
    let entries = entries_of(root);
    if entries.is_empty() {
        outln!("Nothing unreleased: no fragments in .changelog/.");
        return;
    }
    outln!(
        "## [Unreleased]\n\n{}",
        section_body(&entries, &areas_of(root), project_url(root).as_deref())
    );
}

fn release(root: &Path, args: &Args) {
    let version = args.words(&[]).into_iter().next().unwrap_or_default();
    if !VERSION.is_match(&version) {
        fail(&version_refusal(&version, "release"));
    }
    let url = project_url(root);
    let file = root.join(CHANGELOG);
    let text = fs::read_to_string(&file).unwrap_or_else(|_| empty_changelog(url.as_deref()));
    let entries = entries_of(root);
    let with_links: Vec<Entry> = super::gitlab::in_turns(&entries, 8, |entry| Entry {
        mr: entry.file.as_deref().and_then(|file| mr_of_fragment(root, file)),
        ..entry.clone()
    });
    let mut all = with_links.clone();
    let versions = versions_of(&text);
    if let Some(library) = library_entry_of(root, versions.first().map(String::as_str), url.as_deref()) {
        all.push(library);
    }
    if all.is_empty() {
        fail(&format!("no fragments in {FRAGMENTS}/: nothing to release"));
    }
    let body = section_body(&all, &areas_of(root), url.as_deref());
    let folded = fold_into(
        &text,
        &version,
        &breadboard_core::time::day(breadboard_core::time::now_ms()),
        &body,
        url.as_deref(),
    )
    .unwrap_or_else(|error| fail(&error));
    if let Err(error) = fs::write(&file, folded) {
        fail(&format!("couldn't write {CHANGELOG}: {error}"));
    }
    for entry in &entries {
        if let Some(fragment) = &entry.file {
            let _ = fs::remove_file(root.join(fragment));
        }
    }
    let vita = root.join("products/vita/web/package.json");
    if let Ok(json) = fs::read_to_string(&vita) {
        static FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""version": "[^"]*""#).unwrap());
        let _ = fs::write(
            &vita,
            FIELD
                .replacen(&json, 1, format!("\"version\": \"{version}\"").as_str())
                .as_ref(),
        );
    }
    let unlinked = with_links.iter().filter(|entry| entry.mr.is_none()).count();
    let note = if unlinked > 0 {
        format!("\n  {unlinked} without an MR link (not merged yet, or glab couldn't say)")
    } else {
        String::new()
    };
    outln!(
        "✓ {CHANGELOG}: [{version}] with {}, {} deleted, vita at {version}{note}\n  Read it, then commit: chore(release): v{version}",
        counted(all.len(), "entry", "entries"),
        counted(entries.len(), "fragment", "fragments")
    );
}

/// `1 entry`, `2 entries`.
fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn notes(root: &Path, args: &Args) {
    let Some(version) = args.words(&[]).into_iter().next() else {
        fail(&version_refusal("", "notes"))
    };
    let text = fs::read_to_string(root.join(CHANGELOG)).ok();
    let plain = version.strip_prefix('v').unwrap_or(&version);
    match text.and_then(|text| notes_of(&text, plain)) {
        Some(found) => outln!("{found}"),
        None => fail(&format!("{CHANGELOG} has no section {version}")),
    }
}

fn check(root: &Path, args: &Args) {
    let entries = entries_of(root);
    outln!(
        "✓ {} fragment{} valid",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" }
    );
    if !args.has("--mr") {
        return;
    }
    let Some(base) = base_of(root) else {
        fail(&format!(
            "no base to compare with (fetch {})",
            breadboard_core::git::base_of(root).remote
        ))
    };
    let changed: Vec<(String, String)> = git(&["diff", "--name-status", "--no-renames", &base, "HEAD"], root)
        .text()
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (status, file) = line.split_once('\t').unwrap_or((line, ""));
            (status.to_string(), file.to_string())
        })
        .collect();
    let labels = std::env::var("CI_MERGE_REQUEST_LABELS").unwrap_or_default();
    let labels: Vec<&str> = labels.split(',').collect();
    let (is_ok, reason) = verdict_of(&changed, &labels);
    if !is_ok {
        fail(&format!(
            "{reason}. Add one (pnpm changelog:add \"<sentence>\", {FRAGMENTS}/README.md),\n  or label the MR {NO_CHANGELOG} when nobody would miss it in the changelog, then retry this job."
        ));
    }
    outln!("✓ {reason}");
}

/// `pnpm changelog:<sub>`.
pub fn command(sub: &str, args: &Args) -> ExitCode {
    if !["add", "unreleased", "release", "notes", "check"].contains(&sub) {
        fail(USAGE);
    }
    let root = root_of();
    match sub {
        "add" => add(&root, args),
        "unreleased" => unreleased(&root),
        "release" => release(&root, args),
        "notes" => notes(&root, args),
        _ => check(&root, args),
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_one_and_many() {
        assert_eq!(counted(1, "entry", "entries"), "1 entry");
        assert_eq!(counted(2, "entry", "entries"), "2 entries");
        assert_eq!(counted(1, "fragment", "fragments"), "1 fragment");
    }

    #[test]
    fn says_what_is_wrong_with_a_version() {
        assert_eq!(
            version_refusal("v1.5.0", "release"),
            "v1.5.0 is the tag, the version is 1.5.0: pnpm changelog:release 1.5.0"
        );
        assert_eq!(
            version_refusal("", "notes"),
            "the version is missing: pnpm changelog:notes <X.Y.Z>"
        );
        assert_eq!(
            version_refusal("1.5", "release"),
            "1.5 is no X.Y.Z version: pnpm changelog:release <X.Y.Z>"
        );
    }

    fn areas() -> Vec<String> {
        ["vita", "breadboard", "ci", "repo"]
            .iter()
            .map(|a| a.to_string())
            .collect()
    }

    #[test]
    fn checks_a_fragment() {
        assert!(problems_of("f", "---\ntype: added\narea: vita\n---\n\nIt works.\n", &areas()).is_empty());
        assert_eq!(
            problems_of("f", "---\ntype: big\narea: x # note\nwho: me\n---\n\n# H\n", &areas()),
            vec![
                "f: type \"big\" is none of added, changed, deprecated, removed, fixed, security",
                "f: area \"x\" is none of vita, breadboard, ci, repo",
                "f: unknown field who",
                "f: has a heading; an entry is a sentence or two",
            ]
        );
        assert_eq!(
            problems_of("f", "no", &areas()),
            vec!["f: starts with a frontmatter block (--- type: … area: … ---)"]
        );
        assert_eq!(area_of_path("packages/monocircuit-keycloak-theme/a"), "keycloak-theme");
        assert_eq!(area_of_path(".gitlab/ci/x.yml"), "ci");
        assert_eq!(issue_of_name(".changelog/291-x.md"), Some(291));
    }

    #[test]
    fn writes_a_section() {
        let entries = [
            Entry {
                file: Some("b".into()),
                kind: "fixed".into(),
                area: "vita".into(),
                text: "B.".into(),
                issue: Some(2),
                mr: None,
            },
            Entry {
                file: Some("a".into()),
                kind: "added".into(),
                area: "repo".into(),
                text: "A\nmore".into(),
                issue: None,
                mr: Some((5, "u/5".into())),
            },
        ];
        assert_eq!(
            section_body(&entries, &areas(), Some("https://g/p")),
            "### Added\n\n- **repo:** A\n  more ([!5](u/5))\n\n### Fixed\n\n- **vita:** B. ([#2](https://g/p/-/issues/2))"
        );
    }

    #[test]
    fn folds_a_release_in() {
        let text = empty_changelog(Some("https://g/p"));
        let folded = fold_into(&text, "1.0.0", "2026-10-01", "### Added\n\n- x", Some("https://g/p")).unwrap();
        assert!(folded.contains("## [Unreleased]\n\n## [1.0.0] - 2026-10-01\n\n### Added\n\n- x\n"));
        assert!(
            folded.ends_with(
                "[Unreleased]: https://g/p/-/compare/v1.0.0...develop\n[1.0.0]: https://g/p/-/tags/v1.0.0\n"
            )
        );
        let again = fold_into(&folded, "1.1.0", "2026-10-02", "### Fixed\n\n- y", Some("https://g/p")).unwrap();
        assert!(again.contains("[1.1.0]: https://g/p/-/compare/v1.0.0...v1.1.0\n[1.0.0]:"));
        assert_eq!(notes_of(&again, "1.0.0").as_deref(), Some("### Added\n\n- x"));
        assert_eq!(
            fold_into(&again, "1.1.0", "d", "b", None),
            Err("CHANGELOG.md has a section 1.1.0 already".into())
        );
    }

    #[test]
    fn judges_an_mr() {
        let changed = vec![("M".to_string(), "a".to_string())];
        assert_eq!(
            verdict_of(&changed, &[]),
            (false, "this MR adds no .changelog/*.md".into())
        );
        assert!(verdict_of(&changed, &["no-changelog"]).0);
        let added = vec![("A".to_string(), ".changelog/1-x.md".to_string())];
        assert_eq!(verdict_of(&added, &[]), (true, "adds .changelog/1-x.md".into()));
    }
}
