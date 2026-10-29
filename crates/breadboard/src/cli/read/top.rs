//! `pnpm top [--json]` (#513): what the node's processes take, per
//! worktree and agent run, as the snapshot carries it
//! (`breadboard_core::resources`): the machine, a table of the worktrees,
//! the runs and the rest, then the top processes. `--json` prints the
//! summary itself.

use super::args::{Takes, fail, main_dir, pad};
use breadboard_core::resources::{Resources, Sum, now_of};
use std::path::Path;
use std::process::ExitCode;

pub const TAKES: Takes = Takes {
    valued: &[],
    lists: &[],
    optional: &[],
    switches: &["--json"],
    words: 0,
    about: "What the node's processes take, CPU and RAM per worktree and agent run (by cwd, compose stack, run), then the top processes (--json: the summary itself).",
    usage: "usage: pnpm top [--json]",
};

pub fn run(args: &[String]) -> ExitCode {
    let main = match main_dir() {
        Ok(main) => main,
        Err(code) => return code,
    };
    let Some(resources) = now_of(std::slice::from_ref(&main)) else {
        return fail("this machine's processes couldn't be read");
    };
    if args.iter().any(|arg| arg == "--json") {
        match serde_json::to_string_pretty(&resources) {
            Ok(text) => println!("{text}"),
            Err(error) => return fail(&format!("the summary didn't serialize: {error}")),
        }
    } else {
        print!("{}", table(&resources));
    }
    ExitCode::SUCCESS
}

/// The machine's line, the shares' table, the top processes.
fn table(resources: &Resources) -> String {
    let machine = &resources.machine;
    let mut text = format!(
        "CPU {:.0}% of {} cores · load {:.2} {:.2} {:.2} · RAM {} of {} used ({} free) · swap {} of {}\n\n",
        machine.cpu,
        machine.cores,
        machine.load[0],
        machine.load[1],
        machine.load[2],
        size(machine.used),
        size(machine.memory),
        size(machine.available),
        size(machine.swap_used),
        size(machine.swap),
    );
    let mut rows: Vec<(String, Sum)> = resources
        .worktrees
        .iter()
        .map(|share| (name(&share.worktree), share.sum))
        .collect();
    rows.extend(
        resources
            .runs
            .iter()
            .map(|run| (format!("run {} ({})", run.name, short(&run.session)), run.sum)),
    );
    rows.push(("other".into(), resources.other));
    let width = rows
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0)
        .max(8);
    text.push_str(&format!(
        "{}  {:>5}  {:>7}  {:>9}\n",
        pad("WORKTREE", width),
        "PROCS",
        "CPU",
        "RAM"
    ));
    for (name, sum) in &rows {
        text.push_str(&format!(
            "{}  {:>5}  {:>6.1}%  {:>9}\n",
            pad(name, width),
            sum.processes,
            sum.cpu,
            size(sum.rss)
        ));
    }
    text.push_str(&format!(
        "\n{:>7}  {:>6}  {:>9}  {}\n",
        "PID", "CPU", "RAM", "OF  COMMAND"
    ));
    for process in &resources.top {
        let of = process.worktree.as_deref().map(name).unwrap_or_else(|| "other".into());
        text.push_str(&format!(
            "{:>7}  {:>5.1}%  {:>9}  {of}  {}\n",
            process.pid,
            process.cpu,
            size(process.rss),
            process.command
        ));
    }
    text
}

/// A worktree by its folder's name.
fn name(dir: &Path) -> String {
    dir.file_name()
        .map_or_else(|| dir.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn short(session: &str) -> &str {
    session.get(..8).unwrap_or(session)
}

/// Bytes as `812 MB`, `12.4 GB`.
fn size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else {
        format!("{:.0} MB", bytes / 1024.0 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::resources::{Machine, Process, RunShare, Share, Via};

    #[test]
    fn prints_the_machine_the_shares_and_the_top() {
        let gb = 1024 * 1024 * 1024;
        let sum = |processes, cpu, rss| Sum { processes, cpu, rss };
        let resources = Resources {
            machine: Machine {
                cores: 16,
                cpu: 92.4,
                memory: 31 * gb,
                used: 29 * gb,
                available: 2 * gb,
                swap: 8 * gb,
                swap_used: gb,
                load: [30.0, 20.5, 10.25],
            },
            top: vec![Process {
                pid: 812,
                ppid: 800,
                command: "node vitest".into(),
                cpu: 98.0,
                rss: 2 * gb,
                worktree: Some("/r/platform-7-x".into()),
                run: None,
                stack: None,
                via: Via::Cwd,
            }],
            worktrees: vec![Share {
                worktree: "/r/platform-7-x".into(),
                sum: sum(264, 1210.5, 26 * gb),
            }],
            runs: vec![RunShare {
                session: "0123456789abcdef".into(),
                name: "7-x".into(),
                dir: "/r/platform-7-x".into(),
                sum: sum(3, 5.0, 300 * 1024 * 1024),
            }],
            other: sum(400, 30.0, 3 * gb),
        };
        assert_eq!(
            table(&resources),
            "CPU 92% of 16 cores · load 30.00 20.50 10.25 · RAM 29.0 GB of 31.0 GB used (2.0 GB free) · swap 1.0 GB of 8.0 GB\n\
             \n\
             WORKTREE            PROCS      CPU        RAM\n\
             platform-7-x          264  1210.5%    26.0 GB\n\
             run 7-x (01234567)      3     5.0%     300 MB\n\
             other                 400    30.0%     3.0 GB\n\
             \n    \
             PID     CPU        RAM  OF  COMMAND\n    \
             812   98.0%     2.0 GB  platform-7-x  node vitest\n"
        );
    }
}
