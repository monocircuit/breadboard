//! `wt:batch <product> --fix` (#413): the fix of a red shipped batch. Its
//! issue names the batch MR, the failed jobs and the tasks merged into the
//! batch, the likeliest first with their files. The fix goes into the
//! product's current batch like any task, and that batch's ship replaces the
//! red one (#464). Queued in the product's orchestration: the overview hands
//! it to the free agent that knows those files, else to a fresh one.

use super::super::common::{Outcome, say};
use super::super::glab;
use crate::cli::lifecycle::proc::exe;
use breadboard_core::batch::{self, Shipping};
use breadboard_core::batch_red::{Suspect, covers, job_of, shipped_of, suspects};
use breadboard_core::git::git;
use serde_json::Value;
use std::path::Path;
use std::process::{Command, ExitCode};

/// At most this many files of the suspects in the issue: what the hand-out matches agents on.
const FILES: usize = 12;

/// A batch shipped before whose MR into develop failed its last pipeline.
struct Red {
    shipping: Shipping,
    /// The failed jobs, each once: its name and the page of its first copy.
    jobs: Vec<(String, String)>,
}

/// The oldest red batch of `product`, its failed jobs read; None while none is red.
fn red_of(main: &Path, product: &str) -> Option<Red> {
    batch::shipping_of(main, product).into_iter().find_map(|shipping| {
        let mr = glab::api(main, &format!("projects/:fullpath/merge_requests/{}", shipping.mr)).json()?;
        let pipeline = mr.get("head_pipeline")?;
        if glab::text_of(pipeline, "status") != "failed" {
            return None;
        }
        let id = pipeline.get("id")?.as_i64()?;
        let listed = glab::api(
            main,
            &format!("projects/:fullpath/pipelines/{id}/jobs?scope[]=failed&per_page=100"),
        )
        .json()
        .unwrap_or_default();
        Some(Red {
            shipping,
            jobs: jobs_of(listed.as_array().map(Vec::as_slice).unwrap_or_default()),
        })
    })
}

/// The failed jobs of GitLab's list, each job once (its copies are one), in the order they ran.
fn jobs_of(listed: &[Value]) -> Vec<(String, String)> {
    let mut listed: Vec<&Value> = listed.iter().collect();
    listed.sort_by_key(|job| job.get("id").and_then(Value::as_i64).unwrap_or(0));
    let mut jobs: Vec<(String, String)> = Vec::new();
    for job in listed {
        let name = job_of(glab::text_of(job, "name"));
        if !name.is_empty() && !jobs.iter().any(|(known, _)| known == name) {
            jobs.push((name.to_string(), glab::text_of(job, "web_url").to_string()));
        }
    }
    jobs
}

/// `fix(breadboard): batch/breadboard-2 red in check:vita`.
fn title_of(product: &str, red: &Red) -> String {
    let job = red.jobs.first().map_or("its pipeline", |(job, _)| job.as_str());
    format!("fix({product}): {} red in {job}", red.shipping.branch)
}

/// The issue's description: what is red, the suspects the likeliest first
/// with their files, the box that closes it; `into`: the current batch.
fn body_of(red: &Red, into: &str, suspects: &[Suspect], globs: &[(String, Vec<String>)]) -> String {
    let branch = &red.shipping.branch;
    let mr = red.shipping.mr;
    let jobs: Vec<String> = red
        .jobs
        .iter()
        .map(|(job, url)| {
            if url.is_empty() {
                format!("`{job}`")
            } else {
                format!("[`{job}`]({url})")
            }
        })
        .collect();
    let mut body = format!(
        "## Bug\n\nThe batch `{branch}` is red (#413): the pipeline of !{mr} into develop failed in {}.\n\n## Suspects, the likeliest first\n\n",
        if jobs.is_empty() {
            "a job".to_string()
        } else {
            jobs.join(", ")
        }
    );
    if suspects.is_empty() {
        body.push_str("None read: the batch's tasks weren't in this clone.\n");
    }
    let has_covered = suspects.iter().any(|suspect| suspect.is_covered);
    let mut named = 0;
    for suspect in suspects {
        let task = &suspect.merged;
        let mut line = match task.mr {
            Some(iid) => format!("- !{iid}"),
            None => format!("- {}", task.sha.chars().take(8).collect::<String>()),
        };
        if let Some(issue) = task.issue {
            line.push_str(&format!(" #{issue}"));
        }
        line.push_str(&format!(" {}", task.title));
        let covering = globs
            .iter()
            .find(|(_, globs)| task.files.iter().any(|file| covers(globs, file)));
        if let Some((job, _)) = covering {
            line.push_str(&format!(": its files run in `{job}`"));
        }
        // The files the hand-out matches agents on: the likeliest tasks', else every task's.
        if suspect.is_covered || !has_covered {
            let files: Vec<String> = task
                .files
                .iter()
                .take(FILES.saturating_sub(named))
                .map(|file| format!("`{file}`"))
                .collect();
            named += files.len();
            if !files.is_empty() {
                line.push_str(&format!(" ({})", files.join(", ")));
            }
        }
        body.push_str(&line);
        body.push('\n');
    }
    body.push_str(&format!(
        "\n## Scope\n\n- [ ] The fix of what fails in !{mr} goes into `{into}`, the current batch: its ship replaces !{mr}, which is closed then (#464)\n"
    ));
    body
}

/// The issue already filed for `branch`, while it is open.
fn filed_for(main: &Path, product: &str, branch: &str) -> Option<i64> {
    batch::fixes_of(main, product)
        .into_iter()
        .filter(|(_, of)| of == branch)
        .map(|(number, _)| number)
        .find(|number| {
            glab::api(main, &format!("projects/:fullpath/issues/{number}"))
                .json()
                .is_some_and(|issue| glab::text_of(&issue, "state") == "opened")
        })
}

/// The id of whoever runs it, for the issue's assignee.
fn me(main: &Path) -> Option<i64> {
    glab::api(main, "user").json()?.get("id")?.as_i64()
}

/// The milestone of the likeliest suspect's issue: the fix belongs where it does.
fn milestone_of(main: &Path, suspects: &[Suspect]) -> Option<i64> {
    let issue = suspects.iter().find_map(|suspect| suspect.merged.issue)?;
    let found = glab::api(main, &format!("projects/:fullpath/issues/{issue}")).json()?;
    found.get("milestone")?.get("id")?.as_i64()
}

pub fn run(main: &Path, product: &str, develop: &str, orchestration: Option<&str>, is_dry: bool) -> Outcome {
    let _ = git(main, &["fetch", "--quiet", "origin"]);
    let Some(red) = red_of(main, product) else {
        return Err(format!("no batch of {product} shipped before is red: nothing to fix"));
    };
    let branch = red.shipping.branch.clone();
    if let Some(number) = filed_for(main, product, &branch) {
        say(&format!("\n✓ #{number} fixes {branch} already\n"));
        return Ok(ExitCode::SUCCESS);
    }
    let shipped = shipped_of(main, &branch, develop).unwrap_or_default();
    let globs: Vec<(String, Vec<String>)> = red
        .jobs
        .iter()
        .filter_map(|(job, _)| Some((job.clone(), shipped.jobs.get(job)?.clone())))
        .collect();
    let given: Vec<&[String]> = globs.iter().map(|(_, globs)| globs.as_slice()).collect();
    let ranked = suspects(&shipped.tasks, &given);
    let Some(current) = batch::current_of(main, product) else {
        return Err(format!("{product} has no batch for the fix of {branch} to go into"));
    };
    let title = title_of(product, &red);
    let body = body_of(&red, &current.branch, &ranked, &globs);
    let name = orchestration.map(String::from).or_else(|| {
        breadboard_core::record::orchestrations_of_product(main, product)
            .into_iter()
            .next()
    });
    if is_dry {
        say(&format!(
            "\nwould file the issue {title}, queued in {}:\n\n{body}",
            name.as_deref().unwrap_or("no orchestration")
        ));
        return Ok(ExitCode::SUCCESS);
    }
    let me = me(main).map(|id| format!("[{id}]"));
    let milestone = milestone_of(main, &ranked).map(|id| id.to_string());
    let mut fields = vec![
        glab::Field::Raw("title", &title),
        glab::Field::Text("description", &body),
    ];
    if let Some(me) = &me {
        // A JSON array: glab 1.123 refuses a field name with brackets (#464).
        fields.push(glab::Field::Typed("assignee_ids", me));
    }
    if let Some(milestone) = &milestone {
        fields.push(glab::Field::Typed("milestone_id", milestone));
    }
    let created = glab::send(main, "POST", "projects/:fullpath/issues", &fields);
    let Some(number) = created
        .json()
        .and_then(|issue| issue.get("iid").and_then(Value::as_i64))
    else {
        return Err(format!("glab could not file the fix of {branch}: {}", created.why()));
    };
    batch::write_fix(main, product, number, &branch)
        .map_err(|error| format!("#{number} is filed, but {product}'s file not written: {error}"))?;
    say(&format!("\n✓ #{number} {title}"));
    let Some(name) = name else {
        say(&format!(
            "! no orchestration of {product}: queue it with pnpm wt:orchestrate <name> {number} --queue\n"
        ));
        return Ok(ExitCode::SUCCESS);
    };
    // Queued: the overview hands it to the free agent that knows its files, else to a fresh one.
    let queued = Command::new(exe())
        .args(["wt:orchestrate", &name, &number.to_string(), "--queue"])
        .current_dir(main)
        .status();
    match queued {
        Ok(status) if status.success() => {
            say(&format!(
                "✓ queued in {name}: to the agent that knows its files, else a fresh one, off {}\n",
                current.branch
            ));
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(format!(
            "#{number} is filed, but not queued: pnpm wt:orchestrate {name} {number} --queue"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::batch_red::Merged;
    use serde_json::json;

    fn red() -> Red {
        Red {
            shipping: Shipping {
                branch: "batch/breadboard-2".into(),
                mr: 500,
            },
            jobs: vec![("check:vita".into(), "https://g/p/-/jobs/7".into())],
        }
    }

    fn suspect(iid: i64, issue: u64, files: &[&str], is_covered: bool) -> Suspect {
        Suspect {
            merged: Merged {
                sha: format!("{iid:08}"),
                subject: String::new(),
                title: format!("feat: task {iid}"),
                mr: Some(iid),
                issue: Some(issue),
                files: files.iter().map(|file| file.to_string()).collect(),
            },
            is_covered,
        }
    }

    #[test]
    fn reads_each_failed_job_once_in_the_order_they_ran() {
        let listed = [
            json!({ "id": 9, "name": "build:image 2/2", "web_url": "u9" }),
            json!({ "id": 7, "name": "check:vita", "web_url": "u7" }),
            json!({ "id": 8, "name": "build:image 1/2", "web_url": "u8" }),
        ];
        assert_eq!(
            jobs_of(&listed),
            [
                ("check:vita".to_string(), "u7".to_string()),
                ("build:image".to_string(), "u8".to_string())
            ]
        );
    }

    #[test]
    fn the_issue_names_the_red_batch_its_jobs_and_the_likeliest_tasks_with_their_files() {
        let globs = vec![("check:vita".to_string(), vec!["products/vita/**/*".to_string()])];
        let ranked = [
            suspect(431, 411, &["products/vita/page.tsx"], true),
            suspect(440, 413, &["packages/breadboard/x.rs"], false),
        ];
        assert_eq!(
            title_of("breadboard", &red()),
            "fix(breadboard): batch/breadboard-2 red in check:vita"
        );
        let body = body_of(&red(), "batch/breadboard-3", &ranked, &globs);
        assert_eq!(
            body,
            "## Bug\n\nThe batch `batch/breadboard-2` is red (#413): the pipeline of !500 into develop failed in [`check:vita`](https://g/p/-/jobs/7).\n\n\
             ## Suspects, the likeliest first\n\n\
             - !431 #411 feat: task 431: its files run in `check:vita` (`products/vita/page.tsx`)\n\
             - !440 #413 feat: task 440\n\n\
             ## Scope\n\n- [ ] The fix of what fails in !500 goes into `batch/breadboard-3`, the current batch: its ship replaces !500, which is closed then (#464)\n"
        );
        // What the hand-out matches agents on: the likeliest task's files only.
        assert_eq!(breadboard_core::gitlab::paths_of(&body), ["products/vita/page.tsx"]);
        // None likelier: every task's files.
        let even = [suspect(440, 413, &["packages/breadboard/x.rs"], false)];
        assert!(
            body_of(&red(), "batch/breadboard-3", &even, &[])
                .contains("- !440 #413 feat: task 440 (`packages/breadboard/x.rs`)\n")
        );
    }
}
