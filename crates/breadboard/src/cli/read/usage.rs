//! `pnpm gitlab:usage [--minutes <n>]` (#462): who of the user's breadboard
//! asked GitLab how often, from the request log every process writes
//! (core's `gitlab::budget`): per caller its requests, a minute on average
//! and in its busiest minute, the 429s and a pause in force. GitLab's
//! budget is the user's, all callers together: 100 a minute on the free plan.

use super::args::{Takes, fail, flag_value, pad};
use breadboard_core::gitlab::budget::{self, Logged};
use breadboard_core::time::{hm, now_ms};
use std::collections::BTreeMap;
use std::process::ExitCode;

pub const TAKES: Takes = Takes {
    valued: &["--minutes"],
    lists: &[],
    optional: &[],
    switches: &[],
    words: 0,
    about: "Who of breadboard asked GitLab how often (the request log of every process of the user): per caller a minute on average and at its peak, the 429s, a pause in force.",
    usage: "usage: pnpm gitlab:usage [--minutes <n>]   (default 30)",
};

/// The window looked at by default, in minutes.
const MINUTES: i64 = 30;

pub fn run(args: &[String]) -> ExitCode {
    let minutes = match flag_value(args, "--minutes").map(|text| text.parse::<i64>()) {
        None => MINUTES,
        Some(Ok(minutes)) if minutes > 0 => minutes,
        Some(_) => return fail("--minutes takes a whole number above 0"),
    };
    let now = now_ms();
    let since = now - minutes * 60_000;
    print!("{}", report(&budget::logged_since(since), since, now));
    if let Some(pause) = budget::paused() {
        println!("\n{}", pause.text());
    }
    ExitCode::SUCCESS
}

/// The table of `logged` between `since` and `now`.
fn report(logged: &[Logged], since: i64, now: i64) -> String {
    let minutes = ((now - since) as f64 / 60_000.0).max(1.0 / 60.0);
    let mut out = format!(
        "GitLab requests of breadboard, {}–{}: {}, {:.1} a minute\n",
        hm(since),
        hm(now),
        logged.len(),
        logged.len() as f64 / minutes
    );
    if logged.is_empty() {
        out.push_str("  none logged (the request log: ");
        out.push_str(&budget::log_file().display().to_string());
        out.push_str(")\n");
        return out;
    }
    // caller → its requests per minute of the window.
    let mut callers: BTreeMap<String, BTreeMap<i64, usize>> = BTreeMap::new();
    for line in logged {
        *callers
            .entry(format!("{} · {}", line.by, line.task))
            .or_default()
            .entry((line.at - since) / 60_000)
            .or_default() += 1;
    }
    let mut rows: Vec<(String, usize, usize)> = callers
        .into_iter()
        .map(|(caller, by_minute)| {
            let count = by_minute.values().sum();
            let peak = by_minute.values().copied().max().unwrap_or(0);
            (caller, count, peak)
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let width = rows.iter().map(|row| row.0.chars().count()).max().unwrap_or(0).max(6);
    out.push_str(&format!(
        "  {}  requests  a minute  peak minute\n",
        pad("caller", width)
    ));
    for (caller, count, peak) in rows {
        out.push_str(&format!(
            "  {}  {count:>8}  {:>8.1}  {peak:>11}\n",
            pad(&caller, width),
            count as f64 / minutes
        ));
    }
    let limited = logged.iter().filter(|line| line.status == Some(429)).count();
    if limited > 0 {
        out.push_str(&format!("  {limited} answered 429\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_caller_on_average_and_at_its_peak() {
        let at = |minute: i64, by: &str, task: &str, status: Option<u16>| Logged {
            at: minute * 60_000 + 1,
            by: by.into(),
            task: task.into(),
            what: "graphql".into(),
            status,
        };
        let logged = vec![
            at(0, "serve", "ci queue", Some(200)),
            at(0, "serve", "ci queue", Some(200)),
            at(0, "serve", "ci queue", Some(200)),
            at(1, "serve", "ci queue", Some(429)),
            at(1, "agent", "glab", None),
        ];
        let text = report(&logged, 0, 2 * 60_000);
        assert!(text.contains(": 5, 2.5 a minute"), "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[2].starts_with("  serve · ci queue"), "{text}");
        assert!(lines[2].ends_with("4       2.0            3"), "{text}");
        assert!(lines[3].starts_with("  agent · glab"), "{text}");
        assert!(text.contains("1 answered 429"), "{text}");
        assert!(report(&[], 0, 60_000).contains("none logged"));
    }
}
