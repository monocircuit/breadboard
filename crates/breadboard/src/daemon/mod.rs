//! `breadboard serve`: the one reader behind every live overview of the user
//! (rewrite-plan.md, "daemon"). Today every overview asks herdr every 2 s and
//! GitLab every 30 s by itself: N overviews, N times the calls. The daemon
//! asks once and every overview takes the answer:
//!
//! - herdr's reads (`pane list`, `tab list`, …): answered from one call made
//!   at most once in HERDR_FRESH for all overviews together (`client` puts
//!   core's `herdr::herdr` reads through it);
//! - GitLab: one feed per view (`feed`), read on the overview's cadence
//!   (`gitlab::read_gitlab`) and sent to every overview of that view; `r` in
//!   any of them reads again for all; an overview opened later gets the last
//!   answer at once;
//! - docker isn't the daemon's: core reads it once in 12 s for every process
//!   of the user through a file (`local::containers`, #331), with or without it.
//!
//! The first overview starts it (`client::connect`), it detaches, and it ends
//! GRACE after its last connection closed. One per binary and environment:
//! the socket's name carries a key of both, so a rebuilt binary never talks
//! to an old daemon and a gauntlet world (its own PATH, HOME, herdr) never
//! reaches the user's. It only reads (rewrite rule 6): what an overview does
//! stays in the overview. An overview that can't reach it reads by itself.

pub mod client;
mod feed;
pub mod gitlab;
mod serve;

pub use gitlab::{Feed, read_gitlab};
pub use serve::main;

use breadboard_core::reader::RemoteAsk;
use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// How old a herdr read may be for another overview: the looks of all overviews fall on the same even seconds.
pub const HERDR_FRESH: Duration = Duration::from_millis(1000);
/// How long the daemon stays after its last connection closed: an overview reloaded finds it warm.
pub const GRACE: Duration = Duration::from_secs(30);
/// How often the daemon says on a quiet feed that it is there (an empty line): a slow GitLab isn't a hung daemon.
pub const ALIVE_EVERY: Duration = Duration::from_secs(5);
/// The longest socket path the platforms take (`sun_path`, less its end).
const SOCKET_MAX: usize = 100;

/// Why an overview reads GitLab by itself after its daemon fell silent (its error line).
pub const SILENT_TEXT: &str = "breadboard serve stopped answering: this overview reads GitLab by itself now";

/// The first line a client sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// One herdr read: answered with one line, its result (`null`: none).
    Herdr { args: Vec<String> },
    /// The GitLab reads of a view (`view`: which one, `ask`: what its local
    /// read asks for): answered with a `Feed` a line while the connection
    /// stays open; the client's lines are `Said`.
    Gitlab {
        main: PathBuf,
        view: String,
        ask: RemoteAsk,
    },
}

/// What a client says on its GitLab feed after the request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Said {
    /// Its local read asks for something else now: the feed's next read takes it.
    Ask(RemoteAsk),
    /// `r`: read again now, full, for every overview of the view.
    Refresh,
}

/// The socket of the daemon of this binary and environment.
pub fn socket() -> &'static Path {
    static SOCKET: OnceLock<PathBuf> = OnceLock::new();
    SOCKET.get_or_init(|| socket_in(&breadboard_core::local::runtime_dir(), key()))
}

/// Whether the socket's folder is this user's alone (made if missing): in
/// one someone else made, no daemon is started or asked (its herdr reads
/// decide what the overview does).
pub fn is_socket_ours() -> bool {
    static IS_OURS: OnceLock<bool> = OnceLock::new();
    *IS_OURS.get_or_init(|| socket().parent().is_some_and(breadboard_core::local::own_dir))
}

/// `<dir>/serve-<key>.sock`, in `/tmp/breadboard-<uid>` where `dir` makes it too long for a socket.
fn socket_in(dir: &Path, key: u64) -> PathBuf {
    let name = format!("serve-{key:016x}.sock");
    let socket = dir.join(&name);
    if socket.as_os_str().len() <= SOCKET_MAX {
        return socket;
    }
    // SAFETY: getuid never fails.
    PathBuf::from(format!("/tmp/breadboard-{}", unsafe { libc::getuid() })).join(name)
}

/// What the daemon must share with its clients: the binary (its path and
/// build) and the environment its reads depend on.
fn key() -> u64 {
    let mut hasher = DefaultHasher::new();
    if let Ok(exe) = std::env::current_exe() {
        exe.hash(&mut hasher);
        if let Ok(meta) = std::fs::metadata(&exe) {
            meta.len().hash(&mut hasher);
            meta.modified().ok().hash(&mut hasher);
        }
    }
    for name in [
        "PATH",
        "HOME",
        "XDG_CONFIG_HOME",
        "HERDR_ENV",
        "HERDR_SOCKET_PATH",
        "BREADBOARD_GITLAB_URL",
        "GITLAB_TOKEN",
    ] {
        (name, std::env::var_os(name)).hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_socket_short_enough_to_bind() {
        let short = socket_in(Path::new("/run/user/1000/breadboard"), 0xab);
        assert_eq!(
            short,
            Path::new("/run/user/1000/breadboard/serve-00000000000000ab.sock")
        );
        let long = socket_in(&Path::new("/home").join("x".repeat(120)), 0xab);
        assert!(long.starts_with("/tmp"), "{}", long.display());
        assert!(long.as_os_str().len() <= SOCKET_MAX);
    }

    #[test]
    fn says_its_requests_in_one_line_each() {
        let request = Request::Herdr {
            args: vec!["pane".into(), "list".into()],
        };
        let line = serde_json::to_string(&request).unwrap();
        assert_eq!(line, r#"{"kind":"herdr","args":["pane","list"]}"#);
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);
        let request = Request::Gitlab {
            main: "/repo".into(),
            view: "everything".into(),
            ask: RemoteAsk::Worktree { branch: None },
        };
        let line = serde_json::to_string(&request).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);
        assert_eq!(serde_json::to_string(&Said::Refresh).unwrap(), r#""refresh""#);
    }
}
