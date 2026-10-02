//! GitLab as these commands read and write it: through `glab`, exactly the
//! calls today's breadboard makes (rewrite rule 6), in the main checkout,
//! whose remote names the project.

use super::proc::output;
use serde_json::Value;
use std::path::Path;

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

/// `glab api <endpoint>` → its JSON, None when glab fails or answers none.
pub fn api(main: &Path, endpoint: &str) -> Option<Value> {
    let out = output("glab", &["api", endpoint], main, None);
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

/// `glab <args> -F json` of an issue or an MR (`issue view 12`, `mr view 3`).
pub fn view(main: &Path, args: &[&str]) -> Result<Value, Unread> {
    let mut all = args.to_vec();
    all.extend(["-F", "json"]);
    let out = output("glab", &all, main, None);
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
    }

    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(encode("fix/303-a_b.c"), "fix%2F303-a_b.c");
        assert_eq!(encode("ä b"), "%C3%A4%20b");
    }
}
