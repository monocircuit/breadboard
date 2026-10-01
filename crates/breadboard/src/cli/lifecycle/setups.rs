//! An issue's start while it runs (#272, worktree/setups.mjs): a handover or
//! a fresh start claims its issue as `.claude/orchestrations/<name>.setups/
//! <number>.json`, gone with its process. wt:remove leaves the worktrees a
//! setup involves alone. core has this written (setups.rs) but does not
//! export it yet: this goes once it does.

use breadboard_core::json::{self, int_of, str_of};
use breadboard_core::lock::is_alive;
use breadboard_core::record::records_dir;
use breadboard_core::time::{now_ms, to_iso};
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
pub fn setups_of(main: &Path, name: &str) -> Vec<Setup> {
    let Ok(entries) = fs::read_dir(setups_dir(main, name)) else {
        return Vec::new();
    };
    let mut setups: Vec<(PathBuf, Setup)> = Vec::new();
    for entry in entries.flatten() {
        let file = entry.path();
        match json::read(&file).as_ref().and_then(Setup::from_value) {
            Some(setup) if is_alive(setup.pid) => setups.push((file, setup)),
            _ => {
                let _ = fs::remove_file(&file);
            }
        }
    }
    setups.sort_by(|a, b| a.0.cmp(&b.0));
    setups.into_iter().map(|(_, setup)| setup).collect()
}

/// A claim on the start of an issue, released when dropped (also on a
/// failure: the command returns, the claim goes).
pub struct Claim {
    file: Option<PathBuf>,
}

impl Claim {
    /// No claim at all: a dry run takes none.
    pub fn none() -> Claim {
        Claim { file: None }
    }

    /// Gives the claim up now.
    pub fn release(&mut self) {
        if let Some(file) = self.file.take() {
            let pid = json::read(&file).and_then(|setup| int_of(&setup, "pid"));
            if pid == Some(i64::from(std::process::id())) {
                let _ = fs::remove_file(&file);
            }
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.release();
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
                return Ok(Ok(Claim { file: Some(file) }));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let holder = json::read(&file).as_ref().and_then(Setup::from_value);
                if let Some(holder) = holder.filter(|holder| is_alive(holder.pid)) {
                    return Ok(Err(Some(holder)));
                }
                // Its process ended without a release: the claim is free again.
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

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lifecycle-setups-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn one_start_of_an_issue_at_a_time() {
        let main = scratch("claim");
        let mut first = claim_setup(&main, "o", 7, "/r/p-7-x", Some("3-y"), Some("/r/p-3-y"))
            .unwrap()
            .unwrap();
        let holder = claim_setup(&main, "o", 7, "/r/p-7-x", None, None)
            .unwrap()
            .err()
            .flatten()
            .unwrap();
        assert_eq!(holder.to.as_deref(), Some("3-y"));
        assert!(
            setup_text(&holder).starts_with(&format!("a handover of #7 to 3-y (pid {}, since ", std::process::id()))
        );
        let setups = setups_of(&main, "o");
        assert_eq!(setup_at(&setups, "/r/p-3-y").map(|s| s.number), Some(7));
        first.release();
        assert!(setups_of(&main, "o").is_empty());
        let _ = fs::remove_dir_all(&main);
    }

    #[test]
    fn a_dead_holder_is_taken_over() {
        let main = scratch("dead");
        fs::create_dir_all(setups_dir(&main, "o")).unwrap();
        let dead = json!({ "number": 5, "dir": "/d", "to": null, "from": null, "pid": 999_999_999, "at": "2026-10-01T10:00:00.000Z" });
        fs::write(setups_dir(&main, "o").join("5.json"), dead.to_string()).unwrap();
        assert!(setups_of(&main, "o").is_empty());
        assert!(claim_setup(&main, "o", 5, "/d", None, None).unwrap().is_ok());
        let _ = fs::remove_dir_all(&main);
    }
}
