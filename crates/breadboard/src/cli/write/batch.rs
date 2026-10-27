//! `wt:batch <product> --ship` (#414): the product's batch goes into develop.
//! One MR batch → develop, opened or brought up to date: its description
//! lists every task MR merged into the batch and closes their issues, and
//! it merges with the merge commits of the tasks (no squash), by the user's
//! two clicks. The batch carries its changelog line, the next batch opens
//! off it at once and the MRs still on their way into it go into the next,
//! their worktrees' base with them (#464). A batch shipped before whose MR
//! is red (#413) had its fix go into this one: the ship replaces its MR
//! (closed, its tasks listed here, #464).

mod fix;

use super::common::{Argv, Here, Outcome, Takes, help, say};
use super::epic::{mrs_of, query};
use super::glab;
use super::mr::closed_by;
use breadboard_core::batch::{self, Batch, Shipping};
use breadboard_core::git::{Worktree, git, issue_of_branch};
use breadboard_core::record;
use serde_json::Value;
use std::path::Path;
use std::process::ExitCode;

const ABOUT: &str = "pnpm wt:batch: the batch of a product into develop (#414), the fix of a red one (#413)";
const USAGE: &str = "usage: pnpm wt:batch <product> --ship [--dry-run]
       pnpm wt:batch <product> --fix [--orchestration <name>] [--dry-run]

  --ship      open the MR batch → develop, or bring its description up to date:
              it lists every task MR merged into the batch and closes their
              issues; the next batch opens off it and takes the MRs still open;
              a red batch shipped before is replaced: its MR closed (#464)
  --fix       file the fix of a red shipped batch (#413): the issue names its
              failed jobs and the tasks in it, the likeliest first; queued in
              the product's orchestration (or <name>), off the current batch
  --dry-run   show the MR or the issue, send nothing";
const TAKES: Takes = Takes {
    valued: &["--orchestration"],
    switches: &["--ship", "--fix", "--dry-run"],
    words: Some(1),
    usage: USAGE,
    command: "wt:batch",
};

/// A task MR merged into the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Task {
    iid: i64,
    issue: Option<u64>,
    title: String,
}

/// The tasks of the batch's merged MRs, by their iids.
fn tasks_of(merged: &[Value]) -> Vec<Task> {
    let mut tasks: Vec<Task> = merged
        .iter()
        .filter(|mr| !glab::text_of(mr, "merged_at").is_empty())
        .map(|mr| Task {
            iid: mr.get("iid").and_then(Value::as_i64).unwrap_or(0),
            issue: issue_of_branch(glab::text_of(mr, "source_branch")),
            title: glab::text_of(mr, "title").to_string(),
        })
        .filter(|task| task.iid > 0)
        .collect();
    tasks.sort_by_key(|task| task.iid);
    tasks.dedup_by_key(|task| task.iid);
    tasks
}

/// The issues the tasks close, each once.
fn issues_of(tasks: &[Task]) -> Vec<u64> {
    let mut issues: Vec<u64> = tasks.iter().filter_map(|task| task.issue).collect();
    issues.sort_unstable();
    issues.dedup();
    issues
}

/// `!431 !433 !440`: the task MRs as the changelog and the summary name them.
fn mrs_line(tasks: &[Task]) -> String {
    tasks
        .iter()
        .map(|task| format!("!{}", task.iid))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The batch MR's description: the red MRs it replaces, every task MR, then the issues it closes.
fn body_of(batch: &Batch, develop: &str, tasks: &[Task], replaced: &[Shipping]) -> String {
    let listed: Vec<String> = tasks
        .iter()
        .map(|task| match task.issue {
            Some(issue) => format!("- !{} #{issue} {}", task.iid, task.title),
            None => format!("- !{} {}", task.iid, task.title),
        })
        .collect();
    let closes: Vec<String> = issues_of(tasks).iter().map(|number| format!("#{number}")).collect();
    let mut body = format!(
        "The batch `{}` into {develop} (#414): {} {}, each merged into the batch by its own MR, with its merge commit.\n\n{}\n",
        batch.branch,
        tasks.len(),
        if tasks.len() == 1 { "task" } else { "tasks" },
        listed.join("\n")
    );
    if !replaced.is_empty() {
        let listed: Vec<String> = replaced
            .iter()
            .map(|red| format!("!{} (`{}`)", red.mr, red.branch))
            .collect();
        body.push_str(&format!(
            "\nIt replaces {}, red (#413): its fix is in this batch, its tasks are listed above, and it is closed.\n",
            listed.join(", ")
        ));
    }
    if !closes.is_empty() {
        body.push_str(&format!("\nCloses {}\n", closes.join(", ")));
    }
    body
}

/// The changelog fragment of the shipped batch: where it goes and what it says.
fn changelog_of(product: &str, batch: &Batch, tasks: &[Task]) -> (String, String) {
    (
        format!(".changelog/batch-{product}-{}.md", batch.number),
        format!(
            "---\ntype: changed\narea: {product}\n---\n\nbatch {} shipped: {}\n",
            batch.number,
            mrs_line(tasks)
        ),
    )
}

/// The fragment on the batch as origin has it, committed when it isn't there yet or says less.
fn ensure_changelog(main: &Path, batch: &Batch, path: &str, text: &str) -> Result<bool, String> {
    let shown = git(main, &["show", &format!("refs/remotes/origin/{}:{path}", batch.branch)]);
    if shown.is_ok && shown.stdout == text {
        return Ok(false);
    }
    let message = format!("docs(changelog): {} shipped", batch.branch);
    batch::commit_file(main, &batch.branch, path, text, &message).map(|_| true)
}

/// The open MRs into `from` retargeted to `to`: the iids moved with their
/// source branches, and the lines of those that didn't.
fn retarget_open(main: &Path, from: &str, to: &str) -> (Vec<(i64, String)>, Vec<String>) {
    let mrs = "projects/:fullpath/merge_requests";
    let Ok(open) = mrs_of(
        main,
        &format!("{mrs}?state=opened&target_branch={}&per_page=100", query(from)),
    ) else {
        return (
            Vec::new(),
            vec![format!("the MRs into {from} not read: retarget them to {to}")],
        );
    };
    let (mut moved, mut failed) = (Vec::new(), Vec::new());
    for mr in &open {
        let iid = mr.get("iid").and_then(Value::as_i64).unwrap_or(0);
        if iid == 0 {
            continue;
        }
        let sent = glab::send(
            main,
            "PUT",
            &format!("{mrs}/{iid}"),
            &[glab::Field::Raw("target_branch", to)],
        );
        if sent.is_ok {
            moved.push((iid, glab::text_of(mr, "source_branch").to_string()));
        } else {
            failed.push(format!(
                "!{iid} not retargeted: `glab mr update {iid} --target-branch {to}`"
            ));
        }
    }
    (moved, failed)
}

/// The worktrees of `branches` whose base is still the shipped batch `from`:
/// they follow their MR into the next one (#464), as a push of theirs would (#411).
fn followers<'a>(
    trees: &'a [Worktree],
    branches: &[String],
    from: &str,
    base_of: impl Fn(&Path) -> Option<String>,
) -> Vec<&'a Worktree> {
    trees
        .iter()
        .filter(|tree| tree.branch.as_ref().is_some_and(|branch| branches.contains(branch)))
        .filter(|tree| base_of(&tree.dir).as_deref() == Some(from))
        .collect()
}

/// The moved MRs' worktrees based on `to` now: the lines of those not switched.
fn follow_moved(main: &Path, branches: &[String], from: &str, to: &str) -> Vec<String> {
    let trees = breadboard_core::git::worktrees(main);
    followers(&trees, branches, from, record::base_in)
        .into_iter()
        .filter_map(|tree| {
            record::write_base(&tree.dir, to)
                .err()
                .map(|error| format!("{}'s base not switched to {to}: {error}", tree.dir.display()))
        })
        .collect()
}

/// The batches of `product` shipped before `current` whose MR into develop
/// is open and failed its last pipeline (#413): the ship replaces them (#464).
fn reds_before(main: &Path, product: &str, current: &str) -> Vec<Shipping> {
    batch::shipping_of(main, product)
        .into_iter()
        .filter(|entry| entry.branch != current)
        .filter(|entry| {
            glab::api(main, &format!("projects/:fullpath/merge_requests/{}", entry.mr))
                .json()
                .is_some_and(|mr| is_red(&mr))
        })
        .collect()
}

/// An MR into develop that is open and whose last pipeline failed.
fn is_red(mr: &Value) -> bool {
    glab::text_of(mr, "state") == "opened"
        && mr
            .get("head_pipeline")
            .is_some_and(|pipeline| glab::text_of(pipeline, "status") == "failed")
}

/// The red batch MR `red` closed, `iid` named as what replaces it; Err the line why not.
fn close_replaced(main: &Path, red: &Shipping, iid: i64) -> Result<(), String> {
    let endpoint = format!("projects/:fullpath/merge_requests/{}", red.mr);
    let note =
        format!("Replaced by !{iid}: the fix of this red batch went into the next one, which ships its tasks (#464).");
    let _ = glab::send(
        main,
        "POST",
        &format!("{endpoint}/notes"),
        &[glab::Field::Text("body", &note)],
    );
    let closed = glab::send(main, "PUT", &endpoint, &[glab::Field::Raw("state_event", "close")]);
    if closed.is_ok {
        Ok(())
    } else {
        Err(format!(
            "!{} ({}) not closed: {}: `glab mr close {}`",
            red.mr,
            red.branch,
            closed.why(),
            red.mr
        ))
    }
}

pub fn run(argv: &Argv) -> Outcome {
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let Some(product) = argv.first() else {
        return Err(USAGE.into());
    };
    if !argv.has("--ship") && !argv.has("--fix") {
        return Err(format!(
            "say what to do with the batch of {product}: --ship or --fix\n{USAGE}"
        ));
    }
    let here = Here::find();
    let Some(main) = here.main.as_deref() else {
        return Err("not in a git checkout of the repo".into());
    };
    let loaded = breadboard_core::config::of(main);
    if !loaded.config.products.contains_key(product) {
        return Err(format!(
            "{product} is no product of breadboard.toml ([products.<name>])"
        ));
    }
    let develop = loaded.branch.clone();
    if argv.has("--fix") {
        return fix::run(
            main,
            product,
            &develop,
            argv.given("--orchestration"),
            argv.has("--dry-run"),
        );
    }
    let _ = git(main, &["fetch", "--quiet", "origin"]);
    let Some(current) = batch::current_of(main, product) else {
        return Err(format!("{product} has no batch yet: its first task opens one"));
    };
    let mrs = "projects/:fullpath/merge_requests";
    let merged = mrs_of(
        main,
        &format!(
            "{mrs}?state=merged&target_branch={}&order_by=merged_at&sort=desc&per_page=100",
            query(&current.branch)
        ),
    )?;
    let own = tasks_of(&merged);
    // A red batch shipped before (#413): its fix is in this one, which replaces it (#464).
    let reds = reds_before(main, product, &current.branch);
    let mut every = merged;
    for red in &reds {
        every.extend(mrs_of(
            main,
            &format!("{mrs}?state=merged&target_branch={}&per_page=100", query(&red.branch)),
        )?);
    }
    let tasks = tasks_of(&every);
    if tasks.is_empty() {
        return Err(format!(
            "no task is merged into {} yet: nothing to ship",
            current.branch
        ));
    }
    let body = body_of(&current, &develop, &tasks, &reds);
    debug_assert_eq!(closed_by(&body), issues_of(&tasks));
    // The red batches carry their own line already.
    let (path, text) = changelog_of(product, &current, if own.is_empty() { &tasks } else { &own });
    let open = mrs_of(
        main,
        &format!(
            "{mrs}?state=opened&source_branch={}&target_branch={}&per_page=1",
            query(&current.branch),
            query(&develop)
        ),
    )?;
    let next_name = batch::branch_of(product, current.number + 1);
    if argv.has("--dry-run") {
        say(&format!(
            "\n{} the MR {} → {develop} ({}), {path} in it, the next batch {next_name} off it{}:\n\n{body}",
            if open.is_empty() { "would open" } else { "would update" },
            current.branch,
            mrs_line(&tasks),
            reds.iter()
                .map(|red| format!(", closing !{}", red.mr))
                .collect::<String>()
        ));
        return Ok(ExitCode::SUCCESS);
    }
    ensure_changelog(main, &current, &path, &text)?;
    let (iid, url) = match open.first() {
        Some(mr) => {
            let iid = mr.get("iid").and_then(Value::as_i64).unwrap_or(0);
            let sent = glab::put_description(main, &format!("{mrs}/{iid}"), &body);
            if !sent.is_ok {
                return Err(format!("glab could not update !{iid}: {}", sent.why()));
            }
            (iid, glab::text_of(mr, "web_url").to_string())
        }
        None => {
            let title = format!("chore(batch): {} into {develop}", current.branch);
            let created = glab::send(
                main,
                "POST",
                mrs,
                &[
                    glab::Field::Raw("source_branch", &current.branch),
                    glab::Field::Raw("target_branch", &develop),
                    glab::Field::Raw("title", &title),
                    glab::Field::Text("description", &body),
                    // Each task keeps its merge commit (#411).
                    glab::Field::Typed("squash", "false"),
                    glab::Field::Typed("remove_source_branch", "false"),
                ],
            );
            if !created.is_ok {
                return Err(format!(
                    "glab could not open the MR {} → {develop}: {}",
                    current.branch,
                    created.why()
                ));
            }
            let mr = created.json().unwrap_or_default();
            (
                mr.get("iid").and_then(Value::as_i64).unwrap_or(0),
                glab::text_of(&mr, "web_url").to_string(),
            )
        }
    };
    // The red MRs it replaces are closed, and no longer on their way (#464).
    let mut replaced = Vec::new();
    let mut not_closed = Vec::new();
    for red in &reds {
        match close_replaced(main, red, iid) {
            Ok(()) => replaced.push(red),
            Err(line) => not_closed.push(line),
        }
    }
    let kept = batch::shipping_of(main, product);
    let mut shipping: Vec<Shipping> = kept
        .iter()
        .filter(|entry| !replaced.iter().any(|red| red.branch == entry.branch))
        .cloned()
        .collect();
    if !shipping.iter().any(|entry| entry.branch == current.branch) {
        shipping.push(Shipping {
            branch: current.branch.clone(),
            mr: iid,
        });
    }
    if shipping != kept {
        batch::write_shipping(main, product, &shipping)
            .map_err(|error| format!("!{iid} is open, but {product}'s file not written: {error}"))?;
    }
    let next = batch::open_next_off(main, product, &current)
        .map_err(|why| format!("!{iid} is open, but the next batch didn't open: {why}"))?;
    let (moved, mut failed) = retarget_open(main, &current.branch, &next.branch);
    failed.extend(not_closed);
    let branches: Vec<String> = moved.iter().map(|(_, branch)| branch.clone()).collect();
    failed.extend(follow_moved(main, &branches, &current.branch, &next.branch));
    let mut lines = vec![
        format!(
            "\n✓ !{iid} {} → {develop}: {} ({}), closes {}",
            current.branch,
            mrs_line(&tasks),
            if open.is_empty() {
                "opened"
            } else {
                "brought up to date"
            },
            issues_of(&tasks)
                .iter()
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!("  {url}"),
        format!("✓ the next batch {} is open off it", next.branch),
    ];
    lines.extend(
        replaced
            .iter()
            .map(|red| format!("✓ !{} {} closed: !{iid} replaces it", red.mr, red.branch)),
    );
    if !moved.is_empty() {
        let listed: Vec<String> = moved.iter().map(|(iid, _)| format!("!{iid}")).collect();
        lines.push(format!("✓ {} go into {} now", listed.join(" "), next.branch));
    }
    lines.extend(failed.into_iter().map(|line| format!("! {line}")));
    say(&(lines.join("\n") + "\n"));
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merged(iid: i64, branch: &str, title: &str) -> Value {
        json!({ "iid": iid, "source_branch": branch, "title": title, "merged_at": "2026-10-18T10:00:00Z" })
    }

    fn batch() -> Batch {
        Batch {
            branch: "batch/breadboard-3".into(),
            number: 3,
            from: None,
        }
    }

    #[test]
    fn a_red_batch_shipped_before_is_its_open_mr_with_a_failed_pipeline() {
        let mr = |state: &str, status: Option<&str>| match status {
            Some(status) => json!({ "state": state, "head_pipeline": { "status": status } }),
            None => json!({ "state": state }),
        };
        assert!(is_red(&mr("opened", Some("failed"))));
        assert!(!is_red(&mr("opened", Some("running"))));
        assert!(!is_red(&mr("opened", None)));
        assert!(!is_red(&mr("closed", Some("failed"))), "replaced already");
    }

    #[test]
    fn the_tasks_are_the_merged_mrs_in_order_each_once() {
        let mrs = [
            merged(440, "feature/413-red-batch", "feat(breadboard): a red batch"),
            merged(431, "feature/411-rolling-batch", "feat(breadboard): rolling batches"),
            merged(431, "feature/411-rolling-batch", "feat(breadboard): rolling batches"),
            merged(442, "chore/no-number", "chore: tidy"),
            json!({ "iid": 450, "source_branch": "feature/450-x", "merged_at": null }),
        ];
        let tasks = tasks_of(&mrs);
        assert_eq!(tasks.iter().map(|task| task.iid).collect::<Vec<_>>(), [431, 440, 442]);
        assert_eq!(issues_of(&tasks), [411, 413]);
        assert_eq!(mrs_line(&tasks), "!431 !440 !442");
    }

    #[test]
    fn the_description_lists_every_task_mr_and_closes_their_issues() {
        let tasks = tasks_of(&[
            merged(431, "feature/411-rolling-batch", "feat(breadboard): rolling batches"),
            merged(442, "chore/no-number", "chore: tidy"),
        ]);
        let body = body_of(&batch(), "develop", &tasks, &[]);
        assert!(
            body.contains("- !431 #411 feat(breadboard): rolling batches\n"),
            "{body}"
        );
        assert!(body.contains("- !442 chore: tidy\n"), "{body}");
        assert!(body.contains("`batch/breadboard-3` into develop"), "{body}");
        assert!(!body.contains("replaces"), "{body}");
        assert_eq!(closed_by(&body), [411]);
        // A red batch shipped before (#464): replaced, its tasks among them.
        let red = Shipping {
            branch: "batch/breadboard-2".into(),
            mr: 500,
        };
        let body = body_of(&batch(), "develop", &tasks, &[red]);
        assert!(
            body.contains("\nIt replaces !500 (`batch/breadboard-2`), red (#413): its fix is in this batch, its tasks are listed above, and it is closed.\n"),
            "{body}"
        );
        assert_eq!(closed_by(&body), [411]);
    }

    #[test]
    fn the_worktrees_of_moved_mrs_follow_them_off_the_shipped_batch() {
        let tree = |dir: &str, branch: Option<&str>| Worktree {
            dir: dir.into(),
            branch: branch.map(String::from),
        };
        let trees = [
            tree("/p", Some("develop")),
            tree("/p-1-a", Some("feature/1-a")),
            tree("/p-2-b", Some("feature/2-b")),
            tree("/p-3-c", Some("feature/3-c")),
            tree("/p-4-d", None),
        ];
        let moved = ["feature/1-a".to_string(), "feature/3-c".to_string()];
        let base_of = |dir: &Path| match dir.to_str() {
            Some("/p-1-a" | "/p-2-b") => Some("batch/breadboard-6".to_string()),
            Some("/p-3-c") => Some("epic/15-x".to_string()),
            _ => None,
        };
        let found: Vec<&str> = followers(&trees, &moved, "batch/breadboard-6", base_of)
            .iter()
            .filter_map(|tree| tree.dir.to_str())
            .collect();
        assert_eq!(found, ["/p-1-a"]);
    }

    #[test]
    fn the_changelog_says_the_batch_shipped_with_its_mrs() {
        let tasks = tasks_of(&[merged(431, "feature/411-a", "a"), merged(440, "feature/413-b", "b")]);
        let (path, text) = changelog_of("breadboard", &batch(), &tasks);
        assert_eq!(path, ".changelog/batch-breadboard-3.md");
        assert_eq!(
            text,
            "---\ntype: changed\narea: breadboard\n---\n\nbatch 3 shipped: !431 !440\n"
        );
    }
}
