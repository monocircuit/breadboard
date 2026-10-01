//! GitLab's REST API, read only, for the setup commands: `glab api
//! <endpoint>` (`projects/:fullpath/…`), glab's login and host. One process
//! per call, as lib/glab.mjs does without a token; several side by side
//! where a command reads many. The kept-alive HTTP client with the token
//! belongs in breadboard-core (rewrite-plan.md), shared with the overview.

use serde_json::Value;
use std::process::{Command, Stdio};
use std::thread;

/// `glab api <endpoint>` parsed, or why it failed (`GitLab: <reason> (<endpoint>)`).
pub fn api(endpoint: &str) -> Result<Value, String> {
    let cwd = super::repo::main_dir();
    let output = Command::new("glab")
        .args(["api", endpoint])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => {
                format!("GitLab: glab is not installed (https://gitlab.com/gitlab-org/cli) ({endpoint})")
            }
            _ => format!("GitLab: glab did not start: {error} ({endpoint})"),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.trim().lines().last().unwrap_or("glab failed").to_string();
        return Err(format!("GitLab: {reason} ({endpoint})"));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("GitLab: {error} ({endpoint})"))
}

/// `api` of an endpoint that answers a list; an error is an empty list where `is_lenient`.
pub fn list(endpoint: &str) -> Result<Vec<Value>, String> {
    api(endpoint).map(|value| value.as_array().cloned().unwrap_or_default())
}

/// `run` for each of `items`, `at_once` side by side, the results in their order.
pub fn in_turns<T: Sync, R: Send>(items: &[T], at_once: usize, run: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let mut results = Vec::with_capacity(items.len());
    for chunk in items.chunks(at_once.max(1)) {
        let done: Vec<R> = thread::scope(|scope| {
            let handles: Vec<_> = chunk.iter().map(|item| scope.spawn(|| run(item))).collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("a GitLab read panicked"))
                .collect()
        });
        results.extend(done);
    }
    results
}

/// `encodeURIComponent`.
pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_branch() {
        assert_eq!(encode("feature/40-x"), "feature%2F40-x");
        assert_eq!(in_turns(&[1, 2, 3], 2, |n| n * 2), vec![2, 4, 6]);
    }
}
