//! The queue's loop: every EVERY a look at the open MRs and the runner's
//! slots, one decision (core's `ci::decide`), its step done by the module
//! of that step, what was asked for and where each MR stands written.

use super::preempt::{self, Preempter};
use super::{clicks, every, fix, is_queued, start};
use breadboard_core::ci::{self, Asked, Decision, Queue, Ranks, RunNow, Slots};
use breadboard_core::gitlab::client::Gitlab;
use breadboard_core::gitlab::read::open_mrs_of;
use breadboard_core::time::now_ms;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// The queue in the request log (#462).
const TASK: &str = "ci queue";

/// The queue of `main`, as long as the daemon runs.
pub async fn schedule(main: PathBuf) {
    let gitlab = Gitlab::new(&main).for_task(TASK);
    let mut preempter = Preempter::default();
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
        let project = match super::open_mrs_project(&gitlab, &main).await {
            Ok(project) => project,
            // GitLab paused after a 429 (#462): the queue still runs, its heartbeat stays fresh,
            // or every push would drop `ci.skip` and start a pipeline of its own meanwhile.
            Err(error) if error.status == Some(429) => {
                beat(&main);
                continue;
            }
            Err(_) => continue,
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
            // `[merge] needs_pipeline = false` (#443): MRs into the default branch merge on wt:check.
            let loaded = breadboard_core::config::of_main(&main_now);
            ranks.no_pipeline_into = (!loaded.config.merge.needs_pipeline).then(|| loaded.branch.clone());
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
        } else if let Some(preempt) = preempt.filter(|_| preempter.is_due()) {
            preempter.mark();
            let main = main.clone();
            let done = tokio::task::spawn_blocking(move || {
                let said = preempt::step(&main, preempt, &mut asked, now);
                (asked, said)
            })
            .await;
            let Ok((again, said)) = done else {
                continue;
            };
            (asked, last) = (again, Some(said));
        }
        if let Some(start) = start.filter(|_| !is_dry) {
            let main = main.clone();
            let done = tokio::task::spawn_blocking(move || {
                let said = start::step(&main, &start, &mut asked, now);
                (asked, said)
            })
            .await;
            let Ok((again, said)) = done else {
                continue;
            };
            (asked, last) = (again, Some(said));
        }
        if !is_dry {
            let main = main.clone();
            let mrs = mrs.clone();
            let done = tokio::task::spawn_blocking(move || {
                let max = breadboard_core::config::of(&main).config.ci.max_fix_attempts;
                let said = fix::step(&main, "glab", &mrs, &members, &mut asked, &mut looked, max, now);
                (asked, looked, said)
            })
            .await;
            let Ok((again, again_looked, said)) = done else {
                looked = HashMap::new();
                continue;
            };
            (asked, looked) = (again, again_looked);
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
        clicks::forget(&main, &run_now, &started, &open, now);
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

/// The queue's heartbeat (`at` of queue.json) renewed, the rest as it was.
fn beat(main: &std::path::Path) {
    let file = ci::queue_file(main);
    let mut queue = breadboard_core::json::read_object(&file);
    if queue.is_empty() {
        return;
    }
    queue.insert("at".into(), now_ms().into());
    let _ = breadboard_core::json::write_pretty(&file, &serde_json::Value::Object(queue));
}
