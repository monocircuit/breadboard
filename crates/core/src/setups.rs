//! An issue's start while it runs (#272): a handover or a fresh start claims
//! its issue as `.claude/orchestrations/<name>.setups/<number>.json` first,
//! gone with its process (worktree/setups.mjs).

use crate::json::{self, int_of, str_of};
use crate::lock::is_alive;
use crate::record::records_dir;
use crate::time::{now_ms, to_iso};
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn setups_dir(main: &Path, name: &str) -> PathBuf {
    records_dir(main).join(format!("{name}.setups"))
}

/// A start running: `dir` the worktree it makes, `to` the agent it is for
/// (None: a fresh one), `from` that agent's worktree now.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub number: i64,
    pub dir: Option<String>,
    pub to: Option<String>,
    pub from: Option<String>,
    pub pid: i64,
    pub at: String,
}

impl Setup {
    fn from_value(value: &Value) -> Option<Setup> {
        Some(Setup {
            number: int_of(value, "number")?,
            dir: str_of(value, "dir").map(String::from),
            to: str_of(value, "to").map(String::from),
            from: str_of(value, "from").map(String::from),
            pid: int_of(value, "pid").unwrap_or(0),
            at: str_of(value, "at").unwrap_or_default().to_string(),
        })
    }
}

/// The setups of `name` whose process still runs; one whose process ended
/// without its release (killed) is dropped.
pub fn setups_of(main: &Path, name: &str, alive: impl Fn(i64) -> bool) -> Vec<Setup> {
    let Ok(entries) = fs::read_dir(setups_dir(main, name)) else {
        return Vec::new();
    };
    let mut setups: Vec<(PathBuf, Setup)> = Vec::new();
    for entry in entries.flatten() {
        let file = entry.path();
        match json::read(&file).as_ref().and_then(Setup::from_value) {
            Some(setup) if alive(setup.pid) => setups.push((file, setup)),
            _ => {
                let _ = fs::remove_file(&file);
            }
        }
    }
    setups.sort_by(|a, b| a.0.cmp(&b.0));
    setups.into_iter().map(|(_, setup)| setup).collect()
}

/// The setups of `name` with live processes.
pub fn live_setups_of(main: &Path, name: &str) -> Vec<Setup> {
    setups_of(main, name, is_alive)
}

/// A claim on the start of an issue, released when dropped.
pub struct Claim {
    file: PathBuf,
}

impl Drop for Claim {
    fn drop(&mut self) {
        let pid = json::read(&self.file).and_then(|setup| int_of(&setup, "pid"));
        if pid == Some(i64::from(std::process::id())) {
            let _ = fs::remove_file(&self.file);
        }
    }
}

/// Claims the start of the issue `number` for this process: Ok(claim), or
/// Err(holder) when a running setup has it already.
pub fn claim_setup(
    main: &Path,
    name: &str,
    number: i64,
    dir: &str,
    to: Option<&str>,
    from: Option<&str>,
) -> io::Result<Result<Claim, Option<Setup>>> {
    let folder = setups_dir(main, name);
    fs::create_dir_all(&folder)?;
    let file = folder.join(format!("{number}.json"));
    let entry = json!({
        "number": number, "dir": dir, "to": to, "from": from,
        "pid": std::process::id(), "at": to_iso(now_ms()),
    });
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(&file) {
            Ok(mut created) => {
                created.write_all(format!("{}\n", json::pretty(&entry)).as_bytes())?;
                return Ok(Ok(Claim { file }));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let holder = json::read(&file).as_ref().and_then(Setup::from_value);
                if let Some(holder) = holder.filter(|holder| is_alive(holder.pid)) {
                    return Ok(Err(Some(holder)));
                }
                let _ = fs::remove_file(&file);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Err(json::read(&file).as_ref().and_then(Setup::from_value)))
}

/// `a handover of #267 to 268-b (pid 1, since 10:00 UTC)` / `the fresh start of #267 (…)`.
pub fn setup_text(setup: &Setup) -> String {
    let what = match &setup.to {
        Some(to) => format!("a handover of #{} to {to}", setup.number),
        None => format!("the fresh start of #{}", setup.number),
    };
    format!(
        "{what} (pid {}, since {} UTC)",
        setup.pid,
        setup.at.get(11..16).unwrap_or("")
    )
}

/// The setup that makes the worktree `dir` or moves its agent out.
pub fn setup_at<'a>(setups: &'a [Setup], dir: &str) -> Option<&'a Setup> {
    setups
        .iter()
        .find(|setup| setup.dir.as_deref() == Some(dir) || setup.from.as_deref() == Some(dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::scratch::Scratch;

    #[test]
    fn claims_an_issue_once() {
        let scratch = Scratch::new();
        let main = scratch.path();
        let first = claim_setup(main, "o", 267, "/x/267-a", Some("268-b"), Some("/x/268-b")).unwrap();
        assert!(first.is_ok());
        let second = claim_setup(main, "o", 267, "/x/267-a", None, None).unwrap();
        let holder = second.err().flatten().unwrap();
        assert_eq!(holder.to.as_deref(), Some("268-b"));
        assert_eq!(holder.pid, i64::from(std::process::id()));
        assert!(setup_text(&holder).starts_with("a handover of #267 to 268-b "));
        assert_eq!(live_setups_of(main, "o").len(), 1);
        drop(first);
        assert!(live_setups_of(main, "o").is_empty());
        assert!(claim_setup(main, "o", 267, "/x/267-a", None, None).unwrap().is_ok());
    }

    #[test]
    fn drops_the_claim_of_a_process_that_ended() {
        let scratch = Scratch::new();
        let main = scratch.path();
        let folder = setups_dir(main, "o");
        fs::create_dir_all(&folder).unwrap();
        let file = folder.join("5.json");
        fs::write(
            &file,
            r#"{"number":5,"dir":"/x/5","to":null,"from":null,"pid":999999999,"at":"T"}"#,
        )
        .unwrap();
        assert!(setups_of(main, "o", |_| false).is_empty());
        assert!(!file.exists());
        fs::write(
            &file,
            r#"{"number":5,"dir":"/x/5","to":null,"from":null,"pid":999999999,"at":"T"}"#,
        )
        .unwrap();
        assert!(
            claim_setup(main, "o", 5, "/x/5", None, None).unwrap().is_ok(),
            "a dead pid's claim is taken over"
        );
    }

    #[test]
    fn finds_the_setup_of_a_worktree_it_makes_or_moves_an_agent_out_of() {
        let setups = vec![Setup {
            number: 1,
            dir: Some("/x/1".into()),
            to: Some("0-a".into()),
            from: Some("/x/0".into()),
            pid: 1,
            at: "T".into(),
        }];
        assert_eq!(setup_at(&setups, "/x/1").unwrap().number, 1);
        assert_eq!(setup_at(&setups, "/x/0").unwrap().number, 1);
        assert!(setup_at(&setups, "/x/2").is_none());
    }
}
