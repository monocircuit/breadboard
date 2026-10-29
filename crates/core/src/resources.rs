//! What the node's processes take (#513): CPU and RAM per process, each
//! one attributed to the worktree that caused it, and the machine's totals.
//! One night 264 vitest workers of parallel wt:checks filled 29 of 31 GB
//! and nobody could see which worktree they were of.
//!
//! A sample is cheap: `/proc` on Linux (a few files per process), `ps`,
//! `lsof` and `sysctl` on macOS. CPU is what a process used between two
//! samples (`Sampler` keeps the last), 100 being one core as `top` says it.
//!
//! A process belongs to (`attribute`, in this order):
//! 1. the worktree its cwd or the nearest ancestor's cwd lies in;
//! 2. else the worktree whose compose stack runs it (its container's
//!    cgroup, the project of `local::containers`);
//! 3. else the agent run that started it (an ancestor is a run's claude,
//!    the run engine's pid), counted to the run's worktree;
//! 4. else "other".
//!
//! Every process with a run's claude among its ancestors counts to that run
//! too. RSS adds up shared pages once per process: a sum is an upper bound.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How many processes a summary names: the biggest by RAM, then the busiest by CPU not among them.
pub const TOP: usize = 15;
/// How long a first sample waits for its second: CPU is a difference.
pub const FIRST_WAIT: Duration = Duration::from_millis(500);
/// How long a command line in a summary may be.
const COMMAND_MAX: usize = 160;

/// One process as a sample read it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Proc {
    pub pid: i64,
    pub ppid: i64,
    /// When it started (Linux: ticks since boot): a pid used again is another process.
    pub start: u64,
    pub command: String,
    /// The CPU time it used so far, in ticks (Linux); None where the OS says its CPU % itself.
    pub ticks: Option<u64>,
    /// Its CPU % as the OS says it (macOS's `ps`), None on Linux.
    pub cpu: Option<f64>,
    /// Bytes.
    pub rss: u64,
    /// None where it can't be read (another user's process).
    pub cwd: Option<PathBuf>,
    /// The id of the docker container it runs in (64 hex digits).
    pub container: Option<String>,
}

/// The machine's side of a sample.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Totals {
    pub cores: u32,
    /// All CPU time so far and the idle part of it, in ticks (Linux); None on macOS.
    pub ticks: Option<(u64, u64)>,
    /// Bytes.
    pub memory: u64,
    pub available: u64,
    pub swap: u64,
    pub swap_free: u64,
    pub load: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sample {
    pub totals: Totals,
    pub processes: Vec<Proc>,
}

/// A sample with each process's CPU over the last interval.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Usage {
    pub machine: Machine,
    /// Each process with its CPU %.
    pub processes: Vec<(Proc, f64)>,
}

/// Samples the node, the last one kept: each sample's CPU is since the one before.
#[derive(Debug, Default)]
pub struct Sampler {
    last: Option<Sample>,
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler::default()
    }

    /// What the node's processes take now; the first sample waits FIRST_WAIT
    /// for a second. None where the OS can't be read.
    pub fn sample(&mut self) -> Option<Usage> {
        if self.last.is_none() {
            self.last = read();
            std::thread::sleep(FIRST_WAIT);
        }
        let now = read()?;
        let usage = usage_of(self.last.as_ref(), &now);
        self.last = Some(now);
        Some(usage)
    }
}

/// What the node's processes take now, attributed to the repos of `mains`:
/// one sample, FIRST_WAIT long. None where the OS can't be read.
pub fn now_of(mains: &[PathBuf]) -> Option<Resources> {
    let usage = Sampler::new().sample()?;
    Some(summary_of(&usage, &Owners::read(mains)))
}

/// CPU from two samples: a process's ticks since `before` against the
/// machine's per core; a process new since then counts from its start.
pub fn usage_of(before: Option<&Sample>, now: &Sample) -> Usage {
    let cores = now.totals.cores.max(1);
    let elapsed = match (before.and_then(|it| it.totals.ticks), now.totals.ticks) {
        (Some((then, idle_then)), Some((total, idle))) if total > then => {
            Some(((total - then) as f64, idle.saturating_sub(idle_then) as f64))
        }
        _ => None,
    };
    let earlier: HashMap<(i64, u64), u64> = before
        .map(|it| {
            it.processes
                .iter()
                .filter_map(|proc| Some(((proc.pid, proc.start), proc.ticks?)))
                .collect()
        })
        .unwrap_or_default();
    let processes: Vec<(Proc, f64)> = now
        .processes
        .iter()
        .map(|proc| {
            let cpu = match (proc.cpu, proc.ticks, elapsed) {
                (Some(cpu), ..) => cpu,
                (None, Some(ticks), Some((total, _))) => {
                    // One seen first now started since `before`: all its ticks are of the interval.
                    let used = ticks.saturating_sub(earlier.get(&(proc.pid, proc.start)).copied().unwrap_or(0));
                    round(used as f64 / (total / f64::from(cores)) * 100.0)
                }
                _ => 0.0,
            };
            (proc.clone(), cpu)
        })
        .collect();
    let cpu = match elapsed {
        Some((total, idle)) => round((total - idle) / total * 100.0),
        // macOS: the processes' own, over all cores.
        None => round(processes.iter().map(|(_, cpu)| cpu).sum::<f64>() / f64::from(cores)),
    };
    let totals = &now.totals;
    Usage {
        machine: Machine {
            cores,
            cpu: cpu.clamp(0.0, 100.0),
            memory: totals.memory,
            used: totals.memory.saturating_sub(totals.available),
            available: totals.available,
            swap: totals.swap,
            swap_used: totals.swap.saturating_sub(totals.swap_free),
            load: totals.load,
        },
        processes,
    }
}

fn round(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

// ---------------------------------------------------------------------------
// The summary: what the snapshot and `top` carry.
// ---------------------------------------------------------------------------

/// The node's resources, compact: the machine, the top processes, the sums
/// per worktree and per agent run, and what is no one's.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resources {
    pub machine: Machine,
    /// The TOP biggest by RAM, then the busiest by CPU not among them; biggest first.
    pub top: Vec<Process>,
    /// Every worktree with a process, the biggest first.
    pub worktrees: Vec<Share>,
    /// Every agent run with a process, the biggest first.
    pub runs: Vec<RunShare>,
    /// What belongs to no worktree and no run.
    pub other: Sum,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub cores: u32,
    /// Of all cores together, 0 to 100.
    pub cpu: f64,
    /// Bytes: all, in use (all less available), available.
    pub memory: u64,
    pub used: u64,
    pub available: u64,
    pub swap: u64,
    pub swap_used: u64,
    /// The load average of 1, 5 and 15 minutes.
    pub load: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Process {
    pub pid: i64,
    pub ppid: i64,
    pub command: String,
    /// 100 is one core.
    pub cpu: f64,
    /// Bytes.
    pub rss: u64,
    pub worktree: Option<PathBuf>,
    /// The session of the agent run it runs under.
    pub run: Option<String>,
    /// The compose project of the container it runs in.
    pub stack: Option<String>,
    pub via: Via,
}

/// How a process was attributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Via {
    /// Its cwd or an ancestor's lies in the worktree.
    Cwd,
    /// It runs in a container of the worktree's stack.
    Stack,
    /// An agent run started it.
    Run,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sum {
    pub processes: usize,
    pub cpu: f64,
    pub rss: u64,
}

impl Sum {
    fn add(&mut self, cpu: f64, rss: u64) {
        self.processes += 1;
        self.cpu = round(self.cpu + cpu);
        self.rss += rss;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    pub worktree: PathBuf,
    #[serde(flatten)]
    pub sum: Sum,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunShare {
    pub session: String,
    pub name: String,
    pub dir: PathBuf,
    #[serde(flatten)]
    pub sum: Sum,
}

/// What processes can belong to.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Owners {
    /// Every worktree, main checkouts included.
    pub worktrees: Vec<PathBuf>,
    /// The worktree of each compose project of its own (a shared one: its main checkout's).
    pub stacks: BTreeMap<String, PathBuf>,
    /// The compose project of each container, by its id (docker's short one or the full).
    pub containers: BTreeMap<String, String>,
    pub runs: Vec<RunPid>,
}

/// An agent run in flight: its claude's pid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPid {
    pub pid: i64,
    pub session: String,
    pub name: String,
    pub dir: PathBuf,
}

impl Owners {
    /// The owners of the repos of `mains`: their worktrees, their stacks, the
    /// containers docker knows (`local::containers`, shared), their runs in flight.
    pub fn read(mains: &[PathBuf]) -> Owners {
        let mut owners = Owners::default();
        for main in mains {
            for tree in crate::git::all_worktrees(main) {
                let stack = crate::stack::of(&tree.dir);
                let env = crate::stack::env_of(&tree.dir);
                let project = crate::stack::project_in(stack.as_ref(), env.as_ref());
                // The main checkout comes first: a shared stack is its.
                if !project.is_empty() {
                    owners.stacks.entry(project).or_insert_with(|| tree.dir.clone());
                }
                owners.worktrees.push(tree.dir);
            }
            let table: crate::runs::Table = crate::runs::read_or_default(&crate::runs::status_file(main));
            owners.runs.extend(table.runs.into_iter().filter_map(|(session, run)| {
                Some(RunPid {
                    pid: run.pid.filter(|pid| crate::lock::is_alive(*pid))?,
                    session,
                    name: run.name,
                    dir: run.dir,
                })
            }));
        }
        owners.containers = crate::local::containers()
            .unwrap_or_default()
            .into_iter()
            .filter(|container| !container.id.is_empty())
            .map(|container| (container.id, container.project))
            .collect();
        owners
    }

    /// The worktree `dir` lies in, the deepest one.
    fn worktree_of(&self, dir: &Path) -> Option<&PathBuf> {
        self.worktrees
            .iter()
            .filter(|tree| dir.starts_with(tree))
            .max_by_key(|tree| tree.components().count())
    }

    /// The compose project of the container `id`.
    fn project_of(&self, id: &str) -> Option<&String> {
        self.containers
            .iter()
            .find(|(known, _)| !known.is_empty() && (id.starts_with(known.as_str()) || known.starts_with(id)))
            .map(|(_, project)| project)
    }
}

/// Whom one process belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribution {
    pub worktree: Option<PathBuf>,
    pub run: Option<usize>,
    pub stack: Option<String>,
    pub via: Via,
}

/// The attribution of each of `processes`, in their order.
pub fn attribute(processes: &[Proc], owners: &Owners) -> Vec<Attribution> {
    let by_pid: HashMap<i64, &Proc> = processes.iter().map(|proc| (proc.pid, proc)).collect();
    let runs: HashMap<i64, usize> = owners.runs.iter().enumerate().map(|(at, run)| (run.pid, at)).collect();
    // The process itself, then its ancestors: a cycle or a missing parent ends it.
    let chain = |proc: &Proc| {
        let mut chain = vec![proc.pid];
        let mut ppid = proc.ppid;
        while let Some(parent) = by_pid.get(&ppid).filter(|_| ppid > 0 && chain.len() < 256) {
            if chain.contains(&parent.pid) {
                break;
            }
            chain.push(parent.pid);
            ppid = parent.ppid;
        }
        chain
    };
    processes
        .iter()
        .map(|proc| {
            let chain = chain(proc);
            let by_cwd = chain.iter().find_map(|pid| {
                let cwd = by_pid.get(pid)?.cwd.as_ref()?;
                owners.worktree_of(cwd)
            });
            let stack = proc.container.as_deref().and_then(|id| owners.project_of(id)).cloned();
            let run = chain.iter().find_map(|pid| runs.get(pid).copied());
            let (worktree, via) = if let Some(tree) = by_cwd {
                (Some(tree.clone()), Via::Cwd)
            } else if let Some(tree) = stack.as_ref().and_then(|project| owners.stacks.get(project)) {
                (Some(tree.clone()), Via::Stack)
            } else if let Some(run) = run {
                (Some(owners.runs[run].dir.clone()), Via::Run)
            } else {
                (None, Via::Other)
            };
            Attribution {
                worktree,
                run,
                stack,
                via,
            }
        })
        .collect()
}

/// The summary of `usage` with its processes attributed to `owners`.
pub fn summary_of(usage: &Usage, owners: &Owners) -> Resources {
    let procs: Vec<Proc> = usage.processes.iter().map(|(proc, _)| proc.clone()).collect();
    let attributions = attribute(&procs, owners);
    let mut worktrees: BTreeMap<PathBuf, Sum> = BTreeMap::new();
    let mut runs: BTreeMap<usize, Sum> = BTreeMap::new();
    let mut other = Sum::default();
    let mut all: Vec<Process> = Vec::with_capacity(procs.len());
    for ((proc, cpu), attribution) in usage.processes.iter().zip(attributions) {
        match &attribution.worktree {
            Some(tree) => worktrees.entry(tree.clone()).or_default().add(*cpu, proc.rss),
            None if attribution.run.is_none() => other.add(*cpu, proc.rss),
            None => {}
        }
        if let Some(run) = attribution.run {
            runs.entry(run).or_default().add(*cpu, proc.rss);
        }
        all.push(Process {
            pid: proc.pid,
            ppid: proc.ppid,
            command: shortened(&proc.command),
            cpu: *cpu,
            rss: proc.rss,
            worktree: attribution.worktree,
            run: attribution.run.map(|run| owners.runs[run].session.clone()),
            stack: attribution.stack,
            via: attribution.via,
        });
    }
    let mut worktrees: Vec<Share> = worktrees
        .into_iter()
        .map(|(worktree, sum)| Share { worktree, sum })
        .collect();
    worktrees.sort_by(|a, b| b.sum.rss.cmp(&a.sum.rss).then_with(|| a.worktree.cmp(&b.worktree)));
    let mut runs: Vec<RunShare> = runs
        .into_iter()
        .map(|(at, sum)| {
            let run = &owners.runs[at];
            RunShare {
                session: run.session.clone(),
                name: run.name.clone(),
                dir: run.dir.clone(),
                sum,
            }
        })
        .collect();
    runs.sort_by(|a, b| b.sum.rss.cmp(&a.sum.rss).then_with(|| a.session.cmp(&b.session)));
    Resources {
        machine: usage.machine.clone(),
        top: top_of(all),
        worktrees,
        runs,
        other,
    }
}

/// The TOP biggest by RAM, then up to TOP/2 busy by CPU not among them; biggest first.
fn top_of(mut all: Vec<Process>) -> Vec<Process> {
    all.sort_by(|a, b| b.rss.cmp(&a.rss).then_with(|| a.pid.cmp(&b.pid)));
    let rest = all.split_off(TOP.min(all.len()));
    let mut busy: Vec<Process> = rest.into_iter().filter(|proc| proc.cpu > 0.0).collect();
    busy.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then_with(|| a.pid.cmp(&b.pid)));
    all.extend(busy.into_iter().take(TOP / 2));
    all.sort_by(|a, b| b.rss.cmp(&a.rss).then_with(|| a.pid.cmp(&b.pid)));
    all
}

fn shortened(command: &str) -> String {
    match command.char_indices().nth(COMMAND_MAX) {
        Some((at, _)) => format!("{}…", &command[..at]),
        None => command.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Reading the OS.
// ---------------------------------------------------------------------------

/// One sample of the node, None where the OS can't be read.
pub fn read() -> Option<Sample> {
    if cfg!(target_os = "macos") {
        macos::read()
    } else {
        linux::read(Path::new("/proc"))
    }
}

/// The docker container id in a process's `/proc/<pid>/cgroup`: a segment
/// of 64 hex digits (`docker-<id>.scope`, `/docker/<id>`, `libpod-<id>`).
pub fn container_in(cgroup: &str) -> Option<String> {
    cgroup.lines().find_map(|line| {
        let path = line.splitn(3, ':').nth(2)?;
        path.split(['/', '-', '.']).find_map(|part| {
            (part.len() == 64 && part.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| part.to_string())
        })
    })
}

pub mod linux {
    use super::{Proc, Sample, Totals, container_in};
    use std::path::Path;

    /// A sample from the `/proc` at `root`.
    pub fn read(root: &Path) -> Option<Sample> {
        let mut totals = totals_in(
            &std::fs::read_to_string(root.join("stat")).ok()?,
            &std::fs::read_to_string(root.join("meminfo")).ok()?,
        );
        totals.load = load_in(&std::fs::read_to_string(root.join("loadavg")).unwrap_or_default());
        // SAFETY: sysconf only reads a constant of the system.
        let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096);
        let processes = std::fs::read_dir(root)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<i64>().ok())
            .filter_map(|pid| process_of(&root.join(pid.to_string()), pid, page))
            .collect();
        Some(Sample { totals, processes })
    }

    /// One process from its folder, None for one gone meanwhile or a kernel thread.
    fn process_of(dir: &Path, pid: i64, page: u64) -> Option<Proc> {
        let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
        let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
        let mut proc = process_in(pid, &stat, &cmdline, page)?;
        proc.cwd = std::fs::read_link(dir.join("cwd")).ok();
        proc.container = std::fs::read_to_string(dir.join("cgroup"))
            .ok()
            .and_then(|cgroup| container_in(&cgroup));
        Some(proc)
    }

    /// A process from its `stat` and `cmdline`; None for a kernel thread (no command line, no memory).
    pub fn process_in(pid: i64, stat: &str, cmdline: &[u8], page: u64) -> Option<Proc> {
        let (head, rest) = stat.rsplit_once(')')?;
        let name = head.split_once('(').map_or("", |(_, name)| name);
        let fields: Vec<&str> = rest.split_whitespace().collect();
        // After the name: state 0, ppid 1, …, utime 11, stime 12, …, starttime 19, vsize 20, rss 21.
        let number = |at: usize| fields.get(at).and_then(|field| field.parse::<u64>().ok());
        let rss = number(21)? * page;
        let command = cmdline
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf8_lossy)
            .collect::<Vec<_>>()
            .join(" ");
        if command.is_empty() && rss == 0 {
            return None;
        }
        Some(Proc {
            pid,
            ppid: fields.get(1)?.parse().ok()?,
            start: number(19)?,
            command: if command.is_empty() {
                format!("[{name}]")
            } else {
                command
            },
            ticks: Some(number(11)? + number(12)?),
            cpu: None,
            rss,
            cwd: None,
            container: None,
        })
    }

    /// The machine from `/proc/stat` and `/proc/meminfo`.
    pub fn totals_in(stat: &str, meminfo: &str) -> Totals {
        let cpu: Vec<u64> = stat
            .lines()
            .find(|line| line.starts_with("cpu "))
            .map(|line| {
                line.split_whitespace()
                    .skip(1)
                    .filter_map(|field| field.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        // user nice system idle iowait irq softirq steal; guest is in user already.
        let total: u64 = cpu.iter().take(8).sum();
        let idle = cpu.get(3).copied().unwrap_or(0) + cpu.get(4).copied().unwrap_or(0);
        let cores = stat
            .lines()
            .filter(|line| line.starts_with("cpu") && line.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
            .count();
        let kb = |key: &str| {
            meminfo
                .lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
                .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
                .unwrap_or(0)
                * 1024
        };
        Totals {
            cores: u32::try_from(cores).unwrap_or(1).max(1),
            ticks: (total > 0).then_some((total, idle)),
            memory: kb("MemTotal"),
            available: kb("MemAvailable"),
            swap: kb("SwapTotal"),
            swap_free: kb("SwapFree"),
            load: [0.0; 3],
        }
    }

    /// The three load averages of `/proc/loadavg`.
    pub fn load_in(loadavg: &str) -> [f64; 3] {
        let mut fields = loadavg.split_whitespace().map(|field| field.parse().unwrap_or(0.0));
        [(); 3].map(|()| fields.next().unwrap_or(0.0))
    }
}

pub mod macos {
    use super::{Proc, Sample, Totals};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    /// A sample from `ps`, `lsof` (the cwds, one call) and `sysctl`/`vm_stat`.
    pub fn read() -> Option<Sample> {
        let ps = output("ps", &["-axo", "pid=,ppid=,%cpu=,rss=,command="])?;
        let cwds = cwds_in(&output("lsof", &["-a", "-d", "cwd", "-n", "-P", "-F", "pn"]).unwrap_or_default());
        let processes = processes_in(&ps)
            .into_iter()
            .map(|mut proc| {
                proc.cwd = cwds.get(&proc.pid).cloned();
                proc
            })
            .collect();
        let sysctl = output("sysctl", &["-n", "hw.ncpu", "hw.memsize", "vm.loadavg", "vm.swapusage"])?;
        let vm_stat = output("vm_stat", &[]).unwrap_or_default();
        Some(Sample {
            totals: totals_in(&sysctl, &vm_stat),
            processes,
        })
    }

    fn output(program: &str, args: &[&str]) -> Option<String> {
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        // lsof exits 1 when it can't see some processes: what it printed holds.
        (output.status.success() || !output.stdout.is_empty())
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// The processes `ps -axo pid=,ppid=,%cpu=,rss=,command=` printed.
    pub fn processes_in(ps: &str) -> Vec<Proc> {
        ps.lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse().ok()?;
                let ppid = fields.next()?.parse().ok()?;
                let cpu = fields.next()?.replace(',', ".").parse().ok()?;
                let rss: u64 = fields.next()?.parse().ok()?;
                Some(Proc {
                    pid,
                    ppid,
                    command: fields.collect::<Vec<_>>().join(" "),
                    cpu: Some(cpu),
                    rss: rss * 1024,
                    ..Proc::default()
                })
            })
            .collect()
    }

    /// The cwd of each process in what `lsof -a -d cwd -F pn` printed (`p<pid>`, then `n<path>`).
    pub fn cwds_in(lsof: &str) -> HashMap<i64, PathBuf> {
        let mut cwds = HashMap::new();
        let mut pid = None;
        for line in lsof.lines() {
            if let Some(number) = line.strip_prefix('p') {
                pid = number.parse::<i64>().ok();
            } else if let (Some(path), Some(pid)) = (line.strip_prefix('n'), pid) {
                cwds.insert(pid, PathBuf::from(path));
            }
        }
        cwds
    }

    /// The machine from `sysctl -n hw.ncpu hw.memsize vm.loadavg vm.swapusage` and `vm_stat`.
    pub fn totals_in(sysctl: &str, vm_stat: &str) -> Totals {
        let lines: Vec<&str> = sysctl.lines().collect();
        let cores = lines.first().and_then(|line| line.trim().parse().ok()).unwrap_or(1);
        let memory = lines.get(1).and_then(|line| line.trim().parse().ok()).unwrap_or(0);
        let mut load = lines
            .get(2)
            .map(|line| line.trim_matches(|c: char| c == '{' || c == '}' || c.is_whitespace()))
            .unwrap_or_default()
            .split_whitespace()
            .map(|field| field.replace(',', ".").parse().unwrap_or(0.0));
        let load = [(); 3].map(|()| load.next().unwrap_or(0.0));
        // `total = 2048.00M  used = 1024.00M  free = 1024.00M  (encrypted)`
        let swap_of = |key: &str| {
            let line = lines.get(3).copied().unwrap_or_default();
            let at = line.find(key)? + key.len();
            let value = line[at..].trim_start_matches([' ', '=']).split_whitespace().next()?;
            let (number, unit) = value.split_at(value.len().checked_sub(1)?);
            let factor = match unit {
                "K" => 1u64 << 10,
                "M" => 1 << 20,
                "G" => 1 << 30,
                _ => 1,
            };
            Some((number.replace(',', ".").parse::<f64>().ok()? * factor as f64) as u64)
        };
        // Available as Activity Monitor counts it roughly: free, inactive, speculative and purgeable pages.
        let page = vm_stat
            .lines()
            .next()
            .and_then(|line| {
                line.split("page size of ")
                    .nth(1)?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .unwrap_or(16_384);
        let pages = |key: &str| {
            vm_stat
                .lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
                .and_then(|value| value.trim().trim_end_matches('.').parse::<u64>().ok())
                .unwrap_or(0)
        };
        let available =
            (pages("Pages free") + pages("Pages inactive") + pages("Pages speculative") + pages("Pages purgeable"))
                * page;
        Totals {
            cores,
            ticks: None,
            memory,
            available: available.min(memory),
            swap: swap_of("total").unwrap_or(0),
            swap_free: swap_of("free").unwrap_or(0),
            load,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i64, ppid: i64, cwd: Option<&str>, rss: u64) -> Proc {
        Proc {
            pid,
            ppid,
            command: format!("p{pid}"),
            rss,
            cwd: cwd.map(PathBuf::from),
            ..Proc::default()
        }
    }

    fn owners() -> Owners {
        Owners {
            worktrees: vec!["/r/platform".into(), "/r/platform-7-x".into(), "/r/platform-8-y".into()],
            stacks: [
                ("vita".to_string(), PathBuf::from("/r/platform")),
                ("vita-8".to_string(), PathBuf::from("/r/platform-8-y")),
            ]
            .into(),
            containers: [("abc123def456".to_string(), "vita-8".to_string())].into(),
            runs: vec![RunPid {
                pid: 50,
                session: "s-1".into(),
                name: "7-x".into(),
                dir: "/r/platform-7-x".into(),
            }],
        }
    }

    #[test]
    fn attributes_a_process_tree_under_a_worktree_by_the_nearest_cwd() {
        let processes = vec![
            proc(1, 0, Some("/"), 10),
            // pnpm wt:check in 7-x, its vitest workers without a readable cwd, one in a subfolder.
            proc(10, 1, Some("/r/platform-7-x"), 100),
            proc(11, 10, None, 1000),
            proc(12, 11, Some("/r/platform-7-x/packages/breadboard"), 1000),
            // A process in /tmp under the main checkout's shell: the shell's.
            proc(20, 1, Some("/r/platform"), 5),
            proc(21, 20, Some("/tmp"), 7),
            // `/r/platform-7-xy` is no part of `/r/platform-7-x`, nor of `/r/platform`.
            proc(30, 1, Some("/r/platform-7-xy"), 3),
        ];
        let got: Vec<(Option<PathBuf>, Via)> = attribute(&processes, &owners())
            .into_iter()
            .map(|it| (it.worktree, it.via))
            .collect();
        let tree = |dir: &str| (Some(PathBuf::from(dir)), Via::Cwd);
        assert_eq!(
            got,
            [
                (None, Via::Other),
                tree("/r/platform-7-x"),
                tree("/r/platform-7-x"),
                tree("/r/platform-7-x"),
                tree("/r/platform"),
                tree("/r/platform"),
                (None, Via::Other),
            ]
        );
    }

    #[test]
    fn attributes_a_compose_container_to_its_worktrees_stack() {
        let mut postgres = proc(40, 39, None, 300);
        postgres.container = container_in(&format!(
            "0::/system.slice/docker-abc123def456{}.scope\n",
            "0".repeat(52)
        ));
        let mut unknown = proc(41, 39, None, 1);
        unknown.container = Some("f".repeat(64));
        let processes = vec![proc(39, 1, None, 2), postgres, unknown];
        let got = attribute(&processes, &owners());
        assert_eq!(got[1].worktree, Some(PathBuf::from("/r/platform-8-y")));
        assert_eq!((got[1].via, got[1].stack.as_deref()), (Via::Stack, Some("vita-8")));
        assert_eq!((got[2].worktree.clone(), got[2].via), (None, Via::Other));
        assert_eq!(got[0].via, Via::Other);
    }

    #[test]
    fn attributes_a_runs_child_outside_the_worktree_to_the_run() {
        let processes = vec![
            proc(50, 1, None, 400),
            // A tool of the run in /tmp, and its child.
            proc(51, 50, Some("/tmp"), 20),
            proc(52, 51, None, 30),
            // A child of the run in another worktree: that worktree's, still the run's.
            proc(53, 50, Some("/r/platform-8-y"), 40),
        ];
        let got = attribute(&processes, &owners());
        let run_tree = (Some(PathBuf::from("/r/platform-7-x")), Via::Run, Some(0));
        let view = |it: &Attribution| (it.worktree.clone(), it.via, it.run);
        assert_eq!(view(&got[0]), run_tree);
        assert_eq!(view(&got[1]), run_tree);
        assert_eq!(view(&got[2]), run_tree);
        assert_eq!(
            view(&got[3]),
            (Some(PathBuf::from("/r/platform-8-y")), Via::Cwd, Some(0))
        );

        let usage = Usage {
            machine: Machine::default(),
            processes: processes
                .into_iter()
                .map(|proc| (proc, 25.0))
                .chain([(proc(60, 1, None, 9), 1.0)])
                .collect(),
        };
        let summary = summary_of(&usage, &owners());
        let sums: Vec<(PathBuf, usize, u64)> = summary
            .worktrees
            .iter()
            .map(|share| (share.worktree.clone(), share.sum.processes, share.sum.rss))
            .collect();
        assert_eq!(
            sums,
            [("/r/platform-7-x".into(), 3, 450), ("/r/platform-8-y".into(), 1, 40)]
        );
        assert_eq!(summary.runs.len(), 1);
        assert_eq!(
            (summary.runs[0].session.as_str(), summary.runs[0].sum),
            (
                "s-1",
                Sum {
                    processes: 4,
                    cpu: 100.0,
                    rss: 490
                }
            )
        );
        assert_eq!(
            summary.other,
            Sum {
                processes: 1,
                cpu: 1.0,
                rss: 9
            }
        );
        assert_eq!(
            summary.top.first().map(|it| (it.pid, it.run.as_deref())),
            Some((50, Some("s-1")))
        );
    }

    #[test]
    fn names_the_biggest_then_the_busiest_processes() {
        let all: Vec<Process> = (0..40)
            .map(|pid| Process {
                pid,
                ppid: 1,
                command: String::new(),
                cpu: if pid == 3 { 90.0 } else { 0.0 },
                rss: 1000 - pid as u64,
                worktree: None,
                run: None,
                stack: None,
                via: Via::Other,
            })
            .collect();
        let mut all = all;
        all[39].cpu = 50.0;
        let top = top_of(all);
        assert_eq!(top.len(), TOP + 1);
        assert_eq!(top.last().map(|it| it.pid), Some(39));
    }

    #[test]
    fn counts_cpu_between_two_samples_per_core() {
        let sample = |total: u64, idle: u64, ticks: &[(i64, u64, u64)]| Sample {
            totals: Totals {
                cores: 4,
                ticks: Some((total, idle)),
                memory: 32,
                available: 8,
                ..Totals::default()
            },
            processes: ticks
                .iter()
                .map(|(pid, start, ticks)| Proc {
                    pid: *pid,
                    start: *start,
                    ticks: Some(*ticks),
                    ..Proc::default()
                })
                .collect(),
        };
        let before = sample(1000, 600, &[(1, 5, 100), (2, 5, 50)]);
        // 400 ticks over 4 cores: 100 per core. 1 used 100 (a core), 2 is another process now (pid again), 3 is new.
        let now = sample(1400, 800, &[(1, 5, 200), (2, 9, 30), (3, 9, 10)]);
        let usage = usage_of(Some(&before), &now);
        let cpu: Vec<f64> = usage.processes.iter().map(|(_, cpu)| *cpu).collect();
        assert_eq!(cpu, [100.0, 30.0, 10.0]);
        assert_eq!(usage.machine.cpu, 50.0);
        assert_eq!((usage.machine.used, usage.machine.available), (24, 8));
        assert!(usage_of(None, &now).processes.iter().all(|(_, cpu)| *cpu == 0.0));
    }

    #[test]
    fn reads_linux_proc_files() {
        let stat = "1234 (node (vitest)) S 1200 1234 1200 0 -1 4194304 1 0 0 0 150 50 0 0 20 0 11 0 98765 1000 2560 18446744073709551615";
        let proc = linux::process_in(1234, stat, b"node\0vitest\0--run\0", 4096).unwrap();
        assert_eq!(
            (proc.ppid, proc.start, proc.ticks, proc.rss),
            (1200, 98765, Some(200), 2560 * 4096)
        );
        assert_eq!(proc.command, "node vitest --run");
        let kthread = "2 (kthreadd) S 0 0 0 0 -1 2129984 0 0 0 0 0 0 0 0 20 0 1 0 1 0 0 18446744073709551615";
        assert_eq!(linux::process_in(2, kthread, b"", 4096), None);

        let totals = linux::totals_in(
            "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\ncpu1 50 0 25 400 25 0 0 0 0 0\nintr 1\n",
            "MemTotal:       32000000 kB\nMemFree:  100 kB\nMemAvailable:    4000000 kB\nSwapTotal: 8000000 kB\nSwapFree: 2000000 kB\n",
        );
        assert_eq!(totals.cores, 2);
        assert_eq!(totals.ticks, Some((1000, 850)));
        assert_eq!((totals.memory, totals.available), (32_000_000 * 1024, 4_000_000 * 1024));
        assert_eq!((totals.swap, totals.swap_free), (8_000_000 * 1024, 2_000_000 * 1024));
        assert_eq!(linux::load_in("1.50 0.75 0.25 3/900 4321\n"), [1.5, 0.75, 0.25]);

        assert_eq!(container_in("0::/user.slice/user-1000.slice\n"), None);
        let id = "a".repeat(64);
        assert_eq!(container_in(&format!("12:memory:/docker/{id}\n")), Some(id.clone()));
        assert_eq!(container_in(&format!("0::/system.slice/docker-{id}.scope\n")), Some(id));
    }

    #[test]
    fn reads_what_macos_prints() {
        let processes =
            macos::processes_in("  1     0   0,5  12000 /sbin/launchd\n 812   1  97.3 2048000 node vitest --run\n");
        assert_eq!(processes.len(), 2);
        assert_eq!(
            (processes[1].pid, processes[1].ppid, processes[1].cpu, processes[1].rss),
            (812, 1, Some(97.3), 2_048_000 * 1024)
        );
        assert_eq!(processes[1].command, "node vitest --run");
        let cwds = macos::cwds_in("p1\nfcwd\nn/\np812\nfcwd\nn/Users/mira/platform-7-x\n");
        assert_eq!(cwds.get(&812), Some(&PathBuf::from("/Users/mira/platform-7-x")));
        let totals = macos::totals_in(
            "10\n34359738368\n{ 2.10 1.80 1.50 }\ntotal = 2048.00M  used = 512.00M  free = 1536.00M  (encrypted)\n",
            "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:      1000.\nPages inactive:  2000.\nPages speculative: 500.\nPages purgeable: 500.\n",
        );
        assert_eq!(
            (totals.cores, totals.memory, totals.load),
            (10, 34_359_738_368, [2.1, 1.8, 1.5])
        );
        assert_eq!((totals.swap, totals.swap_free), (2048 << 20, 1536 << 20));
        assert_eq!(totals.available, 4000 * 16384);
    }

    #[test]
    fn reads_this_machine() {
        if cfg!(target_os = "linux") {
            let sample = linux::read(Path::new("/proc")).unwrap();
            let me = i64::from(std::process::id());
            assert!(sample.processes.iter().any(|proc| proc.pid == me && proc.rss > 0));
            assert!(sample.totals.memory > 0 && sample.totals.cores > 0);
        }
    }
}
