//! The batch an orchestration's tasks branch off (#411): its product's
//! rolling `batch/<product>-<n>`, opened from origin/develop when the
//! product has none or its batch shipped. An orchestration still on an epic
//! branch (#350) keeps it until it ships; one of no product, or with
//! its setting `batching` off (#415), works on develop.

use crate::cli::lifecycle::handover::read_issue;
use crate::cli::setup::proc::{git, step};
use crate::cli::setup::repo;
use breadboard_core::batch::{Opened, current_of, ensure_current, is_shipped, parse};
use breadboard_core::epic::{epic_of, is_epic};
use breadboard_core::git::Base;
use breadboard_core::record;
use std::path::Path;

/// The branch the orchestration `name`'s tasks branch off: its epic, else
/// its product's batch, opened when there is none open; None for develop.
/// Err why the batch can't be opened.
pub fn ensure_batch(name: &str, product: Option<&str>, is_dry: bool) -> Result<Option<String>, String> {
    let main = repo::main_dir();
    if let Some(epic) = breadboard_core::json::read(&record::record_file(&main, name)).and_then(|raw| epic_of(&raw)) {
        return Ok(Some(epic));
    }
    let Some(product) = product else {
        return Ok(None);
    };
    // The setting `batching` of the product (#415): off, develop.
    if !breadboard_core::settings::of_product(&main, Some(product)).batching {
        return Ok(None);
    }
    let _ = git(&["fetch", "--quiet", "origin"], &main);
    let develop = breadboard_core::git::branch(&main);
    let opened = ensure_current(&main, product, &develop, is_dry)?;
    say_opened(&opened, &develop, is_dry);
    Ok(Some(opened.batch().branch.clone()))
}

/// The base of a new worktree without `--base` (#411), origin fetched: develop
/// for a hotfix (`--fix`, its issue labelled `hotfix`); else its
/// orchestration's epic still open; else the batch of its orchestration's
/// product or of its issue's milestone's, opened when there is none open;
/// else develop. Err why the batch can't be opened.
pub fn base_of_new(feature: &str, origin: Option<&str>, is_fix: bool) -> Result<Base, String> {
    let main = repo::main_dir();
    let develop = breadboard_core::git::branch(&main);
    let record = origin.and_then(|name| record::record_of(&main, name));
    if let Some(epic) = record
        .as_ref()
        .and_then(|record| record.base.clone())
        .filter(|base| is_epic(base))
    {
        return Ok(Base::of(&epic));
    }
    let issue = feature
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| read_issue(&repo::lifecycle(), feature).ok())
        .flatten();
    if is_fix && issue.as_ref().is_some_and(|issue| issue.is_hotfix) {
        step(&format!("a hotfix: from {develop}, past the batch"));
        return Ok(Base::of(&develop));
    }
    let config = &breadboard_core::config::of(&main).config;
    let product = record.and_then(|record| record.product).or_else(|| {
        issue
            .as_ref()
            .and_then(|issue| issue.milestone.as_deref())
            .and_then(|milestone| config.product_of_milestone(milestone))
    });
    let Some(product) = product.filter(|product| breadboard_core::settings::of_product(&main, Some(product)).batching)
    else {
        return Ok(Base::of(&develop));
    };
    let opened = ensure_current(&main, &product, &develop, false)?;
    say_opened(&opened, &develop, false);
    Ok(Base::of(&opened.batch().branch))
}

/// A step line for a batch [`ensure_current`] made.
fn say_opened(opened: &Opened, develop: &str, is_dry: bool) {
    if let Opened::Made { batch, shipped } = opened {
        let after = shipped
            .as_ref()
            .map(|shipped| format!(", {shipped} shipped"))
            .unwrap_or_default();
        let verb = if is_dry { "would open" } else { "opened" };
        step(&format!(
            "{verb} the batch {} from origin/{develop}{after}",
            batch.branch
        ));
    }
}

/// A worktree `dir` whose base is a batch that shipped gets its product's
/// next batch as its base at its push (#411), its open MR retargeted to it:
/// no rebase, the shipped batch is in develop and the next one starts from
/// there. The line that says so, None when its base stays.
pub fn follow_shipped(dir: &Path, branch: &str) -> Option<String> {
    let base = record::base_in(dir)?;
    let (product, _) = parse(&base)?;
    let main = repo::main_dir();
    let develop = breadboard_core::git::branch(&main);
    let _ = git(&["fetch", "--quiet", "origin", &base, &develop], &main);
    let kept = current_of(&main, product);
    if kept.as_ref().is_some_and(|kept| kept.branch == base) && !is_shipped(&main, kept.as_ref()?, &develop) {
        return None;
    }
    let opened = match ensure_current(&main, product, &develop, false) {
        Ok(opened) => opened,
        Err(why) => return Some(format!("! {base} shipped, but the next batch didn't open: {why}")),
    };
    let next = opened.batch().branch.clone();
    if next == base {
        return None;
    }
    if let Err(error) = record::write_base(dir, &next) {
        return Some(format!("! {base} shipped, the base not switched to {next}: {error}"));
    }
    let _ = git(&["fetch", "--quiet", "origin", &next], &main);
    let retargeted = retarget(branch, &next);
    Some(format!(
        "breadboard: {base} shipped, the base is {next} now{retargeted}"
    ))
}

/// The open MR of `branch` retargeted to `target`: what the line adds.
fn retarget(branch: &str, target: &str) -> String {
    let endpoint = format!(
        "projects/:fullpath/merge_requests?state=opened&source_branch={}",
        super::gitlab::encode(branch)
    );
    let Some(iid) = super::gitlab::list(&endpoint)
        .ok()
        .and_then(|mrs| mrs.first()?.get("iid")?.as_i64())
    else {
        return String::new();
    };
    let status = std::process::Command::new("glab")
        .args([
            "api",
            "--method",
            "PUT",
            &format!("projects/:fullpath/merge_requests/{iid}"),
            "--raw-field",
            &format!("target_branch={target}"),
        ])
        .current_dir(repo::main_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if status.is_ok_and(|status| status.success()) {
        format!(", !{iid} goes into it")
    } else {
        format!(", !{iid} not retargeted: `glab mr update {iid} --target-branch {target}`")
    }
}
