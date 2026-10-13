//! The CI job watcher (#379): what `breadboard serve` keeps of the project's
//! finished jobs (`.claude/ci/jobs.json`, a rolling history per job name)
//! and what it flags in them (`.claude/ci/watch.json`): a job that is the
//! critical path of most of its pipelines, one whose median grows, one
//! running far past its median now (hung), one that mostly waits (idle, from
//! the local runner's CPU samples). Pure: the daemon reads GitLab and docker,
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

/// What the watcher keeps between its ticks and restarts.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub runs: Vec<Run>,
    /// Pipelines read once finished: id → its `updated_at` then (a retry reads it again).
    pub seen: BTreeMap<i64, String>,
    /// Running jobs' CPU samples by job id.
    pub samples: BTreeMap<i64, Samples>,
    /// The flags already posted as actions (`slow:<job>`, `hung:<id>`).
    pub posted: BTreeSet<String>,
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

/// A job running far past its median.
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
    Some(if values.len() % 2 == 0 {
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
    for run in runs.iter().filter(|run| run.status != "skipped" && run.status != "manual") {
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

/// The running jobs past `hang_factor` times their median.
pub fn hung_of(running: &[Running], runs: &[Run], watch: &CiWatch) -> Vec<Hung> {
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
            })
        })
        .collect()
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
    /// `check:x of !353 runs 25:10, 2.4× its median 10:38`.
    pub fn line(&self) -> String {
        let of = self.mr.map(|mr| format!(" of !{mr}")).unwrap_or_default();
        format!(
            "{}{of} runs {}, {:.1}× its median {}",
            self.job,
            minutes(self.elapsed),
            self.elapsed / self.median.max(1.0),
            minutes(self.median)
        )
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

    /// The hung job of an MR's pipeline, if one runs.
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
        assert!(flags[0].line().ends_with("median 5:00, +50 % (3:20 → 5:00)"), "{}", flags[0].line());
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
        assert!(hung_of(&[running(1200.0)], &runs, &watch).is_empty());
        let hung = hung_of(&[running(1500.0)], &runs, &watch);
        assert_eq!(hung.len(), 1);
        assert_eq!(hung[0].line(), "check:gauntlet of !353 runs 25:00, 2.4× its median 10:38");
        // A job without enough history: nothing to compare with.
        let new = Running {
            name: "check:new".into(),
            ..running(5000.0)
        };
        assert!(hung_of(&[new], &runs, &watch).is_empty());
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
        assert_eq!(mr_of_ref("develop"), None);
    }
}
