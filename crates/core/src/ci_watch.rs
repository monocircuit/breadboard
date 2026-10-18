//! The CI job watcher (#379): what `breadboard serve` keeps of the project's
//! finished jobs (`.claude/ci/jobs.json`, a rolling history per job name)
//! and what it flags in them (`.claude/ci/watch.json`): a job that is the
//! critical path of most of its pipelines, one whose median grows, one
//! running far past its median now whose log stopped growing (hung, #398),
//! one that mostly waits (idle, from the local runner's CPU samples). Pure: the daemon reads GitLab and docker,
//! the overview reads the report.

use crate::ci::dir_of;
use crate::config::CiWatch;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// How long a report counts: the daemon writes one every minute.
pub const REPORT_STALE_MS: i64 = 5 * 60_000;
/// How many pipelines the history remembers as read.
const SEEN_KEPT: usize = 2000;

pub fn jobs_file(main: &Path) -> PathBuf {
    dir_of(main).join("jobs.json")
}

pub fn watch_file(main: &Path) -> PathBuf {
    dir_of(main).join("watch.json")
}

/// Whose watcher reads `main`'s CI: one daemon per checkout.
pub fn watcher_file(main: &Path) -> PathBuf {
    dir_of(main).join("watcher.pid")
}

/// A finished job of GitLab's REST answer (`pipelines/:id/jobs`), None while it runs.
pub fn run_of(job: &serde_json::Value) -> Option<Run> {
    let finished = crate::time::parse_iso(job.get("finished_at")?.as_str()?)?;
    Some(Run {
        id: job.get("id")?.as_i64()?,
        name: job.get("name")?.as_str()?.to_string(),
        pipeline: job
            .pointer("/pipeline/id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        status: job.get("status")?.as_str()?.to_string(),
        duration: job.get("duration").and_then(serde_json::Value::as_f64).unwrap_or(0.0),
        queued: job
            .get("queued_duration")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        runner: job
            .pointer("/runner/description")
            .and_then(serde_json::Value::as_str)
            .map(String::from),
        finished,
        cpu: None,
        mem: None,
    })
}

/// A job of GitLab's REST answer that runs now, its seconds counted to `now`.
pub fn running_of(job: &serde_json::Value, now: i64) -> Option<Running> {
    if job.get("status")?.as_str()? != "running" {
        return None;
    }
    let started = crate::time::parse_iso(job.get("started_at")?.as_str()?)?;
    Some(Running {
        id: job.get("id")?.as_i64()?,
        name: job.get("name")?.as_str()?.to_string(),
        pipeline: job
            .pointer("/pipeline/id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        mr: job
            .pointer("/pipeline/ref")
            .and_then(serde_json::Value::as_str)
            .and_then(mr_of_ref),
        elapsed: (now - started).max(0) as f64 / 1000.0,
        url: job.get("web_url").and_then(serde_json::Value::as_str).map(String::from),
    })
}

/// docker's `312.5MiB / 15.5GiB` (its first half) in MB.
fn megabytes(text: &str) -> Option<f64> {
    let used = text.split('/').next()?.trim();
    let split = used.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, unit) = used.split_at(split);
    let number: f64 = number.trim().parse().ok()?;
    let factor = match unit {
        "B" => 1.0 / 1_048_576.0,
        "KiB" | "kB" | "KB" => 1.0 / 1024.0,
        "MiB" | "MB" => 1.0,
        "GiB" | "GB" => 1024.0,
        _ => return None,
    };
    Some(number * factor)
}

/// The local runner's jobs and what they use now: `ps` lines `<container>\t<job id>`,
/// `stats` lines `<container>\t<cpu %>\t<memory>`; job id → (cores, MB),
/// summed over its containers (its services too).
pub fn usage_of(ps: &str, stats: &str) -> BTreeMap<i64, (f64, f64)> {
    let jobs: BTreeMap<&str, i64> = ps
        .lines()
        .filter_map(|line| {
            let (container, job) = line.trim().split_once('\t')?;
            Some((container, job.trim().parse().ok()?))
        })
        .collect();
    let mut usage: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
    for line in stats.lines() {
        let mut parts = line.trim().split('\t');
        let (Some(container), Some(cpu), Some(mem)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let Some(job) = jobs.get(container) else { continue };
        let cores = cpu.trim().trim_end_matches('%').parse::<f64>().unwrap_or(0.0) / 100.0;
        let entry = usage.entry(*job).or_default();
        entry.0 += cores;
        entry.1 += megabytes(mem).unwrap_or(0.0);
    }
    usage
}

/// A finished job.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub id: i64,
    pub name: String,
    pub pipeline: i64,
    pub status: String,
    /// Seconds it ran.
    pub duration: f64,
    /// Seconds it waited for a runner.
    pub queued: f64,
    pub runner: Option<String>,
    /// When it ended (ms).
    pub finished: i64,
    /// Its mean CPU in cores and peak memory in MB, where the local runner ran it.
    pub cpu: Option<f64>,
    pub mem: Option<f64>,
}

/// The CPU samples of a job while it runs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Samples {
    pub count: u32,
    pub cores: f64,
    pub mem: f64,
}

impl Samples {
    pub fn add(&mut self, cores: f64, mem: f64) {
        self.count += 1;
        self.cores += cores;
        self.mem = self.mem.max(mem);
    }
}

/// A running job's log as the watcher last read it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Trace {
    /// Its length in bytes.
    pub len: usize,
    /// When it last grew, or was first read (ms).
    pub grew: i64,
}

/// What the watcher keeps between its ticks and restarts.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub runs: Vec<Run>,
    /// Pipelines read once finished: id → its `updated_at` then (a retry reads it again).
    pub seen: BTreeMap<i64, String>,
    /// Running jobs' CPU samples by job id.
    pub samples: BTreeMap<i64, Samples>,
    /// The logs of running jobs past their median, by job id.
    pub traces: BTreeMap<i64, Trace>,
    /// The flags posted as actions (`slow:<job>`, `hung:<id>`): each
    /// orchestration's record and the action's id in it.
    pub posted: BTreeMap<String, Vec<(String, i64)>>,
}

impl History {
    /// The finished jobs of a pipeline added, each once; a job's samples become its CPU.
    pub fn add(&mut self, runs: Vec<Run>) {
        for mut run in runs {
            if let Some(samples) = self.samples.remove(&run.id).filter(|samples| samples.count > 0) {
                run.cpu = Some(samples.cores / f64::from(samples.count));
                run.mem = Some(samples.mem);
            }
            match self.runs.iter_mut().find(|known| known.id == run.id) {
                Some(known) => {
                    if run.cpu.is_none() {
                        (run.cpu, run.mem) = (known.cpu, known.mem);
                    }
                    *known = run;
                }
                None => self.runs.push(run),
            }
        }
    }

    /// A running job's log is `len` long now: the seconds since it last
    /// grew. The first read counts as growth, there is nothing to compare with.
    pub fn progress(&mut self, id: i64, len: usize, now: i64) -> f64 {
        let trace = self.traces.entry(id).or_insert(Trace { len, grew: now });
        if trace.len != len {
            *trace = Trace { len, grew: now };
        }
        (now - trace.grew).max(0) as f64 / 1000.0
    }

    /// What is older than `days` goes, and pipelines past SEEN_KEPT.
    pub fn prune(&mut self, days: i64, now: i64) {
        self.runs.retain(|run| now - run.finished <= days * DAY_MS);
        while self.seen.len() > SEEN_KEPT {
            self.seen.pop_first();
        }
    }
}

/// A job running now.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Running {
    pub id: i64,
    pub name: String,
    pub pipeline: i64,
    /// The MR of its pipeline (`refs/merge-requests/<iid>/head`).
    pub mr: Option<i64>,
    /// Seconds it runs.
    pub elapsed: f64,
    pub url: Option<String>,
}

/// The MR of a pipeline's ref.
pub fn mr_of_ref(reference: &str) -> Option<i64> {
    reference
        .strip_prefix("refs/merge-requests/")?
        .strip_suffix("/head")?
        .parse()
        .ok()
}

/// Why a job is flagged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Why {
    /// The last to finish, the others waiting, in `of` of `among` of its pipelines.
    Critical { of: usize, among: usize },
    /// Its median of the last days over the one of the days before.
    Slower { before: f64, now: f64 },
    /// Its median run on so few cores.
    Idle { cores: f64 },
}

/// A job that holds pipelines up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flag {
    pub job: String,
    /// Its median duration, seconds.
    pub median: f64,
    pub why: Vec<Why>,
}

/// A job running far past its median, its log not growing.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Hung {
    pub id: i64,
    pub job: String,
    pub pipeline: i64,
    pub mr: Option<i64>,
    pub elapsed: f64,
    pub median: f64,
    pub url: Option<String>,
    /// Seconds its log hasn't grown.
    pub quiet: f64,
    /// The local runner's load while it is busy, when the job runs there.
    pub load: Option<f64>,
    /// The section of its log it is in (`step_script`, `archive_cache`), when the log says.
    pub step: Option<String>,
}

/// What the watcher found, for the overview.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    /// When (ms).
    pub at: i64,
    pub flags: Vec<Flag>,
    pub hung: Vec<Hung>,
}

fn middle_of(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    })
}

/// A job's runs that count (passed or failed), newest first.
fn runs_of<'a>(runs: &'a [Run], job: &str) -> Vec<&'a Run> {
    let mut of: Vec<&Run> = runs
        .iter()
        .filter(|run| run.name == job && matches!(run.status.as_str(), "success" | "failed"))
        .collect();
    of.sort_by_key(|run| std::cmp::Reverse(run.finished));
    of
}

/// The median duration of a job's newest `window` runs.
pub fn median_of(runs: &[Run], job: &str, window: usize) -> Option<f64> {
    let mut durations: Vec<f64> = runs_of(runs, job).iter().take(window).map(|run| run.duration).collect();
    middle_of(&mut durations)
}

/// The job each pipeline waited for: the last to finish, when the one
/// before it ended `margin` seconds earlier or more. pipeline → job.
pub fn critical_paths(runs: &[Run], margin: f64) -> BTreeMap<i64, String> {
    let mut by_pipeline: BTreeMap<i64, Vec<&Run>> = BTreeMap::new();
    for run in runs
        .iter()
        .filter(|run| run.status != "skipped" && run.status != "manual")
    {
        by_pipeline.entry(run.pipeline).or_default().push(run);
    }
    by_pipeline
        .into_iter()
        .filter_map(|(pipeline, mut runs)| {
            if runs.len() < 2 {
                return None;
            }
            runs.sort_by_key(|run| std::cmp::Reverse(run.finished));
            let gap = (runs[0].finished - runs[1].finished) as f64 / 1000.0;
            (gap >= margin).then(|| (pipeline, runs[0].name.clone()))
        })
        .collect()
}

/// The jobs that hold pipelines up, by the thresholds of `[ci.watch]`.
pub fn flags_of(runs: &[Run], watch: &CiWatch, now: i64) -> Vec<Flag> {
    let critical = critical_paths(runs, watch.critical_margin_seconds);
    let names: BTreeSet<&str> = runs.iter().map(|run| run.name.as_str()).collect();
    let mut flags = Vec::new();
    for job in names {
        let of = runs_of(runs, job);
        if of.len() < watch.min_runs {
            continue;
        }
        let Some(median) = median_of(runs, job, watch.window) else {
            continue;
        };
        let mut why = Vec::new();
        if median >= watch.min_seconds {
            let pipelines: Vec<i64> = of.iter().take(watch.window).map(|run| run.pipeline).collect();
            let last = pipelines
                .iter()
                .filter(|pipeline| critical.get(pipeline).is_some_and(|name| name == job))
                .count();
            if last as f64 > watch.critical_share * pipelines.len() as f64 {
                why.push(Why::Critical {
                    of: last,
                    among: pipelines.len(),
                });
            }
            let days = watch.slowdown_days * DAY_MS;
            let mut recent: Vec<f64> = of
                .iter()
                .filter(|run| now - run.finished <= days)
                .map(|run| run.duration)
                .collect();
            let mut before: Vec<f64> = of
                .iter()
                .filter(|run| now - run.finished > days && now - run.finished <= 2 * days)
                .map(|run| run.duration)
                .collect();
            let enough = watch.min_runs.div_ceil(2).max(2);
            if recent.len() >= enough
                && before.len() >= enough
                && let (Some(recent), Some(before)) = (middle_of(&mut recent), middle_of(&mut before))
                && recent >= before * (1.0 + watch.slowdown)
            {
                why.push(Why::Slower { before, now: recent });
            }
        }
        let sampled: Vec<&&Run> = of.iter().take(watch.window).filter(|run| run.cpu.is_some()).collect();
        if sampled.len() >= watch.min_runs.div_ceil(2).max(2) {
            let mut durations: Vec<f64> = sampled.iter().map(|run| run.duration).collect();
            let mut cores: Vec<f64> = sampled.iter().filter_map(|run| run.cpu).collect();
            if let (Some(duration), Some(cores)) = (middle_of(&mut durations), middle_of(&mut cores))
                && duration >= watch.idle_seconds
                && cores < watch.idle_cores
            {
                why.push(Why::Idle { cores });
            }
        }
        if !why.is_empty() {
            flags.push(Flag {
                job: job.to_string(),
                median,
                why,
            });
        }
    }
    // The one holding up the most pipelines first.
    flags.sort_by(|a, b| b.median.total_cmp(&a.median));
    flags
}

/// The running jobs past `hang_factor` times their median: hung once their
/// log stays quiet (`is_hung`).
pub fn overdue_of(running: &[Running], runs: &[Run], watch: &CiWatch) -> Vec<Hung> {
    running
        .iter()
        .filter_map(|job| {
            if runs_of(runs, &job.name).len() < watch.min_runs {
                return None;
            }
            let median = median_of(runs, &job.name, watch.window)?;
            (job.elapsed >= watch.hang_seconds && job.elapsed > watch.hang_factor * median).then(|| Hung {
                id: job.id,
                job: job.name.clone(),
                pipeline: job.pipeline,
                mr: job.mr,
                elapsed: job.elapsed,
                median,
                url: job.url.clone(),
                ..Hung::default()
            })
        })
        .collect()
}

/// The section a job's log is in now: the innermost of GitLab's
/// `section_start:<time>:<name>[…]` markers without its `section_end`.
pub fn step_of(trace: &str) -> Option<String> {
    let mut open: Vec<&str> = Vec::new();
    for (at, _) in trace.match_indices("section_") {
        let rest = &trace[at + "section_".len()..];
        let (is_start, rest) = if let Some(rest) = rest.strip_prefix("start:") {
            (true, rest)
        } else if let Some(rest) = rest.strip_prefix("end:") {
            (false, rest)
        } else {
            continue;
        };
        let Some((time, rest)) = rest.split_once(':') else {
            continue;
        };
        if time.is_empty() || !time.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let name = &rest[..rest
            .find(|c: char| c == '[' || c == '\r' || c == '\x1b' || c.is_whitespace())
            .unwrap_or(rest.len())];
        if name.is_empty() {
            continue;
        }
        if is_start {
            open.push(name);
        } else if let Some(index) = open.iter().rposition(|known| *known == name) {
            open.truncate(index);
        }
    }
    open.last().map(|name| (*name).to_string())
}

/// Whether a section of the log moves the cache or artifacts: it logs
/// nothing for minutes while it packs `target/` (#424).
fn is_caching_step(step: &str) -> bool {
    ["cache", "archive", "upload"].iter().any(|word| step.contains(word)) || step == "download_artifacts"
}

/// GitLab's words for its own sections, the name itself for one of the script's.
fn step_label(step: &str) -> &str {
    match step {
        "prepare_executor" => "Preparing the executor",
        "prepare_script" => "Preparing environment",
        "get_sources" => "Getting source",
        "restore_cache" => "Restoring cache",
        "download_artifacts" => "Downloading artifacts",
        "step_script" => "Executing step script",
        "after_script" => "Running after_script",
        "archive_cache" | "archive_cache_on_failure" => "Creating cache",
        "upload_artifacts_on_success" | "upload_artifacts_on_failure" => "Uploading artifacts",
        "cleanup_file_variables" => "Cleaning up",
        other => other,
    }
}

/// `10:38` of 638 seconds.
pub fn minutes(seconds: f64) -> String {
    let seconds = seconds.round() as i64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl Flag {
    /// `check:x holds pipelines up · median 10:38, critical path in 9/10, idle at 0.4 cores`.
    pub fn line(&self) -> String {
        let mut parts = vec![format!("median {}", minutes(self.median))];
        for why in &self.why {
            parts.push(match why {
                Why::Critical { of, among } => format!("critical path in {of}/{among}"),
                Why::Slower { before, now } => format!(
                    "{:+.0} % ({} → {})",
                    (now / before - 1.0) * 100.0,
                    minutes(*before),
                    minutes(*now)
                ),
                Why::Idle { cores } => format!("idle at {cores:.1} cores"),
            });
        }
        format!("{} holds pipelines up · {}", self.job, parts.join(", "))
    }

    /// The key it is posted under: once per job while it stays flagged.
    pub fn key(&self) -> String {
        format!("slow:{}", self.job)
    }
}

impl Hung {
    /// Whether its log stayed quiet long enough: one that still writes is slow, not hung.
    pub fn is_quiet(&self, watch: &CiWatch) -> bool {
        self.quiet >= watch.hang_quiet_seconds
    }

    /// Whether it moves its cache or artifacts now: quiet there is work, not a hang (#424).
    pub fn is_caching(&self) -> bool {
        self.step.as_deref().is_some_and(is_caching_step)
    }

    /// Quiet outside a cache step: worth a question.
    pub fn is_hung(&self, watch: &CiWatch) -> bool {
        self.is_quiet(watch) && !self.is_caching()
    }

    /// `check:x of !353 runs 25:10, 2.4× its median 10:38, no new log for 1:10, in: Executing step script`.
    pub fn line(&self) -> String {
        let of = self.mr.map(|mr| format!(" of !{mr}")).unwrap_or_default();
        let step = self
            .step
            .as_deref()
            .map(|step| format!(", in: {}", step_label(step)))
            .unwrap_or_default();
        format!(
            "{}{of} runs {}, {:.1}× its median {}, no new log for {}{step}",
            self.job,
            minutes(self.elapsed),
            self.elapsed / self.median.max(1.0),
            minutes(self.median),
            minutes(self.quiet)
        )
    }

    /// What it may be: `hung?`, or `slow: runner at load 24` on a busy local runner.
    pub fn verdict(&self) -> String {
        match self.load {
            Some(load) => format!("slow: runner at load {load:.0}"),
            None => "hung?".into(),
        }
    }

    pub fn key(&self) -> String {
        format!("hung:{}", self.id)
    }
}

impl Report {
    /// The report of `main` while the watcher keeps it, else None.
    pub fn read(main: &Path, now: i64) -> Option<Report> {
        let report: Report = crate::ci::read_or_default(&watch_file(main));
        (report.at > 0 && now - report.at < REPORT_STALE_MS).then_some(report)
    }

    /// The overview's CI line: the first flag, `+N more`.
    pub fn line(&self) -> Option<String> {
        let first = self.flags.first()?;
        let more = self.flags.len() - 1;
        Some(if more == 0 {
            format!("CI: {}", first.line())
        } else {
            format!("CI: {} · +{more} more", first.line())
        })
    }

    /// The quiet job of an MR's pipeline, if one runs: hung, or caching.
    pub fn hung_of(&self, mr: i64) -> Option<&Hung> {
        self.hung.iter().find(|hung| hung.mr == Some(mr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn run(id: i64, name: &str, pipeline: i64, duration: f64, finished: i64) -> Run {
        Run {
            id,
            name: name.into(),
            pipeline,
            status: "success".into(),
            duration,
            finished,
            ..Run::default()
        }
    }

    /// Ten pipelines of `check:fast` (60 s) and `check:gauntlet` (`slow` s), the gauntlet last in all but `except`.
    fn pipelines(slow: f64, except: &[i64]) -> Vec<Run> {
        let mut runs = Vec::new();
        for pipeline in 1..=10 {
            let start = NOW - pipeline * 3_600_000;
            let gauntlet_end = if except.contains(&pipeline) { 100.0 } else { slow };
            runs.push(run(pipeline * 10, "check:fast", pipeline, 60.0, start + 60_000));
            runs.push(run(
                pipeline * 10 + 1,
                "check:gauntlet",
                pipeline,
                gauntlet_end,
                start + (gauntlet_end * 1000.0) as i64,
            ));
            if except.contains(&pipeline) {
                runs.push(run(pipeline * 10 + 2, "check:vita", pipeline, 400.0, start + 400_000));
            }
        }
        runs
    }

    #[test]
    fn the_job_most_pipelines_wait_for_is_their_critical_path() {
        let watch = CiWatch::default();
        let runs = pipelines(638.0, &[3]);
        let flags = flags_of(&runs, &watch, NOW);
        assert_eq!(flags.len(), 1, "{flags:?}");
        assert_eq!(flags[0].job, "check:gauntlet");
        assert_eq!(flags[0].why, vec![Why::Critical { of: 9, among: 10 }]);
        assert_eq!(
            Report {
                at: NOW,
                flags,
                hung: Vec::new()
            }
            .line()
            .as_deref(),
            Some("CI: check:gauntlet holds pipelines up · median 10:38, critical path in 9/10")
        );
        // A short job last in every pipeline (a deploy) holds nothing up.
        assert!(flags_of(&pipelines(90.0, &[]), &watch, NOW).is_empty());
        // One that ends with the others isn't waited for.
        let together: Vec<Run> = pipelines(638.0, &[])
            .into_iter()
            .map(|mut run| {
                if run.name == "check:fast" {
                    run.finished += 600_000;
                }
                run
            })
            .collect();
        assert!(flags_of(&together, &watch, NOW).is_empty());
    }

    #[test]
    fn a_median_that_grows_is_a_slowdown() {
        let watch = CiWatch::default();
        let mut runs = Vec::new();
        for day in 0..12 {
            let duration = if day < 6 { 300.0 } else { 200.0 };
            // One job a pipeline: no critical path.
            runs.push(run(day, "build", day, duration, NOW - day * DAY_MS - 1000));
        }
        let flags = flags_of(&runs, &watch, NOW);
        assert_eq!(
            flags[0].why,
            vec![Why::Slower {
                before: 200.0,
                now: 300.0
            }]
        );
        assert!(
            flags[0].line().ends_with("median 5:00, +50 % (3:20 → 5:00)"),
            "{}",
            flags[0].line()
        );
        // +10 %: no flag.
        for run in runs.iter_mut().filter(|run| run.duration == 300.0) {
            run.duration = 220.0;
        }
        assert!(flags_of(&runs, &watch, NOW).is_empty());
    }

    #[test]
    fn a_long_job_on_few_cores_is_idle() {
        let watch = CiWatch::default();
        let mut runs: Vec<Run> = (0..6)
            .map(|n| Run {
                cpu: Some(0.4),
                ..run(n, "gauntlet", n, 630.0, NOW - n * 1000)
            })
            .collect();
        assert_eq!(flags_of(&runs, &watch, NOW)[0].why, vec![Why::Idle { cores: 0.4 }]);
        for run in &mut runs {
            run.cpu = Some(3.5);
        }
        assert!(flags_of(&runs, &watch, NOW).is_empty());
    }

    #[test]
    fn a_job_far_past_its_median_is_hung() {
        let watch = CiWatch::default();
        let runs = pipelines(638.0, &[]);
        let running = |elapsed| Running {
            id: 99,
            name: "check:gauntlet".into(),
            pipeline: 11,
            mr: Some(353),
            elapsed,
            url: None,
        };
        assert!(overdue_of(&[running(1200.0)], &runs, &watch).is_empty());
        let mut hung = overdue_of(&[running(1500.0)], &runs, &watch);
        assert_eq!(hung.len(), 1);
        // Past its median but its log just read: not hung yet.
        assert!(!hung[0].is_hung(&watch));
        hung[0].quiet = 70.0;
        assert!(hung[0].is_hung(&watch));
        // Quiet while it packs its cache: no hang (#424).
        hung[0].step = Some("archive_cache".into());
        assert!(hung[0].is_quiet(&watch) && hung[0].is_caching() && !hung[0].is_hung(&watch));
        assert!(
            hung[0].line().ends_with("no new log for 1:10, in: Creating cache"),
            "{}",
            hung[0].line()
        );
        hung[0].step = None;
        assert_eq!(
            hung[0].line(),
            "check:gauntlet of !353 runs 25:00, 2.4× its median 10:38, no new log for 1:10"
        );
        assert_eq!(hung[0].verdict(), "hung?");
        hung[0].load = Some(23.6);
        assert_eq!(hung[0].verdict(), "slow: runner at load 24");
        // A job without enough history: nothing to compare with.
        let new = Running {
            name: "check:new".into(),
            ..running(5000.0)
        };
        assert!(overdue_of(&[new], &runs, &watch).is_empty());
    }

    #[test]
    fn the_step_is_the_innermost_open_section() {
        let start = |name: &str| format!("\x1b[0Ksection_start:1760000000:{name}[collapsed=true]\r\x1b[0KTitle\n");
        let end = |name: &str| format!("\x1b[0Ksection_end:1760000100:{name}\r\x1b[0K\n");
        let script = format!(
            "{}{}compiling\n{}",
            start("get_sources"),
            end("get_sources"),
            start("step_script")
        );
        assert_eq!(step_of(&script).as_deref(), Some("step_script"));
        let nested = format!("{script}{}cargo test\n", start("tests"));
        assert_eq!(step_of(&nested).as_deref(), Some("tests"));
        let cache = format!(
            "{nested}{}{}{}Creating cache breadboard-rs-protected...\n",
            end("tests"),
            end("step_script"),
            start("archive_cache")
        );
        assert_eq!(step_of(&cache).as_deref(), Some("archive_cache"));
        assert!(is_caching_step("archive_cache") && is_caching_step("restore_cache"));
        assert!(is_caching_step("upload_artifacts_on_success") && !is_caching_step("step_script"));
        // Ended, or a log without sections: no step.
        assert_eq!(step_of(&format!("{cache}{}", end("archive_cache"))), None);
        assert_eq!(step_of("plain log, no section_start here\n"), None);
    }

    #[test]
    fn a_log_that_grows_is_progress() {
        let mut history = History::default();
        // The first read: nothing to compare with, it counts as growth.
        assert_eq!(history.progress(7, 100, NOW), 0.0);
        assert_eq!(history.progress(7, 100, NOW + 30_000), 30.0);
        // New output: quiet again.
        assert_eq!(history.progress(7, 180, NOW + 60_000), 0.0);
        assert_eq!(history.progress(7, 180, NOW + 130_000), 70.0);
    }

    #[test]
    fn keeps_each_job_once_with_its_samples() {
        let mut history = History::default();
        history.samples.entry(1).or_default().add(0.2, 100.0);
        history.samples.entry(1).or_default().add(0.6, 300.0);
        history.add(vec![run(1, "a", 1, 10.0, NOW), run(2, "b", 1, 10.0, NOW - 20 * DAY_MS)]);
        history.add(vec![run(1, "a", 1, 12.0, NOW)]);
        assert_eq!(history.runs.len(), 2);
        assert_eq!(history.runs[0].duration, 12.0);
        history.prune(14, NOW);
        assert_eq!(history.runs.len(), 1);
        assert!(history.samples.is_empty());
        let mut sampled = History::default();
        sampled.samples.entry(1).or_default().add(0.2, 100.0);
        sampled.samples.entry(1).or_default().add(0.6, 300.0);
        sampled.add(vec![run(1, "a", 1, 10.0, NOW)]);
        assert_eq!(sampled.runs[0].cpu, Some(0.4));
        assert_eq!(sampled.runs[0].mem, Some(300.0));
        assert_eq!(mr_of_ref("refs/merge-requests/353/head"), Some(353));
    }

    #[test]
    fn reads_gitlab_s_jobs_and_docker_s_usage() {
        let job = serde_json::json!({
            "id": 7, "name": "check:x", "status": "success", "duration": 638.2, "queued_duration": 1.5,
            "runner": { "description": "laptop" }, "started_at": "2026-10-13T12:00:00Z",
            "finished_at": "2026-10-13T12:10:38Z", "pipeline": { "id": 3, "ref": "refs/merge-requests/353/head" },
        });
        let run = run_of(&job).unwrap();
        assert_eq!(
            (run.id, run.pipeline, run.duration, run.runner.as_deref()),
            (7, 3, 638.2, Some("laptop"))
        );
        assert!(running_of(&job, NOW).is_none());
        let mut live = job.clone();
        live["status"] = "running".into();
        live["finished_at"] = serde_json::Value::Null;
        assert!(run_of(&live).is_none());
        let started = crate::time::parse_iso("2026-10-13T12:00:00Z").unwrap();
        let running = running_of(&live, started + 90_000).unwrap();
        assert_eq!((running.elapsed, running.mr), (90.0, Some(353)));
        let ps = "aaa\t7\nbbb\t7\nccc\t8\n";
        let stats = "aaa\t35.00%\t300MiB / 15GiB\nbbb\t5.00%\t1GiB / 15GiB\nddd\t99%\t1GiB / 1GiB\n";
        let usage = usage_of(ps, stats);
        assert_eq!(usage.len(), 1);
        let (cores, mem) = usage[&7];
        assert!(
            (cores - 0.4).abs() < 1e-9 && (mem - 1324.0).abs() < 1e-9,
            "{cores} {mem}"
        );
        assert_eq!(mr_of_ref("develop"), None);
    }
}
