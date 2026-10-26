//! GitLab as these commands read and write it: through `glab`, exactly the
//! calls today's breadboard makes (rewrite rule 6), in the main checkout,
//! whose remote names the project.

use super::proc::{Wait, output_within};
use breadboard_core::git::Out;
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long one read may take: core's client gives a call as long. Past it
/// GitLab counts as not answering, as one that refuses (#306); today's
/// `glab` waited without end.
pub const LIMIT: Duration = Duration::from_secs(15);

/// How long a read may take that only names something in a refusal.
pub const NAMING: Duration = Duration::from_secs(5);

/// Set once a read went unanswered to its limit: the rest of this run asks
/// no more (a wt:prune of five merged worktrees waited 15 s for each).
static IS_SILENT: AtomicBool = AtomicBool::new(false);

/// `glab <args>` within `limit`, or not at all once GitLab was silent.
fn glab(main: &Path, args: &[&str], what: &str, limit: Duration) -> Out {
    if IS_SILENT.load(Ordering::Relaxed) {
        return Out {
            is_ok: false,
            status: -1,
            stdout: String::new(),
            stderr: "timeout: GitLab was silent before in this run".into(),
        };
    }
    // A 429 paused GitLab for every caller (#462): nothing is asked before it ends.
    if let Some(pause) = breadboard_core::gitlab::budget::glab_may("glab", args) {
        return Out {
            is_ok: false,
            status: -1,
            stdout: String::new(),
            stderr: pause,
        };
    }
    let mut out = output_within("glab", args, main, &wait_for(what, limit));
    if !out.is_ok
        && let Some(pause) = breadboard_core::gitlab::budget::glab_failed(&out.stderr)
    {
        out.stderr = pause;
    }
    if out.stderr.starts_with("timeout") {
        IS_SILENT.store(true, Ordering::Relaxed);
    }
    out
}

/// The wait of a read of `what` (`the MRs of fix/303-x`).
fn wait_for(what: &str, limit: Duration) -> Wait {
    Wait::new(format!("GitLab ({what})"), limit)
}

/// `encodeURIComponent`: everything but `A-Z a-z 0-9 - _ . ! ~ * ' ( )` as `%XX`.
pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// `glab api <endpoint>` → its JSON, None when glab fails, answers none or
/// not within `limit`; `what` it reads is said while it waits.
pub fn api(main: &Path, endpoint: &str, what: &str, limit: Duration) -> Option<Value> {
    let out = glab(main, &["api", endpoint], what, limit);
    if !out.is_ok {
        return None;
    }
    serde_json::from_str(&out.stdout).ok()
}

/// Why `view` read nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unread {
    /// GitLab answered that it is not there (404).
    Missing,
    /// GitLab did not answer, or not with JSON.
    Unanswered,
}

/// What glab's `stderr` says about a failed read.
fn unread_of(stderr: &str) -> Unread {
    let text = stderr.to_lowercase();
    if text.contains("404") || text.contains("not found") {
        Unread::Missing
    } else {
        Unread::Unanswered
    }
}

/// `glab <args> -F json` of an issue or an MR (`issue view 12`, `mr view 3`),
/// `what` it reads said while it waits.
pub fn view(main: &Path, args: &[&str], what: &str) -> Result<Value, Unread> {
    let mut all = args.to_vec();
    all.extend(["-F", "json"]);
    let out = glab(main, &all, what, LIMIT);
    if !out.is_ok {
        return Err(unread_of(&out.stderr));
    }
    serde_json::from_str(&out.stdout).map_err(|_| Unread::Unanswered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_404_is_missing_anything_else_unanswered() {
        assert_eq!(
            unread_of("glab: 404 {\"message\":\"404 Issue Not Found\"}"),
            Unread::Missing
        );
        assert_eq!(
            unread_of("error connecting to gitlab.com: dial tcp: connection refused"),
            Unread::Unanswered
        );
        assert_eq!(unread_of(""), Unread::Unanswered);
        assert_eq!(unread_of("timeout: no answer in 15 s"), Unread::Unanswered);
    }

    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(encode("fix/303-a_b.c"), "fix%2F303-a_b.c");
        assert_eq!(encode("ä b"), "%C3%A4%20b");
    }
}
