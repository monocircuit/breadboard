//! GitLab through the `glab` CLI, as today's commands talk to it: views and
//! writes with glab's own login, `cwd` the checkout, quiet, never failing
//! (proc.mjs `run(…, { quiet, allowFail })`). Reads of the REST API go
//! through `glab api` too: these commands make a handful of calls, a
//! process each costs nothing next to the one a minute of wt:wait (rule 4).

use serde_json::Value;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

/// What a glab call answered.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    pub is_ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    fn of(output: std::io::Result<Output>) -> Ran {
        match output {
            Ok(output) => Ran {
                is_ok: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
            Err(error) => Ran {
                is_ok: false,
                stdout: String::new(),
                stderr: format!("glab: {error}"),
            },
        }
    }

    /// The output as JSON, None when the call failed or it is none.
    pub fn json(&self) -> Option<Value> {
        if !self.is_ok {
            return None;
        }
        serde_json::from_str(self.stdout.trim()).ok()
    }

    /// Whether GitLab answered 404 (glab prints the status on stderr).
    pub fn is_not_found(&self) -> bool {
        !self.is_ok
            && self
                .stderr
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| word == "404")
    }
}

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("glab");
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// `glab <args>` in `dir`.
pub fn glab(dir: &Path, args: &[&str]) -> Ran {
    Ran::of(command(dir, args).output())
}

/// `glab <args>` started, to be waited for with [`finish`]: calls side by side.
pub fn start(dir: &Path, args: &[&str]) -> std::io::Result<Child> {
    command(dir, args).spawn()
}

pub fn finish(child: std::io::Result<Child>) -> Ran {
    Ran::of(child.and_then(Child::wait_with_output))
}

/// An issue or MR as `glab issue|mr view <ref> -F json` gives it, None when
/// GitLab has none (or glab can't say).
pub fn view(dir: &Path, what: &str, reference: &str) -> Option<Value> {
    glab(dir, &[what, "view", reference, "-F", "json"])
        .json()
        .filter(Value::is_object)
}

/// `GET <endpoint>` of the REST API (`projects/:fullpath/…`).
pub fn api(dir: &Path, endpoint: &str) -> Ran {
    glab(dir, &["api", endpoint])
}

/// A description written through the REST API, `PUT <endpoint>`: the text
/// itself, not `--description-file`, which glab 1.114 doesn't have (#157).
/// `--raw-field` sends it as it is (`--field` would read `@…` as a file).
pub fn put_description(dir: &Path, endpoint: &str, text: &str) -> bool {
    let field = format!("description={text}");
    glab(dir, &["api", "--method", "PUT", endpoint, "--raw-field", &field]).is_ok
}

/// A string field of a JSON object, "" without one (a template string's `${…}` of a missing field aside).
pub fn text_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A JSON number or string as a template string shows it (`${mr.iid}`), "" for none.
pub fn shown(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_404_from_glab_s_stderr() {
        let ran = Ran {
            is_ok: false,
            stdout: String::new(),
            stderr: "glab: 404 Not Found (HTTP 404)\n".into(),
        };
        assert!(ran.is_not_found());
        let other = Ran {
            is_ok: false,
            stdout: String::new(),
            stderr: "glab: 4040 things\n".into(),
        };
        assert!(!other.is_not_found());
        assert!(
            !Ran {
                is_ok: true,
                ..Default::default()
            }
            .is_not_found()
        );
    }

    #[test]
    fn reads_json_only_of_a_call_that_worked() {
        let ran = Ran {
            is_ok: true,
            stdout: "{\"iid\":3}\n".into(),
            stderr: String::new(),
        };
        assert_eq!(ran.json().unwrap()["iid"], 3);
        assert!(
            Ran {
                is_ok: true,
                stdout: "not json".into(),
                stderr: String::new()
            }
            .json()
            .is_none()
        );
    }
}
