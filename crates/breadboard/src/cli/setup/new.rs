//! `pnpm wt:new <feature> [--shared] [--fix] [--origin <orchestration>] [--base <branch>]`
//! (commands/new.mjs): a worktree on a new branch from its base (#349:
//! `--base`, else develop for a hotfix, its orchestration's epic, its
//! product's batch (#411), else develop), the
//! empty branch pushed as the claim on its issue, installed, its own stack,
//! its herdr space; the agent that ran it moves along.

use super::conversations;
use super::epic::{self, Epic};
use super::herdr::{self, field};
use super::proc::{self, Args, How, Shape, fail, name_refusal, outln, set_hint, step};
use super::repo;
use super::stack::{self, Summary};
use super::sweep;
use crate::cli::lifecycle::handover::claim_on_origin;
use breadboard_core::git::{Base, Worktree, base_of};
use breadboard_core::record::{self, Origin, is_name};
use std::fs;
use std::path::Path;
use std::process::{ExitCode, Stdio};

/// What `wt:new` takes.
pub const SHAPE: Shape = Shape::new(
    &["--shared", "--fix", "--origin", "--base"],
    &["--origin", "--base"],
    Some(1),
    USAGE,
);

pub const USAGE: &str =
    "usage: pnpm wt:new <feature-in-kebab-case> [--shared] [--fix] [--origin <orchestration>] [--base <branch>]";

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
    let gone = conversations::bound_session(&previous.dir);
    breadboard_core::conversations::unbind_leaving(&previous.dir, gone.as_deref());
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
            "commits not in its base"
        };
        return format!("{feature} stays ({why}): its space keeps overview and nvim");
    }
    let base = base_of(&previous.dir).local;
    let removed = proc::child()
        .args(["wt:remove", &feature, "--close-space"])
        .current_dir(&main)
        .stdin(Stdio::null())
        .output();
    match removed {
        Ok(output) if output.status.success() => format!("{feature} removed, its work is in {base}"),
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
    let feature = match args.first_word(&["--origin", "--base"]) {
        Some(feature) if is_name(&feature) => feature,
        given => fail(&name_refusal("the feature", given.as_deref(), USAGE)),
    };
    let branch = format!("{}/{feature}", if args.has("--fix") { "fix" } else { "feature" });
    let dir = repo::dir_of(&feature);
    if dir.exists() {
        fail(&format!(
            "{} exists already: work in it (pnpm wt:list names every worktree), or pnpm wt:remove {feature} first",
            dir.display()
        ));
    }
    let origin_name = args.value("--origin");
    if let Some(name) = origin_name.as_deref().filter(|name| !is_name(name)) {
        fail(&name_refusal("the orchestration", Some(name), USAGE));
    }
    let given_base = match args.value("--base") {
        Some(base) if record::is_branch(&base) => Some(Base::of(&base)),
        Some(base) => fail(&format!("{base} is no branch name\n{USAGE}")),
        None => None,
    };
    super::heal_plugin();
    let origin = origin_name.clone().map(|name| Origin {
        slot: herdr::slot_of(&name),
        name,
    });
    let stays = if origin.is_some() {
        None
    } else {
        herdr::orchestration_here()
    };
    // An agent's own wt:new (not an orchestration's): it leaves the worktree it was in.
    let previous = if proc::is_agent() && origin.is_none() && stays.is_none() {
        leaving_worktree()
    } else {
        None
    };
    let main = repo::main_dir();

    step("git fetch");
    proc::git(&["fetch", "--prune"], &main);
    if proc::git(&["rev-parse", "--verify", "--quiet", &branch], &main).is_ok {
        fail(&format!(
            "the branch {branch} exists already: pnpm wt:list shows whether a worktree holds it; else start the task under another name"
        ));
    }
    // The base: given, else a hotfix's develop, the orchestration's epic, the product's batch (#411), develop.
    let base = given_base.unwrap_or_else(|| {
        super::batch::base_of_new(&feature, origin_name.as_deref(), args.has("--fix"))
            .unwrap_or_else(|why| fail(&format!("no batch to branch off: {why}")))
    });
    if !proc::git(&["rev-parse", "--verify", "--quiet", &base.remote], &main).is_ok {
        fail(&format!(
            "origin has no branch {}: nothing to start from. git fetch origin, or name the base with --base <branch>",
            base.local
        ));
    }
    if proc::git(&["ls-remote", "--exit-code", "--heads", "origin", &branch], &main).is_ok {
        fail(&format!(
            "origin has a branch {branch} already: someone claimed this work. Say so instead of starting (pnpm wt:issues lists the claims)"
        ));
    }
    // One claim check with wt:orchestrate's (handover.rs): `feature/34-…`, `fix/34-…`, not `…/134-…`.
    // An origin that doesn't answer stops nothing here: the push of the claim says so.
    if let Ok(Some(taken)) = claim_on_origin(&main, &feature) {
        fail(&format!(
            "origin has the branch {taken} — someone works on that issue already"
        ));
    }

    let dir_text = dir.to_string_lossy().into_owned();
    step(&format!("git worktree add {dir_text} -b {branch} {}", base.remote));
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
            &base.remote,
        ],
        How {
            cwd: Some(&main),
            ..How::default()
        },
    );
    conversations::share_conversations();
    let written = origin
        .as_ref()
        .map_or(Ok(()), |origin| record::write_origin(&dir, origin))
        .and_then(|()| record::write_base(&dir, &base.local));
    if let Err(error) = written {
        fail(&format!(
            "couldn't write {}: {error}",
            record::origin_file(&dir).display()
        ));
    }
    // A step that fails names the commands that finish the worktree from there.
    let loaded = breadboard_core::config::of(&main);
    let setup = loaded.config.worktrees.setup.clone();
    let main_env = repo::env_file_of(&main);
    let stack_step = if !main_env.exists() {
        None
    } else if args.has("--shared") {
        Some(format!(
            "cp {} {}",
            main_env.display(),
            repo::env_file_of(&dir).display()
        ))
    } else {
        Some(loaded.command("wt:stack"))
    };
    let finish = |rest: &[breadboard_core::config::Step]| {
        let rest: Vec<&str> = rest
            .iter()
            .map(|step| step.run.as_str())
            .chain(stack_step.as_deref())
            .collect();
        format!(
            "the worktree is only half set up: fix the cause, then finish it in {dir_text}: {}; or start over with {} {feature}",
            rest.join(" && "),
            loaded.command("wt:remove")
        )
    };
    set_hint(&finish(&setup));

    step(&format!(
        "push guard: {} (#184)",
        crate::cli::write::mr::install_push_guard(&dir)
    ));
    if breadboard_core::ci::set_push_option(&dir, breadboard_core::time::now_ms())
        == Some(breadboard_core::ci::PushOption::Set)
    {
        step("CI: pushes skip CI, breadboard's queue starts the pipelines (#357)");
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

    // breadboard.toml's `[[worktrees.setup]]` in order (the install, what lint loads); a quiet one
    // shows its output only when it fails, every one quiet for an agent.
    for (at, run) in setup.iter().enumerate() {
        if let Some(says) = &run.says {
            step(says);
        } else if !run.is_quiet {
            step(&run.run);
        }
        proc::run(
            "sh",
            &["-c", &run.run],
            How {
                cwd: Some(&dir),
                is_quiet: run.is_quiet || proc::is_agent(),
                ..How::default()
            },
        );
        set_hint(&finish(&setup[at + 1..]));
    }

    let has_stack = loaded.config.stack.is_some();
    let offset = if !has_stack {
        // No [stack] in breadboard.toml: nothing to share or own.
        0
    } else if !main_env.exists() {
        outln!(
            "\n! {} is missing, the worktree has no env file and no stack",
            main_env.display()
        );
        0
    } else if args.has("--shared") {
        step("share the dev stack of the main checkout (copy its env file)");
        let target = repo::env_file_of(&dir);
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(error) = fs::copy(&main_env, &target) {
            fail(&format!("couldn't copy {}: {error}", main_env.display()));
        }
        0
    } else if let Some(epic) = Epic::of_task(&dir) {
        // An orchestration's task: its stack starts when needed (#351).
        step(&format!(
            "task stack on the epic {}: its .env.local, started by {run} stack:up",
            epic.name,
            run = breadboard_core::config::run_word()
        ));
        epic::reserve_task(&dir, &feature, &epic)
    } else {
        stack::create_stack(&dir, &feature)
    };
    let epic_port = epic::epic_port_of(&dir);
    // Set up: what follows only tells and arranges, nothing to finish by hand.
    set_hint("");
    // GitLab not answering costs the count, never the worktree; an orchestration's skips the sweep.
    let silent = if origin.is_some() {
        0
    } else {
        sweep::stale_issue_count().unwrap_or(0)
    };
    // No herdr space of its own (#409): its rows in the overview are where it shows.
    // Its stack set up and running: its dev server in the background (#407).
    let procs = stack::procs_of(&dir);
    if !procs.is_empty() {
        step(&format!("in the background: {}", procs.join(" ")));
    }
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
        tab: None,
        stale,
        silent,
        left,
        origin: origin.as_ref().map(|origin| origin.name.as_str()),
        is_stack_only: false,
        is_stackless: has_stack && !main_env.exists(),
        is_unpushed: !pushed.is_ok,
        epic_port,
    });
    ExitCode::SUCCESS
}
