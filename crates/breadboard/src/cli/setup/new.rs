//! `pnpm wt:new <feature> [--shared] [--fix] [--origin <orchestration>]`
//! (commands/new.mjs): a worktree on a new branch from origin/develop, the
//! empty branch pushed as the claim on its issue, installed, its own stack,
//! its herdr space; the agent that ran it moves along.

use super::conversations::{self, session_file};
use super::herdr::{self, field};
use super::proc::{self, Args, How, fail, name_refusal, outln, set_hint, step};
use super::repo::{self, VITA};
use super::stack::{self, Summary};
use super::sweep;
use crate::cli::lifecycle::handover::claim_on_origin;
use breadboard_core::git::Worktree;
use breadboard_core::record::{self, Origin, is_name};
use std::fs;
use std::path::Path;
use std::process::{ExitCode, Stdio};

const INSTALL: &str = "pnpm install --frozen-lockfile";
const PRETTIER: &str = "pnpm --filter @monocircuit/prettier-config build";
const ESLINT: &str = "pnpm --filter @monocircuit/eslint-config build:prod";

pub const USAGE: &str = "usage: pnpm wt:new <feature-in-kebab-case> [--shared] [--fix] [--origin <orchestration>]";

/// The worktree an agent leaves for the new one: the space its tab stands
/// in, else where it ran this. None for a human.
fn leaving_worktree() -> Option<Worktree> {
    if !proc::is_agent() {
        return None;
    }
    let label = if herdr::is_in_herdr() {
        herdr::pane_here().and_then(|pane| {
            herdr::spaces()
                .into_iter()
                .find(|space| field(space, "workspace_id") == field(&pane, "workspace_id"))
                .map(|space| field(&space, "label").to_string())
        })
    } else {
        None
    };
    let here = repo::here();
    repo::worktrees().into_iter().skip(1).find(|tree| {
        label.as_deref() == Some(repo::feature_of(&tree.dir).as_str()) || repo::is_inside(&here, &tree.dir)
    })
}

/// The conversation left `previous`: its binding goes; when the work there
/// is done (merged and gone from origin, nothing uncommitted) the worktree
/// goes too. A line for the summary.
fn hand_over(previous: &Worktree) -> String {
    let _ = fs::remove_file(session_file(&previous.dir));
    let feature = repo::feature_of(&previous.dir);
    let is_clean = proc::git(&["status", "--porcelain"], &previous.dir).text().is_empty();
    let main = repo::main_dir();
    let is_on_origin = previous
        .branch
        .as_ref()
        .is_some_and(|branch| proc::git(&["ls-remote", "--exit-code", "--heads", "origin", branch], &main).is_ok);
    let is_done = previous
        .branch
        .as_ref()
        .is_some_and(|branch| is_clean && !is_on_origin && !repo::has_own_commits(branch, &previous.dir));
    if !is_done {
        let why = if !is_clean {
            "uncommitted changes"
        } else if is_on_origin {
            "its branch is still on origin"
        } else {
            "commits not in develop"
        };
        return format!("{feature} stays ({why}): its space keeps overview and nvim");
    }
    let removed = proc::child()
        .args(["wt:remove", &feature, "--close-space"])
        .current_dir(&main)
        .stdin(Stdio::null())
        .output();
    match removed {
        Ok(output) if output.status.success() => format!("{feature} removed, its work is in develop"),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!(
                "{feature} could not be removed: {}",
                stderr.trim().lines().last().unwrap_or_default()
            )
        }
        Err(error) => format!("{feature} could not be removed: {error}"),
    }
}

pub fn command(args: &Args) -> ExitCode {
    let feature = match args.first_word(&["--origin"]) {
        Some(feature) if is_name(&feature) => feature,
        given => fail(&name_refusal("the feature", given.as_deref(), USAGE)),
    };
    let branch = format!("{}/{feature}", if args.has("--fix") { "fix" } else { "feature" });
    let dir = repo::dir_of(&feature);
    if dir.exists() {
        fail(&format!("{} exists already", dir.display()));
    }
    let origin_name = args.value("--origin");
    if let Some(name) = origin_name.as_deref().filter(|name| !is_name(name)) {
        fail(&name_refusal("the orchestration", Some(name), USAGE));
    }
    super::heal_plugin();
    let origin = origin_name.map(|name| Origin {
        slot: herdr::slot_of(&name),
        name,
    });
    let stays = if origin.is_some() {
        None
    } else {
        herdr::orchestration_here()
    };
    let previous = if herdr::moves_tab(proc::is_agent(), origin.as_ref(), stays.as_deref()) {
        leaving_worktree()
    } else {
        None
    };
    let main = repo::main_dir();

    step("git fetch");
    proc::git(&["fetch", "--prune"], &main);
    if proc::git(&["rev-parse", "--verify", "--quiet", &branch], &main).is_ok {
        fail(&format!("the branch {branch} exists already"));
    }
    if proc::git(&["ls-remote", "--exit-code", "--heads", "origin", &branch], &main).is_ok {
        fail(&format!("origin has a branch {branch} already"));
    }
    // One claim check with wt:orchestrate's (handover.rs): `feature/34-…`, `fix/34-…`, not `…/134-…`.
    // An origin that doesn't answer stops nothing here: the push of the claim says so.
    if let Ok(Some(taken)) = claim_on_origin(&main, &feature) {
        fail(&format!(
            "origin has the branch {taken} — someone works on that issue already"
        ));
    }

    let dir_text = dir.to_string_lossy().into_owned();
    step(&format!("git worktree add {dir_text} -b {branch} origin/develop"));
    proc::run(
        "git",
        &[
            "worktree",
            "add",
            "--quiet",
            "--no-track",
            &dir_text,
            "-b",
            &branch,
            "origin/develop",
        ],
        How {
            cwd: Some(&main),
            ..How::default()
        },
    );
    conversations::share_conversations();
    if let Some(origin) = &origin
        && let Err(error) = record::write_origin(&dir, origin)
    {
        fail(&format!(
            "couldn't write {}: {error}",
            record::origin_file(&dir).display()
        ));
    }
    // A step that fails names the commands that finish the worktree from there.
    let main_env = repo::env_file_of(&main);
    let stack_step = if !main_env.exists() {
        None
    } else if args.has("--shared") {
        Some(format!("cp {} {VITA}/.env.local", main_env.display()))
    } else {
        Some("pnpm wt:stack".to_string())
    };
    let finish = |rest: &[&str]| {
        let rest: Vec<&str> = rest.iter().copied().chain(stack_step.as_deref()).collect();
        format!(
            "the worktree is only half set up: fix the cause, then finish it in {dir_text}: {}; or start over with pnpm wt:remove {feature}",
            rest.join(" && ")
        )
    };
    set_hint(&finish(&[INSTALL, PRETTIER, ESLINT]));

    step(&format!(
        "push guard: {} (#184)",
        crate::cli::write::mr::install_push_guard(&dir)
    ));
    if let Some(said) = breadboard_core::ci::set_push_option(&dir) {
        step(&format!("CI: {said}"));
    }
    step(&format!("git push -u origin {branch} (the empty claim on the issue)"));
    let pushed = proc::run(
        "git",
        &["push", "--quiet", "-u", "origin", &branch],
        How {
            cwd: Some(&dir),
            is_quiet: true,
            can_fail: true,
            ..How::default()
        },
    );
    if !pushed.is_ok {
        outln!("! origin refused the branch, push it yourself: git push -u origin {branch}");
    }

    step("pnpm install");
    proc::run(
        "pnpm",
        &["install", "--frozen-lockfile"],
        How {
            cwd: Some(&dir),
            is_quiet: proc::is_agent(),
            ..How::default()
        },
    );
    set_hint(&finish(&[PRETTIER, ESLINT]));
    step("build the lint and format configs (lint and the hooks load them)");
    let quiet = || How {
        cwd: Some(&dir),
        is_quiet: true,
        ..How::default()
    };
    proc::run("pnpm", &["--filter", "@monocircuit/prettier-config", "build"], quiet());
    proc::run(
        "pnpm",
        &["--filter", "@monocircuit/eslint-config", "build:prod"],
        quiet(),
    );
    set_hint(&finish(&[]));

    let offset = if !main_env.exists() {
        outln!(
            "\n! {} is missing, the worktree has no .env.local and no stack",
            main_env.display()
        );
        0
    } else if args.has("--shared") {
        step("share the dev stack of the main checkout (copy its .env.local)");
        let target = dir.join(VITA).join(".env.local");
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(error) = fs::copy(&main_env, &target) {
            fail(&format!("couldn't copy {}: {error}", main_env.display()));
        }
        0
    } else {
        stack::create_stack(&dir, &feature)
    };
    // Set up: what follows only tells and arranges, nothing to finish by hand.
    set_hint("");
    // GitLab not answering costs the count, never the worktree; an orchestration's skips the sweep.
    let silent = if origin.is_some() {
        0
    } else {
        sweep::stale_issue_count().unwrap_or(0)
    };
    let space = herdr::open_space(&feature, &dir, origin.as_ref(), stays.as_deref());
    // An orchestration's worktree opens no app tab (#337): nobody looks at it
    // yet, and a tab that comes to the front then passes for another's page.
    if origin.is_none() {
        stack::open_app(&dir, offset);
    }
    let left = previous.as_ref().map(hand_over);
    let stale = if origin.is_some() { 0 } else { sweep::prunable_count() };
    stack::summary(&Summary {
        dir: Path::new(&dir_text),
        branch: &branch,
        offset,
        tab: space,
        stale,
        silent,
        left,
        origin: origin.as_ref().map(|origin| origin.name.as_str()),
        is_stack_only: false,
        is_stackless: !main_env.exists(),
        is_unpushed: !pushed.is_ok,
    });
    ExitCode::SUCCESS
}
