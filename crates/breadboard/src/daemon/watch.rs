//! The CI job watcher of a repo (#379), run by the daemon while any overview
//! of that repo is open, next to its CI queue: every EVERY it reads the
//! project's newest pipelines, keeps each finished job in core's history
//! (`.claude/ci/jobs.json`), samples what the local runner's jobs use
//! (`docker stats`) and writes what holds pipelines up to
//! `.claude/ci/watch.json` for the overview. A flagged job becomes an action
//! in every orchestration's record, once: a `plan` to file an issue for a
//! slow one, a `decide` (retry, cancel, wait) for a hung one, which goes
//! again once the job no longer runs or writes its log again and nobody
//! answered it. A job far past its median is hung only once its log stops
//! growing (#398): on a busy runner it is slow, and a retry throws its work
//! away; one quiet while it packs or fetches its cache is caching, not hung
//! (#424), and goes unasked. It reads GitLab only: what the user answers, the orchestrator does.

use breadboard_core::actions::{NewAction, actions_of, add_action, done_action};
use breadboard_core::ci_watch::{
    self, Flag, History, Hung, Report, Run, Running, flags_of, overdue_of, run_of, running_of, usage_of,
};
use breadboard_core::config::CiWatch;
use breadboard_core::time::now_ms;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// How often it reads; `BREADBOARD_CI_WATCH_EVERY_MS` another (the gauntlet's ticks).
pub const EVERY: Duration = Duration::from_secs(60);
/// The pipelines read each tick, newest first; an empty history reads BACKFILL pages of them.
const PER_PAGE: u32 = 100;
const BACKFILL: u32 = 3;

fn every() -> Duration {
    std::env::var("BREADBOARD_CI_WATCH_EVERY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(EVERY)
}

/// The watcher of `main`, as long as the daemon runs.
pub async fn watch(main: PathBuf) {
    let mut tick = tokio::time::interval(every());
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if !main.is_dir() {
            return;
        }
        let main = main.clone();
        let _ = tokio::task::spawn_blocking(move || look(&main, "glab", "docker")).await;
    }
}

/// One look: GitLab read, docker sampled, the report and the actions written.
fn look(main: &Path, glab: &str, docker: &str) {
    let watch = breadboard_core::config::of(main).config.ci.watch.clone();
    if !watch.is_on || !breadboard_core::ci::is_served(main) {
        let _ = std::fs::remove_file(ci_watch::watch_file(main));
        return;
    }
    // Another daemon watches this checkout.
    if !breadboard_core::ci::holds(main, &ci_watch::watcher_file(main)) {
        return;
    }
    let file = ci_watch::jobs_file(main);
    let mut history: History = breadboard_core::ci::read_or_default(&file);
    let Some(report) = read(main, glab, docker, &mut history, &watch) else {
        return;
    };
    post(main, &mut history, &report);
    let _ = breadboard_core::ci::write_whole(&file, &history);
    let _ = breadboard_core::ci::write_whole(&ci_watch::watch_file(main), &report);
}

/// GitLab's newest pipelines into `history`, docker sampled, what holds
/// pipelines up; None when GitLab didn't answer.
fn read(main: &Path, glab: &str, docker: &str, history: &mut History, watch: &CiWatch) -> Option<Report> {
    let now = now_ms();
    let pages = if history.runs.is_empty() { BACKFILL } else { 1 };
    let mut pipelines: Vec<Value> = Vec::new();
    for page in 1..=pages {
        let Ok(Value::Array(list)) = super::glab::glab_get(
            main,
            glab,
            &format!("projects/:fullpath/pipelines?per_page={PER_PAGE}&page={page}"),
        ) else {
            break;
        };
        let is_last = list.len() < PER_PAGE as usize;
        pipelines.extend(list);
        if is_last {
            break;
        }
    }
    if pipelines.is_empty() {
        return None;
    }
    let mut running: Vec<Running> = Vec::new();
    for pipeline in &pipelines {
        let Some(id) = pipeline.get("id").and_then(Value::as_i64) else {
            continue;
        };
        let status = pipeline.get("status").and_then(Value::as_str).unwrap_or_default();
        let updated = pipeline
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let created = pipeline
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(breadboard_core::time::parse_iso)
            .unwrap_or(now);
        if now - created > watch.history_days * 24 * 3_600_000 {
            continue;
        }
        let is_live = matches!(status, "running" | "pending");
        let is_ended = matches!(status, "success" | "failed" | "canceled");
        if !is_live && !(is_ended && history.seen.get(&id) != Some(&updated)) {
            continue;
        }
        let Ok(Value::Array(jobs)) = super::glab::glab_get(
            main,
            glab,
            &format!("projects/:fullpath/pipelines/{id}/jobs?per_page=100"),
        ) else {
            continue;
        };
        // A job's pipeline as the list has it: GitLab's job carries its id and ref too.
        let jobs: Vec<Value> = jobs
            .into_iter()
            .map(|mut job| {
                if job.get("pipeline").is_none_or(Value::is_null) {
                    job["pipeline"] = serde_json::json!({ "id": id, "ref": pipeline.get("ref") });
                }
                job
            })
            .collect();
        history.add(jobs.iter().filter_map(run_of).collect::<Vec<Run>>());
        running.extend(jobs.iter().filter_map(|job| running_of(job, now)));
        if is_ended {
            history.seen.insert(id, updated);
        }
    }
    // What the local runner's jobs use now, kept until each one ends.
    let usage = sample(docker);
    for (job, (cores, mem)) in &usage {
        if running.iter().any(|run| run.id == *job) {
            history.samples.entry(*job).or_default().add(*cores, *mem);
        }
    }
    history
        .samples
        .retain(|job, _| running.iter().any(|run| run.id == *job));
    // A job past its median is hung once its log stopped growing; on a busy local runner, slow.
    let busy = busy_load();
    let mut hung = overdue_of(&running, &history.runs, watch);
    hung.retain_mut(|job| {
        // GitLab didn't answer: the log as last read, nothing without one.
        let trace = super::glab::glab_text(main, glab, &format!("projects/:fullpath/jobs/{}/trace", job.id)).ok();
        let Some(len) = trace
            .as_ref()
            .map(String::len)
            .or_else(|| history.traces.get(&job.id).map(|trace| trace.len))
        else {
            return false;
        };
        job.quiet = history.progress(job.id, len, now);
        job.load = busy.filter(|_| usage.contains_key(&job.id));
        job.step = trace.as_deref().and_then(ci_watch::step_of);
        // A quiet one packing its cache stays in the report, for its row's mark: no question (#424).
        job.is_quiet(watch)
    });
    history.traces.retain(|job, _| running.iter().any(|run| run.id == *job));
    history.prune(watch.history_days, now);
    Some(Report {
        at: now,
        flags: flags_of(&history.runs, watch, now),
        hung,
    })
}

/// This machine's load (`/proc/loadavg`, the last minute) while it is at
/// least half its cores: a job of the local runner is slower then.
fn busy_load() -> Option<f64> {
    let load: f64 = std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let cores = std::thread::available_parallelism().map_or(1, usize::from) as f64;
    (load >= cores / 2.0).then_some(load)
}

/// What the local runner's job containers use now: job id → (cores, MB). Nothing without docker.
fn sample(docker: &str) -> std::collections::BTreeMap<i64, (f64, f64)> {
    let run = |args: &[&str]| {
        Command::new(docker)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let Some(ps) = run(&[
        "ps",
        "--filter",
        "label=com.gitlab.gitlab-runner.job.id",
        "--format",
        "{{.ID}}\t{{.Label \"com.gitlab.gitlab-runner.job.id\"}}",
    ]) else {
        return Default::default();
    };
    let containers: Vec<&str> = ps.lines().filter_map(|line| line.split('\t').next()).collect();
    if containers.is_empty() {
        return Default::default();
    }
    let mut args = vec![
        "stats",
        "--no-stream",
        "--format",
        "{{.ID}}\t{{.CPUPerc}}\t{{.MemUsage}}",
    ];
    args.extend(containers);
    run(&args).map(|stats| usage_of(&ps, &stats)).unwrap_or_default()
}

/// The question of a slow job: an issue to speed it up, with its numbers.
pub fn plan_text(flag: &Flag) -> String {
    format!("file an issue to speed up {}? {}", flag.job, flag.line())
}

/// The question of a hung job, its id and page to act on.
pub fn hung_text(hung: &Hung) -> String {
    let url = hung.url.as_deref().map(|url| format!(", {url}")).unwrap_or_default();
    format!("{}: {} (job {}{url})", hung.line(), hung.verdict(), hung.id)
}

/// The report's flags as actions, each once into every orchestration's
/// record; a hung job's question goes once the job ended unanswered.
fn post(main: &Path, history: &mut History, report: &Report) {
    let names = breadboard_core::record::recorded(main);
    let mut wanted: Vec<(String, NewAction)> = report
        .flags
        .iter()
        .map(|flag| {
            (
                flag.key(),
                NewAction {
                    kind: "plan".into(),
                    text: plan_text(flag),
                    ..NewAction::default()
                },
            )
        })
        .collect();
    wanted.extend(report.hung.iter().filter(|hung| !hung.is_caching()).map(|hung| {
        (
            hung.key(),
            NewAction {
                kind: "decide".into(),
                text: hung_text(hung),
                options: vec!["retry".into(), "cancel".into(), "wait".into()],
                descriptions: vec![
                    format!("glab ci retry {}", hung.id),
                    format!("glab ci cancel job {}", hung.id),
                    "it may still finish".into(),
                ],
                // Only a job whose log stopped is asked about: a retry loses nothing.
                recommend: Some("retry".into()),
                ..NewAction::default()
            },
        )
    }));
    for (key, new) in &wanted {
        if history.posted.contains_key(key) {
            continue;
        }
        let mut posted = Vec::new();
        for name in &names {
            if let Ok(action) = add_action(&breadboard_core::record::record_file(main, name), new.clone()) {
                posted.push((name.clone(), action.id));
            }
        }
        history.posted.insert(key.clone(), posted);
    }
    // Gone from the report: a slow job's plan stays for the user, a hung
    // job's unanswered question goes (it ended, or writes its log again).
    let gone: Vec<String> = history
        .posted
        .keys()
        .filter(|key| !wanted.iter().any(|(wanted, _)| wanted == *key))
        .cloned()
        .collect();
    for key in gone {
        let posted = history.posted.remove(&key).unwrap_or_default();
        if !key.starts_with("hung:") {
            continue;
        }
        for (name, id) in posted {
            let file = breadboard_core::record::record_file(main, &name);
            let is_open = actions_of(&file)
                .iter()
                .any(|action| action.id == id && action.answer.as_deref().is_none_or(str::is_empty));
            if is_open {
                let _ = done_action(&file, id);
            }
        }
    }
}

/// `breadboard ci:watch`: what the watcher sees now, from the kept history
/// and GitLab, written nowhere and posted nowhere; None when GitLab didn't answer.
pub fn once(main: &Path) -> Option<Report> {
    let watch = breadboard_core::config::of(main).config.ci.watch.clone();
    let mut history: History = breadboard_core::ci::read_or_default(&ci_watch::jobs_file(main));
    read(main, "glab", "docker", &mut history, &watch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::ci_watch::Why;

    #[test]
    fn says_what_to_do_with_a_flag() {
        let flag = Flag {
            job: "check:g".into(),
            median: 638.0,
            why: vec![Why::Critical { of: 9, among: 10 }],
        };
        assert_eq!(
            plan_text(&flag),
            "file an issue to speed up check:g? check:g holds pipelines up · median 10:38, critical path in 9/10"
        );
        let hung = Hung {
            id: 5,
            job: "check:g".into(),
            pipeline: 1,
            mr: Some(353),
            elapsed: 1500.0,
            median: 638.0,
            url: Some("https://g/jobs/5".into()),
            quiet: 70.0,
            load: None,
            step: None,
        };
        assert_eq!(
            hung_text(&hung),
            "check:g of !353 runs 25:00, 2.4× its median 10:38, no new log for 1:10: hung? (job 5, https://g/jobs/5)"
        );
        let busy = Hung {
            load: Some(23.6),
            ..hung
        };
        assert_eq!(
            hung_text(&busy),
            "check:g of !353 runs 25:00, 2.4× its median 10:38, no new log for 1:10: slow: runner at load 24 (job 5, https://g/jobs/5)"
        );
        let scripting = Hung {
            step: Some("step_script".into()),
            ..busy
        };
        assert_eq!(
            hung_text(&scripting),
            "check:g of !353 runs 25:00, 2.4× its median 10:38, no new log for 1:10, in: Executing step script: slow: runner at load 24 (job 5, https://g/jobs/5)"
        );
    }
}
