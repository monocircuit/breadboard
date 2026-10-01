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
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr.trim().lines().last().unwrap_or("").trim().to_string();
        let reason = if last.is_empty() {
            format!("glab api exited with {}", output.status)
        } else {
            last
        };
        return Err(failed(&reason));
    }
    serde_json::from_slice(&output.stdout).map_err(|_| failed("its answer was no JSON"))
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
