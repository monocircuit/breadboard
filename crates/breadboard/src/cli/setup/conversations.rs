//! The claude conversations the setup commands touch (conversations.mjs):
//! core's reading of them (`breadboard_core::conversations`), the lifecycle
//! group's sharing of the projects (every worktree's project a link to the
//! main checkout's, so `/resume` there lists them all), and what only `--end`
//! does: a conversation kept for the main checkout.

use super::repo;
use crate::cli::lifecycle::claude::{is_conversation_running, project_of};
use breadboard_core::conversations::transcript_of;
use std::fs;
use std::time::{Duration, Instant};

pub use breadboard_core::conversations::{bound_session, left_in, running_claudes, session_file};

/// Every worktree's project a link to the main checkout's; what moved. Never fails.
pub fn share_conversations() -> Vec<String> {
    let main = repo::main_dir();
    let trees: Vec<_> = repo::worktrees().into_iter().skip(1).map(|tree| tree.dir).collect();
    crate::cli::lifecycle::claude::share_conversations(&main, &trees)
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
