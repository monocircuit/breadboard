//! GitLab through the `glab` CLI, as today's commands talk to it: views and
//! writes with glab's own login, `cwd` the checkout, quiet. Reads of the
//! REST API go through `glab api` too: these commands make a handful of
//! calls, a process each costs nothing next to the one a minute of wt:wait
//! (rule 4). Unlike Node (`allowFail`), a view tells "GitLab has none" (404,
//! no MR for the branch) from "glab failed" (no login, no network, no glab):
//! read as "none", a failure made wt:action change report a merge block that
//! never happened, and wt:mr open a second MR (rule 6).

use super::common::temp_dir;
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
    /// glab never ran (not installed, no temp folder for its text): `stderr` says why.
    pub is_unstarted: bool,
}

impl Ran {
    fn of(output: std::io::Result<Output>) -> Ran {
        match output {
            Ok(output) => Ran {
                is_ok: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                is_unstarted: false,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ran::unstarted("glab is not installed or not on PATH".into())
            }
            Err(error) => Ran::unstarted(format!("glab could not be started: {error}")),
        }
    }

    fn unstarted(why: String) -> Ran {
        Ran {
            stderr: why,
            is_unstarted: true,
            ..Ran::default()
        }
    }

    /// The output as JSON, None when the call failed or it is none.
    pub fn json(&self) -> Option<Value> {
        if !self.is_ok {
            return None;
        }
        serde_json::from_str(self.stdout.trim()).ok()
    }

    /// The HTTP status GitLab refused with, as glab names it on its first
    /// line (`glab: 404 Not Found (HTTP 404)`); None when it never got one.
    pub fn status(&self) -> Option<u16> {
        if self.is_ok || self.is_unstarted {
            return None;
        }
        let line = self.stderr.lines().find(|line| !line.trim().is_empty())?;
        let found = STATUS.captures(line.trim())?;
        found.get(1).or_else(|| found.get(2))?.as_str().parse().ok()
    }

    /// Whether GitLab answered 404.
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }

    /// Whether GitLab has none of what was asked: a 404, or glab's "no merge
    /// request for <branch>" of `glab mr view <branch>`.
    pub fn is_not_there(&self) -> bool {
        let is_unanswered = !self.is_ok && !self.is_unstarted && self.status().is_none();
        self.is_not_found() || (is_unanswered && NO_MR.is_match(&self.stderr))
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

    /// What GitLab said with its refusal: the `message` (or `error`) of the
    /// JSON it answered, else glab's line without the status.
    fn reason(&self, status: u16) -> String {
        let said = [&self.stderr, &self.stdout].into_iter().find_map(|text| {
            let at = text.find('{')?;
            let value = serde_json::Deserializer::from_str(&text[at..])
                .into_iter::<Value>()
                .next()?
                .ok()?;
            value.get("message").or_else(|| value.get("error")).map(spoken)
        });
        let said = said.unwrap_or_else(|| {
            let line = self.first_line();
            line.split(" (HTTP ").next().unwrap_or(&line).to_string()
        });
        let code = status.to_string();
        said.trim()
            .strip_prefix(&code)
            .unwrap_or(said.trim())
            .trim()
            .to_string()
    }

    /// Why it failed, as exact as glab lets it be told: GitLab's refusal
    /// with its status, else glab's own line and what shows whether glab can
    /// reach GitLab at all. Only a call that never got an answer points at
    /// the login: a 404 or 403 is no matter of `glab auth status`.
    pub fn why(&self) -> String {
        if self.is_unstarted {
            return self.stderr.clone();
        }
        if self.is_ok {
            return "glab answered no JSON".into();
        }
        let Some(status) = self.status() else {
            return format!("{} (glab auth status)", self.first_line());
        };
        let reason = self.reason(status);
        let said = if reason.is_empty() {
            status.to_string()
        } else {
            format!("{status} {reason}")
        };
        match status {
            401 => format!("GitLab does not take glab's login ({said}): glab auth login"),
            403 => format!("GitLab refused it ({said}): the account glab uses may not do this here (glab auth status)"),
            500.. => format!("GitLab failed ({said}): try again in a minute"),
            _ => format!("GitLab answered {said}"),
        }
    }

    /// The failure of a read as the commands say it: GitLab's answer, or `glab failed: <why>`.
    pub fn failure(&self) -> String {
        if self.status().is_some() {
            self.why()
        } else {
            format!("glab failed: {}", self.why())
        }
    }
}

/// A JSON message as a line: GitLab's validation errors come as `{"title":["is too long"]}`.
fn spoken(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(list) => list.iter().map(spoken).collect::<Vec<_>>().join("; "),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| format!("{key} {}", spoken(value)))
            .collect::<Vec<_>>()
            .join("; "),
        other => other.to_string(),
    }
}

/// The status on glab's first line: at its start (`glab: 404 Not Found`,
/// `ERROR: 403 …`) or as `(HTTP 404)`; a number inside a branch name is none.
static STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:(?:glab|error):\s*)*([45]\d\d)\b|\bHTTP ([45]\d\d)\b").unwrap());

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

/// A field of a write through the REST API.
pub enum Field<'a> {
    /// `--raw-field key=value`: a string as it is, an `@` included.
    Raw(&'a str, &'a str),
    /// `--field key=value`: a number or `true`, sent as one.
    Typed(&'a str, &'a str),
    /// A text of any size, through a file: `--field key=@<file>`. One
    /// argument holds at most 128 KiB on Linux (a description of 180 KB
    /// failed with E2BIG), and the value is always a path, so a text that
    /// starts with `@` is never read as one. Not `--description-file`, which
    /// glab 1.114 doesn't have (#157).
    Text(&'a str, &'a str),
}

/// `<method> <endpoint>` of the REST API with `fields` (glab sends them as
/// JSON); the files of the texts are gone afterwards.
pub fn send(dir: &Path, method: &str, endpoint: &str, fields: &[Field]) -> Ran {
    let folder = match temp_dir("wt-glab-").and_then(std::path::absolute) {
        Ok(folder) => folder,
        Err(error) => return Ran::unstarted(format!("no temp folder for the text glab sends: {error}")),
    };
    let ran = match args_of(method, endpoint, fields, &folder) {
        Ok(args) => glab(dir, &args.iter().map(String::as_str).collect::<Vec<_>>()),
        Err(problem) => Ran::unstarted(problem),
    };
    let _ = std::fs::remove_dir_all(folder);
    ran
}

/// The arguments of `glab api`, each text written to `<folder>/<key>.md`.
fn args_of(method: &str, endpoint: &str, fields: &[Field], folder: &Path) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = vec!["api".into(), "--method".into(), method.into(), endpoint.into()];
    for field in fields {
        let (flag, pair) = match field {
            Field::Raw(key, value) => ("--raw-field", format!("{key}={value}")),
            Field::Typed(key, value) => ("--field", format!("{key}={value}")),
            Field::Text(key, text) => {
                let file = folder.join(format!("{key}.md"));
                std::fs::write(&file, text)
                    .map_err(|error| format!("{} could not be written: {error}", file.display()))?;
                ("--field", format!("{key}=@{}", file.display()))
            }
        };
        args.extend([flag.to_string(), pair]);
    }
    Ok(args)
}

/// A description written through the REST API, `PUT <endpoint>` (#157): the
/// route GitLab sees is the PUT `glab issue|mr update` makes.
pub fn put_description(dir: &Path, endpoint: &str, text: &str) -> Ran {
    send(dir, "PUT", endpoint, &[Field::Text("description", text)])
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

    fn failed_with(stderr: &str) -> Ran {
        Ran {
            stderr: stderr.into(),
            ..Ran::default()
        }
    }

    fn answered(stdout: &str) -> Ran {
        Ran {
            is_ok: true,
            stdout: stdout.into(),
            ..Ran::default()
        }
    }

    #[test]
    fn reads_the_status_from_glab_s_first_line_only() {
        assert_eq!(failed_with("glab: 404 Not Found (HTTP 404)\n").status(), Some(404));
        assert_eq!(failed_with("ERROR: 403 Forbidden\n").status(), Some(403));
        assert_eq!(failed_with("request failed (HTTP 502)\n").status(), Some(502));
        assert_eq!(failed_with("glab: 4040 things\n").status(), None);
        assert_eq!(failed_with("no merge request for feature/404-x\n").status(), None);
        assert_eq!(failed_with("connection refused\nglab: 404\n").status(), None);
        assert_eq!(answered("").status(), None);
        assert!(failed_with("glab: 404 Not Found\n").is_not_found());
    }

    #[test]
    fn says_each_refusal_exactly_and_the_login_only_where_it_matters() {
        let not_found = failed_with("glab: 404 {\"message\":\"404 Not found\"}\n");
        assert_eq!(not_found.why(), "GitLab answered 404 Not found");
        assert_eq!(not_found.failure(), "GitLab answered 404 Not found");
        assert_eq!(
            failed_with("glab: 403 Forbidden (HTTP 403)\n").why(),
            "GitLab refused it (403 Forbidden): the account glab uses may not do this here (glab auth status)"
        );
        assert_eq!(
            failed_with("glab: 401 Unauthorized\n").why(),
            "GitLab does not take glab's login (401 Unauthorized): glab auth login"
        );
        let invalid = failed_with("glab: 400 {\"message\":{\"title\":[\"is too long\"]}}\n");
        assert_eq!(invalid.why(), "GitLab answered 400 title is too long");
        assert_eq!(
            failed_with("glab: 503 Service Unavailable (HTTP 503)\n").why(),
            "GitLab failed (503 Service Unavailable): try again in a minute"
        );
        // No answer at all: glab's line, and the login is worth a look.
        assert_eq!(
            failed_with("dial tcp: lookup gitlab.com: no such host\n").why(),
            "dial tcp: lookup gitlab.com: no such host (glab auth status)"
        );
        let unstarted = Ran::of(Err(std::io::Error::from_raw_os_error(7)));
        assert!(
            unstarted
                .why()
                .starts_with("glab could not be started: Argument list too long")
        );
        assert!(!unstarted.why().contains("auth"));
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
        // glab couldn't ask: what it said and what to run.
        assert_eq!(
            viewed(&failed_with("glab: 401 Unauthorized\nmore\n")),
            Err("GitLab does not take glab's login (401 Unauthorized): glab auth login".into())
        );
        assert_eq!(
            viewed(&Ran::of(Err(std::io::Error::from(std::io::ErrorKind::NotFound)))),
            Err("glab failed: glab is not installed or not on PATH".into())
        );
        assert_eq!(
            viewed(&failed_with("")),
            Err("glab failed: it ended without a word (glab auth status)".into())
        );
        assert_eq!(
            viewed(&answered("<html>")),
            Err("glab failed: glab answered no JSON".into())
        );
        assert_eq!(viewed(&answered("{\"iid\":3}")).unwrap().unwrap()["iid"], 3);
    }

    #[test]
    fn reads_json_only_of_a_call_that_worked() {
        assert_eq!(answered("{\"iid\":3}\n").json().unwrap()["iid"], 3);
        assert!(answered("not json").json().is_none());
        assert!(failed_with("{\"iid\":3}").json().is_none());
    }

    #[test]
    fn hands_a_text_of_any_size_to_glab_through_a_file() {
        let scratch = crate::cli::write::common::scratch::Scratch::new();
        let text = "@not/a/path\n".repeat(20_000);
        let fields = [
            Field::Raw("title", "Draft: @x"),
            Field::Typed("assignee_id", "7"),
            Field::Text("description", &text),
        ];
        let args = args_of("POST", "projects/:fullpath/merge_requests", &fields, scratch.path()).unwrap();
        let file = scratch.path().join("description.md");
        assert_eq!(
            args,
            [
                "api",
                "--method",
                "POST",
                "projects/:fullpath/merge_requests",
                "--raw-field",
                "title=Draft: @x",
                "--field",
                "assignee_id=7",
                "--field",
                &format!("description=@{}", file.display()),
            ]
        );
        assert!(args.iter().all(|arg| arg.len() < 4096));
        assert_eq!(std::fs::read_to_string(file).unwrap(), text);
        let gone = scratch.path().join("gone");
        assert!(args_of("PUT", "x", &[Field::Text("description", "t")], &gone).is_err());
    }
}
