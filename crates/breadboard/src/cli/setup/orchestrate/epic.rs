//! The epic branch of a new orchestration (#350): `epic/<n>-<name>` made
//! from origin/develop and pushed before its first task starts, kept in its
//! record; its checkout then stands on it (layout/orchestrations.rs). An
//! orchestration that works without one (made before #350) keeps develop.

use super::issue::{Item, object};
use crate::cli::setup::proc::{git, step};
use crate::cli::setup::repo;
use breadboard_core::epic::{branch_of, epic_of, number_of_issues};
use breadboard_core::record;
use serde_json::{Value, json};
use std::path::Path;

/// The epic the orchestration works on: the one its record keeps; for a new
/// orchestration (no record yet) the one `--epic` names or its issues'
/// milestone, made; None for one that works on develop. Err why it can't
/// be made.
pub fn ensure_epic(name: &str, plan: &[Item], given: Option<&str>, is_dry: bool) -> Result<Option<String>, String> {
    let main = repo::main_dir();
    let raw = breadboard_core::json::read(&record::record_file(&main, name));
    if let Some(raw) = &raw {
        return Ok(epic_of(raw));
    }
    let number = match given {
        Some(given) => given
            .parse::<i64>()
            .map_err(|_| format!("--epic takes the epic's number (15 for Epic 15), not {given}"))?,
        None => {
            let milestones: Vec<Option<String>> = plan.iter().map(|item| item.milestone.clone()).collect();
            number_of_issues(&milestones).map_err(|why| format!("{why}: name the epic with --epic <n>"))?
        }
    };
    let branch = branch_of(number, name);
    if is_dry {
        step(&format!("the epic branch would be {branch}, from origin/develop"));
        return Ok(Some(branch));
    }
    make_branch(&main, &branch)?;
    record::update_record(&main, name, object(epic_fields(&branch, number)))
        .map_err(|error| format!("the record of {name} not written: {error}"))?;
    Ok(Some(branch))
}

/// What the record keeps of the epic: the branch, its epic, and the base its
/// worktrees branch off (#349 reads `base`).
fn epic_fields(branch: &str, number: i64) -> Value {
    json!({ "epic": { "branch": branch, "number": number }, "base": branch })
}

/// `branch` on origin, from origin/develop when it isn't there yet: pushed
/// at once, the claim on the epic, as a task's branch is.
fn make_branch(main: &Path, branch: &str) -> Result<(), String> {
    let _ = git(&["fetch", "--quiet", "origin"], main);
    let remote = format!("refs/remotes/origin/{branch}");
    if git(&["rev-parse", "--verify", "--quiet", &remote], main).is_ok {
        step(&format!(
            "the epic branch {branch} is on origin already: its tasks branch off it"
        ));
        return Ok(());
    }
    let pushed = git(
        &[
            "push",
            "--quiet",
            "origin",
            &format!("refs/remotes/origin/develop:refs/heads/{branch}"),
        ],
        main,
    );
    if !pushed.is_ok {
        return Err(format!("{branch} not pushed: {}", pushed.err.trim()));
    }
    let _ = git(&["fetch", "--quiet", "origin", branch], main);
    step(&format!(
        "made the epic branch {branch} from origin/develop and pushed it"
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_keeps_the_epic_and_the_base() {
        assert_eq!(
            epic_fields("epic/15-breadboard", 15),
            json!({ "epic": { "branch": "epic/15-breadboard", "number": 15 }, "base": "epic/15-breadboard" })
        );
    }
}
