//! `wt:plan <orchestration> [--work <issue>…] [--merge <issue>…] [--why <text>]`
//! (plan.mjs): the order the orchestrator works and merges its issues in,
//! printed, and set with the flags. The overview's NEXT, WORKTREES and MERGE
//! REQUESTS and herdr's space order read it from the record.

use super::args::{Takes, fail, first, flag_value, main_dir};
use breadboard_core::lock::with_dir_lock;
use breadboard_core::record::{self, Plan, is_name, record_file, record_of};
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "usage: pnpm wt:plan <orchestration> [--work <issue>…] [--merge <issue>…] [--why <text>]";
/// How long the record's `<file>.lock` holds before another takes it over (as actions.json's).
const LOCK_STALE: Duration = Duration::from_millis(5000);

pub const TAKES: Takes = Takes {
    valued: &["--why"],
    lists: &["--work", "--merge"],
    optional: &[],
    switches: &[],
    words: 1,
    about: "The order an orchestration works and merges its issues in: printed, and set with the flags. A list flag without issues clears that list.",
    usage: USAGE,
};

pub fn run(args: &[String]) -> ExitCode {
    let Some(name) = first(args) else { return fail(USAGE) };
    let main = match main_dir() {
        Ok(main) => main,
        Err(code) => return code,
    };
    // A name outside kebab-case would read a file outside the records (`../x`).
    if !is_name(name) {
        return fail(&format!(
            "{name} is no orchestration's name: names are kebab-case (pnpm wt:orchestrate <name> <issue>…)"
        ));
    }
    let Some(record) = record_of(&main, name) else {
        return fail(&format!("no orchestration {name} is recorded"));
    };
    let (work, merge) = match (numbers_after(args, "--work"), numbers_after(args, "--merge")) {
        (Ok(work), Ok(merge)) => (work, merge),
        (Err(value), _) | (_, Err(value)) => return fail(&format!("{value} is no issue number\n{USAGE}")),
    };
    let why = flag_value(args, "--why");

    let mut known: Vec<i64> = Vec::new();
    for number in record.issues.iter().filter_map(|issue| issue.number) {
        if !known.contains(&number) {
            known.push(number);
        }
    }
    let strangers: Vec<i64> = work
        .iter()
        .chain(merge.iter())
        .flatten()
        .copied()
        .filter(|number| !known.contains(number))
        .collect();
    if !strangers.is_empty() {
        return fail(&format!(
            "not in ⇶ {name}: {} (its issues: {})",
            hashes(&strangers, ", "),
            hashes(&known, ", ")
        ));
    }

    let plan = if work.is_some() || merge.is_some() || why.is_some() {
        // Under the record's lock, as wt:action --orchestration changes it.
        let written = with_dir_lock(&record_file(&main, name), LOCK_STALE, || {
            record::write_plan(&main, name, work, merge, why)
        });
        match written {
            Ok(Ok(plan)) => Some(plan),
            Ok(Err(error)) | Err(error) => {
                return fail(&format!(
                    "the plan could not be written to .claude/orchestrations/{name}.json: {error}"
                ));
            }
        }
    } else {
        record.plan
    };
    println!("{}", text_of(name, plan.as_ref()));
    ExitCode::SUCCESS
}

/// What `wt:plan` prints, between empty lines.
fn text_of(name: &str, plan: Option<&Plan>) -> String {
    let list = |numbers: Option<&Vec<i64>>| match numbers {
        Some(numbers) if !numbers.is_empty() => hashes(numbers, " → "),
        _ => "not set".to_string(),
    };
    let why = plan.and_then(|plan| plan.why.as_deref()).filter(|why| !why.is_empty());
    format!(
        "\n⇶ {name}\n  work:  {}\n  merge: {}{}\n",
        list(plan.map(|plan| &plan.work)),
        list(plan.map(|plan| &plan.merge)),
        why.map(|why| format!("\n  why:   {why}")).unwrap_or_default()
    )
}

fn hashes(numbers: &[i64], separator: &str) -> String {
    numbers
        .iter()
        .map(|number| format!("#{number}"))
        .collect::<Vec<_>>()
        .join(separator)
}

/// The issue numbers after `flag` up to the next flag (`#` allowed), None
/// when it isn't given, an empty list when it is given bare (it clears);
/// Err the first that is no positive whole number.
fn numbers_after(args: &[String], flag: &str) -> Result<Option<Vec<i64>>, String> {
    let Some(at) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    let values = args[at + 1..].iter().take_while(|arg| !arg.starts_with("--"));
    values
        .map(|value| number_of(value).ok_or_else(|| value.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// `Number(value.replace(/^#/, ""))` when it is a positive integer.
fn number_of(value: &str) -> Option<i64> {
    let number: f64 = value.strip_prefix('#').unwrap_or(value).trim().parse().ok()?;
    (number.is_finite() && number.fract() == 0.0 && number > 0.0 && number < 9e15).then_some(number as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn numbers_run_to_the_next_flag_and_a_bare_flag_clears() {
        let args = strings(&["x", "--work", "#3", "4", "--merge", "--why", "so"]);
        assert_eq!(numbers_after(&args, "--work"), Ok(Some(vec![3, 4])));
        assert_eq!(numbers_after(&args, "--merge"), Ok(Some(vec![])));
        assert_eq!(numbers_after(&args, "--other"), Ok(None));
        assert_eq!(numbers_after(&strings(&["--work", "0"]), "--work"), Err("0".into()));
        assert_eq!(numbers_after(&strings(&["--work", "3a"]), "--work"), Err("3a".into()));
        assert_eq!(numbers_after(&strings(&["--work", "2.5"]), "--work"), Err("2.5".into()));
        // The `=` form is no list (plan.mjs).
        assert_eq!(numbers_after(&strings(&["--work=3"]), "--work"), Ok(None));
    }

    #[test]
    fn prints_the_plan_with_its_why_only_when_there_is_one() {
        let plan = Plan {
            work: vec![306, 303],
            merge: vec![],
            why: Some(String::new()),
        };
        assert_eq!(
            text_of("breadboard", Some(&plan)),
            "\n⇶ breadboard\n  work:  #306 → #303\n  merge: not set\n"
        );
        let plan = Plan {
            why: Some("first the fix".into()),
            ..plan
        };
        assert!(text_of("breadboard", Some(&plan)).ends_with("  merge: not set\n  why:   first the fix\n"));
        assert_eq!(text_of("b", None), "\n⇶ b\n  work:  not set\n  merge: not set\n");
    }
}
