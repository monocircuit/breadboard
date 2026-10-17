//! `breadboard serve`: the one reader behind every live overview of the user
//! (rewrite-plan.md, "daemon"). Today every overview asks herdr every 2 s and
//! GitLab every 30 s by itself: N overviews, N times the calls. The daemon
//! asks once and every overview takes the answer:
//!
//! - herdr's reads (`pane list`, `tab list`, …): answered from one call made
//!   at most once in HERDR_FRESH for all overviews together (`client` puts
//!   core's `herdr::herdr` reads through it);
//! - git's key check of a worktree (`git status`, core's `reader::STATUS`):
//!   one call per worktree and stamp of its files in STALE_MS for all
//!   overviews together; what the key holds (merge base, diff, ahead,
//!   cherry, …) each overview keeps in its own memo while the key holds;
//! - GitLab: one feed per view (`feed`), read on the overview's cadence
//!   (`gitlab::read_gitlab`) and sent to every overview of that view; `r` in
//!   any of them reads again for all; an overview opened later gets the last
//!   answer at once;
//! - docker isn't the daemon's: core reads it once in 12 s for every process
//!   of the user through a file (`local::containers`, #331), with or without it.
//!
//! The first overview starts it (`client::connect`), it detaches, and it ends
//! GRACE after its last connection closed. One per user, protocol and the
//! programs it runs: the socket's name carries a key of them (`key`), so a
//! build of other sources never talks to another's daemon and a gauntlet
//! world (its own herdr, glab, HOME) never reaches the user's. It only reads
//! (rewrite rule 6): what an overview does stays in the overview, but for
//! what the user asked of it: the headless task agents (#358, `runs`), whose
//! queued lines it runs with `claude -p`, staying while any are queued or in
//! flight; and the CI queue (`ci`, #357), which creates the MR pipelines of a
//! repo in their order. An overview that can't reach it reads by itself.
//! And one more (#348, `keep_up`): it fast-forwards the main checkouts of
//! its overviews to origin/develop, when that is safe, and goes on in a
//! build installed over its binary; and it merges develop into each
//! orchestration's epic checkout once develop moved, pushed when the epic's
//! wt:check passes, else a look action of the orchestration (#352).

mod ci;
pub mod client;
mod feed;
pub mod gitlab;
mod keep_up;
pub mod runs;
mod serve;
pub mod watch;

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
    /// `git status` of a worktree under the stamp of its files (core's
    /// `reader::SharedStatus`): answered with one line, `[at, stdout]`
    /// (`null`: ask git here).
    Status { dir: PathBuf, stamp: u64 },
    /// The GitLab reads of a view (`view`: which one, `ask`: what its local
    /// read asks for): answered with a `Feed` a line while the connection
    /// stays open; the client's lines are `Said`.
    Gitlab {
        main: PathBuf,
        view: String,
        ask: RemoteAsk,
    },
    /// Lines were queued for the headless agents of `main` (#358): its
    /// engine runs from now on; answered with `true`.
    Runs { main: PathBuf },
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

/// The protocol of the socket: a hash of the sources of what goes over it (build.rs).
const PROTOCOL: &str = env!("BREADBOARD_PROTOCOL");

/// The programs the daemon's reads run, found on PATH.
const TOOLS: [&str; 4] = ["herdr", "glab", "git", "claude"];

/// The environment the daemon's reads depend on besides those programs.
const ENV: [&str; 8] = [
    "HOME",
    "XDG_CONFIG_HOME",
    "HERDR_ENV",
    "HERDR_SOCKET_PATH",
    "BREADBOARD_GITLAB_URL",
    "GITLAB_TOKEN",
    // A dry queue (a try) never shares the daemon of a live one.
    "BREADBOARD_CI_DRY",
    // The run engine's memory (#358): a world's low memory never reaches the user's daemon.
    "BREADBOARD_MEMINFO",
];

/// What the daemon must share with its clients: its protocol, the programs
/// its reads run (resolved: a PATH with node_modules/.bin in front, as `pnpm
/// overview` has it, runs the same herdr) and the environment they read.
/// Never the binary's path or build: every overview of the user (`pnpm
/// overview`, a tab's `'<exe>' overview` of any checkout, a copy) shares one
/// daemon while its sources' protocol is the same (rewrite-log, daemon round 4).
fn key() -> u64 {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let tools = TOOLS.map(|tool| breadboard_core::local::executable_on(&path, tool));
    let env = ENV.map(|name| (name, std::env::var_os(name)));
    key_of(PROTOCOL, &tools, &env)
}

fn key_of(protocol: &str, tools: &[Option<PathBuf>], env: &[(&str, Option<std::ffi::OsString>)]) -> u64 {
    let mut hasher = DefaultHasher::new();
    (protocol, tools, env).hash(&mut hasher);
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
    fn keys_the_daemon_by_what_answers_never_by_the_binary_or_path() {
        let herdr = Some(PathBuf::from("/usr/bin/herdr"));
        let env = |home: &str| [("HOME", Some(std::ffi::OsString::from(home)))];
        let mine = key_of("p1", std::slice::from_ref(&herdr), &env("/home/me"));
        // Another PATH that runs the same programs, another binary of the same sources: the same daemon.
        assert_eq!(key_of("p1", std::slice::from_ref(&herdr), &env("/home/me")), mine);
        // Another herdr (a world's fake), another HOME, another protocol: another daemon.
        assert_ne!(key_of("p1", &[Some("/w/bin/herdr".into())], &env("/home/me")), mine);
        assert_ne!(key_of("p1", std::slice::from_ref(&herdr), &env("/w/home")), mine);
        assert_ne!(key_of("p2", &[herdr], &env("/home/me")), mine);
        // A PATH with node_modules/.bin in front, as `pnpm overview` runs it, finds the same programs.
        let path = std::env::join_paths(["/nowhere/node_modules/.bin", "/usr/bin", "/bin"]).unwrap();
        let plain = std::env::join_paths(["/usr/bin", "/bin"]).unwrap();
        assert_eq!(
            breadboard_core::local::executable_on(&path, "sh"),
            breadboard_core::local::executable_on(&plain, "sh")
        );
        assert_eq!(PROTOCOL.len(), 16);
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
