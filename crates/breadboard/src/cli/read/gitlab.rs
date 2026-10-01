//! GitLab reads of this group: `glab api <endpoint>` (a GET) in the main
//! checkout, so glab finds the host, the project (`:fullpath`) and the login
//! itself. Until core offers its GitLab client (rewrite-plan.md, step 1);
//! then these calls move to it.

use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

/// Calls in flight at once: GitLab does not like a burst of a hundred.
const PARALLEL: usize = 10;

/// A GET of `endpoint`, parsed; Err says why in the words of `GitlabError`:
/// `GitLab: <reason> (<endpoint>)`.
pub fn get(main: &Path, endpoint: &str) -> Result<Value, String> {
    let failed = |reason: &str| format!("GitLab: {reason} ({endpoint})");
    let output = Command::new("glab")
        .args(["api", endpoint])
        .current_dir(main)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => failed("glab is not installed (https://gitlab.com/gitlab-org/cli)"),
            _ => failed(&format!("glab did not start: {error}")),
        })?;
    if !output.status.success() {
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

/// A GET whose answer is text (a job's log).
pub fn text(main: &Path, endpoint: &str) -> Result<String, String> {
    let output = Command::new("glab")
        .args(["api", endpoint])
        .current_dir(main)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("GitLab: glab did not start: {error} ({endpoint})"))?;
    if !output.status.success() {
        let reason = reason_of(&String::from_utf8_lossy(&output.stderr)).unwrap_or_else(|| "glab api failed".into());
        return Err(format!("GitLab: {reason} ({endpoint})"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
    endpoints
        .chunks(PARALLEL)
        .flat_map(|chunk| {
            thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|endpoint| scope.spawn(move || list(main, endpoint)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err("GitLab: the read broke off".into()))
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
