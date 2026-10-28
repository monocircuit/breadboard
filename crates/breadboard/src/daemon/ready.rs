//! The MRs of a repo marked ready by their bars (#399), run by the daemon
//! beside the CI queue while any overview of that repo is open: every EVERY
//! it reads the open MRs and the orchestrations' published bars, and core's
//! `ready::steps_of` says which to mark ready or put back to draft. The user
//! asked for these writes (#399, rewrite rule 6): the title of an
//! orchestration's MR, `Draft:` off or on, nothing else. GitLab read back
//! right before (still at the head decided for, still a draft or not) and
//! after (its `draft` as written); each done step a line in the
//! orchestration's journal. Only for a repo GitLab serves: a gauntlet world
//! opts in with its push URL, as for the CI queue.

use super::ci::every;
use super::glab::{glab_api, glab_get};
use crate::tui::overview::actor::glab::ready_title;
use breadboard_core::ci;
use breadboard_core::gitlab::client::Gitlab;
use breadboard_core::gitlab::read::open_mrs_of;
use breadboard_core::ready::{self, Mark, Marks, Step};
use breadboard_core::time::now_ms;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The marks in the request log (#462).
const TASK: &str = "ready marks";

/// The marks of `main`, as long as the daemon runs.
pub async fn mark(main: PathBuf) {
    let gitlab = Gitlab::new(&main).for_task(TASK);
    let mut tick = tokio::time::interval(every());
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if !main.is_dir() {
            return;
        }
        let is_on = {
            let main = main.clone();
            tokio::task::spawn_blocking(move || ci::is_served(&main) && ci::holds(&main, &ready::marker_file(&main)))
                .await
                .unwrap_or(false)
        };
        if !is_on {
            continue;
        }
        let Ok(project) = super::ci::open_mrs_project(&gitlab, &main).await else {
            continue;
        };
        let mrs = open_mrs_of(&project);
        let main = main.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut marks = Marks::read(&main);
            let before = marks.clone();
            for step in ready::steps_of(&mrs, &ready::published_by_name(&main), &marks) {
                if apply(&main, "glab", &step).is_ok() {
                    marks.mrs.insert(
                        step.iid,
                        Mark {
                            sha: step.sha.clone(),
                            is_ready: step.is_ready,
                        },
                    );
                    let _ = breadboard_core::journal::append(
                        &main,
                        &step.orchestration,
                        &ready::journal_line(&step),
                        now_ms(),
                    );
                }
            }
            ready::forget_closed(&mut marks, &mrs);
            if marks != before {
                let _ = ci::write_whole(&ready::ready_file(&main), &marks);
            }
        })
        .await;
    }
}

/// One step: the MR read first (still at its head, its draft not yet as
/// decided), its title written, read back as decided.
fn apply(main: &Path, glab: &str, step: &Step) -> Result<(), String> {
    let endpoint = format!("projects/:fullpath/merge_requests/{}", step.iid);
    let mr = glab_get(main, glab, &endpoint)?;
    let is_draft = |mr: &Value| mr.get("draft").and_then(Value::as_bool).unwrap_or(false);
    if mr.get("sha").and_then(Value::as_str) != Some(step.sha.as_str()) || is_draft(&mr) != step.is_ready {
        return Err("pushed or changed meanwhile".into());
    }
    let title = ready_title(mr.get("title").and_then(Value::as_str).unwrap_or_default());
    let title = if step.is_ready {
        title
    } else {
        format!("Draft: {title}")
    };
    let field = format!("title={title}");
    glab_api(main, glab, &["api", "--method", "PUT", &endpoint, "--field", &field])?;
    let back = glab_get(main, glab, &endpoint)?;
    if is_draft(&back) == step.is_ready {
        return Err(format!(
            "!{} still {}",
            step.iid,
            if step.is_ready { "a draft" } else { "ready" }
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A glab that answers the MR's GET from `state` (a file it rewrites on the PUT), each call logged.
    fn fake_glab(scratch: &Path, sha: &str, is_draft: bool) -> String {
        std::fs::create_dir_all(scratch).unwrap();
        let state = scratch.join("draft");
        std::fs::write(&state, if is_draft { "true" } else { "false" }).unwrap();
        let log = scratch.join("glab.log");
        let glab = scratch.join("glab");
        std::fs::write(
            &glab,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{log}'\n\
                 case \"$*\" in\n\
                 *'--method PUT'*'title=Draft:'*) echo true > '{state}'; echo '{{}}' ;;\n\
                 *'--method PUT'*) echo false > '{state}'; echo '{{}}' ;;\n\
                 *) echo \"{{\\\"sha\\\":\\\"{sha}\\\",\\\"title\\\":\\\"Draft: x\\\",\\\"draft\\\":$(cat '{state}')}}\" ;;\n\
                 esac\n",
                log = log.display(),
                state = state.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
        glab.to_string_lossy().into_owned()
    }

    fn step(sha: &str, is_ready: bool) -> Step {
        Step {
            iid: 383,
            sha: sha.into(),
            is_ready,
            orchestration: "breadboard".into(),
        }
    }

    #[test]
    fn marks_ready_and_back_reading_gitlab_before_and_after() {
        let scratch = std::env::temp_dir().join(format!("ready-apply-test-{}", std::process::id()));
        let glab = fake_glab(&scratch, "a", true);
        assert_eq!(apply(&scratch, &glab, &step("a", true)), Ok(()));
        let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
        assert_eq!(
            log,
            "api projects/:fullpath/merge_requests/383\n\
             api --method PUT projects/:fullpath/merge_requests/383 --field title=x\n\
             api projects/:fullpath/merge_requests/383\n"
        );
        assert_eq!(apply(&scratch, &glab, &step("a", false)), Ok(()));
        assert!(
            std::fs::read_to_string(scratch.join("glab.log"))
                .unwrap()
                .contains("--field title=Draft: x")
        );
        // Pushed meanwhile: nothing written.
        let _ = std::fs::remove_file(scratch.join("glab.log"));
        assert!(apply(&scratch, &glab, &step("b", true)).is_err());
        assert!(
            !std::fs::read_to_string(scratch.join("glab.log"))
                .unwrap()
                .contains("PUT")
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
