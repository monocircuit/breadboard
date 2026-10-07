//! The claude conversations the setup commands touch (conversations.mjs):
//! every worktree's project a link to the main checkout's, so `/resume`
//! there lists them all; whether a conversation runs; how much of its
//! context it used; a conversation kept for the main checkout.

use super::repo;
use breadboard_core::json::str_of;
use breadboard_core::lock::is_alive;
use serde_json::Value;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// claude's config directory: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".claude")
        })
}

/// The project claude keeps the transcripts of `dir` in.
pub fn project_of(dir: &Path) -> PathBuf {
    let name: String = dir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    claude_dir().join("projects").join(name)
}

/// Where the conversation bound to a worktree's space is written down.
pub fn session_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("herdr-session")
}

/// The conversation bound to a worktree's space.
pub fn bound_session(dir: &Path) -> Option<String> {
    let id = fs::read_to_string(session_file(dir)).ok()?.trim().to_string();
    breadboard_core::record::is_session_id(&id).then_some(id)
}

/// Everything of `from` into `to` (folders merged, a file `to` has arrives
/// as `<name>.from-<suffix>`). True when `from` ended up empty and is gone.
fn merge_into(from: &Path, to: &Path, suffix: &str) -> std::io::Result<bool> {
    for entry in fs::read_dir(from)?.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        match fs::symlink_metadata(&target) {
            Err(_) => fs::rename(&source, &target)?,
            Ok(existing) if entry.file_type()?.is_dir() && existing.is_dir() => {
                merge_into(&source, &target, suffix)?;
            }
            Ok(_) => {
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

const LIVE: Duration = Duration::from_secs(10 * 60);

fn link_projects() -> std::io::Result<Vec<String>> {
    let home = project_of(&repo::main_dir());
    let projects = home.parent().map(Path::to_path_buf).unwrap_or_default();
    fs::create_dir_all(&home)?;
    let home_name = home.file_name().map(|name| name.to_os_string()).unwrap_or_default();
    let link = |project: &Path| -> std::io::Result<()> {
        if fs::symlink_metadata(project).is_ok() {
            return Ok(());
        }
        std::os::unix::fs::symlink(&home_name, project)
    };
    for tree in repo::worktrees().iter().skip(1) {
        link(&project_of(&tree.dir))?;
    }
    let prefix = format!("{}-", home_name.to_string_lossy());
    let mut merged = Vec::new();
    for entry in fs::read_dir(&projects)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type()?.is_dir() || !name.starts_with(&prefix) {
            continue;
        }
        let project = entry.path();
        let is_live = fs::read_dir(&project)?.flatten().any(|file| {
            file.file_name().to_string_lossy().ends_with(".jsonl")
                && file
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|at| SystemTime::now().duration_since(at).ok())
                    .is_some_and(|age| age < LIVE)
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

/// Every worktree's project a link to the main checkout's; what moved. Never fails.
pub fn share_conversations() -> Vec<String> {
    link_projects().unwrap_or_default()
}

/// The transcript of a conversation, in whichever project it was started.
pub fn transcript_of(id: &str) -> Option<PathBuf> {
    let projects = claude_dir().join("projects");
    fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(format!("{id}.jsonl")))
        .find(|file| file.exists())
}

/// The claudes running here, from `~/.claude/sessions/*.json`: `(pid, id, name, cwd)`.
pub fn running_claudes() -> Vec<(i64, String, Option<String>, String)> {
    let Ok(files) = fs::read_dir(claude_dir().join("sessions")) else {
        return Vec::new();
    };
    files
        .flatten()
        .filter(|file| file.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|file| {
            let value: Value = serde_json::from_str(&fs::read_to_string(file.path()).ok()?).ok()?;
            let pid = value.get("pid")?.as_i64()?;
            if !is_alive(pid) {
                return None;
            }
            let id = str_of(&value, "sessionId").unwrap_or_default().to_string();
            let cwd = fs::read_link(format!("/proc/{pid}/cwd"))
                .map(|dir| dir.to_string_lossy().into_owned())
                .unwrap_or_else(|_| str_of(&value, "cwd").unwrap_or_default().to_string());
            Some((pid, id, str_of(&value, "name").map(String::from), cwd))
        })
        .collect()
}

/// Whether a claude runs the conversation already, anywhere on this machine.
pub fn is_conversation_running(id: &str) -> bool {
    running_claudes().iter().any(|(_, running, _, _)| running == id)
}

fn context_window() -> u64 {
    let settings = fs::read_to_string(claude_dir().join("settings.json")).unwrap_or_default();
    let model = serde_json::from_str::<Value>(&settings)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .map(|model| model.as_str().map(String::from).unwrap_or_else(|| model.to_string()))
        })
        .unwrap_or_default();
    if model.contains("[1m]") { 1_000_000 } else { 200_000 }
}

/// `(used, window)` of the last answer in `lines`, oldest first.
pub fn usage_of(lines: &[&str]) -> (u64, u64) {
    let last = lines
        .iter()
        .rev()
        .filter(|line| line.contains("\"usage\""))
        .find_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            let message = entry.get("message")?;
            let is_answer = str_of(&entry, "type") == Some("assistant")
                && !entry.get("isSidechain").and_then(Value::as_bool).unwrap_or(false)
                && str_of(message, "model") != Some("<synthetic>");
            is_answer.then(|| message.get("usage").cloned()).flatten()
        });
    let number = |usage: &Value, key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let used = last.map_or(0, |usage| {
        number(&usage, "input_tokens")
            + number(&usage, "cache_creation_input_tokens")
            + number(&usage, "cache_read_input_tokens")
            + number(&usage, "output_tokens")
    });
    (used, if used > 200_000 { 1_000_000 } else { context_window() })
}

/// The share of its window (0 to 1) the conversation `id` used at its last answer.
pub fn context_share_of(id: &str) -> Option<f64> {
    let transcript = transcript_of(id)?;
    let mut file = fs::File::open(transcript).ok()?;
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

/// The transcript of a conversation that ended into the main checkout's
/// project (waiting up to `wait` while it still runs). What moved, or None.
pub fn keep_conversation(id: &str, wait: Duration) -> Option<String> {
    let until = Instant::now() + wait;
    while is_conversation_running(id) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    let transcript = transcript_of(id)?;
    if is_conversation_running(id) {
        return None;
    }
    let home = project_of(&repo::main_dir());
    let from = transcript.parent()?.to_path_buf();
    fs::create_dir_all(&home).ok()?;
    if fs::canonicalize(&from).ok()? == fs::canonicalize(&home).ok()? {
        return None;
    }
    for name in [format!("{id}.jsonl"), id.to_string()] {
        let source = from.join(&name);
        let target = home.join(&name);
        if source.exists() && !target.exists() {
            let _ = fs::rename(source, target);
        }
    }
    Some(format!("the conversation {id} into the main checkout's project"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_last_answers_usage() {
        let lines = [
            r#"{"type":"assistant","message":{"model":"m","usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":100}}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"usage":{"input_tokens":999999}}}"#,
        ];
        assert_eq!(usage_of(&lines).0, 200);
    }

    #[test]
    fn names_the_project_of_a_directory() {
        assert!(project_of(Path::new("/a/b.c")).ends_with("-a-b-c"));
    }
}
