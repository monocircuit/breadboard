//! `wt:orchestrate <name> --retire <feature> --mr <iid>` (#381): an agent
//! above `[agents] fresh_above` retires at its merge instead of taking a
//! next issue. Its claude ends, one last run of its conversation writes its
//! handoff into `.claude/handoff.md` of its worktree (what it learned about
//! the files it changed, a page at most), the handoff is kept next to the
//! record, and the record marks it retired. The overview runs this before
//! the worktree goes; a fresh agent on those files reads the handoff.

use super::args::Args;
use super::claude::{bound_session, context_share_of};
use super::handover::restart::{self, Machine};
use super::handover::{context_limit, pane_of_conversation};
use super::herdr::s;
use super::proc::{Done, fail, step};
use super::repo::Repo;
use breadboard_core::git::work_files;
use breadboard_core::record::{self, handoff_file, is_name, write_worker};
use breadboard_core::runs::{self, args_of};
use breadboard_core::time::{now_ms, to_iso};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: pnpm wt:orchestrate <name> --retire <feature> --mr <iid>";
/// Where the handoff is written in the worktree, and where a fresh agent finds it in its own.
pub const HANDOFF: &str = ".claude/handoff.md";
/// How long the last run may take.
const LAST_RUN: Duration = Duration::from_secs(10 * 60);
/// The files the prompt names at most.
const NAMED: usize = 20;

/// Whether `wt:orchestrate <words>` retires an agent: `--retire` given.
pub fn is_retire(words: &[String]) -> bool {
    words
        .iter()
        .any(|word| word == "--retire" || word.starts_with("--retire="))
}

/// The line that ends an agent's claude in its pane before its last run.
pub fn retire_line(iid: i64, share: f64, limit: f64) -> String {
    format!(
        "[overview] !{iid} merged, your context is at {} %, above {} %: you retire. Your claude ends now, nothing to do or answer; one last run writes your handoff, a fresh agent takes the next issue.",
        percent(share),
        percent(limit)
    )
}

/// The prompt of the last run: the handoff, a page at most, facts only.
pub fn handoff_prompt(iid: i64, share: f64, limit: f64, files: &[String]) -> String {
    let named: Vec<&str> = files.iter().take(NAMED).map(String::as_str).collect();
    let more = if files.len() > NAMED {
        format!(" and {} more", files.len() - NAMED)
    } else {
        String::new()
    };
    let files = if named.is_empty() {
        "the files you worked on".to_string()
    } else {
        format!("the files you changed ({}{more})", named.join(", "))
    };
    format!(
        "[overview] !{iid} is merged. Your context is at {} %, above {} %: you retire, and a fresh agent takes the next issue on these files. Write your handoff now into {HANDOFF} of this worktree, at most one page: what you learned about {files}: where things live, the rules and pitfalls you found, what is unfinished or fragile. Facts a newcomer needs, not the story of your issue. Write only that file, then stop: nothing else to do or answer.",
        percent(share),
        percent(limit)
    )
}

fn percent(share: f64) -> i64 {
    (share * 100.0).round() as i64
}

pub fn command(args: &Args) -> Done {
    let (Some(name), Some(feature), Some(iid)) = (
        args.words().first().filter(|name| is_name(name)),
        args.value("--retire"),
        args.value("--mr")
            .and_then(|iid| iid.trim_start_matches('!').parse::<i64>().ok()),
    ) else {
        return fail(USAGE);
    };
    let repo = Repo::here()?;
    let dir = repo.dir_of(feature);
    let Some(id) = bound_session(&dir) else {
        return fail(format!("{feature} has no agent bound to it ({})", dir.display()));
    };
    let share = context_share_of(&id).unwrap_or(0.0);
    let limit = context_limit(&repo.main);
    let agent = record::record_of(&repo.main, name)
        .and_then(|record| record.worker(&id).cloned())
        .unwrap_or_default();
    let shown = agent.name.clone().unwrap_or_else(|| feature.to_string());
    step(&format!(
        "{shown} retires: its context is at {} %, above {} %",
        percent(share),
        percent(limit)
    ));
    // A pane runs it (not headless): its claude ends first, the last run takes its conversation.
    if !runs::is_headless(&repo.main)
        && let Ok(Some(pane)) = pane_of_conversation(&id)
        && let Err(why) = restart::retire(&Machine, &id, s(&pane, "pane_id"), &retire_line(iid, share, limit))
    {
        return fail(format!("{shown} not retired: {why}"));
    }
    let changed: Vec<String> = work_files(&dir).into_iter().collect();
    let written = dir.join(HANDOFF);
    let _ = fs::remove_file(&written);
    step(&format!("{shown}'s last run writes its handoff ({HANDOFF})"));
    let ran = last_run(&dir, &id, feature, &handoff_prompt(iid, share, limit, &changed));
    let kept = handoff_file(&repo.main, name, &id);
    let handoff = match fs::read_to_string(&written).ok().filter(|text| !text.trim().is_empty()) {
        Some(text) => {
            if let Some(folder) = kept.parent() {
                let _ = fs::create_dir_all(folder);
            }
            fs::write(&kept, text).ok().map(|_| relative(&repo.main, &kept))
        }
        None => {
            step(&format!(
                "{shown} wrote no handoff ({ran}): the fresh agent starts without one"
            ));
            None
        }
    };
    let mut files = agent.files.clone();
    for file in changed {
        if !files.contains(&file) {
            files.push(file);
        }
    }
    let mut mrs = agent.mrs.clone();
    if !mrs.contains(&iid) {
        mrs.push(iid);
    }
    let fields = json!({
        "name": shown,
        "files": files,
        "mrs": mrs,
        "free": null,
        "retired": {
            "at": to_iso(now_ms()),
            "from": feature,
            "mr": iid,
            "context": share,
            "handoff": handoff,
        },
    });
    if let Err(error) = write_worker(&repo.main, name, &id, fields.as_object().cloned().unwrap_or_default()) {
        return fail(format!("the record of {name} could not be written: {error}"));
    }
    // Unbound: nothing resumes it in the worktree, and wt:remove can take it.
    let _ = fs::remove_file(breadboard_core::conversations::session_file(&dir));
    super::proc::done(&format!(
        "{shown} retired at {} %{}",
        percent(share),
        if handoff.is_some() {
            format!(", its handoff kept: {}", relative(&repo.main, &kept))
        } else {
            String::new()
        }
    ));
    Ok(())
}

/// One run of the conversation `id` in `dir` with `prompt`, waited for up to
/// `LAST_RUN`: what became of it, for the line when no handoff came.
fn last_run(dir: &Path, id: &str, name: &str, prompt: &str) -> String {
    let spawned = Command::new("claude")
        .args(args_of(id, name, prompt, true, &[]))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return format!("claude didn't start: {error}"),
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return "its run ended".into(),
            Ok(Some(status)) => return format!("its run failed: {status}"),
            Ok(None) if started.elapsed() > LAST_RUN => {
                let _ = child.kill();
                return format!("its run took over {} minutes", LAST_RUN.as_secs() / 60);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(500)),
            Err(error) => return format!("its run: {error}"),
        }
    }
}

/// `path` as the main checkout names it.
fn relative(main: &Path, path: &Path) -> String {
    path.strip_prefix(main).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_a_page_on_the_files_it_changed() {
        let prompt = handoff_prompt(313, 0.47, 0.4, &["a.rs".into(), "b.rs".into()]);
        assert!(prompt.starts_with("[overview] !313 is merged. Your context is at 47 %, above 40 %: you retire"));
        assert!(prompt.contains("into .claude/handoff.md of this worktree, at most one page"));
        assert!(prompt.contains("the files you changed (a.rs, b.rs)"));
        let many: Vec<String> = (0..25).map(|n| format!("f{n}")).collect();
        assert!(handoff_prompt(1, 0.5, 0.4, &many).contains(", f19 and 5 more)"));
        assert!(handoff_prompt(1, 0.5, 0.4, &[]).contains("about the files you worked on:"));
        assert_eq!(
            retire_line(313, 0.47, 0.4),
            "[overview] !313 merged, your context is at 47 %, above 40 %: you retire. Your claude ends now, nothing to do or answer; one last run writes your handoff, a fresh agent takes the next issue."
        );
    }

    #[test]
    fn is_a_retirement_only_with_its_flag() {
        let words = |list: &[&str]| list.iter().map(|word| word.to_string()).collect::<Vec<_>>();
        assert!(is_retire(&words(&["b", "--retire", "40-x", "--mr", "313"])));
        assert!(is_retire(&words(&["b", "--retire=40-x"])));
        assert!(!is_retire(&words(&["b", "40", "--to", "x"])));
    }
}
