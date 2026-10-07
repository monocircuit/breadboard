//! `wt:orchestrate <name> --end` (end.mjs): once its worktrees are gone, its
//! space closes with the orchestrator's claude (kept for `/resume` in the main
//! checkout), its record and checkout go; its changelog stays.

use super::setups::setups_dir;
use super::space::{orchestrator_checkouts, remove_orchestrator_checkout, space_of_orchestration};
use super::workers::checkout_of;
use crate::cli::setup::conversations::keep_conversation;
use crate::cli::setup::herdr::{self, field, herdr};
use crate::cli::setup::proc::{self, fail, outln, step};
use crate::cli::setup::repo;
use breadboard_core::record;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{ExitCode, Stdio};
use std::time::Duration;

/// Why `name` can't end yet: worktrees left, issues queued.
pub fn why_not_end(name: &str) -> Option<String> {
    let left: Vec<String> = repo::worktrees()
        .into_iter()
        .skip(1)
        .filter(|tree| record::origin_of(&tree.dir).is_some_and(|origin| origin.name == name))
        .map(|tree| repo::feature_of(&tree.dir))
        .collect();
    if !left.is_empty() {
        return Some(format!(
            "worktree{} left: {}",
            if left.len() == 1 { "" } else { "s" },
            left.join(", ")
        ));
    }
    let queued: Vec<String> = record::record_of(&repo::main_dir(), name)
        .map(|found| {
            found
                .issues
                .iter()
                .filter(|issue| issue.is_queued)
                .map(|issue| format!("#{}", issue.number.unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    (!queued.is_empty()).then(|| format!("queued: {}", queued.join(", ")))
}

/// Where a background end writes what it does.
pub fn end_log(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("orchestration-end-{name}.log"))
}

/// The end in a detached child: the space it closes may hold the tab that runs this.
fn end_in_background(name: &str, log: &PathBuf) {
    let Ok(out) = fs::File::create(log) else { return };
    let Ok(err) = out.try_clone() else { return };
    let mut child = proc::child();
    child
        .args(["wt:orchestrate", name, "--end"])
        .current_dir(repo::main_dir())
        .env("NO_COLOR", "1")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_TAB_ID")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    // SAFETY: setsid in the child before exec detaches it from the pane's session.
    unsafe {
        use std::os::unix::process::CommandExt;
        child.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = child.spawn();
}

/// Ends `name`: the lines of what it did, or why it couldn't.
fn end_orchestration(name: &str) -> Result<Vec<String>, String> {
    let main = repo::main_dir();
    let label = record::orchestrator_label(&main, name);
    if let Some(why) = why_not_end(name) {
        return Err(format!("{label} can't end, {why}"));
    }
    let mut done = Vec::new();
    let workspace = space_of_orchestration(name);
    let free: Vec<(String, String)> = record::record_of(&main, name)
        .map(|found| {
            found
                .free_agents()
                .iter()
                .map(|agent| (agent.id.clone(), agent.name.clone().unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    if !free.is_empty() && herdr::is_in_herdr() {
        let own = workspace.as_ref().map(|space| field(space, "workspace_id").to_string());
        let mut closing: Vec<String> = Vec::new();
        for pane in herdr::all_panes() {
            let is_free = herdr::session_of(&pane).is_some_and(|id| free.iter().any(|(free, _)| free == id));
            let tab = field(&pane, "tab_id").to_string();
            if is_free && Some(field(&pane, "workspace_id")) != own.as_deref() && !closing.contains(&tab) {
                closing.push(tab);
            }
        }
        for tab in closing {
            herdr(&["tab", "close", &tab]);
        }
        let names: Vec<&str> = free.iter().map(|(_, name)| name.as_str()).collect();
        done.push(format!(
            "closed the free agent{} {}",
            if free.len() == 1 { "" } else { "s" },
            names.join(", ")
        ));
    }
    if let Some(space) = &workspace {
        let id = field(space, "workspace_id");
        let ids: Vec<String> = herdr::panes_of(id)
            .iter()
            .filter(|pane| pane.get("agent").and_then(Value::as_str) == Some("claude"))
            .filter_map(|pane| herdr::session_of(pane).map(String::from))
            .collect();
        if herdr(&["workspace", "close", id]).is_none() {
            return Err(format!("herdr did not close the space {}", field(space, "label")));
        }
        done.push(format!("closed the space {}", field(space, "label")));
        for id in ids {
            done.push(match keep_conversation(&id, Duration::from_secs(5)) {
                Some(kept) => format!("moved {kept}"),
                None => format!("kept the conversation {id}"),
            });
        }
    }
    let file = record::record_file(&main, name);
    if file.exists() {
        let _ = fs::remove_file(&file);
        let relative = file
            .strip_prefix(&main)
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        done.push(format!("removed the record {relative}"));
    }
    let _ = fs::remove_dir_all(setups_dir(name));
    if orchestrator_checkouts().contains(&checkout_of(name)) {
        done.push(remove_orchestrator_checkout(name).unwrap_or_else(|| format!("removed the checkout of {label}")));
    }
    let changelog = record::changelog_file(&main, name);
    if changelog.exists() {
        done.push(format!("kept the changelog {}", changelog.display()));
    }
    Ok(done)
}

pub fn command(name: &str) -> ExitCode {
    let main = repo::main_dir();
    let label = record::orchestrator_label(&main, name);
    if let Some(why) = why_not_end(name) {
        fail(&format!("{label} can't end, {why}"));
    }
    let workspace = space_of_orchestration(name);
    // Nothing of it anywhere: ended before (an end runs twice), or a typo;
    // ended either way, as Node's, and the line says what was looked for.
    let is_known = workspace.is_some()
        || record::record_file(&main, name).exists()
        || setups_dir(name).exists()
        || orchestrator_checkouts().contains(&checkout_of(name));
    if !is_known {
        outln!("\n✓ {label} ended already: no herdr space, record, setups or checkout of {name} is left");
        return ExitCode::SUCCESS;
    }
    let pane = if workspace.is_some() && std::env::var_os("HERDR_PANE_ID").is_some() {
        herdr::current_pane()
    } else {
        None
    };
    if let (Some(pane), Some(space)) = (&pane, &workspace)
        && field(pane, "workspace_id") == field(space, "workspace_id")
    {
        let log = end_log(name);
        end_in_background(name, &log);
        step(&format!(
            "{label} ends in the background, this tab with it (log: {})",
            log.display()
        ));
        return ExitCode::SUCCESS;
    }
    match end_orchestration(name) {
        Err(error) => fail(&error),
        Ok(done) => {
            for line in done {
                step(&line);
            }
            outln!("\n✓ {label} ended");
        }
    }
    ExitCode::SUCCESS
}
