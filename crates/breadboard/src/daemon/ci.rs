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

use breadboard_core::ci::{self, Asked, Decision, MODE_KEY, OpenMr, Preempt, Preemption, Queue, Ranks, RunNow, Slots};
use breadboard_core::ci_fix::{self, Act, Verdict};
use breadboard_core::gitlab::client::Gitlab;
use breadboard_core::gitlab::read::{CI_QUERY, open_mrs_of};
use breadboard_core::time::now_ms;
use serde_json::json;
use std::collections::{HashMap, HashSet};
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
    // The failed pipelines looked at, each with its `updatedAt` then: once per failure (#371).
    let mut looked: HashMap<i64, String> = HashMap::new();
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
        let preempts = asked.preempts.clone().into_iter().collect();
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
            ranks.preempts = preempts;
            ranks.now = now;
            let members: HashSet<i64> = mrs.iter().filter(|mr| ranks.is_member(mr)).map(|mr| mr.iid).collect();
            (
                ci::decide(&mrs, &ranks, slots, &started),
                slots,
                run_now,
                mrs,
                now,
                members,
            )
        });
        let Ok((decision, slots, run_now, mrs, now, members)) = step.await else {
            continue;
        };
        let Decision { places, start, preempt } = decision;
        if is_dry {
            last = start
                .as_ref()
                .map(|start| format!("would start !{}", start.iid))
                .or(preempt
                    .as_ref()
                    .map(|preempt| format!("would hold !{} for !{}", preempt.iid, preempt.by)))
                .or(last);
        } else if let Some(preempt) = preempt.filter(|_| preempted_at.is_none_or(|at| at.elapsed() >= PREEMPT_EVERY)) {
            preempted_at = Some(std::time::Instant::now());
            let main = main.clone();
            let said = tokio::task::spawn_blocking(move || {
                let said = cancel_young(&main, "glab", &preempt, now_ms());
                (preempt, said)
            })
            .await;
            if let Ok((Preempt { iid, pipeline, sha, by }, said)) = said {
                last = Some(match said {
                    Ok(true) => {
                        asked.preempted.insert(iid, sha);
                        asked.preempts.insert(iid, Preemption { by, at: now });
                        asked.add_cancelled(pipeline);
                        format!("!{iid} preempted by !{by}")
                    }
                    Ok(false) => format!("!{iid} kept, it finishes"),
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
        if !is_dry {
            let main = main.clone();
            let mrs = mrs.clone();
            let tended = tokio::task::spawn_blocking(move || {
                let max = breadboard_core::config::of(&main).config.ci.max_fix_attempts;
                let said = tend_failed(&main, "glab", &mrs, &members, &mut asked, &mut looked, max, now);
                (asked, looked, said)
            })
            .await;
            let Ok((tended_asked, tended_looked, said)) = tended else {
                looked = HashMap::new();
                continue;
            };
            (asked, looked) = (tended_asked, tended_looked);
            last = said.or(last);
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
        asked.fixes.retain(|iid, _| open.contains_key(iid));
        asked
            .preempts
            .retain(|iid, preemption| open.contains_key(iid) && now - preemption.at < ci::PREEMPT_AGAIN_MS);
        if !is_dry {
            let _ = ci::write_whole(&ci::asked_file(&main), &asked);
        }
        let fixes = asked
            .fixes
            .iter()
            .filter_map(|(iid, fixes)| Some((*iid, fixes.now.clone()?)))
            .collect();
        let started: HashMap<i64, (String, i64)> = asked.started.into_iter().collect();
        forget_clicks(&main, &run_now, &started, &open, now);
        let queue = Queue {
            at: now,
            slots,
            places,
            last: last.clone(),
            is_dry,
            fixes,
        };
        let _ = ci::write_whole(&ci::queue_file(&main), &queue);
    }
}

/// The failed pipelines of the members' newest commits (#371), each looked
/// at once per failure: its jobs' tries read, the failed ones retried (the
/// runner's failure, a first try's), a refused one asked for again; a job
/// that failed twice is left to its agent, whom the overview tells. Where
/// each stands goes into `asked.fixes`; a pipeline that passed ends them.
/// What it did, for the band.
#[allow(clippy::too_many_arguments)]
fn tend_failed(
    main: &Path,
    glab: &str,
    mrs: &[OpenMr],
    members: &HashSet<i64>,
    asked: &mut Asked,
    looked: &mut HashMap<i64, String>,
    max: u32,
    now: i64,
) -> Option<String> {
    let mut said = None;
    for mr in mrs.iter().filter(|mr| members.contains(&mr.iid)) {
        let (Some(pipeline), Some(sha)) = (&mr.pipeline, &mr.sha) else {
            continue;
        };
        if pipeline.sha.as_ref() != Some(sha) {
            continue;
        }
        match pipeline.status.as_str() {
            "success" => {
                asked.fixes.remove(&mr.iid);
                continue;
            }
            "failed" => {}
            _ => {
                looked.remove(&pipeline.id);
                continue;
            }
        }
        // A retried job may fail again between two looks: a new failure moves its `updatedAt`.
        let updated = pipeline.updated_at.clone().unwrap_or_default();
        let is_looked = looked.get(&pipeline.id) == Some(&updated);
        // A newer pipeline refused over a passed one: the queue's rescue asks once (#369).
        if mr.blocked_by.is_some_and(|blocking| !asked.ours.contains(&blocking)) || is_looked {
            continue;
        }
        let verdict = if mr.is_refused {
            Verdict::Refused
        } else {
            match glab_get(
                main,
                glab,
                &format!(
                    "projects/:fullpath/pipelines/{}/jobs?include_retried=true&per_page=100",
                    pipeline.id
                ),
            ) {
                Ok(listed) => ci_fix::verdict_of(&ci_fix::tries_of(&listed)),
                Err(_) => continue,
            }
        };
        let is_live = verdict == Verdict::Live;
        let fixes = asked.fixes.entry(mr.iid).or_default();
        let act = ci_fix::step(fixes, pipeline.id, sha, verdict, max, now);
        // A refusal waits its time and is looked at again; a retry under way when it ends.
        let is_waiting = fixes
            .now
            .as_ref()
            .is_some_and(|fix| fix.stage == ci_fix::Stage::Refused);
        if !is_live && !is_waiting {
            looked.insert(pipeline.id, updated);
        }
        match act {
            Some(Act::Retry(jobs)) => {
                let retried: Vec<String> = jobs
                    .iter()
                    .filter(|id| glab_post(main, glab, &format!("projects/:fullpath/jobs/{id}/retry")).is_ok())
                    .map(|id| id.to_string())
                    .collect();
                said = Some(format!("!{} retried job {}", mr.iid, retried.join(", ")));
            }
            Some(Act::AskAgain) => {
                let start = ci::Start {
                    iid: mr.iid,
                    sha: sha.clone(),
                    cancel: None,
                    is_again: true,
                    after: None,
                };
                said = Some(match run_with(main, &start, glab) {
                    Ok(Ran::Created(id)) => {
                        if let Some(id) = id {
                            asked.add_ours(id);
                        }
                        asked.started.insert(mr.iid, (sha.clone(), now));
                        format!("!{} refused, asked again", mr.iid)
                    }
                    Ok(_) => format!("!{} has a newer pipeline", mr.iid),
                    Err(error) => format!("!{} not asked again: {}", mr.iid, first_line(&error)),
                });
            }
            None => {}
        }
    }
    said
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

/// The pipeline to preempt read back first (#419): cancelled only while it
/// is still young (`ci::PREEMPT_GRACE_MS`) and none of its jobs went past
/// its first stage; true when it was cancelled.
fn cancel_young(main: &Path, glab: &str, preempt: &Preempt, now: i64) -> Result<bool, String> {
    let id = preempt.pipeline;
    let pipeline = glab_get(main, glab, &format!("projects/:fullpath/pipelines/{id}"))?;
    let status = pipeline
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let started = pipeline
        .get("started_at")
        .and_then(serde_json::Value::as_str)
        .and_then(breadboard_core::time::parse_iso);
    let is_young = match started {
        Some(started) => now - started < ci::PREEMPT_GRACE_MS,
        None => status != "running",
    };
    if !ci::is_live(status) || !is_young {
        return Ok(false);
    }
    let jobs = glab_get(
        main,
        glab,
        &format!("projects/:fullpath/pipelines/{id}/jobs?per_page=100"),
    )?;
    let jobs: Vec<(i64, String, String)> = jobs
        .as_array()
        .into_iter()
        .flatten()
        .map(|job| {
            let text = |key: &str| {
                job.get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            (
                job.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0),
                text("stage"),
                text("status"),
            )
        })
        .collect();
    if ci::is_past_first_stage(&jobs) {
        return Ok(false);
    }
    glab_post(main, glab, &format!("projects/:fullpath/pipelines/{id}/cancel"))?;
    Ok(true)
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

/// `glab api <endpoint>` of a text answer (a job's log), as the user, from the main checkout.
pub(super) fn glab_text(main: &Path, glab: &str, endpoint: &str) -> Result<String, String> {
    glab_out(main, glab, &["api", endpoint]).map(|out| String::from_utf8_lossy(&out).into_owned())
}

pub(super) fn glab_api(main: &Path, glab: &str, args: &[&str]) -> Result<serde_json::Value, String> {
    glab_out(main, glab, args).map(|out| serde_json::from_slice(&out).unwrap_or(serde_json::Value::Null))
}

fn glab_out(main: &Path, glab: &str, args: &[&str]) -> Result<Vec<u8>, String> {
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
        return Ok(output.stdout);
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
    fn preempts_only_a_pipeline_still_young_and_in_its_first_stage() {
        // #419: GitLab read back right before the cancel; the queue's read may be old.
        let scratch = std::env::temp_dir().join(format!("ci-preempt-test-{}", std::process::id()));
        let now = 1_790_000_000_000;
        let preempt = Preempt {
            iid: 381,
            pipeline: 42,
            sha: "b".into(),
            by: 371,
        };
        const PIPELINE: &str = "api projects/:fullpath/pipelines/42";
        const JOBS: &str = "api projects/:fullpath/pipelines/42/jobs?per_page=100";
        const CANCEL: &str = "api --method POST projects/:fullpath/pipelines/42/cancel";
        let started = |ago: i64| {
            format!(
                r#"{{"status":"running","started_at":"{}"}}"#,
                breadboard_core::time::to_iso(now - ago)
            )
        };
        let check = r#"[{"id":1,"stage":"check","status":"running"},{"id":2,"stage":"build","status":"created"}]"#;
        let build = r#"[{"id":1,"stage":"check","status":"success"},{"id":2,"stage":"build","status":"running"}]"#;
        for (pipeline, jobs, is_cancelled) in [
            (started(30_000), check, true),
            (started(30_000), build, false),
            (started(ci::PREEMPT_GRACE_MS), check, false),
            (r#"{"status":"success"}"#.to_string(), check, false),
        ] {
            let _ = std::fs::remove_file(scratch.join("glab.log"));
            let glab = fake_glab(&scratch, &[(PIPELINE, &pipeline), (JOBS, jobs)]);
            assert_eq!(
                cancel_young(&scratch, &glab, &preempt, now),
                Ok(is_cancelled),
                "{pipeline} {jobs}"
            );
            let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
            assert_eq!(log.contains(CANCEL), is_cancelled, "{log}");
        }
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn retries_a_first_failure_once_and_leaves_a_second_to_the_agent() {
        // #371: a flake passes its retry; a job that failed twice is the MR's doing.
        let scratch = std::env::temp_dir().join(format!("ci-fix-test-{}", std::process::id()));
        const JOBS: &str = "api projects/:fullpath/pipelines/90/jobs?include_retried=true&per_page=100";
        const RETRY: &str = "api --method POST projects/:fullpath/jobs/5/retry";
        let mr = OpenMr {
            iid: 339,
            sha: Some("a".into()),
            pipeline: Some(breadboard_core::gitlab::Pipeline {
                id: 90,
                status: "failed".into(),
                sha: Some("a".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let members = HashSet::from([339]);
        let (mut asked, mut looked) = (Asked::default(), HashMap::new());
        let once = r#"[{"id":5,"name":"check:vita","status":"failed","failure_reason":"script_failure"}]"#;
        let glab = fake_glab(&scratch, &[(JOBS, once)]);
        let said = tend_failed(
            &scratch,
            &glab,
            std::slice::from_ref(&mr),
            &members,
            &mut asked,
            &mut looked,
            3,
            0,
        );
        assert_eq!(said.as_deref(), Some("!339 retried job 5"));
        // Looked at once: the next tick asks nothing while it stays failed.
        tend_failed(
            &scratch,
            &glab,
            std::slice::from_ref(&mr),
            &members,
            &mut asked,
            &mut looked,
            3,
            0,
        );
        let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
        assert_eq!(log, format!("{JOBS}\n{RETRY}\n"));
        // The retry runs, then fails as well: its agent's, the first of three runs.
        let running = OpenMr {
            pipeline: mr.pipeline.clone().map(|pipeline| breadboard_core::gitlab::Pipeline {
                status: "running".into(),
                ..pipeline
            }),
            ..mr.clone()
        };
        tend_failed(&scratch, &glab, &[running], &members, &mut asked, &mut looked, 3, 0);
        let twice = r#"[{"id":5,"name":"check:vita","status":"failed","failure_reason":"script_failure"},{"id":6,"name":"check:vita","status":"failed","failure_reason":"script_failure"}]"#;
        let glab = fake_glab(&scratch, &[(JOBS, twice)]);
        tend_failed(
            &scratch,
            &glab,
            std::slice::from_ref(&mr),
            &members,
            &mut asked,
            &mut looked,
            3,
            0,
        );
        let fix = asked.fixes[&339].now.clone().unwrap();
        assert_eq!(
            (fix.stage, fix.mark_text()),
            (ci_fix::Stage::Fix, "CI fix 1/3".to_string())
        );
        let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
        assert_eq!(log.matches("/retry").count(), 1, "{log}");
        // Failed again between two looks, never seen running: its new `updatedAt` tells.
        let (mut quick, mut quick_looked) = (Asked::default(), HashMap::new());
        let at = |updated: &str| OpenMr {
            pipeline: mr.pipeline.clone().map(|pipeline| breadboard_core::gitlab::Pipeline {
                updated_at: Some(updated.into()),
                ..pipeline
            }),
            ..mr.clone()
        };
        let glab = fake_glab(&scratch, &[(JOBS, once)]);
        tend_failed(
            &scratch,
            &glab,
            &[at("1")],
            &members,
            &mut quick,
            &mut quick_looked,
            3,
            0,
        );
        let glab = fake_glab(&scratch, &[(JOBS, twice)]);
        tend_failed(
            &scratch,
            &glab,
            &[at("2")],
            &members,
            &mut quick,
            &mut quick_looked,
            3,
            0,
        );
        assert_eq!(quick.fixes[&339].now.as_ref().unwrap().stage, ci_fix::Stage::Fix);
        // A pipeline that passed ends its fixes; an MR of no orchestration is never looked at.
        let passed = OpenMr {
            pipeline: mr.pipeline.clone().map(|pipeline| breadboard_core::gitlab::Pipeline {
                status: "success".into(),
                ..pipeline
            }),
            ..mr.clone()
        };
        tend_failed(&scratch, &glab, &[passed], &members, &mut asked, &mut looked, 3, 0);
        assert!(asked.fixes.is_empty());
        let (mut asked, mut looked) = (Asked::default(), HashMap::new());
        tend_failed(&scratch, &glab, &[mr], &HashSet::new(), &mut asked, &mut looked, 3, 0);
        assert!(asked.fixes.is_empty());
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
