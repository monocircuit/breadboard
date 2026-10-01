//! GitLab through the `glab` CLI, as today's commands talk to it: views and
//! writes with glab's own login, `cwd` the checkout, quiet. Reads of the
//! REST API go through `glab api` too: these commands make a handful of
//! calls, a process each costs nothing next to the one a minute of wt:wait
//! (rule 4). Unlike Node (`allowFail`), a view tells "GitLab has none" (404,
//! no MR for the branch) from "glab failed" (no login, no network, no glab):
//! read as "none", a failure made wt:action change report a merge block that
//! never happened, and wt:mr open a second MR (rule 6).

use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::LazyLock;

/// What a glab call answered.
#[derive(Debug, Clone, Default, PartialEq)]
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ran {
                is_ok: false,
                stdout: String::new(),
                stderr: "glab is not installed or not on PATH".into(),
            },
            Err(error) => Ran {
                is_ok: false,
                stdout: String::new(),
                stderr: format!("glab could not be started: {error}"),
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

    /// Whether GitLab has none of what was asked: a 404, or glab's "no merge
    /// request for <branch>" of `glab mr view <branch>`.
    pub fn is_not_there(&self) -> bool {
        self.is_not_found() || (!self.is_ok && NO_MR.is_match(&self.stderr))
    }

    /// glab's first line on stderr, without its `glab: ` (it names itself).
    pub fn first_line(&self) -> String {
        let line = self.stderr.lines().map(str::trim).find(|line| !line.is_empty());
        match line {
            Some(line) => line.strip_prefix("glab: ").unwrap_or(line).to_string(),
            None if self.is_ok => "it answered no JSON".into(),
            None => "it ended without a word".into(),
        }
    }

    /// Why it failed and what shows whether glab can reach GitLab: `<line> (glab auth status)`.
    pub fn why(&self) -> String {
        format!("{} (glab auth status)", self.first_line())
    }

    /// The failure of a read as the commands say it: `glab failed: <why>`.
    pub fn failure(&self) -> String {
        format!("glab failed: {}", self.why())
    }
}

/// glab's words for a branch without an MR (`no open merge request available for "x"`).
static NO_MR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bno (open )?merge requests?\b").unwrap());

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

/// An issue or MR as `glab issue|mr view <ref> -F json` gives it: None when
/// GitLab has none, `Err(glab failed: …)` when glab couldn't ask.
pub fn view(dir: &Path, what: &str, reference: &str) -> Result<Option<Value>, String> {
    viewed(&glab(dir, &[what, "view", reference, "-F", "json"]))
}

fn viewed(ran: &Ran) -> Result<Option<Value>, String> {
    if ran.is_not_there() {
        return Ok(None);
    }
    match ran.json().filter(Value::is_object) {
        Some(value) => Ok(Some(value)),
        None => Err(ran.failure()),
    }
}

/// `GET <endpoint>` of the REST API (`projects/:fullpath/…`).
pub fn api(dir: &Path, endpoint: &str) -> Ran {
    glab(dir, &["api", endpoint])
}

/// A description written through the REST API, `PUT <endpoint>`: the text
/// itself, not `--description-file`, which glab 1.114 doesn't have (#157).
/// `--raw-field` sends it as it is (`--field` would read `@…` as a file).
pub fn put_description(dir: &Path, endpoint: &str, text: &str) -> Ran {
    let field = format!("description={text}");
    glab(dir, &["api", "--method", "PUT", endpoint, "--raw-field", &field])
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

    fn failed_with(stderr: &str) -> Ran {
        Ran {
            is_ok: false,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn tells_none_there_from_glab_failing() {
        // GitLab has none: a 404 of the API, glab's words for a branch without an MR.
        assert_eq!(viewed(&failed_with("glab: 404 Not Found\n")), Ok(None));
        assert_eq!(
            viewed(&failed_with("no open merge request available for \"feature/1-x\"\n")),
            Ok(None)
        );
        assert_eq!(viewed(&failed_with("no merge request for feature/1-x\n")), Ok(None));
        assert!(viewed(&failed_with("403 Forbidden: no access to merge requests\n")).is_err());
        // glab couldn't ask: its first line and what to run.
        assert_eq!(
            viewed(&failed_with("glab: 401 Unauthorized\nmore\n")),
            Err("glab failed: 401 Unauthorized (glab auth status)".into())
        );
        assert_eq!(
            viewed(&Ran::of(Err(std::io::Error::from(std::io::ErrorKind::NotFound)))),
            Err("glab failed: glab is not installed or not on PATH (glab auth status)".into())
        );
        assert_eq!(
            viewed(&failed_with("")),
            Err("glab failed: it ended without a word (glab auth status)".into())
        );
        let garbled = Ran {
            is_ok: true,
            stdout: "<html>".into(),
            stderr: String::new(),
        };
        assert_eq!(
            viewed(&garbled),
            Err("glab failed: it answered no JSON (glab auth status)".into())
        );
        let found = Ran {
            is_ok: true,
            stdout: "{\"iid\":3}".into(),
            stderr: String::new(),
        };
        assert_eq!(viewed(&found).unwrap().unwrap()["iid"], 3);
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
