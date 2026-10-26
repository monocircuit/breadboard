//! GitLab reads of this group: `glab api <endpoint>` (a GET) in the main
//! checkout, so glab finds the host, the project (`:fullpath`) and the login
//! itself.

use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

/// Calls in flight at once: GitLab does not like a burst of a hundred.
const PARALLEL: usize = 10;

/// A GET of `endpoint`, parsed; Err says why in the words of `GitlabError`:
/// `GitLab: <reason> (<endpoint>)`.
pub fn get(main: &Path, endpoint: &str) -> Result<Value, String> {
    glab_api(main, &[endpoint], endpoint)
}

/// `glab api <args>`, its answer parsed; `what` names the call in an error.
fn glab_api(main: &Path, args: &[&str], what: &str) -> Result<Value, String> {
    let failed = |reason: &str| format!("GitLab: {reason} ({what})");
    let mut asked = vec!["api"];
    asked.extend_from_slice(args);
    if let Some(pause) = breadboard_core::gitlab::budget::glab_may("glab", &asked) {
        return Err(pause);
    }
    let output = Command::new("glab")
        .arg("api")
        .args(args)
        .current_dir(main)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => failed("glab is not installed (https://gitlab.com/gitlab-org/cli)"),
            _ => failed(&format!("glab did not start: {error}")),
        })?;
    if !output.status.success() {
        if let Some(pause) = breadboard_core::gitlab::budget::glab_failed(&String::from_utf8_lossy(&output.stderr)) {
            return Err(pause);
        }
        let reason = reason_of(&String::from_utf8_lossy(&output.stderr))
            .unwrap_or_else(|| format!("glab api exited with {}", output.status));
        return Err(failed(&reason));
    }
    serde_json::from_slice(&output.stdout).map_err(|_| failed("its answer was no JSON"))
}

/// What a failed `glab api` says went wrong, in one line: glab's own
/// (`glab: 502 Bad Gateway`), else the error a stack trace names (its cause
/// first), else its first line that isn't the trace's frame.
fn reason_of(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    let is_error = |line: &&&str| {
        let line = line.trim_start_matches('[');
        line.split_once(": ")
            .is_some_and(|(name, _)| name.ends_with("Error") && name.chars().all(char::is_alphanumeric))
    };
    let found = lines
        .iter()
        .find(|line| line.starts_with("glab:") || line.starts_with("ERROR"))
        .map(|line| line.to_string())
        .or_else(|| {
            lines
                .iter()
                .find_map(|line| line.strip_prefix("[cause]:"))
                .map(|cause| cause.trim().to_string())
        })
        .or_else(|| {
            lines.iter().find(is_error).map(|line| {
                line.trim_start_matches('[')
                    .trim_end_matches(['{', ' '])
                    .trim_end_matches(']')
                    .to_string()
            })
        })
        .or_else(|| {
            lines
                .iter()
                .find(|line| {
                    !line.starts_with("node:")
                        && !line.starts_with("at ")
                        && !line.starts_with("Node.js ")
                        && !line.starts_with('^')
                        && !line.contains("triggerUncaughtException")
                })
                .map(|line| line.to_string())
        })?;
    // A long answer of GitLab's (its JSON) in its first words.
    Some(if found.chars().count() > 160 {
        format!("{}…", found.chars().take(159).collect::<String>())
    } else {
        found
    })
}

/// A GitLab error (`GitLab: <reason> (<call>)`) in words a person acts on:
/// what went wrong, the reason as glab gave it; and what to try.
pub fn explain(error: &str) -> (String, String) {
    let error = error.strip_prefix("GitLab: ").unwrap_or(error);
    let (reason, call) = match error.rsplit_once(" (") {
        Some((reason, call)) if call.ends_with(')') => (reason, Some(call.trim_end_matches(')'))),
        _ => (error, None),
    };
    let lower = reason.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    let what = if has(&["not installed"]) {
        return (
            reason.to_string(),
            "install glab: https://gitlab.com/gitlab-org/cli".into(),
        );
    } else if has(&["401", "unauthorized", "token", "auth"]) {
        "glab is not logged in, or its login expired"
    } else if has(&["403", "forbidden"]) {
        "this login may not read the project"
    } else if has(&["404", "not found"]) {
        "GitLab doesn't know the project origin names"
    } else if has(&["429", "rate limit"]) {
        "GitLab's rate limit is reached"
    } else if has(&["500", "502", "503", "504", "bad gateway", "unavailable"]) {
        "GitLab has trouble answering"
    } else if has(&[
        "port",
        "refused",
        "no such host",
        "not resolve",
        "enotfound",
        "dial tcp",
        "timeout",
        "timed out",
        "network",
        "fetch failed",
        "unreachable",
        "invalid url",
    ]) {
        "GitLab can't be reached"
    } else {
        "glab failed"
    };
    let asked = call.map(|call| format!("glab api {call} failed: ")).unwrap_or_default();
    (
        format!("{what} ({reason})"),
        format!("{asked}check the login (glab auth status) and the network"),
    )
}

/// A GET that must answer a list.
pub fn list(main: &Path, endpoint: &str) -> Result<Vec<Value>, String> {
    match get(main, endpoint)? {
        Value::Array(items) => Ok(items),
        _ => Err(format!("GitLab: its answer was no list ({endpoint})")),
    }
}

/// Every endpoint's list, side by side, PARALLEL at a time, in their order.
pub fn lists(main: &Path, endpoints: &[String]) -> Vec<Result<Vec<Value>, String>> {
    each(endpoints, |endpoint| list(main, endpoint))
}

/// `read` of every endpoint, PARALLEL at a time, in their order.
fn each<T: Send>(endpoints: &[String], read: impl Fn(&str) -> T + Sync) -> Vec<T> {
    let read = &read;
    endpoints
        .chunks(PARALLEL)
        .flat_map(|chunk| {
            thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|endpoint| scope.spawn(move || read(endpoint)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("a GitLab read panicked"))
                    .collect::<Vec<_>>()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_what_went_wrong_in_words_and_what_to_try() {
        let (reason, hint) = explain("GitLab: Error: bad port (projects/:fullpath/merge_requests?state=merged)");
        assert_eq!(reason, "GitLab can't be reached (Error: bad port)");
        assert_eq!(
            hint,
            "glab api projects/:fullpath/merge_requests?state=merged failed: check the login (glab auth status) and the network"
        );
        let (reason, _) = explain("GitLab: glab: 401 Unauthorized (projects/:fullpath/issues)");
        assert_eq!(
            reason,
            "glab is not logged in, or its login expired (glab: 401 Unauthorized)"
        );
        let (reason, hint) = explain("GitLab: glab is not installed (https://gitlab.com/gitlab-org/cli) (x)");
        assert!(reason.starts_with("glab is not installed"), "{reason}");
        assert!(hint.starts_with("install glab"), "{hint}");
    }

    #[test]
    fn names_what_went_wrong_in_one_line() {
        let trace = "node:internal/modules/run_main:107\n    triggerUncaughtException(\n    ^\n\n[TypeError: fetch failed] {\n  [cause]: Error: connect ECONNREFUSED 127.0.0.1:59999\n      at x (y)\n}\n\nNode.js v25.8.0\n";
        assert_eq!(
            reason_of(trace).as_deref(),
            Some("Error: connect ECONNREFUSED 127.0.0.1:59999")
        );
        assert_eq!(
            reason_of("glab: 502 Bad Gateway\n").as_deref(),
            Some("glab: 502 Bad Gateway")
        );
        assert_eq!(
            reason_of("x\n[TypeError: fetch failed] {\n}").as_deref(),
            Some("TypeError: fetch failed")
        );
        assert_eq!(
            reason_of("could not find the host\n").as_deref(),
            Some("could not find the host")
        );
        assert_eq!(reason_of("\n"), None);
    }
}
