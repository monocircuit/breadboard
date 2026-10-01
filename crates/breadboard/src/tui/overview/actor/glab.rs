//! GitLab as the actor talks to it: `glab api` as the user who runs the
//! overview, from the main checkout (merge.mjs `glabWrite`, io.mjs `glab`).
//! The reads of the view are core's one GraphQL request; these are the few
//! calls a click or an ended wait makes: a fresh MR before a merge, the
//! merge itself, an MR's diffs after it. The writes are exactly Node's
//! (rewrite rule 6): the title of a draft made ready or put back, the merge.

use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};

/// What a call failed with: glab's stderr (`glab: 405 {…}`, `HTTP 422`), for `reason_of`.
pub type Failed = String;

fn glab(main: &Path, args: &[&str]) -> Result<String, Failed> {
    let output = Command::new("glab")
        .args(args)
        .current_dir(main)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("glab: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            stdout.trim().to_string()
        } else {
            stderr
        })
    }
}

/// A GET of the project's `endpoint` (`projects/:fullpath/merge_requests/5`), parsed.
pub fn get(main: &Path, endpoint: &str) -> Result<Value, Failed> {
    let text = glab(main, &["api", endpoint])?;
    serde_json::from_str(&text).map_err(|error| format!("{endpoint}: {error}"))
}

/// A write: `glab api --method <method> <endpoint> --field k=v …`.
pub fn write(main: &Path, method: &str, endpoint: &str, fields: &[(&str, String)]) -> Result<Value, Failed> {
    let pairs: Vec<String> = fields.iter().map(|(key, value)| format!("{key}={value}")).collect();
    let mut args = vec!["api", "--method", method, endpoint];
    for pair in &pairs {
        args.push("--field");
        args.push(pair);
    }
    let text = glab(main, &args)?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// An MR's endpoint.
pub fn mr_endpoint(iid: i64) -> String {
    format!("projects/:fullpath/merge_requests/{iid}")
}

/// The title of an MR without its draft prefix (`[draft]`, `(draft)`, `draft:`, any case) and the spaces after it.
pub fn ready_title(title: &str) -> String {
    let lower = title.to_lowercase();
    for prefix in ["[draft]", "(draft)", "draft:"] {
        if lower.starts_with(prefix) {
            return title[prefix.len()..].trim_start().to_string();
        }
    }
    title.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_draft_prefix_off_a_title() {
        assert_eq!(ready_title("Draft: overview: x"), "overview: x");
        assert_eq!(ready_title("[Draft] x"), "x");
        assert_eq!(ready_title("(draft)x"), "x");
        assert_eq!(ready_title("drafting tool"), "drafting tool");
    }
}
