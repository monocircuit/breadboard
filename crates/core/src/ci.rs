//! CI in the order that matters (#357). GitLab has no job priorities: its
//! runner takes jobs first come, first served. So breadboard decides which
//! pipelines exist at all: worktree pushes skip CI (`ci.skip`, `SKIP_OPTION`),
//! and `breadboard serve` creates an MR's pipeline when its turn comes,
//! highest class first, while the runner has free slots.
//!
//! The files of `<main>/.claude/ci/`, each written whole by one side:
//! - `order-<orchestration>.json` (`Published`): an orchestration's merge
//!   order and each MR's bar (its boxes, its wt:check), by its live overviews;
//! - `run-now.json` (`RunNow`): the MRs a click moved to the front;
//! - `queue.json` (`Queue`): where each MR stands, by the scheduler, for the rows.

use crate::gitlab::Pipeline;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What `git push` carries from a worktree: GitLab creates no pipeline for it.
pub const SKIP_OPTION: &str = "ci.skip";
/// `git config breadboard.ci push`: pushes start their pipelines again, as before #357.
pub const MODE_KEY: &str = "breadboard.ci";
/// How old a queue may be before the rows stop showing it: no scheduler runs.
pub const QUEUE_STALE_MS: i64 = 60_000;
/// Runner slots assumed when its config can't be read.
pub const SLOTS_UNKNOWN: u32 = 2;
/// How long a pipeline may run and still be preempted (#419): past it, it finishes.
pub const PREEMPT_GRACE_MS: i64 = 2 * 60_000;
/// How long an MR preempted once is never preempted again (#419).
pub const PREEMPT_AGAIN_MS: i64 = 30 * 60_000;

/// A pipeline still to finish.
pub fn is_live(status: &str) -> bool {
    matches!(
        status,
        "created" | "waiting_for_resource" | "preparing" | "pending" | "running" | "scheduled"
    )
}

/// A pipeline whose jobs wait for the runner: the runner is full.
fn is_waiting(status: &str) -> bool {
    matches!(status, "created" | "waiting_for_resource" | "preparing" | "pending")
}

// ---------------------------------------------------------------------------
// The pushes
// ---------------------------------------------------------------------------

/// A remote reached over the network (`git@host:p.git`, `ssh://`, `https://`):
/// one GitLab serves, which takes push options.
pub fn is_network_url(url: &str) -> bool {
    if url.starts_with("file://") || url.starts_with('/') || url.starts_with('.') {
        return false;
    }
    url.contains("://")
        || url
            .split_once(':')
            .is_some_and(|(host, _)| host.contains('@') || host.contains('.'))
}

/// What `set_push_option` changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOption {
    /// Pushes skip CI from now on: a queue runs.
    Set,
    /// Pushes start their pipelines again, and why: no queue runs (its
    /// heartbeat is stale), `breadboard.ci = push`, or git refused the option.
    Dropped(String),
}

/// What a worktree's pushes carry, set so (#357): `ci.skip` in its own
/// config (`git config --worktree push.pushOption`) while a queue runs to
/// start its pipelines (the heartbeat: `queue.json` younger than
/// QUEUE_STALE_MS), else none, so nothing stalls while no `breadboard serve`
/// runs; none with `git config breadboard.ci push` or a local origin either.
/// The main checkout is left alone: develop's pipelines deploy. None when
/// nothing changed.
pub fn set_push_option(dir: &Path, now: i64) -> Option<PushOption> {
    let git = |args: &[&str]| crate::git::git(dir, args);
    let dirs = git(&["rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir"]);
    let mut lines = dirs.lines();
    let (own, common) = (lines.next()?, lines.next()?);
    if !dirs.is_ok || own == common {
        return None;
    }
    // A local origin (a path, `file://`) takes no push options: a push with one would fail.
    let origin = git(&["remote", "get-url", "--push", "origin"]);
    let is_network = origin.is_ok && is_network_url(origin.out());
    let is_on = git(&["config", "--get", MODE_KEY]).out() != "push";
    let is_running = crate::git::main_dir(dir)
        .and_then(|main| Queue::read(&main, now))
        .is_some_and(|queue| !queue.is_dry);
    let is_queued = is_network && is_on && is_running;
    let options = git(&["config", "--get-all", "push.pushOption"]);
    let has_skip = options.lines().any(|line| line.trim() == SKIP_OPTION);
    if is_queued == has_skip {
        return None;
    }
    if !is_queued {
        git(&[
            "config",
            "--worktree",
            "--unset-all",
            "push.pushOption",
            &format!("^{SKIP_OPTION}$"),
        ]);
        let why = if !is_on {
            "breadboard.ci = push"
        } else if !is_network {
            "its origin takes no push options"
        } else {
            "no breadboard serve runs the CI queue"
        };
        return Some(PushOption::Dropped(why.into()));
    }
    // A config of the worktree's own needs git's extension, once for the clone.
    if git(&["config", "--get", "extensions.worktreeConfig"]).out() != "true" {
        git(&["config", "extensions.worktreeConfig", "true"]);
    }
    let set = git(&["config", "--worktree", "--add", "push.pushOption", SKIP_OPTION]);
    Some(if set.is_ok {
        PushOption::Set
    } else {
        PushOption::Dropped(set.stderr.trim().to_string())
    })
}

// ---------------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------------

pub fn dir_of(main: &Path) -> PathBuf {
    main.join(".claude").join("ci")
}

pub fn order_file(main: &Path, orchestration: &str) -> PathBuf {
    dir_of(main).join(format!("order-{orchestration}.json"))
}

pub fn run_now_file(main: &Path) -> PathBuf {
    dir_of(main).join("run-now.json")
}

pub fn queue_file(main: &Path) -> PathBuf {
    dir_of(main).join("queue.json")
}

pub fn asked_file(main: &Path) -> PathBuf {
    dir_of(main).join("asked.json")
}

pub fn scheduler_file(main: &Path) -> PathBuf {
    dir_of(main).join("scheduler.pid")
}

/// How many of its own pipelines the queue remembers: enough for every open MR's last few.
const OURS_KEPT: usize = 500;

/// What the queue asked GitLab for (#369), kept in `asked.json` over its
/// ticks and a daemon's restart: a commit asked for once is never asked
/// for again by itself, and only a pipeline it created is ever cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Asked {
    /// iid → the commit asked for and when (ms).
    pub started: BTreeMap<i64, (String, i64)>,
    /// iid → the commit whose pipeline it cancelled for an MR ahead: queued again.
    pub preempted: BTreeMap<i64, String>,
    /// iid → the blocking pipeline it asked a new one over (`OpenMr::blocked_by`).
    pub rescued: BTreeMap<i64, i64>,
    /// The pipelines it created, oldest first.
    pub ours: Vec<i64>,
    /// The pipelines it cancelled itself, oldest first (#383): a head pipeline
    /// among them is no person's cancel.
    pub cancelled: Vec<i64>,
    /// iid → the last preemption of its pipeline (#419), kept PREEMPT_AGAIN_MS.
    pub preempts: BTreeMap<i64, Preemption>,
}

/// A pipeline cancelled to make room (#419): for which MR, and when (ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preemption {
    pub by: i64,
    pub at: i64,
}

impl Asked {
    /// A pipeline it created, the oldest forgotten past OURS_KEPT.
    pub fn add_ours(&mut self, id: i64) {
        if !self.ours.contains(&id) {
            self.ours.push(id);
        }
        let over = self.ours.len().saturating_sub(OURS_KEPT);
        self.ours.drain(..over);
    }

    /// A pipeline it cancelled, the oldest forgotten past OURS_KEPT.
    pub fn add_cancelled(&mut self, id: i64) {
        if !self.cancelled.contains(&id) {
            self.cancelled.push(id);
        }
        let over = self.cancelled.len().saturating_sub(OURS_KEPT);
        self.cancelled.drain(..over);
    }
}

/// Whether this process schedules `main`'s queue: one per checkout (#369,
/// two daemons each started their own pipelines). The first one alive keeps
/// it; a dead one's is taken over.
pub fn holds_scheduler(main: &Path) -> bool {
    holds(main, &scheduler_file(main))
}

/// Whether this process holds the pid file `file` of `main`: the first one
/// alive keeps it, a dead one's is taken over.
pub fn holds(main: &Path, file: &Path) -> bool {
    let file = file.to_path_buf();
    let own = i64::from(std::process::id());
    let held = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok());
    match held {
        Some(pid) if pid == own => true,
        Some(pid) if crate::lock::is_alive(pid) => false,
        _ => {
            let _ = std::fs::create_dir_all(dir_of(main));
            let beside = file.with_extension(format!("pid.{own}"));
            std::fs::write(&beside, format!("{own}\n")).is_ok() && std::fs::rename(&beside, &file).is_ok()
        }
    }
}

/// Where an MR's work stands (#357, the user's rule: no pipeline before the
/// bar is full): its issue's boxes, those after the deploy left out, and
/// whether `wt:check` passed at its HEAD.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Bar {
    pub ticked: usize,
    pub total: usize,
    pub is_checked: bool,
    /// The commit `wt:check` passed at; None without a worktree here (its boxes say it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// What keeps it a draft though its boxes are ticked (#399): an open
    /// try, a requested change. None: a full bar marks it ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holds: Option<String>,
}

impl Bar {
    /// Every box ticked, at least one (#399): its MR is ready, whatever wt:check says.
    pub fn is_boxes_full(&self) -> bool {
        self.total > 0 && self.ticked >= self.total
    }

    /// Every box ticked and checked: its pipeline may start.
    pub fn is_full(&self) -> bool {
        self.ticked >= self.total && self.is_checked
    }

    /// Full, and checked at `sha` (#369): a check of another commit doesn't start this one.
    pub fn is_full_at(&self, sha: Option<&str>) -> bool {
        self.is_full()
            && self.sha.as_deref().is_none_or(|checked| {
                sha.is_some_and(|sha| !sha.is_empty() && (sha.starts_with(checked) || checked.starts_with(sha)))
            })
    }
}

/// What an orchestration's live overviews publish on each read.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Published {
    /// The open MRs (iids) in the merge order.
    pub order: Vec<i64>,
    /// Each open MR's bar, by iid.
    pub bars: BTreeMap<i64, Bar>,
    /// The MRs held by a hard wait (a prerequisite it needs, a `wt:wait`), by
    /// iid: `after !339`. They get no pipeline until it is merged (#392);
    /// MRs that only share files with another are not here, they only rank.
    pub held: BTreeMap<i64, String>,
}

/// The MRs a click moved to the front: iid → when (ms).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RunNow {
    pub at: BTreeMap<i64, i64>,
}

/// Where an MR stands in the queue, as its row says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Place {
    /// Its newest commit's pipeline runs.
    Running,
    /// Its newest commit's pipeline waits for the runner.
    Pending,
    /// Next to start; `at` 1 the very next.
    Next { at: usize },
    /// Its bar isn't full: no pipeline before it is (a click starts one anyway).
    Boxes(Bar),
    /// A hard wait holds it (#392): `after !339`, `waits for !339`. No pipeline
    /// by itself before that MR is merged; a click starts one anyway.
    Held { why: String },
    /// Its pipeline was cancelled to make room for `by` (#419): it starts first once a slot is free.
    Preempted { by: i64 },
    /// No pipeline of its newest commit runs and none starts by itself (#383):
    /// why, in a few words (a person cancelled it, GitLab took none).
    Stopped { why: String },
}

impl Place {
    /// Waiting for its pipeline: next or held.
    pub fn is_waiting(&self) -> bool {
        matches!(
            self,
            Place::Next { .. } | Place::Preempted { .. } | Place::Boxes(_) | Place::Held { .. }
        )
    }

    /// `CI: running`, `CI: next (2nd)`, `CI: held, after !339`.
    pub fn text(&self) -> String {
        match self {
            Place::Running => "CI: running".into(),
            Place::Pending => "CI: pending".into(),
            Place::Next { at: 1 } => "CI: next".into(),
            Place::Next { at } => format!("CI: next ({})", ordinal(*at)),
            Place::Preempted { by } => format!("CI: preempted by !{by}, restarts next"),
            Place::Boxes(bar) if bar.ticked < bar.total => {
                format!("CI: after the boxes ({}/{})", bar.ticked, bar.total)
            }
            Place::Boxes(_) => "CI: after wt:check".into(),
            Place::Held { why } => format!("CI: held, {why}"),
            Place::Stopped { why } => format!("CI: {why}, a click runs it"),
        }
    }
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`, `22nd`.
pub fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// The runner's slots: its `concurrent` and the jobs it runs now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Slots {
    pub concurrent: u32,
    pub running: u32,
    /// Read from the runner, not assumed.
    pub is_known: bool,
}

impl Slots {
    pub fn free(&self) -> u32 {
        self.concurrent.saturating_sub(self.running)
    }
}

/// The runner's config, where `gitlab-runner-autoscale` sets `concurrent`;
/// `BREADBOARD_RUNNER_CONFIG` names another (a test's).
fn runner_config() -> PathBuf {
    std::env::var_os("BREADBOARD_RUNNER_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/gitlab-runner/config.toml"))
}

/// `concurrent = 4` of a runner's config.toml.
fn concurrent_of(config: &str) -> Option<u32> {
    config.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "concurrent").then(|| value.trim().parse().ok())?
    })
}

/// The jobs the local runner runs now, counted as gitlab-runner-autoscale
/// does: distinct job ids over its containers (a job may have only its
/// helper container yet, or no build container any more).
fn running_of(docker_ps: &str) -> u32 {
    let ids: HashSet<&str> = docker_ps
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    ids.len() as u32
}

impl Slots {
    /// The local runner's slots; unknown (`SLOTS_UNKNOWN`, `running` from
    /// the MRs' pipelines) when its config or docker can't be read.
    pub fn read(running_pipelines: u32) -> Slots {
        let concurrent = std::fs::read_to_string(runner_config())
            .ok()
            .and_then(|config| concurrent_of(&config));
        let running = std::process::Command::new("docker")
            .args([
                "ps",
                "--filter",
                "label=com.gitlab.gitlab-runner.job.id",
                "--format",
                "{{.Label \"com.gitlab.gitlab-runner.job.id\"}}",
            ])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| running_of(&String::from_utf8_lossy(&output.stdout)));
        match (concurrent, running) {
            (Some(concurrent), Some(running)) => Slots {
                concurrent,
                running,
                is_known: true,
            },
            _ => Slots {
                concurrent: SLOTS_UNKNOWN,
                running: running_pipelines,
                is_known: false,
            },
        }
    }
}

/// What the scheduler wrote last.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Queue {
    /// When (ms).
    pub at: i64,
    pub slots: Slots,
    /// iid → its place; an MR without one has nothing to wait for.
    pub places: BTreeMap<i64, Place>,
    /// What it did last, for the band: `started !339`.
    pub last: Option<String>,
    /// A try's queue: it starts and cancels nothing, so no push may skip CI for it.
    pub is_dry: bool,
}

impl Queue {
    /// The queue of `main` while a scheduler keeps it, else None.
    pub fn read(main: &Path, now: i64) -> Option<Queue> {
        let queue: Queue = serde_json::from_value(crate::json::read(&queue_file(main))?).ok()?;
        (now - queue.at < QUEUE_STALE_MS).then_some(queue)
    }

    /// The band's words: `CI 3/4 slots · 2 next`.
    pub fn load_text(&self) -> String {
        let waiting = self
            .places
            .values()
            .filter(|place| matches!(place, Place::Next { .. }))
            .count();
        let slots = if self.slots.is_known {
            format!("CI {}/{} slots", self.slots.running, self.slots.concurrent)
        } else {
            format!("CI {} running", self.slots.running)
        };
        let text = if waiting == 0 {
            slots
        } else {
            format!("{slots} · {waiting} queued")
        };
        if self.is_dry { format!("{text} (dry)") } else { text }
    }
}

/// `value` into `file` whole: written beside it, then renamed, so a reader never sees half.
pub fn write_whole<T: Serialize>(file: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    let beside = file.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&beside, format!("{text}\n"))?;
    std::fs::rename(&beside, file)
}

/// `file` as `T`, the default when missing or broken.
pub fn read_or_default<T: for<'a> Deserialize<'a> + Default>(file: &Path) -> T {
    crate::json::read(file)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// Every orchestration's published order in `main`.
pub fn published_of(main: &Path) -> Vec<Published> {
    let Ok(entries) = std::fs::read_dir(dir_of(main)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("order-") && name.ends_with(".json"))
        })
        .collect();
    files.sort();
    files.iter().map(|file| read_or_default(file)).collect()
}

// ---------------------------------------------------------------------------
// The queue
// ---------------------------------------------------------------------------

/// An open MR as the scheduler reads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OpenMr {
    pub iid: i64,
    pub source: String,
    pub target: String,
    pub is_draft: bool,
    pub labels: Vec<String>,
    /// Its newest commit.
    pub sha: Option<String>,
    /// Its head pipeline: the newest GitLab has, maybe of an older commit.
    pub pipeline: Option<Pipeline>,
    /// The newest pipeline of its newest commit, cancelled or failed at its
    /// creation, while an older one of that commit passed: GitLab blocks the
    /// merge on it (#369).
    pub blocked_by: Option<i64>,
}

impl OpenMr {
    fn pipeline_status(&self) -> Option<&str> {
        self.pipeline.as_ref().map(|pipeline| pipeline.status.as_str())
    }
    /// Its head pipeline is of its newest commit.
    fn is_head_current(&self) -> bool {
        match (&self.pipeline, &self.sha) {
            (Some(pipeline), Some(sha)) => pipeline.sha.as_deref() == Some(sha.as_str()),
            _ => false,
        }
    }
    /// Never cancelled: an MR into `main` (a release, a hotfix).
    fn is_into_main(&self) -> bool {
        self.target == "main"
    }
}

/// How urgent an MR's pipeline is, lower first (#357's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    /// A click on its row.
    RunNow,
    /// A hotfix, anything into `main`.
    Hotfix,
    /// Ready: in the merge order, then the others oldest first.
    Ready,
    /// Its last pipeline failed and it has a fix pushed.
    Fix,
}

/// What the ranking knows besides the MRs.
#[derive(Debug, Clone, Default)]
pub struct Ranks {
    /// Each orchestration's merge order.
    pub orders: Vec<Vec<i64>>,
    /// The bars of the orchestrations' MRs; an MR of none has no bar to wait for.
    pub bars: HashMap<i64, Bar>,
    /// The MRs a hard wait holds, by iid, why (`Published::held`).
    pub held: HashMap<i64, String>,
    pub run_now: BTreeMap<i64, i64>,
    /// The commits whose pipeline the queue cancelled for an MR ahead, by iid: queued again.
    pub preempted: HashMap<i64, String>,
    /// The pipelines the queue created itself: the only ones it ever cancels (#369).
    pub ours: HashSet<i64>,
    /// iid → the blocking pipeline a new one was asked for over (`OpenMr::blocked_by`).
    pub rescued: HashMap<i64, i64>,
    /// The pipelines the queue cancelled itself (#383): no person's cancel.
    pub cancelled: HashSet<i64>,
    /// The last preemption of each MR's pipeline (`Asked::preempts`).
    pub preempts: HashMap<i64, Preemption>,
    /// Now (ms): how long a pipeline ran, how long ago an MR was preempted.
    pub now: i64,
}

impl Ranks {
    pub fn of(published: &[Published], run_now: &RunNow) -> Ranks {
        Ranks {
            orders: published.iter().map(|one| one.order.clone()).collect(),
            bars: published.iter().flat_map(|one| one.bars.clone()).collect(),
            held: published.iter().flat_map(|one| one.held.clone()).collect(),
            run_now: run_now.at.clone(),
            preempted: HashMap::new(),
            ours: HashSet::new(),
            rescued: HashMap::new(),
            cancelled: HashSet::new(),
            preempts: HashMap::new(),
            now: 0,
        }
    }

    /// An MR of an orchestration whose order the queue reads: the only ones it touches (#369).
    pub fn is_member(&self, iid: i64) -> bool {
        self.bars.contains_key(&iid) || self.place_in_order(iid).is_some()
    }

    /// Its newest commit's pipeline was preempted and waits to start again (#419).
    fn was_preempted(&self, mr: &OpenMr) -> bool {
        mr.sha.is_some() && self.preempted.get(&mr.iid) == mr.sha.as_ref()
    }

    /// Where `iid` stands in the merge orders: the same place in two orchestrations ranks the same.
    fn place_in_order(&self, iid: i64) -> Option<usize> {
        self.orders
            .iter()
            .filter_map(|order| order.iter().position(|&one| one == iid))
            .min()
    }
}

/// Its class; None while a hard wait holds it, its bar isn't full at its
/// newest commit, or it has no bar (no click asked for it).
pub fn class_of(mr: &OpenMr, ranks: &Ranks) -> Option<Class> {
    if ranks.run_now.contains_key(&mr.iid) {
        return Some(Class::RunNow);
    }
    // A hard wait holds its pipeline too: only a click starts one (#392).
    if ranks.held.contains_key(&mr.iid) {
        return None;
    }
    let bar = ranks.bars.get(&mr.iid);
    if bar.is_none_or(|bar| !bar.is_full_at(mr.sha.as_deref())) {
        return None;
    }
    let is_fix = mr.pipeline_status() == Some("failed") && !mr.is_head_current();
    Some(
        if mr.is_into_main()
            || mr.source.starts_with("hotfix/")
            || mr.labels.iter().any(|label| label.eq_ignore_ascii_case("hotfix"))
        {
            Class::Hotfix
        } else if is_fix {
            Class::Fix
        } else {
            // An orchestration's MR with its bar full is marked ready only after its green pipeline.
            Class::Ready
        },
    )
}

/// A pipeline to create, and the superseded one of the same MR to cancel first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub iid: i64,
    pub sha: String,
    pub cancel: Option<i64>,
    /// A click asks for it: a pipeline of this commit that ended doesn't hold it.
    pub is_again: bool,
    /// The blocking pipeline it replaces (`OpenMr::blocked_by`): only a pipeline newer than it holds it.
    pub after: Option<i64>,
}

/// A live pipeline to cancel for the MR ahead (#419: a click's or a hotfix's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preempt {
    pub iid: i64,
    pub pipeline: i64,
    pub sha: String,
    /// The MR it makes room for.
    pub by: i64,
}

/// What the scheduler does now and what the rows say.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decision {
    pub places: BTreeMap<i64, Place>,
    pub start: Option<Start>,
    /// A live pipeline to cancel while the runner is full: the lowest-ranked
    /// of those behind the first MR waiting.
    pub preempt: Option<Preempt>,
}

/// Whether a live pipeline may still be preempted (#419): waiting for the
/// runner, or running for less than PREEMPT_GRACE_MS. One that runs since
/// a time not known is left to finish.
fn is_young(pipeline: &Pipeline, now: i64) -> bool {
    if is_waiting(&pipeline.status) {
        return true;
    }
    pipeline
        .started_at
        .as_deref()
        .and_then(crate::time::parse_iso)
        .is_some_and(|started| now - started < PREEMPT_GRACE_MS)
}

/// Whether a pipeline's jobs have gone past its first stage (#419, the
/// build): a job of a later stage runs or ended. `jobs`: (id, stage, status)
/// as GitLab lists them; the first stage is the one of its first job.
pub fn is_past_first_stage(jobs: &[(i64, String, String)]) -> bool {
    let Some((_, first, _)) = jobs.iter().min_by_key(|(id, _, _)| *id) else {
        return false;
    };
    jobs.iter().any(|(_, stage, status)| {
        stage != first
            && !matches!(
                status.as_str(),
                "created" | "pending" | "manual" | "skipped" | "scheduled"
            )
    })
}

/// Whether the head pipeline of `mr` answers for its newest commit: one of
/// that commit that runs, passed, failed or a person cancelled. A skipped
/// one is a push's `ci.skip` (#383: every push leaves one), one the queue
/// cancelled itself is no person's word: neither answers.
fn has_answer(mr: &OpenMr, ranks: &Ranks) -> bool {
    let Some(pipeline) = mr.pipeline.as_ref().filter(|_| mr.is_head_current()) else {
        return false;
    };
    match pipeline.status.as_str() {
        "skipped" => false,
        "canceled" | "canceling" => !ranks.cancelled.contains(&pipeline.id),
        _ => true,
    }
}

/// Whether `mr` waits for a pipeline of its newest commit: none yet (a
/// push's skipped one is none), a click asks for one again, or the queue
/// cancelled it. A pipeline of that commit that ended, or that a person
/// cancelled, stays so without a click. Asked for once, a commit is never
/// asked for again by itself (#369): GitLab may not show it, or refuse it.
fn wants_pipeline(mr: &OpenMr, ranks: &Ranks, started: &HashMap<i64, (String, i64)>) -> bool {
    let Some(sha) = &mr.sha else {
        return false;
    };
    if mr.is_head_current() && mr.pipeline_status().is_some_and(is_live) {
        return false;
    }
    // A newer pipeline cancelled or refused blocks the merge of a commit that
    // passed: one new pipeline, once per blocking one; never over its own
    // (a refusal of its own again would loop).
    if let Some(blocking) = mr.blocked_by.filter(|blocking| !ranks.ours.contains(blocking)) {
        return ranks.rescued.get(&mr.iid) != Some(&blocking);
    }
    let clicked = ranks.run_now.get(&mr.iid).copied();
    let was_preempted = ranks.was_preempted(mr);
    if let Some((asked, at)) = started.get(&mr.iid)
        && asked == sha
        && !clicked.is_some_and(|clicked| clicked > *at)
        && !(was_preempted && mr.is_head_current())
    {
        return false;
    }
    if has_answer(mr, ranks) {
        return clicked.is_some() || was_preempted;
    }
    true
}

/// Why a member MR without a live pipeline of its newest commit gets none
/// by itself (#383: the row says it), None when it needs none: its newest
/// commit's pipeline passed or failed, or it has no commit.
fn stopped_why(mr: &OpenMr, ranks: &Ranks, started: &HashMap<i64, (String, i64)>) -> Option<String> {
    let sha = mr.sha.as_ref()?;
    if has_answer(mr, ranks) {
        let status = mr.pipeline_status()?;
        return matches!(status, "canceled" | "canceling").then(|| "cancelled by hand".to_string());
    }
    started
        .get(&mr.iid)
        .filter(|(asked, _)| asked == sha)
        .map(|_| "asked for, GitLab made none".to_string())
}

/// The queue now: each MR's place, and the one pipeline to start, if any.
///
/// - one start per call, and only while the runner has a free slot and no
///   pipeline waits for it (a waiting one says the runner is full);
/// - always for the newest commit; its MR's older live pipeline is cancelled
///   first when the queue created it, never one into `main`;
/// - only for an orchestration's MR (one with a bar or a place in an order);
///
/// `started`: what was asked for, iid → (sha, when), so a lagging read doesn't start it twice.
pub fn decide(mrs: &[OpenMr], ranks: &Ranks, slots: Slots, started: &HashMap<i64, (String, i64)>) -> Decision {
    let mut places = BTreeMap::new();
    let mut is_runner_full = false;
    for mr in mrs {
        let Some(status) = mr.pipeline_status().filter(|status| is_live(status)) else {
            continue;
        };
        is_runner_full |= is_waiting(status);
        if mr.is_head_current() && ranks.is_member(mr.iid) {
            let place = if is_waiting(status) {
                Place::Pending
            } else {
                Place::Running
            };
            places.insert(mr.iid, place);
        }
    }
    let mut waiting: Vec<(Class, (usize, i64, i64), &OpenMr)> = Vec::new();
    // An MR of no orchestration is never touched: no place, no start, no cancel (#369).
    let members = mrs.iter().filter(|mr| ranks.is_member(mr.iid));
    for mr in members {
        if !wants_pipeline(mr, ranks, started) {
            let is_live_here = mr.is_head_current() && mr.pipeline_status().is_some_and(is_live);
            if let Some(why) = stopped_why(mr, ranks, started).filter(|_| !is_live_here) {
                places.insert(mr.iid, Place::Stopped { why });
            }
            continue;
        }
        match class_of(mr, ranks) {
            Some(class) => waiting.push((class, key_of(class, mr, ranks), mr)),
            None => {
                let place = match ranks.held.get(&mr.iid) {
                    Some(why) => Place::Held { why: why.clone() },
                    None => Place::Boxes(ranks.bars.get(&mr.iid).cloned().unwrap_or_default()),
                };
                places.insert(mr.iid, place);
            }
        }
    }
    fn key_of(class: Class, mr: &OpenMr, ranks: &Ranks) -> (usize, i64, i64) {
        match class {
            // The latest click first.
            Class::RunNow => (0, -ranks.run_now.get(&mr.iid).copied().unwrap_or(0), mr.iid),
            // One preempted gets its slot back first (#419), then the merge order.
            Class::Ready | Class::Fix if ranks.was_preempted(mr) => (0, 0, mr.iid),
            Class::Ready | Class::Fix => (ranks.place_in_order(mr.iid).map_or(usize::MAX, |at| at + 1), 0, mr.iid),
            _ => (0, 0, mr.iid),
        }
    }
    // How an MR ranks, lower first; one whose bar isn't full after every class.
    let rank_of = |mr: &OpenMr| -> (Option<Class>, (usize, i64, i64)) {
        match class_of(mr, ranks) {
            Some(class) => (Some(class), key_of(class, mr, ranks)),
            None => (None, (0, 0, mr.iid)),
        }
    };
    let worse = |a: &(Option<Class>, (usize, i64, i64)), b: &(Option<Class>, (usize, i64, i64))| match (a.0, b.0) {
        (None, Some(_)) => true,
        (Some(_), None) => false,
        _ => (a.0, a.1) > (b.0, b.1),
    };
    waiting.sort_by_key(|a| (a.0, a.1));
    for (at, (_, _, mr)) in waiting.iter().enumerate() {
        let place = match ranks.preempts.get(&mr.iid).filter(|_| ranks.was_preempted(mr)) {
            Some(preemption) => Place::Preempted { by: preemption.by },
            None => Place::Next { at: at + 1 },
        };
        places.insert(mr.iid, place);
    }
    let is_full = is_runner_full || slots.free() == 0;
    // The runner full and a click or a hotfix first (#419: a mere rank in the
    // merge order waits): the live pipeline ranked lowest behind it makes
    // room, one a look, only one the queue created itself for an
    // orchestration's MR, never one into `main` or of a branch, nor one
    // that runs past PREEMPT_GRACE_MS or of an MR preempted within PREEMPT_AGAIN_MS.
    let is_urgent = |class: &Class| matches!(class, Class::RunNow | Class::Hotfix);
    let preempt = waiting
        .first()
        .filter(|(class, _, _)| is_full && is_urgent(class))
        .and_then(|(class, key, first)| {
            let first_rank = (Some(*class), *key);
            mrs.iter()
                .filter(|mr| mr.iid != first.iid && !mr.is_into_main() && ranks.is_member(mr.iid))
                .filter(|mr| {
                    ranks
                        .preempts
                        .get(&mr.iid)
                        .is_none_or(|preemption| ranks.now - preemption.at >= PREEMPT_AGAIN_MS)
                })
                .filter_map(|mr| {
                    let pipeline = mr.pipeline.as_ref()?;
                    (is_live(&pipeline.status) && is_ours(pipeline, ranks) && is_young(pipeline, ranks.now))
                        .then(|| (rank_of(mr), mr, pipeline))
                })
                .filter(|(rank, _, _)| worse(rank, &first_rank))
                .reduce(|a, b| if worse(&b.0, &a.0) { b } else { a })
                .map(|(_, mr, pipeline)| Preempt {
                    iid: mr.iid,
                    pipeline: pipeline.id,
                    sha: pipeline.sha.clone().unwrap_or_default(),
                    by: first.iid,
                })
        });
    let start = waiting.first().filter(|_| !is_full).map(|(_, _, mr)| {
        let cancel = mr
            .pipeline
            .as_ref()
            .filter(|pipeline| is_live(&pipeline.status) && !mr.is_head_current() && !mr.is_into_main())
            .filter(|pipeline| is_ours(pipeline, ranks))
            .map(|pipeline| pipeline.id);
        Start {
            iid: mr.iid,
            sha: mr.sha.clone().unwrap_or_default(),
            cancel,
            is_again: ranks.run_now.contains_key(&mr.iid),
            after: mr.blocked_by.filter(|blocking| !ranks.ours.contains(blocking)),
        }
    });
    Decision { places, start, preempt }
}

/// A pipeline the queue may cancel: one it created (an MR pipeline, never a
/// branch's or a tag's), not one a person or a push started.
fn is_ours(pipeline: &Pipeline, ranks: &Ranks) -> bool {
    ranks.ours.contains(&pipeline.id)
        && pipeline
            .reference
            .as_deref()
            .is_some_and(|reference| reference.starts_with("refs/merge-requests/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;

    fn pipeline(id: i64, status: &str, sha: &str, iid: i64) -> Option<Pipeline> {
        Some(Pipeline {
            id,
            status: status.into(),
            sha: Some(sha.into()),
            reference: Some(format!("refs/merge-requests/{iid}/head")),
            ..Default::default()
        })
    }
    fn mr(iid: i64, sha: &str, pipeline: Option<Pipeline>) -> OpenMr {
        OpenMr {
            iid,
            source: format!("feature/{iid}-x"),
            target: "develop".into(),
            sha: Some(sha.into()),
            pipeline,
            ..Default::default()
        }
    }
    fn draft(iid: i64, sha: &str, pipeline: Option<Pipeline>) -> OpenMr {
        OpenMr {
            is_draft: true,
            ..mr(iid, sha, pipeline)
        }
    }
    /// An orchestration's order, each MR's bar full.
    fn order(iids: &[i64]) -> Ranks {
        let full = Bar {
            ticked: 1,
            total: 1,
            is_checked: true,
            ..Default::default()
        };
        Ranks {
            orders: vec![iids.to_vec()],
            bars: iids.iter().map(|&iid| (iid, full.clone())).collect(),
            ..Default::default()
        }
    }
    fn slots(concurrent: u32, running: u32) -> Slots {
        Slots {
            concurrent,
            running,
            is_known: true,
        }
    }
    fn decide_now(mrs: &[OpenMr], ranks: &Ranks, slots: Slots) -> Decision {
        decide(mrs, ranks, slots, &HashMap::new())
    }

    #[test]
    fn starts_the_mr_next_in_the_merge_order_before_those_behind_it() {
        // #357's evening: !339 first in the order, !347 and !342 far back, all without a pipeline of their head.
        let mrs = [mr(347, "c", None), mr(342, "b", None), mr(339, "a", None)];
        let decision = decide_now(&mrs, &order(&[339, 342, 347]), slots(2, 0));
        assert_eq!(decision.start.as_ref().map(|start| start.iid), Some(339));
        assert_eq!(decision.places[&339], Place::Next { at: 1 });
        assert_eq!(decision.places[&342], Place::Next { at: 2 });
        assert_eq!(decision.places[&342].text(), "CI: next (2nd)");
        assert_eq!(decision.places[&347].text(), "CI: next (3rd)");
    }

    #[test]
    fn holds_every_start_while_the_runner_is_full_or_a_pipeline_waits() {
        let mrs = [mr(339, "a", None), mr(342, "b", pipeline(9, "pending", "b", 342))];
        let ranks = order(&[339, 342]);
        // A pipeline waits for the runner: it is full, whatever its config says.
        assert_eq!(decide_now(&mrs, &ranks, slots(4, 1)).start, None);
        let mrs = [mr(339, "a", None), mr(342, "b", pipeline(9, "running", "b", 342))];
        assert_eq!(decide_now(&mrs, &ranks, slots(2, 2)).start, None);
        assert_eq!(
            decide_now(&mrs, &ranks, slots(3, 2)).start.map(|start| start.iid),
            Some(339)
        );
        // The one running keeps its place on its row.
        assert_eq!(decide_now(&mrs, &ranks, slots(2, 2)).places[&342], Place::Running);
    }

    #[test]
    fn releases_an_mr_that_moves_up_in_the_order() {
        let mrs = [mr(339, "a", None), mr(342, "b", None)];
        assert_eq!(
            decide_now(&mrs, &order(&[339, 342]), slots(1, 0)).start.unwrap().iid,
            339
        );
        // 342 moved up past 339 (339 got conflicts): its pipeline goes first.
        assert_eq!(
            decide_now(&mrs, &order(&[342, 339]), slots(1, 0)).start.unwrap().iid,
            342
        );
    }

    #[test]
    fn ranks_run_now_then_hotfix_then_ready_then_fix() {
        let mut hotfix = mr(5, "h", None);
        hotfix.target = "main".into();
        let mrs = [
            draft(2, "f", pipeline(20, "failed", "old", 2)),
            draft(3, "t", None),
            mr(4, "r", None),
            hotfix,
            draft(6, "n", None),
        ];
        let mut ranks = order(&[4, 2, 5, 6, 3]);
        // An orchestration's draft whose boxes are open: no pipeline yet.
        ranks.bars.insert(
            3,
            Bar {
                ticked: 2,
                total: 5,
                is_checked: true,
                ..Default::default()
            },
        );
        ranks.run_now.insert(6, NOW - 1000);
        let decision = decide_now(&mrs, &ranks, slots(1, 0));
        let at = |iid: i64| match decision.places[&iid] {
            Place::Next { at } => at,
            _ => usize::MAX,
        };
        assert_eq!([at(6), at(5), at(4), at(2)], [1, 2, 3, 4]);
        assert_eq!(decision.places[&3].text(), "CI: after the boxes (2/5)");
        assert_eq!(decision.start.unwrap().iid, 6);
    }

    #[test]
    fn starts_no_pipeline_for_an_mr_under_a_hard_wait_until_its_prerequisite_is_merged() {
        // #392: !12 needs !11 (or waits for it): its bar is full, still no pipeline.
        let mrs = [mr(11, "a", None), mr(12, "b", None)];
        let mut ranks = order(&[11, 12]);
        ranks.held.insert(12, "after !11".into());
        let decision = decide_now(&mrs, &ranks, slots(2, 0));
        assert_eq!(decision.start.as_ref().unwrap().iid, 11);
        assert_eq!(decision.places[&12].text(), "CI: held, after !11");
        assert!(decision.places[&12].is_waiting());
        // Merged: nothing holds it any more.
        ranks.held.clear();
        let decision = decide_now(&[mr(12, "b", None)], &ranks, slots(2, 0));
        assert_eq!(decision.start.unwrap().iid, 12);
        // A click starts one anyway.
        let mut ranks = order(&[11, 12]);
        ranks.held.insert(12, "after !11".into());
        ranks.run_now.insert(12, NOW - 1000);
        let decision = decide_now(&[mr(12, "b", None)], &ranks, slots(2, 0));
        assert_eq!(decision.start.unwrap().iid, 12);
    }

    #[test]
    fn an_mr_that_only_shares_files_with_one_ahead_gets_its_pipeline_ranked_behind_it() {
        // Shared files are no entry in `held`: both start, the one ahead first.
        let mrs = [mr(12, "b", None), mr(11, "a", None)];
        let decision = decide_now(&mrs, &order(&[11, 12]), slots(1, 0));
        assert_eq!(decision.start.as_ref().unwrap().iid, 11);
        assert!(
            matches!(decision.places[&12], Place::Next { at: 2 }),
            "{:?}",
            decision.places[&12]
        );
    }

    #[test]
    fn never_touches_an_mr_of_no_orchestration() {
        // #369: vita's !184 (ready), !186 (a draft), a colleague's hotfix: none of the orders.
        let mut hotfix = mr(9, "h", pipeline(90, "running", "old", 9));
        hotfix.target = "main".into();
        let mrs = [
            mr(184, "a", None),
            draft(186, "b", pipeline(86, "running", "old", 186)),
            hotfix,
            mr(1, "c", None),
        ];
        let mut ranks = order(&[1]);
        ranks.ours.extend([86, 90]);
        // A click on its row in the view of everything doesn't make it one either.
        ranks.run_now.insert(184, NOW);
        let decision = decide_now(&mrs, &ranks, slots(4, 2));
        assert_eq!(decision.places.keys().copied().collect::<Vec<_>>(), vec![1]);
        assert_eq!(decision.start.map(|start| start.iid), Some(1));
        // The runner full: still nothing of theirs cancelled.
        assert_eq!(decide_now(&mrs, &ranks, slots(2, 2)).preempt, None);
        assert_eq!(decide_now(&mrs[..3], &ranks, slots(4, 0)).start, None);
    }

    #[test]
    fn starts_no_pipeline_before_the_bar_is_full_but_on_a_click() {
        // #357, 2026-10-11: !349 (0/11) and !351 (0/8) took the runner while !341 (7/7) waited.
        let mrs = [draft(349, "a", None), draft(351, "b", None), draft(341, "c", None)];
        let mut ranks = order(&[349, 351, 341]);
        let bar = |ticked, total, is_checked| Bar {
            ticked,
            total,
            is_checked,
            ..Default::default()
        };
        ranks.bars.insert(349, bar(0, 11, true));
        ranks.bars.insert(351, bar(8, 8, false));
        ranks.bars.insert(341, bar(7, 7, true));
        let decision = decide_now(&mrs, &ranks, slots(4, 0));
        assert_eq!(decision.start.as_ref().map(|start| start.iid), Some(341));
        assert_eq!(decision.places[&341], Place::Next { at: 1 });
        assert_eq!(decision.places[&349].text(), "CI: after the boxes (0/11)");
        assert_eq!(decision.places[&351].text(), "CI: after wt:check");
        // With 341 running, the others still wait for their bars, whatever the free slots.
        let mrs = [draft(349, "a", None), draft(341, "c", pipeline(1, "running", "c", 341))];
        assert_eq!(decide_now(&mrs, &ranks, slots(4, 1)).start, None);
        // A click starts one early.
        ranks.run_now.insert(349, NOW);
        assert_eq!(decide_now(&mrs, &ranks, slots(4, 1)).start.unwrap().iid, 349);
    }

    /// A running pipeline of an MR, started `ago` ms before NOW.
    fn running(id: i64, sha: &str, iid: i64, ago: i64) -> Option<Pipeline> {
        let mut pipeline = pipeline(id, "running", sha, iid)?;
        pipeline.started_at = Some(crate::time::to_iso(NOW - ago));
        Some(pipeline)
    }
    fn at_now(mut ranks: Ranks) -> Ranks {
        ranks.now = NOW;
        ranks
    }

    #[test]
    fn makes_room_for_a_click_by_cancelling_the_lowest_one_behind_it() {
        // The runner full with 342 and 347, both behind 339, which a click moved up: 347 goes, the last.
        let mrs = [
            mr(339, "a", None),
            mr(342, "b", running(42, "b", 342, 30_000)),
            mr(347, "c", running(47, "c", 347, 30_000)),
        ];
        let mut ranks = at_now(order(&[339, 342, 347]));
        ranks.run_now.insert(339, NOW - 1000);
        // Only a pipeline the queue created is cancelled: 47 one a push started, 342's goes.
        ranks.ours.insert(42);
        let preempt = |iid, pipeline: i64, sha: &str| Preempt {
            iid,
            pipeline,
            sha: sha.into(),
            by: 339,
        };
        assert_eq!(
            decide_now(&mrs, &ranks, slots(2, 2)).preempt,
            Some(preempt(342, 42, "b"))
        );
        ranks.ours.insert(47);
        let decision = decide_now(&mrs, &ranks, slots(2, 2));
        assert_eq!(decision.start, None);
        assert_eq!(decision.preempt, Some(preempt(347, 47, "c")));
        // A hotfix makes room too.
        let mut hotfix = mrs.clone();
        hotfix[0].source = "hotfix/339-x".into();
        ranks.run_now.clear();
        assert_eq!(
            decide_now(&hotfix, &ranks, slots(2, 2)).preempt,
            Some(preempt(347, 47, "c"))
        );
    }

    #[test]
    fn never_preempts_for_a_mere_rank_in_the_merge_order() {
        // #419, !381: cancelled twice for !371, !373, !375 as the order shifted.
        let mrs = [
            mr(371, "a", None),
            mr(381, "b", running(81, "b", 381, 30_000)),
            mr(382, "c", pipeline(82, "pending", "c", 382)),
        ];
        let mut ranks = at_now(order(&[371, 381, 382]));
        ranks.ours.extend([81, 82]);
        let decision = decide_now(&mrs, &ranks, slots(2, 2));
        assert_eq!((decision.start, decision.preempt), (None, None));
        assert_eq!(decision.places[&371], Place::Next { at: 1 });
        assert_eq!(decision.places[&381], Place::Running);
    }

    #[test]
    fn lets_a_pipeline_running_longer_than_two_minutes_finish() {
        let mrs = [
            mr(1, "a", None),
            mr(2, "b", running(20, "b", 2, PREEMPT_GRACE_MS + 1000)),
        ];
        let mut ranks = at_now(order(&[1, 2]));
        ranks.ours.insert(20);
        ranks.run_now.insert(1, NOW - 1000);
        assert_eq!(decide_now(&mrs, &ranks, slots(1, 1)).preempt, None);
        // One running since a time not known finishes too.
        let mut unknown = mrs.clone();
        unknown[1].pipeline.as_mut().unwrap().started_at = None;
        assert_eq!(decide_now(&unknown, &ranks, slots(1, 1)).preempt, None);
        // A young one makes room.
        let young = [mr(1, "a", None), mr(2, "b", running(20, "b", 2, 60_000))];
        assert_eq!(decide_now(&young, &ranks, slots(1, 1)).preempt.map(|p| p.iid), Some(2));
    }

    #[test]
    fn tells_a_pipeline_past_its_first_stage() {
        let job = |id, stage: &str, status: &str| (id, stage.to_string(), status.to_string());
        assert!(!is_past_first_stage(&[]));
        assert!(!is_past_first_stage(&[
            job(1, "check", "running"),
            job(2, "build", "created")
        ]));
        assert!(is_past_first_stage(&[
            job(1, "check", "success"),
            job(2, "build", "running")
        ]));
        assert!(is_past_first_stage(&[
            job(2, "build", "failed"),
            job(1, "check", "success")
        ]));
    }

    #[test]
    fn gives_a_preempted_mr_its_slot_back_first_and_never_preempts_it_twice_in_30_minutes() {
        let mut ranks = at_now(order(&[339, 342, 347]));
        ranks.preempted.insert(347, "c".into());
        ranks.preempts.insert(
            347,
            Preemption {
                by: 339,
                at: NOW - 60_000,
            },
        );
        ranks.ours.extend([39, 42, 47]);
        // A slot free again: 347 starts before 342, which ranks ahead of it in the order.
        let mrs = [
            mr(339, "a", running(39, "a", 339, 60_000)),
            mr(342, "b", None),
            mr(347, "c", pipeline(47, "canceled", "c", 347)),
        ];
        let decision = decide_now(&mrs, &ranks, slots(2, 1));
        assert_eq!(decision.start.map(|start| start.iid), Some(347));
        assert_eq!(decision.places[&347].text(), "CI: preempted by !339, restarts next");
        assert_eq!(decision.places[&342], Place::Next { at: 2 });
        // Running again, a click on 342: 347 is not preempted a second time within 30 min.
        ranks.preempted.clear();
        ranks.run_now.insert(342, NOW - 1000);
        let mrs = [
            mr(339, "a", running(39, "a", 339, PREEMPT_GRACE_MS)),
            mr(342, "b", None),
            mr(347, "c", running(48, "c", 347, 10_000)),
        ];
        ranks.ours.insert(48);
        assert_eq!(decide_now(&mrs, &ranks, slots(2, 2)).preempt, None);
        // 30 minutes on, it may be again.
        ranks.preempts.insert(
            347,
            Preemption {
                by: 339,
                at: NOW - PREEMPT_AGAIN_MS,
            },
        );
        assert_eq!(decide_now(&mrs, &ranks, slots(2, 2)).preempt.map(|p| p.iid), Some(347));
    }

    #[test]
    fn never_preempts_with_room_left() {
        let mrs = [mr(1, "a", None), mr(2, "b", pipeline(20, "running", "b", 2))];
        let mut ranks = order(&[1, 2]);
        ranks.ours.insert(20);
        let decision = decide_now(&mrs, &ranks, slots(3, 1));
        assert_eq!(
            (decision.start.map(|start| start.iid), decision.preempt),
            (Some(1), None)
        );
    }

    #[test]
    fn asks_once_for_a_commit_that_got_its_pipeline() {
        // Its pipeline came and passed: not again, however long ago.
        let mrs = [mr(1, "a", pipeline(10, "success", "a", 1))];
        let started = HashMap::from([(1, ("a".to_string(), NOW - 3_600_000))]);
        let mut ranks = order(&[1]);
        assert_eq!(decide(&mrs, &ranks, slots(2, 0), &started).start, None);
        // A click after it: again.
        ranks.run_now.insert(1, NOW - 1000);
        assert!(decide(&mrs, &ranks, slots(2, 0), &started).start.is_some());
    }

    #[test]
    fn starts_no_pipeline_for_a_commit_wt_check_did_not_pass_at() {
        let mrs = [draft(1, "pushed", None)];
        let mut ranks = order(&[1]);
        ranks.bars.get_mut(&1).unwrap().sha = Some("local".into());
        let decision = decide_now(&mrs, &ranks, slots(4, 0));
        assert_eq!(decision.start, None);
        assert_eq!(decision.places[&1].text(), "CI: after wt:check");
        ranks.bars.get_mut(&1).unwrap().sha = Some("pushed".into());
        assert_eq!(decide_now(&mrs, &ranks, slots(4, 0)).start.unwrap().iid, 1);
        // An MR in an order without a bar: nothing known, nothing started.
        ranks.bars.clear();
        assert_eq!(decide_now(&mrs, &ranks, slots(4, 0)).start, None);
    }

    #[test]
    fn starts_only_the_newest_commit_and_cancels_the_superseded_pipeline() {
        let mrs = [mr(337, "new", pipeline(70, "running", "old", 337))];
        let mut ranks = order(&[337]);
        // One the queue didn't create (a push's, a person's) is left running.
        assert_eq!(decide_now(&mrs, &ranks, slots(2, 1)).start.unwrap().cancel, None);
        ranks.ours.insert(70);
        let start = decide_now(&mrs, &ranks, slots(2, 1)).start.unwrap();
        assert_eq!(
            start,
            Start {
                iid: 337,
                sha: "new".into(),
                cancel: Some(70),
                is_again: false,
                after: None,
            }
        );
        // A finished one of an older commit is left alone, nothing to cancel.
        let mrs = [mr(337, "new", pipeline(70, "failed", "old", 337))];
        assert_eq!(
            decide_now(&mrs, &order(&[337]), slots(2, 0)).start.unwrap().cancel,
            None
        );
    }

    #[test]
    fn never_cancels_a_pipeline_into_main_or_of_a_branch() {
        let mut ranks = order(&[1, 2]);
        ranks.ours.extend([10, 11]);
        let mut release = mr(1, "new", pipeline(10, "running", "old", 1));
        release.target = "main".into();
        assert_eq!(decide_now(&[release], &ranks, slots(3, 1)).start.unwrap().cancel, None);
        let mut branch = mr(2, "new", pipeline(11, "running", "old", 2));
        branch.pipeline.as_mut().unwrap().reference = Some("develop".into());
        assert_eq!(decide_now(&[branch], &ranks, slots(3, 1)).start.unwrap().cancel, None);
    }

    #[test]
    fn leaves_a_current_pipeline_alone_until_a_click_asks_again() {
        let done = [mr(1, "a", pipeline(10, "failed", "a", 1))];
        let decision = decide_now(&done, &order(&[1]), slots(2, 0));
        assert_eq!(decision.start, None);
        assert!(decision.places.is_empty());
        let cancelled = [mr(1, "a", pipeline(10, "canceled", "a", 1))];
        assert_eq!(decide_now(&cancelled, &order(&[1]), slots(2, 0)).start, None);
        let mut ranks = order(&[1]);
        ranks.run_now.insert(1, NOW);
        assert_eq!(decide_now(&cancelled, &ranks, slots(2, 0)).start.unwrap().iid, 1);
        // A click on one that runs changes nothing.
        let running = [mr(1, "a", pipeline(10, "running", "a", 1))];
        assert_eq!(decide_now(&running, &ranks, slots(2, 1)).start, None);
    }

    #[test]
    fn starts_an_mr_whose_push_left_a_skipped_pipeline() {
        // #383: every push with ci.skip leaves a skipped pipeline of its commit (!353, !354): no answer.
        let mrs = [mr(353, "a", pipeline(10, "skipped", "a", 353))];
        let decision = decide_now(&mrs, &order(&[353]), slots(1, 0));
        assert_eq!(decision.start.map(|start| start.iid), Some(353));
        assert_eq!(decision.places[&353], Place::Next { at: 1 });
        // Its bar not full yet: the row says so, nothing starts.
        let mut ranks = order(&[353]);
        ranks.bars.get_mut(&353).unwrap().is_checked = false;
        let decision = decide_now(&mrs, &ranks, slots(1, 0));
        assert_eq!(decision.start, None);
        assert_eq!(decision.places[&353].text(), "CI: after wt:check");
    }

    #[test]
    fn a_pipeline_the_queue_cancelled_itself_is_no_persons_cancel() {
        // #383, !361: its pipeline cancelled by the queue (for an MR ahead, superseded) starts again.
        let mrs = [mr(361, "a", pipeline(24, "canceled", "a", 361))];
        let mut ranks = order(&[361]);
        let decision = decide_now(&mrs, &ranks, slots(1, 0));
        assert_eq!(decision.start, None);
        assert_eq!(decision.places[&361].text(), "CI: cancelled by hand, a click runs it");
        ranks.cancelled.insert(24);
        let decision = decide_now(&mrs, &ranks, slots(1, 0));
        assert_eq!(decision.start.map(|start| start.iid), Some(361));
        // Asked for once already at that commit: not again, and the row says why.
        let mut started = HashMap::new();
        started.insert(361, ("a".to_string(), NOW));
        let decision = decide(&mrs, &ranks, slots(1, 0), &started);
        assert_eq!(decision.start, None);
        assert_eq!(
            decision.places[&361].text(),
            "CI: asked for, GitLab made none, a click runs it"
        );
    }

    #[test]
    fn remembers_what_it_cancelled_over_a_restart() {
        let mut asked = Asked::default();
        asked.add_cancelled(24);
        asked.add_cancelled(24);
        let read: Asked = serde_json::from_str(&serde_json::to_string(&asked).unwrap()).unwrap();
        assert_eq!(read.cancelled, vec![24]);
        // An asked.json from before #383 reads without it.
        let old: Asked = serde_json::from_str(r#"{"started":{},"ours":[3]}"#).unwrap();
        assert!(old.cancelled.is_empty());
    }

    #[test]
    fn never_asks_twice_for_the_same_commit() {
        let mrs = [mr(1, "a", None)];
        let mut started = HashMap::new();
        started.insert(1, ("a".to_string(), NOW - 10_000));
        assert_eq!(decide(&mrs, &order(&[1]), slots(2, 0), &started).start, None);
        // An hour on, GitLab still shows none (it refused it, #369): not asked again.
        started.insert(1, ("a".to_string(), NOW - 3_600_000));
        assert_eq!(decide(&mrs, &order(&[1]), slots(2, 0), &started).start, None);
        // Failed at its creation (no jobs): not again either.
        let failed = [mr(1, "a", pipeline(10, "failed", "a", 1))];
        assert_eq!(decide(&failed, &order(&[1]), slots(2, 0), &started).start, None);
        // A newer commit: asked at once.
        started.insert(1, ("old".to_string(), NOW - 10_000));
        assert!(decide(&mrs, &order(&[1]), slots(2, 0), &started).start.is_some());
    }

    #[test]
    fn replaces_a_cancelled_or_refused_pipeline_that_blocks_a_passed_commit_once() {
        // #369: duplicates cancelled by hand, the newest of a commit that
        // passed is a cancelled one, GitLab's merge waits on it.
        let mut blocked = mr(1, "a", pipeline(30, "canceled", "a", 1));
        blocked.blocked_by = Some(30);
        let mut ranks = order(&[1]);
        // Asked for that commit before: still once more, over the blocking one.
        let started = HashMap::from([(1, ("a".to_string(), NOW - 1000))]);
        let start = decide(&[blocked.clone()], &ranks, slots(2, 0), &started).start.unwrap();
        assert_eq!((start.iid, start.after), (1, Some(30)));
        ranks.rescued.insert(1, 30);
        assert_eq!(decide(&[blocked.clone()], &ranks, slots(2, 0), &started).start, None);
        // Its own pipeline refused (the activity limit): not again by itself.
        ranks.rescued.clear();
        ranks.ours.insert(30);
        assert_eq!(decide(&[blocked], &ranks, slots(2, 0), &started).start, None);
    }

    #[test]
    fn reads_the_runner_as_its_autoscaler_does() {
        let config = "listen_address = \"x\"\nconcurrent = 4\ncheck_interval = 0\n[[runners]]\n  limit = 0\n";
        assert_eq!(concurrent_of(config), Some(4));
        assert_eq!(concurrent_of("[[runners]]\n"), None);
        // Helper and build container of one job count once.
        assert_eq!(running_of("812\n812\n813\n\n"), 2);
    }

    #[test]
    fn makes_a_worktree_push_skip_ci_only_while_a_queue_runs() {
        use crate::git::fixture::run;
        let scratch = std::env::temp_dir().join(format!("ci-push-test-{}", std::process::id()));
        let main = scratch.join("main");
        std::fs::create_dir_all(&main).unwrap();
        run(&main, &["init", "-q", "-b", "develop"]);
        run(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let tree = scratch.join("tree");
        run(
            &main,
            &["worktree", "add", "-q", "-b", "feature/1-x", tree.to_str().unwrap()],
        );
        let beat = |at: i64| {
            write_whole(
                &queue_file(&main),
                &Queue {
                    at,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        beat(NOW);
        // A local origin takes no push options: left alone.
        let origin = scratch.join("origin.git");
        run(&main, &["remote", "add", "origin", origin.to_str().unwrap()]);
        assert_eq!(set_push_option(&tree, NOW), None);
        run(
            &main,
            &["remote", "set-url", "origin", "git@gitlab.com:monocircuit/platform.git"],
        );
        assert_eq!(set_push_option(&main, NOW), None);
        assert_eq!(set_push_option(&tree, NOW), Some(PushOption::Set));
        let options = |dir: &Path| crate::git::git(dir, &["config", "--get-all", "push.pushOption"]).stdout;
        assert_eq!(options(&tree).trim(), SKIP_OPTION);
        assert_eq!(options(&main).trim(), "");
        // Once is enough.
        assert_eq!(set_push_option(&tree, NOW), None);
        // The queue's heartbeat stale: pushes start their pipelines again.
        assert_eq!(
            set_push_option(&tree, NOW + QUEUE_STALE_MS),
            Some(PushOption::Dropped("no breadboard serve runs the CI queue".into()))
        );
        assert_eq!(options(&tree).trim(), "");
        // A try's dry queue starts nothing: no heartbeat for the pushes.
        write_whole(
            &queue_file(&main),
            &Queue {
                at: NOW,
                is_dry: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(set_push_option(&tree, NOW), None);
        beat(NOW);
        assert_eq!(set_push_option(&tree, NOW), Some(PushOption::Set));
        run(&main, &["config", MODE_KEY, "push"]);
        assert_eq!(
            set_push_option(&tree, NOW),
            Some(PushOption::Dropped("breadboard.ci = push".into()))
        );
        assert_eq!(options(&tree).trim(), "");
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn tells_a_network_remote_from_a_local_one() {
        assert!(is_network_url("git@gitlab.com:monocircuit/platform.git"));
        assert!(is_network_url("https://gitlab.example.org/p.git"));
        assert!(is_network_url("ssh://git@host/p.git"));
        assert!(!is_network_url("/tmp/world/origin.git"));
        assert!(!is_network_url("file:///tmp/origin.git"));
        assert!(!is_network_url("../origin.git"));
    }

    #[test]
    fn says_the_load_and_ordinals() {
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(12), "12th");
        assert_eq!(ordinal(23), "23rd");
        let mut queue = Queue {
            slots: slots(4, 3),
            ..Default::default()
        };
        assert_eq!(queue.load_text(), "CI 3/4 slots");
        queue.places.insert(1, Place::Next { at: 1 });
        queue.places.insert(2, Place::Running);
        assert_eq!(queue.load_text(), "CI 3/4 slots · 1 queued");
        queue.is_dry = true;
        assert_eq!(queue.load_text(), "CI 3/4 slots · 1 queued (dry)");
    }

    #[test]
    fn reads_the_published_orders_and_keeps_files_whole() {
        let scratch = std::env::temp_dir().join(format!("ci-queue-test-{}", std::process::id()));
        let published = Published {
            order: vec![3, 1],
            bars: BTreeMap::from([(
                3,
                Bar {
                    ticked: 1,
                    total: 2,
                    is_checked: true,
                    sha: Some("abc".into()),
                    ..Default::default()
                },
            )]),
            held: BTreeMap::from([(1, "after !3".to_string())]),
        };
        write_whole(&order_file(&scratch, "breadboard"), &published).unwrap();
        write_whole(&order_file(&scratch, "vita"), &Published::default()).unwrap();
        let read = published_of(&scratch);
        assert_eq!(read, vec![published, Published::default()]);
        let queue = Queue {
            at: NOW,
            ..Default::default()
        };
        write_whole(&queue_file(&scratch), &queue).unwrap();
        assert_eq!(Queue::read(&scratch, NOW + 1000), Some(queue));
        assert_eq!(Queue::read(&scratch, NOW + QUEUE_STALE_MS), None);
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
