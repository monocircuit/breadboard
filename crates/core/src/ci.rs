//! CI in the order that matters (#357). GitLab has no job priorities: its
//! runner takes jobs first come, first served. So breadboard decides which
//! pipelines exist at all: worktree pushes skip CI (`ci.skip`, `SKIP_OPTION`),
//! and `breadboard serve` creates an MR's pipeline when its turn comes,
//! highest class first, while the runner has free slots.
//!
//! The files of `<main>/.claude/ci/`, each written whole by one side:
//! - `order-<orchestration>.json` (`Published`): an orchestration's merge
//!   order and each MR's bar (its boxes, its wt:check), by its acting overview;
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
/// How long a pipeline asked for counts as on its way: GitLab's head pipeline may lag.
pub const STARTED_HOLDS_MS: i64 = 5 * 60_000;
/// How old a queue may be before the rows stop showing it: no scheduler runs.
pub const QUEUE_STALE_MS: i64 = 60_000;
/// Runner slots assumed when its config can't be read.
pub const SLOTS_UNKNOWN: u32 = 2;

/// A pipeline still to finish.
fn is_live(status: &str) -> bool {
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
fn is_network_url(url: &str) -> bool {
    if url.starts_with("file://") || url.starts_with('/') || url.starts_with('.') {
        return false;
    }
    url.contains("://")
        || url
            .split_once(':')
            .is_some_and(|(host, _)| host.contains('@') || host.contains('.'))
}

/// What a worktree's pushes carry, set so (#357): `ci.skip` in its own
/// config (`git config --worktree push.pushOption`), the queue starts its
/// pipelines; none with `git config breadboard.ci push`. The main checkout
/// is left alone: develop's pipelines deploy. Says what changed, None when nothing did.
pub fn set_push_option(dir: &Path) -> Option<String> {
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
    let is_queued = is_network && git(&["config", "--get", MODE_KEY]).out() != "push";
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
        return Some("pushes start their pipelines (breadboard.ci = push)".into());
    }
    // A config of the worktree's own needs git's extension, once for the clone.
    if git(&["config", "--get", "extensions.worktreeConfig"]).out() != "true" {
        git(&["config", "extensions.worktreeConfig", "true"]);
    }
    let set = git(&["config", "--worktree", "--add", "push.pushOption", SKIP_OPTION]);
    Some(if set.is_ok {
        "pushes skip CI, breadboard's queue starts the pipelines (#357)".into()
    } else {
        format!("! pushes still start their pipelines: {}", set.stderr.trim())
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

/// Where an MR's work stands (#357, the user's rule: no pipeline before the
/// bar is full): its issue's boxes, those after the deploy left out, and
/// whether `wt:check` passed at its HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Bar {
    pub ticked: usize,
    pub total: usize,
    pub is_checked: bool,
}

impl Bar {
    /// Every box ticked and checked: its pipeline may start.
    pub fn is_full(&self) -> bool {
        self.ticked >= self.total && self.is_checked
    }
}

/// What an orchestration's acting overview publishes on each read.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Published {
    /// The open MRs (iids) in the merge order.
    pub order: Vec<i64>,
    /// Each open MR's bar, by iid.
    pub bars: BTreeMap<i64, Bar>,
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
    /// A draft at work: it starts once the slots are idle; `after` the MR ahead.
    Held { after: Option<i64> },
    /// Its bar isn't full: no pipeline before it is (a click starts one anyway).
    Boxes(Bar),
}

impl Place {
    /// Waiting for its pipeline: next or held.
    pub fn is_waiting(&self) -> bool {
        matches!(self, Place::Next { .. } | Place::Held { .. } | Place::Boxes(_))
    }

    /// `CI: running`, `CI: next (2nd)`, `CI: held, after !339`.
    pub fn text(&self) -> String {
        match self {
            Place::Running => "CI: running".into(),
            Place::Pending => "CI: pending".into(),
            Place::Next { at: 1 } => "CI: next".into(),
            Place::Next { at } => format!("CI: next ({})", ordinal(*at)),
            Place::Held { after: Some(iid) } => format!("CI: held, after !{iid}"),
            Place::Held { after: None } => "CI: held till a slot is idle".into(),
            Place::Boxes(bar) if bar.ticked < bar.total => {
                format!("CI: after the boxes ({}/{})", bar.ticked, bar.total)
            }
            Place::Boxes(_) => "CI: after wt:check".into(),
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
            .filter(|place| matches!(place, Place::Next { .. } | Place::Held { .. }))
            .count();
        let slots = if self.slots.is_known {
            format!("CI {}/{} slots", self.slots.running, self.slots.concurrent)
        } else {
            format!("CI {} running", self.slots.running)
        };
        if waiting == 0 {
            slots
        } else {
            format!("{slots} · {waiting} queued")
        }
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
    /// A draft at work: only when the slots are idle.
    Idle,
}

/// What the ranking knows besides the MRs.
#[derive(Debug, Clone, Default)]
pub struct Ranks {
    /// Each orchestration's merge order.
    pub orders: Vec<Vec<i64>>,
    /// The bars of the orchestrations' MRs; an MR of none has no bar to wait for.
    pub bars: HashMap<i64, Bar>,
    pub run_now: BTreeMap<i64, i64>,
}

impl Ranks {
    pub fn of(published: &[Published], run_now: &RunNow) -> Ranks {
        Ranks {
            orders: published.iter().map(|one| one.order.clone()).collect(),
            bars: published.iter().flat_map(|one| one.bars.clone()).collect(),
            run_now: run_now.at.clone(),
        }
    }

    /// Where `iid` stands in the merge orders: the same place in two orchestrations ranks the same.
    fn place_in_order(&self, iid: i64) -> Option<usize> {
        self.orders
            .iter()
            .filter_map(|order| order.iter().position(|&one| one == iid))
            .min()
    }
}

/// Its class; None while its bar isn't full (no click asked for it).
pub fn class_of(mr: &OpenMr, ranks: &Ranks) -> Option<Class> {
    if ranks.run_now.contains_key(&mr.iid) {
        return Some(Class::RunNow);
    }
    let bar = ranks.bars.get(&mr.iid);
    if bar.is_some_and(|bar| !bar.is_full()) {
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
        } else if bar.is_some() || !mr.is_draft {
            // An orchestration's MR with its bar full is marked ready only after its green pipeline.
            Class::Ready
        } else {
            Class::Idle
        },
    )
}

/// A pipeline to create, and the superseded one of the same MR to cancel first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub iid: i64,
    pub sha: String,
    pub cancel: Option<i64>,
}

/// What the scheduler does now and what the rows say.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decision {
    pub places: BTreeMap<i64, Place>,
    pub start: Option<Start>,
}

/// Whether `mr` waits for a pipeline of its newest commit: none yet, or a
/// click asks for one again. A pipeline of that commit that a person
/// cancelled or GitLab skipped stays so, without a click.
fn wants_pipeline(mr: &OpenMr, ranks: &Ranks, started: &HashMap<i64, (String, i64)>, now: i64) -> bool {
    let Some(sha) = &mr.sha else {
        return false;
    };
    // Asked for already and GitLab doesn't show it yet: on its way.
    if let Some((asked, at)) = started.get(&mr.iid)
        && asked == sha
        && now - at < STARTED_HOLDS_MS
    {
        let is_clicked_since = ranks.run_now.get(&mr.iid).is_some_and(|clicked| clicked > at);
        if !is_clicked_since {
            return false;
        }
    }
    if mr.is_head_current() {
        return ranks.run_now.contains_key(&mr.iid) && !mr.pipeline_status().is_some_and(is_live);
    }
    true
}

/// The queue now: each MR's place, and the one pipeline to start, if any.
///
/// - one start per call, and only while the runner has a free slot and no
///   pipeline waits for it (a waiting one says the runner is full);
/// - always for the newest commit; its MR's older live pipeline is cancelled
///   first, never one into `main`;
/// - a draft at work (`Class::Idle`) only while no other pipeline runs.
///
/// `started`: what was asked for, iid → (sha, when), so a lagging read doesn't start it twice.
pub fn decide(
    mrs: &[OpenMr],
    ranks: &Ranks,
    slots: Slots,
    started: &HashMap<i64, (String, i64)>,
    now: i64,
) -> Decision {
    let mut places = BTreeMap::new();
    let mut is_runner_full = false;
    let mut busy = 0usize;
    for mr in mrs {
        let Some(status) = mr.pipeline_status().filter(|status| is_live(status)) else {
            continue;
        };
        busy += 1;
        is_runner_full |= is_waiting(status);
        if mr.is_head_current() {
            let place = if is_waiting(status) {
                Place::Pending
            } else {
                Place::Running
            };
            places.insert(mr.iid, place);
        }
    }
    let mut waiting: Vec<(Class, (usize, i64, i64), &OpenMr)> = Vec::new();
    for mr in mrs.iter().filter(|mr| wants_pipeline(mr, ranks, started, now)) {
        match class_of(mr, ranks) {
            Some(class) => waiting.push((class, key_of(class, mr, ranks), mr)),
            None => {
                places.insert(mr.iid, Place::Boxes(ranks.bars[&mr.iid]));
            }
        }
    }
    fn key_of(class: Class, mr: &OpenMr, ranks: &Ranks) -> (usize, i64, i64) {
        {
            match class {
                // The latest click first.
                Class::RunNow => (0, -ranks.run_now.get(&mr.iid).copied().unwrap_or(0), mr.iid),
                Class::Ready | Class::Fix => (ranks.place_in_order(mr.iid).unwrap_or(usize::MAX), 0, mr.iid),
                _ => (0, 0, mr.iid),
            }
        }
    }
    waiting.sort_by_key(|a| (a.0, a.1));
    let mut ahead: Option<i64> = None;
    for (at, (class, _, mr)) in waiting.iter().enumerate() {
        let place = if *class == Class::Idle {
            Place::Held { after: ahead }
        } else {
            Place::Next { at: at + 1 }
        };
        places.insert(mr.iid, place);
        ahead = Some(mr.iid);
    }
    let start = waiting.first().and_then(|(class, _, mr)| {
        if is_runner_full || slots.free() == 0 {
            return None;
        }
        // A draft at work waits for slots no one else uses: its own superseded pipeline aside.
        let others = busy - usize::from(mr.pipeline_status().is_some_and(is_live));
        if *class == Class::Idle && others > 0 {
            return None;
        }
        let cancel = mr
            .pipeline
            .as_ref()
            .filter(|pipeline| is_live(&pipeline.status) && !mr.is_head_current() && !mr.is_into_main())
            .filter(|pipeline| is_mr_ref(pipeline.reference.as_deref()))
            .map(|pipeline| pipeline.id);
        Some(Start {
            iid: mr.iid,
            sha: mr.sha.clone().unwrap_or_default(),
            cancel,
        })
    });
    Decision { places, start }
}

/// An MR pipeline's ref (`refs/merge-requests/12/head`); a branch or tag pipeline is never ours to cancel.
fn is_mr_ref(reference: Option<&str>) -> bool {
    reference.is_some_and(|reference| reference.starts_with("refs/merge-requests/"))
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
    fn order(iids: &[i64]) -> Ranks {
        Ranks {
            orders: vec![iids.to_vec()],
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
        decide(mrs, ranks, slots, &HashMap::new(), NOW)
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
    fn ranks_run_now_then_hotfix_then_ready_then_fix_then_idle() {
        let mut hotfix = mr(5, "h", None);
        hotfix.target = "main".into();
        let mrs = [
            draft(1, "i", None),
            draft(2, "f", pipeline(20, "failed", "old", 2)),
            draft(3, "t", None),
            mr(4, "r", None),
            hotfix,
            draft(6, "n", None),
        ];
        let mut ranks = order(&[4]);
        // An orchestration's draft whose boxes are open: no pipeline yet.
        ranks.bars.insert(
            3,
            Bar {
                ticked: 2,
                total: 5,
                is_checked: true,
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
        assert_eq!(decision.places[&1], Place::Held { after: Some(2) });
        assert_eq!(decision.places[&1].text(), "CI: held, after !2");
        assert_eq!(decision.start.unwrap().iid, 6);
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

    #[test]
    fn starts_a_draft_at_work_only_while_no_other_pipeline_runs() {
        let mrs = [draft(1, "i", None), mr(2, "r", pipeline(20, "running", "r", 2))];
        assert_eq!(decide_now(&mrs, &order(&[2]), slots(4, 1)).start, None);
        let mrs = [draft(1, "i", None), mr(2, "r", pipeline(20, "success", "r", 2))];
        assert_eq!(decide_now(&mrs, &order(&[2]), slots(4, 0)).start.unwrap().iid, 1);
    }

    #[test]
    fn starts_only_the_newest_commit_and_cancels_the_superseded_pipeline() {
        let mrs = [mr(337, "new", pipeline(70, "running", "old", 337))];
        let start = decide_now(&mrs, &order(&[337]), slots(2, 1)).start.unwrap();
        assert_eq!(
            start,
            Start {
                iid: 337,
                sha: "new".into(),
                cancel: Some(70)
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
        let mut release = mr(1, "new", pipeline(10, "running", "old", 1));
        release.target = "main".into();
        assert_eq!(
            decide_now(&[release], &order(&[]), slots(3, 1)).start.unwrap().cancel,
            None
        );
        let mut branch = mr(2, "new", pipeline(11, "running", "old", 2));
        branch.pipeline.as_mut().unwrap().reference = Some("develop".into());
        assert_eq!(
            decide_now(&[branch], &order(&[2]), slots(3, 1)).start.unwrap().cancel,
            None
        );
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
    fn never_asks_twice_for_the_same_commit_while_gitlab_lags() {
        let mrs = [mr(1, "a", None)];
        let mut started = HashMap::new();
        started.insert(1, ("a".to_string(), NOW - 10_000));
        assert_eq!(decide(&mrs, &order(&[1]), slots(2, 0), &started, NOW).start, None);
        // Long enough ago: GitLab took none, ask again.
        started.insert(1, ("a".to_string(), NOW - STARTED_HOLDS_MS));
        assert!(decide(&mrs, &order(&[1]), slots(2, 0), &started, NOW).start.is_some());
        // A newer commit: asked at once.
        started.insert(1, ("old".to_string(), NOW - 10_000));
        assert!(decide(&mrs, &order(&[1]), slots(2, 0), &started, NOW).start.is_some());
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
    fn makes_a_worktree_push_skip_ci_and_leaves_the_main_checkout_alone() {
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
        // A local origin takes no push options: left alone.
        let origin = scratch.join("origin.git");
        run(&main, &["remote", "add", "origin", origin.to_str().unwrap()]);
        assert_eq!(set_push_option(&tree), None);
        run(
            &main,
            &["remote", "set-url", "origin", "git@gitlab.com:monocircuit/platform.git"],
        );
        assert_eq!(set_push_option(&main), None);
        assert!(set_push_option(&tree).is_some());
        let options = |dir: &Path| crate::git::git(dir, &["config", "--get-all", "push.pushOption"]).stdout;
        assert_eq!(options(&tree).trim(), SKIP_OPTION);
        assert_eq!(options(&main).trim(), "");
        // Once is enough.
        assert_eq!(set_push_option(&tree), None);
        run(&main, &["config", MODE_KEY, "push"]);
        assert!(set_push_option(&tree).is_some());
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
                },
            )]),
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
