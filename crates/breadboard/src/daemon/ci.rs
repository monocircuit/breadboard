//! The CI queue of a repo (#357), run by the daemon while any overview of
//! that repo is open: every EVERY it reads the open MRs and the runner's
//! slots, creates at most one MR pipeline (core's `ci::decide`) and writes
//! where each MR stands to `.claude/ci/queue.json` for the rows. The user
//! asked for these writes (#357, rewrite rule 6): a pipeline created, its
//! superseded one cancelled, nothing else. Off with `git config
//! breadboard.ci push`: pushes start their pipelines themselves then. With
//! `BREADBOARD_CI_DRY` set (a try) it decides and says so, and writes nothing
//! to GitLab.

use breadboard_core::ci::{self, Decision, MODE_KEY, Queue, Ranks, RunNow, Slots};
use breadboard_core::gitlab::client::Gitlab;
use breadboard_core::gitlab::read::{CI_QUERY, open_mrs_of};
use breadboard_core::time::now_ms;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// How often the queue is looked at.
pub const EVERY: Duration = Duration::from_secs(10);
/// How long after making room for an MR ahead the queue makes room again:
/// the runner's containers take a while to go, the slot shows free only then.
const PREEMPT_EVERY: Duration = Duration::from_secs(30);
/// How long a click keeps an MR up front when its pipeline never came.
const RUN_NOW_HOLDS_MS: i64 = 60 * 60_000;

/// Whether `main` leaves its pipelines to the queue (the default) or to the pushes.
pub fn is_queued(main: &Path) -> bool {
    let out = breadboard_core::git::git(main, &["config", "--get", MODE_KEY]);
    out.out() != "push"
}

/// The queue of `main`, as long as the daemon runs.
pub async fn schedule(main: PathBuf) {
    let gitlab = Gitlab::new(&main);
    let mut started: HashMap<i64, (String, i64)> = HashMap::new();
    // The commits whose pipeline the queue cancelled for an MR ahead: queued again.
    let mut preempted: HashMap<i64, String> = HashMap::new();
    let mut preempted_at: Option<std::time::Instant> = None;
    let is_dry = std::env::var_os("BREADBOARD_CI_DRY").is_some();
    let mut last: Option<String> = None;
    let mut tick = tokio::time::interval(EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        // Its repo gone (a gauntlet world removed): nothing to queue.
        if !main.is_dir() {
            return;
        }
        let is_on = {
            let main = main.clone();
            tokio::task::spawn_blocking(move || is_queued(&main))
                .await
                .unwrap_or(false)
        };
        if !is_on {
            let _ = std::fs::remove_file(ci::queue_file(&main));
            continue;
        }
        let Ok(project) = gitlab.graphql(CI_QUERY, json!({})).await else {
            continue;
        };
        let mrs = open_mrs_of(&project);
        let main_now = main.clone();
        let asked = std::mem::take(&mut started);
        let cancelled = preempted.clone();
        let step = tokio::task::spawn_blocking(move || {
            let now = now_ms();
            let running = mrs
                .iter()
                .filter(|mr| {
                    mr.pipeline
                        .as_ref()
                        .is_some_and(|pipeline| pipeline.status == "running")
                })
                .count() as u32;
            let slots = Slots::read(running);
            let run_now: RunNow = ci::read_or_default(&ci::run_now_file(&main_now));
            let mut ranks = Ranks::of(&ci::published_of(&main_now), &run_now);
            ranks.preempted = cancelled;
            (
                ci::decide(&mrs, &ranks, slots, &asked, now),
                slots,
                run_now,
                mrs,
                asked,
                now,
            )
        });
        let Ok((decision, slots, run_now, mrs, was_started, now)) = step.await else {
            continue;
        };
        started = was_started;
        let Decision { places, start, preempt } = decision;
        if is_dry {
            last = start
                .as_ref()
                .map(|start| format!("would start !{}", start.iid))
                .or(preempt
                    .as_ref()
                    .map(|(iid, _, _)| format!("would hold !{iid} for the MR ahead")))
                .or(last);
        } else if let Some((iid, id, sha)) =
            preempt.filter(|_| preempted_at.is_none_or(|at| at.elapsed() >= PREEMPT_EVERY))
        {
            preempted_at = Some(std::time::Instant::now());
            let main = main.clone();
            let said = tokio::task::spawn_blocking(move || {
                glab_post(&main, "glab", &format!("projects/:fullpath/pipelines/{id}/cancel"))
            })
            .await;
            if let Ok(said) = said {
                last = Some(match said {
                    Ok(()) => {
                        preempted.insert(iid, sha);
                        format!("!{iid} held for the MR ahead")
                    }
                    Err(error) => format!("!{iid} not held: {}", first_line(&error)),
                });
            }
        }
        if let Some(start) = start.filter(|_| !is_dry) {
            preempted.remove(&start.iid);
            let main = main.clone();
            let done = tokio::task::spawn_blocking(move || {
                let said = run(&main, &start);
                (start, said)
            })
            .await;
            if let Ok((start, said)) = done {
                // Asked for, whether GitLab took it or not: a refusal (no job for
                // these changes) isn't asked again for the same commit.
                started.insert(start.iid, (start.sha.clone(), now));
                last = Some(match said {
                    Ok(()) => format!("started !{}", start.iid),
                    Err(error) => format!("!{} not started: {}", start.iid, first_line(&error)),
                });
            }
        }
        // What was asked for of MRs no longer open goes, a click whose pipeline ended too.
        let open: HashMap<i64, bool> = mrs
            .iter()
            .map(|mr| {
                let is_live = mr
                    .pipeline
                    .as_ref()
                    .is_some_and(|pipeline| pipeline.sha == mr.sha && ci::is_live(&pipeline.status));
                (mr.iid, is_live)
            })
            .collect();
        started.retain(|iid, _| open.contains_key(iid));
        preempted.retain(|iid, _| open.contains_key(iid));
        forget_clicks(&main, &run_now, &started, &open, now);
        let queue = Queue {
            at: now,
            slots,
            places,
            last: last.clone(),
            is_dry,
        };
        let _ = ci::write_whole(&ci::queue_file(&main), &queue);
    }
}

/// The clicks that are done: their MR closed, the pipeline asked for since
/// ended (while it runs the click keeps it up front, never cancelled for
/// another), or an hour gone. `open`: iid → whether its newest commit's pipeline runs.
fn forget_clicks(
    main: &Path,
    run_now: &RunNow,
    started: &HashMap<i64, (String, i64)>,
    open: &HashMap<i64, bool>,
    now: i64,
) {
    let is_done = |iid: &i64, at: &i64| match open.get(iid) {
        None => true,
        Some(is_live) => {
            now - at > RUN_NOW_HOLDS_MS || (!is_live && started.get(iid).is_some_and(|(_, asked)| asked >= at))
        }
    };
    if !run_now.at.iter().any(|(iid, at)| is_done(iid, at)) {
        return;
    }
    // Read again: a click may have come since.
    let file = ci::run_now_file(main);
    let mut fresh: RunNow = ci::read_or_default(&file);
    fresh.at.retain(|iid, at| !is_done(iid, at));
    let _ = ci::write_whole(&file, &fresh);
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// Its superseded pipeline cancelled, then the pipeline of the MR's newest commit created.
fn run(main: &Path, start: &ci::Start) -> Result<(), String> {
    run_with(main, start, "glab")
}

fn run_with(main: &Path, start: &ci::Start, glab: &str) -> Result<(), String> {
    if let Some(id) = start.cancel {
        // A cancel that fails (finished meanwhile) doesn't stop the new one.
        let _ = glab_post(main, glab, &format!("projects/:fullpath/pipelines/{id}/cancel"));
    }
    glab_post(
        main,
        glab,
        &format!("projects/:fullpath/merge_requests/{}/pipelines", start.iid),
    )
}

/// `glab api --method POST <endpoint>` as the user, from the main checkout.
fn glab_post(main: &Path, glab: &str, endpoint: &str) -> Result<(), String> {
    let output = Command::new(glab)
        .args(["api", "--method", "POST", endpoint])
        .current_dir(main)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("glab: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        stderr
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn creates_the_pipeline_after_cancelling_the_superseded_one() {
        let scratch = std::env::temp_dir().join(format!("ci-run-test-{}", std::process::id()));
        let bin = scratch.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = scratch.join("glab.log");
        let glab = bin.join("glab");
        std::fs::write(
            &glab,
            format!("#!/bin/sh\necho \"$*\" >> '{}'\necho '{{}}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
        let start = ci::Start {
            iid: 339,
            sha: "a".into(),
            cancel: Some(70),
        };
        assert_eq!(run_with(&scratch, &start, &glab.to_string_lossy()), Ok(()));
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "api --method POST projects/:fullpath/pipelines/70/cancel\n\
             api --method POST projects/:fullpath/merge_requests/339/pipelines\n"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn forgets_a_click_once_its_pipeline_was_asked_for() {
        let scratch = std::env::temp_dir().join(format!("ci-click-test-{}", std::process::id()));
        let mut run_now = RunNow::default();
        run_now.at.insert(1, 1000);
        run_now.at.insert(2, 1000);
        run_now.at.insert(3, 1000);
        ci::write_whole(&ci::run_now_file(&scratch), &run_now).unwrap();
        let started = HashMap::from([(1, ("a".to_string(), 2000))]);
        // 1 asked for since and ended, 2 waits still, 3 closed.
        let open = HashMap::from([(1, false), (2, false)]);
        forget_clicks(&scratch, &run_now, &started, &open, 3000);
        let left: RunNow = ci::read_or_default(&ci::run_now_file(&scratch));
        assert_eq!(left.at.keys().copied().collect::<Vec<_>>(), vec![2]);
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
