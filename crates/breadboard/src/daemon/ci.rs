//! The CI queue of a repo (#357), run by the daemon while any overview of
//! that repo is open: every EVERY it reads the open MRs and the runner's
//! slots, creates at most one MR pipeline (core's `ci::decide`) and writes
//! where each MR stands to `.claude/ci/queue.json` for the rows. The user
//! asked for these writes (#357, rewrite rule 6): a pipeline created, its
//! superseded one cancelled, nothing else; only for the orchestrations' MRs,
//! one per commit, read back from GitLab right before (#369). What it asked
//! for is kept in `.claude/ci/asked.json`, one daemon per checkout schedules.
//! Off with `git config breadboard.ci push` (the kill switch): pushes start
//! their pipelines themselves then. With `BREADBOARD_CI_DRY` set (a try) it
//! decides and says so, and writes nothing to GitLab.

use breadboard_core::ci::{self, Asked, Decision, MODE_KEY, Queue, Ranks, RunNow, Slots};
use breadboard_core::gitlab::client::Gitlab;
use breadboard_core::gitlab::read::{CI_QUERY, open_mrs_of};
use breadboard_core::time::now_ms;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// How often the queue is looked at; `BREADBOARD_CI_EVERY_MS` another (the gauntlet's ticks).
pub const EVERY: Duration = Duration::from_secs(10);

pub(super) fn every() -> Duration {
    std::env::var("BREADBOARD_CI_EVERY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(EVERY)
}
/// How long after making room for an MR ahead the queue makes room again:
/// the runner's containers take a while to go, the slot shows free only then.
const PREEMPT_EVERY: Duration = Duration::from_secs(30);
/// How long a click keeps an MR up front when its pipeline never came.
const RUN_NOW_HOLDS_MS: i64 = 60 * 60_000;

/// Whether `main` leaves its pipelines to the queue (the default) or to the
/// pushes: `breadboard.ci = push`, or an origin that takes no push options
/// (a gauntlet world's), whose pushes never skip CI (`ci::set_push_option`).
pub fn is_queued(main: &Path) -> bool {
    let git = |args: &[&str]| breadboard_core::git::git(main, args);
    let origin = git(&["remote", "get-url", "--push", "origin"]);
    origin.is_ok && ci::is_network_url(origin.out()) && git(&["config", "--get", MODE_KEY]).out() != "push"
}

/// The queue of `main`, as long as the daemon runs.
pub async fn schedule(main: PathBuf) {
    let gitlab = Gitlab::new(&main);
    let mut preempted_at: Option<std::time::Instant> = None;
    let is_dry = std::env::var_os("BREADBOARD_CI_DRY").is_some();
    let mut last: Option<String> = None;
    let mut tick = tokio::time::interval(every());
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
        // Another daemon schedules this checkout: it writes the queue too.
        if !ci::holds_scheduler(&main) {
            continue;
        }
        // Read anew each tick: what a daemon before asked for counts.
        let mut asked: Asked = ci::read_or_default(&ci::asked_file(&main));
        let Ok(project) = gitlab.graphql(CI_QUERY, json!({})).await else {
            continue;
        };
        let mrs = open_mrs_of(&project);
        let main_now = main.clone();
        let started: HashMap<i64, (String, i64)> = asked.started.clone().into_iter().collect();
        let cancelled: HashMap<i64, String> = asked.preempted.clone().into_iter().collect();
        let ours = asked.ours.iter().copied().collect();
        let rescued = asked.rescued.clone().into_iter().collect();
        let own_cancels = asked.cancelled.iter().copied().collect();
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
            ranks.ours = ours;
            ranks.rescued = rescued;
            ranks.cancelled = own_cancels;
            (ci::decide(&mrs, &ranks, slots, &started), slots, run_now, mrs, now)
        });
        let Ok((decision, slots, run_now, mrs, now)) = step.await else {
            continue;
        };
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
                    Ok(_) => {
                        asked.preempted.insert(iid, sha);
                        asked.add_cancelled(id);
                        format!("!{iid} held for the MR ahead")
                    }
                    Err(error) => format!("!{iid} not held: {}", first_line(&error)),
                });
            }
        }
        if let Some(start) = start.filter(|_| !is_dry) {
            let main = main.clone();
            let done = tokio::task::spawn_blocking(move || {
                let said = run(&main, &start);
                (start, said)
            })
            .await;
            if let Ok((start, said)) = done {
                // Its superseded pipeline, cancelled by the queue: no person's cancel (#383).
                if let (Some(id), false) = (start.cancel, matches!(said, Ok(Ran::Superseded | Ran::Exists(_)))) {
                    asked.add_cancelled(id);
                }
                // Asked for, whether GitLab took it or not: a refusal (no job for
                // these changes, the activity limit) isn't asked again for the
                // same commit; a commit superseded meanwhile is decided anew.
                if !matches!(said, Ok(Ran::Superseded)) {
                    asked.preempted.remove(&start.iid);
                    asked.started.insert(start.iid, (start.sha.clone(), now));
                    if let Some(blocking) = start.after {
                        asked.rescued.insert(start.iid, blocking);
                    }
                }
                last = Some(match said {
                    Ok(Ran::Created(id)) => {
                        if let Some(id) = id {
                            asked.add_ours(id);
                        }
                        format!("started !{}", start.iid)
                    }
                    Ok(Ran::Exists(id)) => format!("!{} has pipeline {id} of its commit", start.iid),
                    Ok(Ran::Superseded) => format!("!{} pushed meanwhile", start.iid),
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
        asked.started.retain(|iid, _| open.contains_key(iid));
        asked.preempted.retain(|iid, _| open.contains_key(iid));
        asked.rescued.retain(|iid, _| open.contains_key(iid));
        if !is_dry {
            let _ = ci::write_whole(&ci::asked_file(&main), &asked);
        }
        let started: HashMap<i64, (String, i64)> = asked.started.into_iter().collect();
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

/// What `run` did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ran {
    /// Created, its id when GitLab said it.
    Created(Option<i64>),
    /// GitLab has one of that commit already: none created (#369).
    Exists(i64),
    /// The MR has a newer commit than the one decided for: nothing done.
    Superseded,
}

/// GitLab read back first (#369: its list may lag or be read wrong): the MR
/// still at the commit decided for, and no pipeline of that commit but a
/// cancelled or skipped one (a click's: none still to finish; one replacing
/// a blocking pipeline: none newer than that one). Then its
/// superseded pipeline cancelled, and the pipeline of the MR's newest commit created.
fn run(main: &Path, start: &ci::Start) -> Result<Ran, String> {
    run_with(main, start, "glab")
}

fn run_with(main: &Path, start: &ci::Start, glab: &str) -> Result<Ran, String> {
    let mr = glab_get(main, glab, &format!("projects/:fullpath/merge_requests/{}", start.iid))?;
    if mr.get("sha").and_then(serde_json::Value::as_str) != Some(start.sha.as_str()) {
        return Ok(Ran::Superseded);
    }
    let pipelines = glab_get(
        main,
        glab,
        &format!("projects/:fullpath/merge_requests/{}/pipelines?per_page=100", start.iid),
    )?;
    let existing = pipelines.as_array().into_iter().flatten().find(|pipeline| {
        let status = pipeline
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let id = pipeline.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0);
        pipeline.get("sha").and_then(serde_json::Value::as_str) == Some(start.sha.as_str())
            && start.after.is_none_or(|blocking| id > blocking)
            && if start.is_again {
                ci::is_live(status)
            } else {
                !matches!(status, "canceled" | "canceling" | "skipped")
            }
    });
    if let Some(pipeline) = existing {
        return Ok(Ran::Exists(
            pipeline.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0),
        ));
    }
    if let Some(id) = start.cancel {
        // A cancel that fails (finished meanwhile) doesn't stop the new one.
        let _ = glab_post(main, glab, &format!("projects/:fullpath/pipelines/{id}/cancel"));
    }
    let created = glab_post(
        main,
        glab,
        &format!("projects/:fullpath/merge_requests/{}/pipelines", start.iid),
    )?;
    Ok(Ran::Created(created.get("id").and_then(serde_json::Value::as_i64)))
}

/// `glab api <endpoint>` as the user, from the main checkout: its JSON.
pub(super) fn glab_get(main: &Path, glab: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    glab_api(main, glab, &["api", endpoint])
}

/// `glab api --method POST <endpoint>` as the user, from the main checkout: its JSON.
pub(super) fn glab_post(main: &Path, glab: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    glab_api(main, glab, &["api", "--method", "POST", endpoint])
}

/// `glab api --method PUT <endpoint>` as the user, from the main checkout: its JSON.
pub(super) fn glab_put(main: &Path, glab: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    glab_api(main, glab, &["api", "--method", "PUT", endpoint])
}

pub(super) fn glab_api(main: &Path, glab: &str, args: &[&str]) -> Result<serde_json::Value, String> {
    let spawn = || {
        Command::new(glab)
            .args(args)
            .current_dir(main)
            .stdin(Stdio::null())
            .output()
    };
    // A glab just written (a test's) is busy while another thread forks: a moment later it runs.
    let mut output = spawn();
    for _ in 0..20 {
        if !matches!(&output, Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
        output = spawn();
    }
    let output = output.map_err(|error| format!("glab: {error}"))?;
    if output.status.success() {
        return Ok(serde_json::from_slice(&output.stdout).unwrap_or(serde_json::Value::Null));
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

    /// A glab answering from `answers` (`$*` → its JSON), each call logged.
    fn fake_glab(scratch: &Path, answers: &[(&str, &str)]) -> String {
        let bin = scratch.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = scratch.join("glab.log");
        let cases: String = answers
            .iter()
            .map(|(args, json)| format!("  '{args}') echo '{json}' ;;\n"))
            .collect();
        let glab = bin.join("glab");
        std::fs::write(
            &glab,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n{cases}  *) echo '{{}}' ;;\nesac\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
        glab.to_string_lossy().into_owned()
    }

    const MR: &str = "api projects/:fullpath/merge_requests/339";
    const PIPELINES: &str = "api projects/:fullpath/merge_requests/339/pipelines?per_page=100";

    #[test]
    fn creates_the_pipeline_after_reading_gitlab_and_cancelling_the_superseded_one() {
        let scratch = std::env::temp_dir().join(format!("ci-run-test-{}", std::process::id()));
        let glab = fake_glab(
            &scratch,
            &[
                (MR, r#"{"sha":"a"}"#),
                (
                    PIPELINES,
                    r#"[{"id":70,"sha":"old","status":"running"},{"id":60,"sha":"a","status":"canceled"}]"#,
                ),
                (
                    "api --method POST projects/:fullpath/merge_requests/339/pipelines",
                    r#"{"id":71}"#,
                ),
            ],
        );
        let start = ci::Start {
            iid: 339,
            sha: "a".into(),
            cancel: Some(70),
            is_again: false,
            after: None,
        };
        assert_eq!(run_with(&scratch, &start, &glab), Ok(Ran::Created(Some(71))));
        assert_eq!(
            std::fs::read_to_string(scratch.join("glab.log")).unwrap(),
            format!(
                "{MR}\n{PIPELINES}\napi --method POST projects/:fullpath/pipelines/70/cancel\n\
                 api --method POST projects/:fullpath/merge_requests/339/pipelines\n"
            )
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn creates_none_when_gitlab_has_one_of_the_commit_or_a_newer_commit() {
        // #369: !337 got seven pipelines of e426e44f, one failed at its creation.
        let scratch = std::env::temp_dir().join(format!("ci-exists-test-{}", std::process::id()));
        let start = ci::Start {
            iid: 339,
            sha: "a".into(),
            cancel: Some(70),
            is_again: false,
            after: None,
        };
        for (status, ran) in [("pending", Ran::Exists(80)), ("failed", Ran::Exists(80))] {
            let pipelines = format!(r#"[{{"id":80,"sha":"a","status":"{status}"}}]"#);
            let glab = fake_glab(&scratch, &[(MR, r#"{"sha":"a"}"#), (PIPELINES, &pipelines)]);
            assert_eq!(run_with(&scratch, &start, &glab), Ok(ran));
        }
        // A click asks again for one that ended, never while one runs.
        let again = ci::Start {
            is_again: true,
            ..start.clone()
        };
        let glab = fake_glab(
            &scratch,
            &[
                (MR, r#"{"sha":"a"}"#),
                (PIPELINES, r#"[{"id":80,"sha":"a","status":"failed"}]"#),
            ],
        );
        assert_eq!(run_with(&scratch, &again, &glab), Ok(Ran::Created(None)));
        // Over a blocking cancelled one: the older pass doesn't hold it, a newer one does.
        let rescue = ci::Start {
            after: Some(80),
            ..start.clone()
        };
        let pipelines = r#"[{"id":80,"sha":"a","status":"canceled"},{"id":70,"sha":"a","status":"success"}]"#;
        let glab = fake_glab(&scratch, &[(MR, r#"{"sha":"a"}"#), (PIPELINES, pipelines)]);
        assert_eq!(run_with(&scratch, &rescue, &glab), Ok(Ran::Created(None)));
        let pipelines = r#"[{"id":81,"sha":"a","status":"pending"},{"id":80,"sha":"a","status":"canceled"}]"#;
        let glab = fake_glab(&scratch, &[(MR, r#"{"sha":"a"}"#), (PIPELINES, pipelines)]);
        assert_eq!(run_with(&scratch, &rescue, &glab), Ok(Ran::Exists(81)));
        // Pushed meanwhile: the old commit gets nothing.
        let glab = fake_glab(&scratch, &[(MR, r#"{"sha":"b"}"#)]);
        assert_eq!(run_with(&scratch, &start, &glab), Ok(Ran::Superseded));
        let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
        assert_eq!(
            log.matches("POST projects/:fullpath/merge_requests").count(),
            2,
            "{log}"
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
