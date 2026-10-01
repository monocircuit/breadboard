//! An issue's start while it runs (#272, setups.mjs): a handover or a fresh
//! start claims its issue as `.claude/orchestrations/<name>.setups/<n>.json`
//! first, gone with its process; a second start of the same issue is refused.

use crate::cli::setup::proc;
use crate::cli::setup::repo;
use breadboard_core::json::{int_of, str_of};
use breadboard_core::lock::is_alive;
use breadboard_core::time::{now_ms, to_iso};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

pub fn setups_dir(name: &str) -> PathBuf {
    breadboard_core::record::records_dir(&repo::main_dir()).join(format!("{name}.setups"))
}

fn read(file: &PathBuf) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(file).ok()?).ok()
}

/// A claim on the start of an issue, released by `release` or when this process fails.
pub struct Claim {
    file: PathBuf,
}

impl Claim {
    pub fn release(&self) {
        if read(&self.file).and_then(|setup| int_of(&setup, "pid")) == Some(i64::from(std::process::id())) {
            let _ = fs::remove_file(&self.file);
        }
        proc::unhold(&self.file);
    }
}

/// Claims the start of `number` for this process: Ok, or Err(the setup holding it).
pub fn claim_setup(name: &str, number: i64, dir: &str, to: Option<&str>, from: Option<&str>) -> Result<Claim, Value> {
    let folder = setups_dir(name);
    let _ = fs::create_dir_all(&folder);
    let file = folder.join(format!("{number}.json"));
    let entry = json!({
        "number": number, "dir": dir, "to": to, "from": from,
        "pid": std::process::id(), "at": to_iso(now_ms()),
    });
    let text = format!("{}\n", serde_json::to_string_pretty(&entry).unwrap_or_default());
    for _ in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(&file) {
            Ok(mut handle) => {
                let _ = handle.write_all(text.as_bytes());
                proc::hold(&file);
                return Ok(Claim { file });
            }
            Err(_) => match read(&file) {
                Some(holder) if int_of(&holder, "pid").is_some_and(is_alive) => return Err(holder),
                _ => {
                    let _ = fs::remove_file(&file);
                }
            },
        }
    }
    Err(read(&file).unwrap_or(Value::Null))
}

/// `a handover of #267 to 268-b (pid 1, since 10:00 UTC)` / `the fresh start of #267 (…)`.
pub fn setup_text(setup: &Value) -> String {
    let number = int_of(setup, "number").unwrap_or_default();
    let what = match str_of(setup, "to") {
        Some(to) => format!("a handover of #{number} to {to}"),
        None => format!("the fresh start of #{number}"),
    };
    let at = str_of(setup, "at").unwrap_or_default();
    let since = at.get(11..16).unwrap_or_default();
    format!(
        "{what} (pid {}, since {since} UTC)",
        int_of(setup, "pid").unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_what_a_setup_is() {
        let setup = json!({ "number": 267, "to": "268-b", "pid": 1, "at": "2026-10-01T10:00:00.000Z" });
        assert_eq!(
            setup_text(&setup),
            "a handover of #267 to 268-b (pid 1, since 10:00 UTC)"
        );
        let fresh = json!({ "number": 3, "to": null, "pid": 2, "at": "2026-10-01T09:05:00.000Z" });
        assert_eq!(setup_text(&fresh), "the fresh start of #3 (pid 2, since 09:05 UTC)");
    }
}
