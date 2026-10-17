//! `wt:epic <name> --mr` (#352): the epic of an orchestration goes to
//! develop at a milestone. One MR epic → develop, opened or brought up to
//! date: its description closes every task merged into the epic since the
//! last milestone, so GitLab closes their issues when it lands (develop is
//! the default branch). The epic branch stays: the orchestration goes on.

use super::common::{Argv, Here, Outcome, Takes, help, say};
use super::glab;
use super::mr::closed_by;
use breadboard_core::epic::epic_of;
use breadboard_core::git::issue_of_branch;
use breadboard_core::record::{is_name, record_of};
use serde_json::Value;
use std::path::Path;
use std::process::ExitCode;

const ABOUT: &str = "pnpm wt:epic: the epic of an orchestration into develop, at a milestone (#352)";
const USAGE: &str = "usage: pnpm wt:epic <orchestration> --mr [--dry-run]

  --mr        open the MR epic → develop, or bring its description up to date:
              it closes every task merged into the epic since the last milestone
  --dry-run   show the MR, send nothing";
const TAKES: Takes = Takes {
    valued: &[],
    switches: &["--mr", "--dry-run"],
    words: Some(1),
    usage: USAGE,
    command: "wt:epic",
};

/// A branch as a query parameter: `epic/15-x` → `epic%2F15-x`.
fn query(branch: &str) -> String {
    branch.replace('%', "%25").replace('/', "%2F").replace('&', "%26")
}

/// The issues of the tasks merged into the epic after `since` (the last
/// milestone's merge, None before the first), by their branches' numbers:
/// one issue per task MR. In order, each once.
fn issues_since(merged: &[Value], since: Option<&str>) -> Vec<u64> {
    let mut numbers: Vec<u64> = merged
        .iter()
        .filter(|mr| {
            let at = glab::text_of(mr, "merged_at");
            !at.is_empty() && since.is_none_or(|since| at > since)
        })
        .filter_map(|mr| issue_of_branch(glab::text_of(mr, "source_branch")))
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

/// The milestone MR's description: what it carries, then the issues it closes.
fn body_of(epic: &str, issues: &[u64]) -> String {
    let listed: Vec<String> = issues.iter().map(|number| format!("#{number}")).collect();
    format!(
        "The epic `{epic}` into develop at a milestone (#352): the tasks merged into it since the last one, each with its own MR into the epic.\n\nThe epic branch stays: the orchestration goes on with it.\n\nCloses {}\n",
        listed.join(", ")
    )
}

/// The MRs of `endpoint`'s list, or why GitLab didn't answer.
fn mrs_of(dir: &Path, endpoint: &str) -> Result<Vec<Value>, String> {
    let ran = glab::api(dir, endpoint);
    match ran.json() {
        Some(Value::Array(mrs)) if ran.is_ok => Ok(mrs),
        _ => Err(format!("glab could not read the merge requests: {}", ran.why())),
    }
}

pub fn run(argv: &Argv) -> Outcome {
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let Some(name) = argv.first() else {
        return Err(USAGE.into());
    };
    if !argv.has("--mr") {
        return Err(format!("say what to do with the epic of {name}: --mr\n{USAGE}"));
    }
    if !is_name(name) {
        return Err(format!(
            "{name} is no orchestration's name: names are kebab-case (pnpm wt:orchestrate <name> <issue>…)"
        ));
    }
    let here = Here::find();
    let Some(main) = here.main.as_deref() else {
        return Err("not in a git checkout of the repo".into());
    };
    let Some(record) = record_of(main, name) else {
        return Err(format!("no orchestration {name} is recorded"));
    };
    let Some(epic) = epic_of(&record.raw) else {
        return Err(format!(
            "{name} has no epic branch: its tasks go to develop one by one (pnpm wt:orchestrate {name} --epic <n>)"
        ));
    };
    let mrs = "projects/:fullpath/merge_requests";
    // The last milestone: the newest merge of the epic into develop.
    let milestones = mrs_of(
        main,
        &format!(
            "{mrs}?state=merged&source_branch={}&target_branch=develop&order_by=merged_at&sort=desc&per_page=1",
            query(&epic)
        ),
    )?;
    let since = milestones.first().map(|mr| glab::text_of(mr, "merged_at").to_string());
    let merged = mrs_of(
        main,
        &format!(
            "{mrs}?state=merged&target_branch={}&order_by=merged_at&sort=desc&per_page=100",
            query(&epic)
        ),
    )?;
    let issues = issues_since(&merged, since.as_deref().filter(|at| !at.is_empty()));
    if issues.is_empty() {
        return Err(format!(
            "no task was merged into {epic} since the last milestone: nothing to take to develop"
        ));
    }
    let body = body_of(&epic, &issues);
    debug_assert_eq!(closed_by(&body), issues);
    let listed = issues.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ");
    let open = mrs_of(
        main,
        &format!(
            "{mrs}?state=opened&source_branch={}&target_branch=develop&per_page=1",
            query(&epic)
        ),
    )?;
    if argv.has("--dry-run") {
        say(&format!(
            "\n{} the MR {epic} → develop, closing {listed}:\n\n{body}",
            if open.is_empty() { "would open" } else { "would update" }
        ));
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(mr) = open.first() {
        let iid = glab::shown(mr.get("iid"));
        let sent = glab::put_description(main, &format!("{mrs}/{iid}"), &body);
        if !sent.is_ok {
            return Err(format!("glab could not update !{iid}: {}", sent.why()));
        }
        say(&format!(
            "\n✓ !{iid} {epic} → develop brought up to date, closes {listed}\n  {}\n",
            glab::text_of(mr, "web_url")
        ));
        return Ok(ExitCode::SUCCESS);
    }
    let title = format!("chore(epic): {epic} into develop");
    let created = glab::send(
        main,
        "POST",
        mrs,
        &[
            glab::Field::Raw("source_branch", &epic),
            glab::Field::Raw("target_branch", "develop"),
            glab::Field::Raw("title", &title),
            glab::Field::Text("description", &body),
            // The epic goes on after its milestone.
            glab::Field::Typed("remove_source_branch", "false"),
        ],
    );
    if !created.is_ok {
        return Err(format!(
            "glab could not open the MR {epic} → develop: {}",
            created.why()
        ));
    }
    let url = created
        .json()
        .map(|mr| glab::text_of(&mr, "web_url").to_string())
        .unwrap_or_default();
    say(&format!(
        "\n✓ the MR {epic} → develop is open, closes {listed}\n  {url}\n"
    ));
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merged(branch: &str, at: &str) -> Value {
        json!({ "source_branch": branch, "merged_at": at })
    }

    #[test]
    fn the_issues_are_the_tasks_merged_since_the_last_milestone_each_once() {
        let mrs = [
            merged("feature/352-epic-keeps-up", "2026-10-12T10:00:00Z"),
            merged("fix/355-avatar", "2026-10-11T09:00:00Z"),
            merged("feature/349-base-ref", "2026-10-09T09:00:00Z"),
            // A second MR of one issue counts once.
            merged("feature/352-epic-keeps-up-2", "2026-10-12T11:00:00Z"),
            // A branch without a number closes nothing.
            merged("feature/no-number", "2026-10-12T12:00:00Z"),
        ];
        assert_eq!(issues_since(&mrs, Some("2026-10-10T00:00:00Z")), [352, 355]);
        assert_eq!(issues_since(&mrs, None), [349, 352, 355]);
    }

    #[test]
    fn the_description_closes_exactly_those_issues_as_gitlab_reads_it() {
        let body = body_of("epic/15-breadboard", &[349, 352, 355]);
        assert_eq!(closed_by(&body), [349, 352, 355]);
        assert!(body.contains("`epic/15-breadboard`"), "{body}");
    }

    #[test]
    fn a_branch_is_one_query_parameter() {
        assert_eq!(query("epic/15-breadboard"), "epic%2F15-breadboard");
    }
}
