//! `breadboard hook pre-push` (git-hooks/pre-push.mjs, push-guard.mjs,
//! check.mjs `checkGate`): the git pre-push hook in Rust. It refuses a push
//! to the branch of a merged or closed MR (#184) and one of a commit
//! `pnpm wt:check` didn't pass (#298, the stamp in `.claude/checked.json`).
//! Same texts as the Node hook: an agent reads them and acts on them. It
//! also sets the worktree's pushes to skip CI (#357, `ci::set_push_option`).

use super::check::{self, Failure};
use super::gitlab;
use super::proc::{errln, fail, git};
use breadboard_core::ci::PushOption;
use breadboard_core::json::str_of;
use breadboard_core::time::{now_ms, to_iso};
use serde_json::{Map, Value, json};
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

pub const USAGE: &str = "usage: breadboard hook pre-push <remote> <url>   (git runs it, the refs on stdin)";

/// What a push updates, from the lines git hands a pre-push hook (`<local
/// ref> <local sha> <remote ref> <remote sha>`): `(branch, sha)`. A deletion
/// updates nothing, and a tag is no branch.
pub fn pushed_refs(input: &str) -> Vec<(String, String)> {
    input
        .split('\n')
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let (sha, remote) = (*parts.get(1)?, *parts.get(2)?);
            let branch = remote.strip_prefix("refs/heads/")?;
            (!sha.bytes().all(|byte| byte == b'0')).then(|| (branch.to_string(), sha.to_string()))
        })
        .collect()
}

/// Why a push to `branch` is refused, or None. `mrs`: its merge requests,
/// newest first. An open one, or none yet, lets the push through; otherwise
/// the newest decides.
pub fn refusal_of(branch: &str, mrs: &[Value]) -> Option<String> {
    if mrs.is_empty() || mrs.iter().any(|mr| str_of(mr, "state") == Some("opened")) {
        return None;
    }
    let last = &mrs[0];
    let state = if str_of(last, "state") == Some("merged") {
        "merged"
    } else {
        "closed"
    };
    let iid = last.get("iid").map(Value::to_string).unwrap_or_default();
    let text = format!(
        "✗ push refused: !{iid} of {branch} is {state}, so no merge request points at this branch any more (#184).
  The push would recreate {branch} on origin, and what it carries would reach no one.
  New work goes on a new branch: file a follow-up issue and start it with pnpm wt:new <n>-<feature>.
  {}",
        str_of(last, "web_url").unwrap_or_default()
    );
    Some(text.trim_end().to_string())
}

/// The refusals of the MR guard. GitLab not answering lets the push through
/// with a warning: the guard must not block offline work.
fn check_push(refs: &[(String, String)]) -> Vec<String> {
    let mut branches: Vec<&str> = Vec::new();
    for (branch, _) in refs {
        if !branches.contains(&branch.as_str()) {
            branches.push(branch);
        }
    }
    let mut refusals = Vec::new();
    for branch in branches {
        let endpoint = format!(
            "projects/:fullpath/merge_requests?state=all&source_branch={}&order_by=created_at&sort=desc",
            gitlab::encode(branch)
        );
        match gitlab::list(&endpoint) {
            Ok(mrs) => refusals.extend(refusal_of(branch, &mrs)),
            Err(failure) => errln!(
                "! push guard: couldn't look up the merge request of {branch}, pushing unchecked ({})",
                failure.reason
            ),
        }
    }
    refusals
}

/// What turbo would check of a commit, or why it can't tell.
pub type Plan = Result<Vec<(String, Vec<String>)>, String>;

/// What the check gate knows of a pushed commit.
pub struct Pushed<'a> {
    pub branch: &'a str,
    pub sha: &'a str,
    pub tree: Option<&'a str>,
    pub record: &'a Map<String, Value>,
    /// None when not asked (nothing changed, or stamped).
    pub plan: Option<&'a Plan>,
    /// Nothing changed against develop (a claim, wt:new).
    pub is_empty: bool,
}

/// The tree of a record's `passed` / `failed`.
fn tree_of_part<'a>(record: &'a Map<String, Value>, part: &str) -> Option<&'a str> {
    record
        .get(part)
        .and_then(|part| str_of(part, "tree"))
        .filter(|tree| !tree.is_empty())
}

/// Why the push of a commit isn't checked (#298), or None.
pub fn gate_refusal_of(pushed: &Pushed) -> Option<String> {
    let Pushed {
        branch,
        sha,
        tree,
        record,
        plan,
        is_empty,
    } = pushed;
    if *is_empty {
        return None;
    }
    if tree.is_some() && tree_of_part(record, "passed") == *tree {
        return None;
    }
    if let Some(Ok(planned)) = plan
        && planned.is_empty()
    {
        return None;
    }
    let short: String = sha.chars().take(8).collect();
    let why = if tree.is_some() && tree_of_part(record, "failed") == *tree {
        let failures: Vec<Failure> = record
            .get("failed")
            .and_then(|failed| failed.get("failures"))
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(failure_of).collect())
            .unwrap_or_default();
        format!(
            "its last check failed: {}.
  Fix it, commit, run pnpm wt:check again. Red on develop too: it is fixed once, in its own fix MR,
  and you wait for that with pnpm wt:wait !<mr>.",
            breadboard_core::check::failures_text(&failures)
        )
    } else if let Some(Err(error)) = plan {
        format!(
            "which packages it changes can't be told ({error}).
  Run pnpm install, then pnpm wt:check."
        )
    } else if let Some(passed) = record.get("passed") {
        let checked: String = str_of(passed, "sha").unwrap_or_default().chars().take(8).collect();
        format!(
            "the last check that passed was of {checked}, not of this commit.
  Run pnpm wt:check (it runs only what changed, a second run is quick)."
        )
    } else {
        let packages = match plan {
            Some(Ok(planned)) => planned
                .iter()
                .map(|(package, _)| package.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            _ => "the packages it changes".to_string(),
        };
        format!(
            "no check of it ran in this worktree yet.
  Run pnpm wt:check: lint, typecheck and test of {packages}."
        )
    };
    Some(format!(
        "✗ push refused: {branch} at {short} isn't checked (#298), {why}
  Push once it passed. Never --no-verify: a failing check is fixed, not pushed."
    ))
}

fn failure_of(value: &Value) -> Option<Failure> {
    Some(Failure {
        package: str_of(value, "package")?.to_string(),
        task: str_of(value, "task").unwrap_or_default().to_string(),
        tests: value
            .get("tests")
            .and_then(Value::as_array)
            .map(|tests| tests.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
    })
}

/// The refusals of the check gate. A docs-only commit passes, and is stamped
/// when it is HEAD: the overview shows it tested.
fn check_gate(refs: &[(String, String)], dir: &Path) -> Vec<String> {
    let record = check::read_check(dir);
    let text_of = |args: &[&str]| Some(git(args, dir).text().to_string()).filter(|text| !text.is_empty());
    let tree_of = |rev: &str| text_of(&["rev-parse", "--verify", "--quiet", &format!("{rev}^{{tree}}")]);
    let mut refusals = Vec::new();
    // Empty against the worktree's base: the claim of a task off an epic is empty against the epic (#349).
    let remote = breadboard_core::git::base_of(dir).remote;
    for (branch, sha) in refs {
        let tree = tree_of(sha);
        let base = text_of(&["merge-base", &remote, sha]);
        let is_empty = base
            .as_ref()
            .is_some_and(|base| git(&["diff", "--quiet", base, sha], dir).is_ok);
        let is_stamped = tree.is_some() && tree_of_part(&record, "passed") == tree.as_deref();
        let plan: Option<Plan> = (!is_empty && !is_stamped).then(|| match (&base, check::turbo_of(dir)) {
            (None, _) => Err(format!("no merge base with {remote}: git fetch origin")),
            (Some(_), None) => Err("turbo isn't installed here: pnpm install first".to_string()),
            (Some(base), Some(turbo)) => {
                check::scope_of(&turbo, dir, base, Some(sha)).and_then(|scope| check::plan_of(&turbo, dir, &scope))
            }
        });
        let pushed = Pushed {
            branch,
            sha,
            tree: tree.as_deref(),
            record: &record,
            plan: plan.as_ref(),
            is_empty,
        };
        if let Some(refusal) = gate_refusal_of(&pushed) {
            refusals.push(refusal);
            continue;
        }
        let is_docs_only = matches!(&plan, Some(Ok(planned)) if planned.is_empty());
        if is_docs_only && tree.is_some() && tree == tree_of("HEAD") {
            check::write_check(
                dir,
                json!({ "passed": { "sha": sha, "tree": tree, "packages": [], "at": to_iso(now_ms()), "duration": 0 } }),
            );
        }
    }
    refusals
}

/// `breadboard hook pre-push <remote> <url>`: the refs on stdin; exit 1 with
/// every refusal on stderr, else 0.
pub fn command(words: &[String]) -> ExitCode {
    match words.first().map(String::as_str) {
        Some("pre-push") => {}
        Some(other) => fail(&format!("no hook {other}: breadboard hook pre-push\n{USAGE}")),
        None => fail(&format!("which hook? breadboard hook pre-push\n{USAGE}")),
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let refs = pushed_refs(&input);
    if refs.is_empty() {
        return ExitCode::SUCCESS;
    }
    let dir = git(&["rev-parse", "--show-toplevel"], &super::repo::cwd())
        .text()
        .to_string();
    let mut refusals = check_push(&refs);
    // git read this push's options before the hook ran: a change counts from the next push.
    match breadboard_core::ci::set_push_option(Path::new(&dir), now_ms()) {
        Some(PushOption::Set) => errln!("breadboard: pushes skip CI from the next one on, breadboard's queue starts the pipelines (#357)"),
        // This push would skip CI with no queue to start its pipeline: once more, without the option.
        Some(PushOption::Dropped(why)) => refusals.push(format!(
            "✗ push refused: it would skip CI, and {why} (#357).\n  The option is dropped now: push again, and GitLab starts the pipeline itself."
        )),
        None => {}
    }
    refusals.extend(check_gate(&refs, Path::new(&dir)));
    if refusals.is_empty() {
        return ExitCode::SUCCESS;
    }
    errln!("{}", refusals.join("\n\n"));
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_a_push_updates() {
        let zero = "0".repeat(40);
        let input = format!(
            "refs/heads/a 1111 refs/heads/feature/1-a {zero}\nrefs/heads/b {zero} refs/heads/fix/2-b 2222\nrefs/tags/v1 3333 refs/tags/v1 {zero}\n"
        );
        assert_eq!(
            pushed_refs(&input),
            vec![("feature/1-a".to_string(), "1111".to_string())]
        );
    }

    #[test]
    fn refuses_a_branch_whose_mr_is_merged() {
        let merged = json!({ "iid": 7, "state": "merged", "web_url": "https://g/mr/7" });
        let open = json!({ "iid": 8, "state": "opened" });
        assert_eq!(refusal_of("feature/1-a", &[]), None);
        assert_eq!(refusal_of("feature/1-a", &[merged.clone(), open]), None);
        let text = refusal_of("feature/1-a", &[merged]).unwrap();
        assert!(text.starts_with("✗ push refused: !7 of feature/1-a is merged, so no merge request"));
        assert!(text.ends_with("pnpm wt:new <n>-<feature>.\n  https://g/mr/7"));
    }

    #[test]
    fn lets_a_stamped_tree_through_and_says_why_not() {
        let record: Map<String, Value> = serde_json::from_value(json!({
            "passed": { "sha": "aaaaaaaaaa", "tree": "t1" },
            "failed": { "tree": "t2", "failures": [{ "package": "@m/x", "task": "test", "tests": ["a > b"] }] }
        }))
        .unwrap();
        let pushed = |tree, plan| Pushed {
            branch: "feature/1-a",
            sha: "bbbbbbbbbbbb",
            tree: Some(tree),
            record: &record,
            plan,
            is_empty: false,
        };
        assert_eq!(gate_refusal_of(&pushed("t1", None)), None);
        let docs: Plan = Ok(Vec::new());
        assert_eq!(gate_refusal_of(&pushed("t3", Some(&docs))), None);
        let failed = gate_refusal_of(&pushed("t2", None)).unwrap();
        assert!(failed.starts_with(
            "✗ push refused: feature/1-a at bbbbbbbb isn't checked (#298), its last check failed: @m/x (a > b)."
        ));
        let planned: Plan = Ok(vec![("@m/x".to_string(), vec!["test".to_string()])]);
        let older = gate_refusal_of(&pushed("t3", Some(&planned))).unwrap();
        assert!(older.contains("the last check that passed was of aaaaaaaa, not of this commit."));
        assert!(older.ends_with("Never --no-verify: a failing check is fixed, not pushed."));
    }
}
