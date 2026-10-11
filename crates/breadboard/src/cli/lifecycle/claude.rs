//! The claude conversations of the worktrees (worktree/conversations.mjs):
//! the one bound to a worktree's space, its transcript and context, the
//! claudes running on this machine, and every conversation kept in the main
//! checkout's project. core has most of this written (conversations.rs) but
//! does not export it yet: these go once it does.

use breadboard_core::json::{self, int_of, str_of};
use breadboard_core::lock::is_alive;
use breadboard_core::record::is_session_id;
use serde_json::Value;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Where the claude conversation bound to a worktree's space is written down.
pub fn session_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("herdr-session")
}

/// The conversation bound to a worktree's space, its id or None.
pub fn bound_session(dir: &Path) -> Option<String> {
    let id = fs::read_to_string(session_file(dir)).ok()?.trim().to_string();
    is_session_id(&id).then_some(id)
}

/// claude's config directory: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude"))
}

/// The project claude keeps a directory's transcripts in: its path with
/// every other character a `-`.
pub fn project_of(dir: &Path) -> PathBuf {
    let name: String = dir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    claude_dir().join("projects").join(name)
}

/// The transcript of a conversation, in whichever project it was started.
pub fn transcript_of(id: &str) -> Option<PathBuf> {
    let projects = claude_dir().join("projects");
    let mut names: Vec<_> = fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|project| project.join(format!("{id}.jsonl")))
        .find(|file| file.exists())
}

/// The context window: 1M with a `[1m]` model in claude's settings, else 200k.
fn context_window() -> u64 {
    let model = json::read(&claude_dir().join("settings.json"))
        .and_then(|settings| settings.get("model").map(|model| model.to_string()))
        .unwrap_or_default();
    if model.contains("[1m]") { 1_000_000 } else { 200_000 }
}

/// `(used, window)` of the last answer in `lines` (oldest first): the prompt
/// it read, cached or not, plus what it wrote (conversations.mjs `usageOf`).
pub fn usage_of(lines: &[&str]) -> (u64, u64) {
    let last = lines
        .iter()
        .rev()
        .filter(|line| line.contains("\"usage\""))
        .find_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            let message = entry.get("message")?;
            let is_answer = str_of(&entry, "type") == Some("assistant")
                && !json::is_truthy(entry.get("isSidechain"))
                && message.get("usage").is_some_and(|usage| !usage.is_null())
                && str_of(message, "model") != Some("<synthetic>");
            is_answer.then(|| message.get("usage").cloned()).flatten()
        });
    let used = last
        .map(|usage| {
            [
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
                "output_tokens",
            ]
            .iter()
            .map(|key| usage.get(key).and_then(Value::as_u64).unwrap_or(0))
            .sum()
        })
        .unwrap_or(0);
    // Past 200k it can only be the 1M window, whatever the settings say now.
    (used, if used > 200_000 { 1_000_000 } else { context_window() })
}

/// The share of its window (0 to 1) the conversation `id` had used at its
/// last answer, from the last 512 KiB of its transcript; None without one.
pub fn context_share_of(id: &str) -> Option<f64> {
    let mut file = fs::File::open(transcript_of(id)?).ok()?;
    let size = file.metadata().ok()?.len();
    let length = size.min(1 << 19);
    file.seek(SeekFrom::Start(size - length)).ok()?;
    let mut buffer = Vec::new();
    file.take(length).read_to_end(&mut buffer).ok()?;
    let text = String::from_utf8_lossy(&buffer);
    let lines: Vec<&str> = text.split('\n').collect();
    let (used, window) = usage_of(&lines);
    Some((used as f64 / window as f64).min(1.0))
}

/// A claude running on this machine: `cwd` where its process stands now.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub pid: i64,
    pub id: String,
    pub name: Option<String>,
    pub cwd: PathBuf,
}

/// The session files of the claudes, each as read.
fn sessions() -> Vec<Value> {
    let Ok(entries) = fs::read_dir(claude_dir().join("sessions")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|entry| json::read(&entry.path()))
        .collect()
}

/// The claudes running on this machine, from `~/.claude/sessions`.
pub fn running_claudes() -> Vec<Running> {
    sessions()
        .into_iter()
        .filter_map(|session| {
            let pid = int_of(&session, "pid").filter(|pid| is_alive(*pid))?;
            let written = str_of(&session, "cwd").map(PathBuf::from);
            Some(Running {
                pid,
                id: str_of(&session, "sessionId").unwrap_or_default().to_string(),
                name: str_of(&session, "name").map(String::from),
                cwd: fs::read_link(format!("/proc/{pid}/cwd"))
                    .ok()
                    .or(written)
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Whether a claude runs the conversation already, anywhere on this machine:
/// a second resume would write into the same transcript.
pub fn is_conversation_running(id: &str) -> bool {
    sessions()
        .iter()
        .any(|session| str_of(session, "sessionId") == Some(id) && int_of(session, "pid").is_some_and(is_alive))
}

/// The claudes that stand in the worktree `dir` without being its agent: a
/// handover bound them elsewhere and they haven't left yet (#252).
pub fn left_in(dir: &Path, claudes: &[Running], bound: Option<&str>) -> Vec<Running> {
    claudes
        .iter()
        .filter(|claude| Some(claude.id.as_str()) != bound && claude.cwd.starts_with(dir))
        .cloned()
        .collect()
}

/// Whether every one of `left` (standing in `dir`) was handed over: its
/// conversation is bound to another of `dirs`, it is on its way there (#252).
pub fn are_handed_over(left: &[Running], dir: &Path, dirs: &[&Path]) -> bool {
    let bound: Vec<String> = dirs
        .iter()
        .filter(|other| **other != dir)
        .filter_map(|other| bound_session(other))
        .collect();
    !left.is_empty() && left.iter().all(|claude| bound.contains(&claude.id))
}

/// `244-x (claude 1234)`, `claude 1234`: the claudes left_in found, each
/// with the process that ends it (today's named an unnamed one only by it).
pub fn left_text(left: &[Running]) -> String {
    left.iter()
        .map(|claude| match &claude.name {
            Some(name) => format!("{name} (claude {})", claude.pid),
            None => format!("claude {}", claude.pid),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Why wt:remove refuses `feature` while the claudes `left` stand in it, and
/// how it goes on: a handed-over one leaves by itself, any other is ended or
/// leaves by hand.
pub fn left_refusal(left: &[Running], feature: &str, is_handed_over: bool) -> String {
    let who = left_text(left);
    if is_handed_over {
        return format!(
            "{who} still stands in {feature}: it restarts in its next worktree, or as a free agent its claude ends; again once it left"
        );
    }
    let pids: Vec<String> = left.iter().map(|claude| claude.pid.to_string()).collect();
    format!(
        "{who} still stands in {feature}, handed over nowhere: end it (/exit in its tab, or kill {}) or let it leave the folder, then again",
        pids.join(" ")
    )
}

/// The directory `claude --resume` finds a conversation from: the last one
/// it worked in, else `dir`.
pub fn session_cwd(id: &str, dir: &Path) -> PathBuf {
    let text = transcript_of(id)
        .and_then(|file| fs::read_to_string(file).ok())
        .unwrap_or_default();
    let last = text.rmatch_indices("\"cwd\":\"").next().and_then(|(at, found)| {
        let rest = &text[at + found.len()..];
        rest.find('"').map(|end| PathBuf::from(&rest[..end]))
    });
    last.filter(|cwd| cwd.exists()).unwrap_or_else(|| dir.to_path_buf())
}

/// A transcript written to within this time may belong to a running
/// conversation: its project is not moved.
const LIVE: Duration = Duration::from_secs(10 * 60);

/// Everything of `from` moved into `to`, a folder both have merged the same
/// way; a file `to` has already arrives as `<name>.from-<suffix>`. Whether
/// `from` ended up empty and is gone.
fn merge_into(from: &Path, to: &Path, suffix: &str) -> std::io::Result<bool> {
    let mut entries: Vec<_> = fs::read_dir(from)?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source = entry.path();
        let target = to.join(entry.file_name());
        let existing = fs::symlink_metadata(&target).ok();
        match existing {
            None => fs::rename(&source, &target)?,
            Some(meta) if meta.is_dir() && entry.file_type()?.is_dir() => {
                merge_into(&source, &target, suffix)?;
            }
            Some(_) => {
                let renamed = PathBuf::from(format!("{}.from-{suffix}", target.display()));
                if !renamed.exists() {
                    fs::rename(&source, renamed)?;
                }
            }
        }
    }
    if fs::read_dir(from)?.next().is_some() {
        return Ok(false);
    }
    fs::remove_dir(from)?;
    Ok(true)
}

/// Every conversation of the repo kept in the main checkout's project, so
/// `/resume` there lists them all: a worktree's project is a relative link
/// to it, an older project directory is merged in and replaced by the link
/// once none of its transcripts was written lately. Only the projects of
/// worktree roots (`<main>-<feature>`, gone ones too) are merged: a session
/// started in a subdirectory (`platform/packages/breadboard`,
/// `platform-314-x/.claude/…`) has a project named like one and stays its
/// own, where today's merged it too. Never fails (comfort); returns the
/// suffixes of the projects that moved.
pub fn share_conversations(main: &Path, trees: &[PathBuf]) -> Vec<String> {
    link_projects(main, trees).unwrap_or_default()
}

fn link_projects(main: &Path, trees: &[PathBuf]) -> std::io::Result<Vec<String>> {
    let home = project_of(main);
    let projects = home.parent().map(Path::to_path_buf).unwrap_or_default();
    fs::create_dir_all(&home)?;
    let home_name = home
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let link = |project: &Path| -> std::io::Result<()> {
        if fs::symlink_metadata(project).is_ok() {
            return Ok(());
        }
        std::os::unix::fs::symlink(&home_name, project)
    };
    for dir in trees {
        link(&project_of(dir))?;
    }
    let mut merged = Vec::new();
    let prefix = format!("{home_name}-");
    let mut entries: Vec<_> = fs::read_dir(&projects)?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type()?.is_dir() || !name.starts_with(&prefix) {
            continue;
        }
        let project = entry.path();
        if !is_tree_project(&project, main) {
            continue;
        }
        let is_live = fs::read_dir(&project)?.flatten().any(|file| {
            file.file_name().to_string_lossy().ends_with(".jsonl")
                && file
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .is_ok_and(|at| SystemTime::now().duration_since(at).unwrap_or_default() < LIVE)
        });
        let suffix = &name[prefix.len()..];
        if is_live || !merge_into(&project, &home, suffix)? {
            continue;
        }
        link(&project)?;
        merged.push(suffix.to_string());
    }
    Ok(merged)
}

/// Whether `project` holds the conversations of a worktree root of `main`
/// (`<main>-<feature>`, there or gone): the `cwd` its transcripts started in
/// says so, the name alone can't (`/` and `-` both become `-`).
fn is_tree_project(project: &Path, main: &Path) -> bool {
    let Some(cwd) = cwd_of(project) else {
        return false;
    };
    let prefix = format!("{}-", main.file_name().unwrap_or_default().to_string_lossy());
    cwd.parent() == main.parent()
        && cwd
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
        && project_of(&cwd).file_name() == project.file_name()
}

/// The `cwd` of the first transcript line in `project` that has one.
fn cwd_of(project: &Path) -> Option<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(project)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|file| file.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    files.sort();
    files.iter().find_map(|file| {
        let mut head = String::new();
        fs::File::open(file)
            .ok()?
            .take(256 * 1024)
            .read_to_string(&mut head)
            .ok();
        head.lines().find_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            Some(PathBuf::from(value.get("cwd")?.as_str()?))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_the_context_of_the_last_real_answer() {
        let lines = [
            r#"{"type":"assistant","message":{"model":"m","usage":{"input_tokens":10,"output_tokens":5}}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"m","usage":{"input_tokens":999}}}"#,
            r#"{"type":"assistant","message":{"model":"<synthetic>","usage":{"input_tokens":999}}}"#,
        ];
        assert_eq!(usage_of(&lines).0, 15);
        let big = [r#"{"type":"assistant","message":{"model":"m","usage":{"input_tokens":300000,"output_tokens":0}}}"#];
        assert_eq!(usage_of(&big), (300_000, 1_000_000));
    }

    #[test]
    fn left_in_skips_the_bound_one_and_other_dirs() {
        let at = |id: &str, cwd: &str| Running {
            pid: 1,
            id: id.into(),
            name: None,
            cwd: cwd.into(),
        };
        let claudes = [at("a", "/r/p-4"), at("b", "/r/p-4/x"), at("c", "/r/p-40")];
        let left = left_in(Path::new("/r/p-4"), &claudes, Some("a"));
        assert_eq!(left.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert_eq!(left_text(&left), "claude 1");
        let named = [Running {
            pid: 4242,
            id: "s".into(),
            name: Some("some-agent".into()),
            cwd: "/r/p-4".into(),
        }];
        assert_eq!(
            left_refusal(&named, "4-x", false),
            "some-agent (claude 4242) still stands in 4-x, handed over nowhere: end it (/exit in its tab, or kill 4242) or let it leave the folder, then again"
        );
        assert!(
            left_refusal(&left, "4-x", true).starts_with("claude 1 still stands in 4-x: it enters its next worktree")
        );
        // Bound nowhere else: not on its way anywhere.
        assert!(!are_handed_over(
            &left,
            Path::new("/r/p-4"),
            &[Path::new("/nowhere/p-5")]
        ));
        assert!(!are_handed_over(&[], Path::new("/r/p-4"), &[]));
    }

    #[test]
    fn merges_only_the_projects_of_worktree_roots() {
        let root = std::env::temp_dir().join(format!("lifecycle-projects-{}", std::process::id()));
        let main = root.join("platform");
        let project = |cwd: &Path| {
            let dir = root.join("projects").join(project_of(cwd).file_name().unwrap());
            fs::create_dir_all(&dir).unwrap();
            let line = serde_json::json!({ "type": "user", "cwd": cwd }).to_string();
            fs::write(dir.join("a.jsonl"), format!("{{\"type\":\"summary\"}}\n{line}\n")).unwrap();
            dir
        };
        assert!(is_tree_project(&project(&root.join("platform-303-x")), &main));
        assert!(!is_tree_project(&project(&main.join("packages/breadboard")), &main));
        assert!(!is_tree_project(
            &project(&root.join("platform-314-x/.claude/gauntlet/clones/cli")),
            &main
        ));
        assert!(!is_tree_project(&project(&root.join("other-1")), &main));
        let empty = root.join("projects/-x-platform-9");
        fs::create_dir_all(&empty).unwrap();
        assert!(!is_tree_project(&empty, &main));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_project_is_the_path_with_dashes() {
        let project = project_of(Path::new("/home/u/platform-3"));
        assert!(project.ends_with("projects/-home-u-platform-3"));
    }
}
