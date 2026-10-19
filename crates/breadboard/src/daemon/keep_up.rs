//! The daemon keeps the main checkouts of its overviews on their default branch
//! (#348, core's `main_update`): one task per main checkout an overview
//! asked GitLab about, looking every LOOK_EVERY whether a look is due
//! (`[main_checkout] every_minutes` since the last, or a merge woke it), the
//! steps after an update right after it; nothing with `follow = false`.
//! The one thing the daemon writes besides its socket: a fast-forward of a
//! clean default branch, which no overview's read could undo. And (#352)
//! each orchestration's epic checkout takes develop in once develop moved
//! (core's `epic_sync`): merged, pushed when the epic's `wt:check` passes;
//! a conflict, a failed check or a checkout it can't touch is a `look`
//! action of the orchestration, never a force. A product's batch (#411)
//! takes develop in only on a conflict: the daemon opens the next batch once
//! one shipped, and says a conflict with develop (a hotfix went past the
//! batch) as a `look` action of the product's orchestrations. Once a
//! shipped batch's MR merged (#414), it closes the issues GitLab left open
//! and drops the batch from the product's `shipping`.

use breadboard_core::actions::{NewAction, add_action};
use breadboard_core::batch;
use breadboard_core::epic::epic_of;
use breadboard_core::epic_sync::{self, Synced, Tried};
use breadboard_core::git::{orchestrations_dir, orchestrator_checkouts};
use breadboard_core::main_update::{self, Look};
use breadboard_core::record::{record_file, record_of, recorded};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// How often it asks whether a look is due: a merge's wake shows within this.
const LOOK_EVERY: Duration = Duration::from_secs(5);

/// Keeps `main` on origin/develop while the daemon runs.
pub async fn keep_up(main: PathBuf) {
    let Some(store) = main_update::store_of(&main) else {
        return;
    };
    let mut tick = tokio::time::interval(LOOK_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        // Its repo gone or going (a gauntlet world removed): nothing to keep.
        if !main.join(".git").exists() {
            return;
        }
        let (main, store) = (main.clone(), store.clone());
        // The steps run here too: the next look waits for the build of this one's sources.
        let _ = tokio::task::spawn_blocking(move || {
            // breadboard.toml's `[main_checkout]`, read again on every tick: a switch shows within one.
            let settings = main_update::Settings::of(&main);
            if !settings.is_on {
                return;
            }
            if let Some(Look::Moved(_, files)) = main_update::look_if_due(&main, &store, &settings) {
                main_update::run_steps(&main, &store, &files, &settings);
            }
            keep_epics_up(&main, &store, "glab");
            keep_batches_up(&main, &store, "glab");
        })
        .await;
    }
}

/// Every epic of `main`'s orchestrations whose checkout lacks the develop
/// commit the clone knows (the main checkout's look fetched it): one try per
/// develop commit, kept in the store.
fn keep_epics_up(main: &Path, store: &Path, glab: &str) {
    let checkouts = orchestrator_checkouts(main);
    for name in recorded(main) {
        let Some(epic) = record_of(main, &name).and_then(|record| epic_of(&record.raw)) else {
            continue;
        };
        let checkout = orchestrations_dir(main).join(&name);
        let tried = epic_sync::read_tried(store, &name);
        if !checkouts.contains(&checkout) || !epic_sync::is_due(main, &tried) {
            continue;
        }
        // Another daemon of the clone at it: its try counts.
        let lock = breadboard_core::lock::lock_of(&epic_sync::tried_file(store, &name));
        if std::fs::create_dir(&lock).is_err() {
            continue;
        }
        let develop = epic_sync::develop_head(main).unwrap_or_default();
        // develop moved: maybe by the epic's milestone MR, whose issues are closed now.
        let milestone = close_milestone(main, &epic, glab, tried.milestone);
        let outcome = sync_epic(main, &name, &epic, &checkout);
        epic_sync::write_tried(
            store,
            &name,
            &Tried {
                develop,
                at: breadboard_core::time::now_ms(),
                outcome,
                milestone,
            },
        );
        let _ = std::fs::remove_dir(&lock);
    }
}

/// Every product's batch: the next opened once it shipped (#411), a
/// conflict with develop said once per pair of heads, and once develop
/// moved, the issues of a shipped batch's merged MR closed (#414).
fn keep_batches_up(main: &Path, store: &Path, glab: &str) {
    let loaded = breadboard_core::config::of(main);
    let develop = loaded.branch.clone();
    for product in loaded.config.products.keys() {
        if !breadboard_core::settings::of_product(main, Some(product)).batching {
            continue;
        }
        let Some(mut current) = batch::current_of(main, product) else {
            // The first batch opens when a task of the product needs it.
            continue;
        };
        if batch::is_shipped(main, &current, &develop) {
            match batch::ensure_current(main, product, &develop, false) {
                Ok(opened) => current = opened.batch().clone(),
                Err(_) => continue,
            }
        }
        let heads = batch::heads_of(main, &current.branch, &develop);
        let looked = batch::read_looked(store, product);
        let is_new = (&heads.batch, &heads.develop) != (&looked.batch, &looked.develop);
        if !is_new && !looked.is_closing {
            continue;
        }
        // develop moved, maybe by a shipped batch's MR: a failed close is tried on the next tick.
        let is_closing = !close_shipped(main, product, glab);
        if is_new && let Some(files) = batch::conflicts_with_develop(main, &current.branch, &develop) {
            for name in recorded(main) {
                if record_of(main, &name).and_then(|record| record.product).as_deref() != Some(product) {
                    continue;
                }
                let text = format!(
                    "{} conflicts with {develop} in {}: a hotfix went past it. Merge {develop} into it in .claude/orchestrations/{name} (git merge origin/{develop}), resolve, pnpm wt:check, git push origin HEAD:{}",
                    current.branch,
                    files.join(", "),
                    current.branch
                );
                let _ = add_action(
                    &record_file(main, &name),
                    NewAction {
                        kind: "look".into(),
                        text,
                        ..Default::default()
                    },
                );
            }
        }
        batch::write_looked(store, product, &batch::Looked { is_closing, ..heads });
    }
}

/// The issues of the epic's newest milestone MR (merged epic → develop)
/// closed where GitLab didn't close them by its `Closes #n` (#352); each
/// milestone once, `seen` the last one done. The newest milestone's iid,
/// `seen` when there is none or GitLab doesn't answer.
fn close_milestone(main: &Path, epic: &str, glab: &str, seen: i64) -> i64 {
    use super::glab::glab_get;
    let endpoint = format!(
        "projects/:fullpath/merge_requests?state=merged&source_branch={}&target_branch=develop&order_by=merged_at&sort=desc&per_page=1",
        epic.replace('/', "%2F")
    );
    let Ok(listed) = glab_get(main, glab, &endpoint) else {
        return seen;
    };
    let Some(mr) = listed.as_array().and_then(|mrs| mrs.first()) else {
        return seen;
    };
    let iid = mr.get("iid").and_then(serde_json::Value::as_i64).unwrap_or(0);
    if iid == 0 || iid == seen {
        return seen;
    }
    let description = mr.get("description").and_then(serde_json::Value::as_str).unwrap_or("");
    // Not all closed: tried again on the next look.
    if close_issues(main, glab, description) {
        iid
    } else {
        seen
    }
}

/// The issues `description`'s `Closes #n` names closed where GitLab left
/// them open (#352, #414); false when one couldn't be.
fn close_issues(main: &Path, glab: &str, description: &str) -> bool {
    use super::glab::{glab_get, glab_put};
    for number in crate::cli::write::mr::closed_by(description) {
        let issue = format!("projects/:fullpath/issues/{number}");
        let is_open = glab_get(main, glab, &issue)
            .ok()
            .is_some_and(|found| found.get("state").and_then(serde_json::Value::as_str) == Some("opened"));
        if is_open && glab_put(main, glab, &format!("{issue}?state_event=close")).is_err() {
            return false;
        }
    }
    true
}

/// The batches of `product` the user shipped (#414) whose MR into develop
/// GitLab says merged: what its `Closes #n` names closed where GitLab left
/// it open, then dropped from the product's file; one closed unmerged is
/// dropped too. False while one merged is left to try again.
fn close_shipped(main: &Path, product: &str, glab: &str) -> bool {
    use super::glab::glab_get;
    let shipping = batch::shipping_of(main, product);
    if shipping.is_empty() {
        return true;
    }
    let mut is_settled = true;
    let mut kept = Vec::new();
    for entry in &shipping {
        let Ok(mr) = glab_get(main, glab, &format!("projects/:fullpath/merge_requests/{}", entry.mr)) else {
            is_settled = false;
            kept.push(entry.clone());
            continue;
        };
        let text = |key: &str| mr.get(key).and_then(serde_json::Value::as_str).unwrap_or("");
        match text("state") {
            "merged" if close_issues(main, glab, text("description")) => {}
            "merged" => {
                is_settled = false;
                kept.push(entry.clone());
            }
            "closed" => {}
            _ => kept.push(entry.clone()),
        }
    }
    if kept.len() != shipping.len() && batch::write_shipping(main, product, &kept).is_err() {
        return false;
    }
    is_settled
}

/// develop into the epic `epic` of `name` in `checkout`: what came of it,
/// and for anything but `pushed` or `up to date` a look action saying why.
fn sync_epic(main: &Path, name: &str, epic: &str, checkout: &Path) -> String {
    let shown = format!(".claude/orchestrations/{name}");
    let (outcome, why) = match epic_sync::merge_develop(checkout, epic) {
        Synced::UpToDate => ("up to date".to_string(), None),
        Synced::Merged { .. } if !is_checked(checkout) => (
            "check failed".into(),
            Some(format!(
                "{epic} took develop in, but its wt:check failed: not pushed. Look in {shown} (pnpm wt:check), fix it there, push"
            )),
        ),
        Synced::Merged { .. } => match epic_sync::push(checkout, epic) {
            Ok(()) => ("pushed".into(), None),
            Err(error) => (
                "push failed".into(),
                Some(format!(
                    "{epic} took develop in, but its push failed ({error}): git push in {shown}"
                )),
            ),
        },
        Synced::Conflict { files } => (
            "conflict".into(),
            Some(format!(
                "{epic} conflicts with develop in {}: merge develop into it by hand in {shown} (git merge origin/develop), resolve, pnpm wt:check, git push",
                files.join(", ")
            )),
        ),
        Synced::Skipped { reason } => (
            format!("skipped: {reason}"),
            Some(format!("{epic} did not take develop in: {reason}")),
        ),
    };
    if let Some(text) = why {
        let _ = add_action(
            &record_file(main, name),
            NewAction {
                kind: "look".into(),
                text,
                ..Default::default()
            },
        );
    }
    outcome
}

/// Whether the epic checkout's `pnpm wt:check` passes: lint, typecheck and
/// test of what the epic changes against develop, as for any push.
fn is_checked(checkout: &Path) -> bool {
    Command::new(crate::cli::lifecycle::proc::exe())
        .arg("wt:check")
        .current_dir(checkout)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A fake remote, the main checkout on develop with `toml` as its
    /// breadboard.toml, and a colleague's clone that pushes to it.
    fn world(root: &Path, toml: &str) -> (PathBuf, PathBuf) {
        let origin = root.join("origin.git");
        let (main, other) = (root.join("platform"), root.join("other"));
        git(
            root,
            &[
                "init",
                "--quiet",
                "--bare",
                "--initial-branch=develop",
                origin.to_str().unwrap(),
            ],
        );
        git(
            root,
            &["init", "--quiet", "--initial-branch=develop", main.to_str().unwrap()],
        );
        git(&main, &["remote", "add", "origin", origin.to_str().unwrap()]);
        std::fs::write(main.join("README.md"), "# platform\n").unwrap();
        std::fs::write(main.join("breadboard.toml"), toml).unwrap();
        git(&main, &["add", "."]);
        git(&main, &["commit", "--quiet", "-m", "chore: begin"]);
        git(&main, &["push", "--quiet", "-u", "origin", "develop"]);
        git(
            root,
            &["clone", "--quiet", origin.to_str().unwrap(), other.to_str().unwrap()],
        );
        (main, other)
    }

    fn push(other: &Path, file: &str) {
        let path = other.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x\n").unwrap();
        git(other, &["add", "."]);
        git(other, &["commit", "--quiet", "-m", &format!("feat: {file}")]);
        git(other, &["push", "--quiet", "origin", "develop"]);
    }

    /// Waits up to `seconds` for `done`.
    fn until(seconds: u64, done: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
        while std::time::Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        done()
    }

    #[test]
    fn follows_develop_on_a_wake_builds_after_a_change_of_its_sources_and_leaves_a_dirty_one() {
        let root = PathBuf::from(format!("/tmp/bb-keep-up-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let (main, other) = world(&root, "[repo]\ndefault_branch = \"develop\"\n");
        // The main checkout's launcher, as the step runs it: it says it ran.
        let launcher = main.join(main_update::LAUNCHER);
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, "echo built >\"$(dirname \"$0\")/built\"\n").unwrap();
        git(&main, &["add", "."]);
        git(&main, &["commit", "--quiet", "-m", "chore: launcher"]);
        git(&main, &["push", "--quiet", "origin", "develop"]);
        git(&other, &["pull", "--quiet", "origin", "develop"]);
        let store = main_update::store_of(&main).unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.spawn(keep_up(main.clone()));
        // The first look comes at once: up to date.
        assert!(until(10, || main_update::read(&store).looked_at > 0));
        // A merge on origin, its wake: the main checkout follows within a look, and builds.
        push(&other, "packages/breadboard/crates/x.rs");
        main_update::wake(&main);
        assert!(until(15, || git(&main, &["rev-parse", "HEAD"])
            == git(&other, &["rev-parse", "HEAD"])));
        assert!(until(10, || launcher.with_file_name("built").exists()));
        let state = main_update::read(&store);
        assert_eq!(state.updated.as_ref().map(|update| update.commits), Some(1));
        assert!(until(5, || main_update::read(&store)
            .steps
            .iter()
            .all(|step| step.state == main_update::StepState::Done)));
        // A local change: never touched, the reason in the state.
        std::fs::write(main.join("README.md"), "mine\n").unwrap();
        let before = git(&main, &["rev-parse", "HEAD"]);
        push(&other, "b.md");
        std::thread::sleep(Duration::from_millis(20));
        main_update::wake(&main);
        assert!(until(15, || main_update::read(&store).skipped.is_some()));
        assert_eq!(git(&main, &["rev-parse", "HEAD"]), before);
        assert_eq!(std::fs::read_to_string(main.join("README.md")).unwrap(), "mine\n");
        drop(runtime);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_epic_that_conflicts_with_develop_is_left_as_it_was_and_said_once() {
        let root = PathBuf::from(format!("/tmp/bb-keep-up-epic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let (main, other) = world(&root, "[repo]\ndefault_branch = \"develop\"\n");
        git(&main, &["config", "user.name", "Test"]);
        git(&main, &["config", "user.email", "test@example.com"]);
        let epic = "epic/15-breadboard";
        git(
            &main,
            &["push", "--quiet", "origin", &format!("develop:refs/heads/{epic}")],
        );
        git(&main, &["fetch", "--quiet", "origin"]);
        let record = record_file(&main, "breadboard");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        let fields = serde_json::json!({ "issues": [], "epic": { "branch": epic, "number": 15 }, "base": epic });
        std::fs::write(&record, fields.to_string()).unwrap();
        let checkout = orchestrations_dir(&main).join("breadboard");
        let start = format!("origin/{epic}");
        git(
            &main,
            &[
                "worktree",
                "add",
                "--quiet",
                "--track",
                "-b",
                epic,
                checkout.to_str().unwrap(),
                &start,
            ],
        );
        // develop and the epic change one line each.
        let change = |branch: &str, text: &str| {
            git(&other, &["fetch", "--quiet", "origin"]);
            git(
                &other,
                &["checkout", "--quiet", "-B", branch, &format!("origin/{branch}")],
            );
            std::fs::write(other.join("README.md"), text).unwrap();
            git(&other, &["commit", "--quiet", "-am", "feat: readme"]);
            git(&other, &["push", "--quiet", "origin", branch]);
        };
        change("develop", "develop's\n");
        change(epic, "the epic's\n");
        git(&main, &["fetch", "--quiet", "origin"]);
        // Caught up with the task merged into the epic, then left there: develop aborted.
        let head = git(&main, &["rev-parse", &start]);
        let store = main_update::store_of(&main).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        keep_epics_up(&main, &store, "/nonexistent/glab");
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), head);
        assert_eq!(git(&checkout, &["status", "--porcelain"]), "");
        let looks = || -> Vec<String> {
            let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
            raw["actions"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter(|action| action["kind"] == "look")
                .map(|action| action["text"].as_str().unwrap_or("").to_string())
                .collect()
        };
        assert_eq!(looks().len(), 1, "{:?}", looks());
        assert!(
            looks()[0].contains("conflicts with develop in README.md"),
            "{:?}",
            looks()
        );
        assert_eq!(epic_sync::read_tried(&store, "breadboard").outcome, "conflict");
        // The same develop commit is not tried again.
        keep_epics_up(&main, &store, "/nonexistent/glab");
        assert_eq!(looks().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shipped_batch_opens_the_next_and_a_hotfix_conflict_is_said_once() {
        let root = PathBuf::from(format!("/tmp/bb-keep-up-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let toml = "[repo]\ndefault_branch = \"develop\"\n[products.breadboard]\npaths = [\"b\"]\n";
        let (main, other) = world(&root, toml);
        let record = record_file(&main, "breadboard");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, r#"{ "issues": [], "product": "breadboard" }"#).unwrap();
        git(&main, &["fetch", "--quiet", "origin"]);
        batch::ensure_current(&main, "breadboard", "develop", false).unwrap();
        let store = main_update::store_of(&main).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        let change = |branch: &str, file: &str, text: &str| {
            git(&other, &["fetch", "--quiet", "origin"]);
            git(
                &other,
                &["checkout", "--quiet", "-B", branch, &format!("origin/{branch}")],
            );
            std::fs::write(other.join(file), text).unwrap();
            git(&other, &["add", "."]);
            git(&other, &["commit", "--quiet", "-m", "feat: x"]);
            git(&other, &["push", "--quiet", "origin", branch]);
            git(&main, &["fetch", "--quiet", "origin"]);
        };
        // A task in the batch, a hotfix of the same line in develop: said once.
        change("batch/breadboard-1", "README.md", "the batch's\n");
        change("develop", "README.md", "the hotfix\n");
        let looks = || -> Vec<String> {
            let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
            raw["actions"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|action| action["text"].as_str().unwrap_or("").to_string())
                .collect()
        };
        keep_batches_up(&main, &store, "true");
        assert_eq!(looks().len(), 1, "{:?}", looks());
        assert!(
            looks()[0].starts_with("batch/breadboard-1 conflicts with develop in README.md"),
            "{:?}",
            looks()
        );
        keep_batches_up(&main, &store, "true");
        assert_eq!(looks().len(), 1, "the same heads: said once");
        // The batch shipped (develop took it in, resolved): the next opens.
        git(&other, &["fetch", "--quiet", "origin"]);
        git(&other, &["checkout", "--quiet", "-B", "develop", "origin/develop"]);
        let merged = Command::new("git")
            .args(["merge", "--quiet", "--no-edit", "origin/batch/breadboard-1"])
            .current_dir(&other)
            .output()
            .unwrap();
        assert!(!merged.status.success(), "it conflicts");
        std::fs::write(other.join("README.md"), "both\n").unwrap();
        git(&other, &["commit", "--quiet", "-am", "Merge the batch"]);
        git(&other, &["push", "--quiet", "origin", "develop"]);
        git(&main, &["fetch", "--quiet", "origin"]);
        keep_batches_up(&main, &store, "true");
        assert_eq!(
            batch::current_of(&main, "breadboard").map(|batch| batch.branch),
            Some("batch/breadboard-2".into())
        );
        assert_eq!(
            git(&main, &["rev-parse", "origin/batch/breadboard-2"]),
            git(&main, &["rev-parse", "origin/develop"])
        );
        assert_eq!(looks().len(), 1, "the new batch has develop");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_milestones_issues_gitlab_left_open_are_closed_once() {
        use std::os::unix::fs::PermissionsExt;
        let root = PathBuf::from(format!("/tmp/bb-keep-up-milestone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let log = root.join("glab.log");
        let glab = root.join("glab");
        let list = "api projects/:fullpath/merge_requests?state=merged&source_branch=epic%2F15-breadboard&target_branch=develop&order_by=merged_at&sort=desc&per_page=1";
        std::fs::write(
            &glab,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n  '{list}') echo '[{{\"iid\":400,\"description\":\"Closes #349, #352\"}}]' ;;\n  'api projects/:fullpath/issues/349') echo '{{\"state\":\"closed\"}}' ;;\n  'api projects/:fullpath/issues/352') echo '{{\"state\":\"opened\"}}' ;;\n  *) echo '{{}}' ;;\nesac\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
        let glab = glab.to_string_lossy().into_owned();
        assert_eq!(close_milestone(&root, "epic/15-breadboard", &glab, 0), 400);
        let calls = std::fs::read_to_string(&log).unwrap();
        // The one GitLab left open closed, the closed one left alone.
        assert!(
            calls.contains("api --method PUT projects/:fullpath/issues/352?state_event=close"),
            "{calls}"
        );
        assert!(!calls.contains("issues/349?state_event"), "{calls}");
        // The same milestone again: nothing more.
        std::fs::remove_file(&log).unwrap();
        assert_eq!(close_milestone(&root, "epic/15-breadboard", &glab, 400), 400);
        assert!(!std::fs::read_to_string(&log).unwrap().contains("PUT"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shipped_batch_s_merged_mr_closes_what_gitlab_left_open_and_goes() {
        use std::os::unix::fs::PermissionsExt;
        let root = PathBuf::from(format!("/tmp/bb-keep-up-shipped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let log = root.join("glab.log");
        let glab = root.join("glab");
        std::fs::write(
            &glab,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n  'api projects/:fullpath/merge_requests/500') echo '{{\"state\":\"merged\",\"description\":\"Closes #411, #413\"}}' ;;\n  'api projects/:fullpath/merge_requests/501') echo '{{\"state\":\"opened\"}}' ;;\n  'api projects/:fullpath/merge_requests/502') echo '{{\"state\":\"closed\"}}' ;;\n  'api projects/:fullpath/issues/411') echo '{{\"state\":\"closed\"}}' ;;\n  'api projects/:fullpath/issues/413') echo '{{\"state\":\"opened\"}}' ;;\n  *) echo '{{}}' ;;\nesac\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
        let glab = glab.to_string_lossy().into_owned();
        let entry = |number: i64, mr: i64| batch::Shipping {
            branch: format!("batch/breadboard-{number}"),
            mr,
        };
        batch::write_shipping(&root, "breadboard", &[entry(1, 500), entry(2, 501), entry(3, 502)]).unwrap();
        assert!(close_shipped(&root, "breadboard", &glab));
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(
            calls.contains("api --method PUT projects/:fullpath/issues/413?state_event=close"),
            "{calls}"
        );
        assert!(!calls.contains("issues/411?state_event"), "{calls}");
        // The merged one and the closed one go, the open one stays.
        assert_eq!(batch::shipping_of(&root, "breadboard"), [entry(2, 501)]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn leaves_the_main_checkout_alone_when_switched_off() {
        let root = PathBuf::from(format!("/tmp/bb-keep-up-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let toml = "[repo]\ndefault_branch = \"develop\"\n\n[main_checkout]\nfollow = false\n";
        let (main, other) = world(&root, toml);
        let store = main_update::store_of(&main).unwrap();
        let before = git(&main, &["rev-parse", "HEAD"]);
        push(&other, "a.md");
        main_update::wake(&main);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.spawn(keep_up(main.clone()));
        // Two ticks and more: no look, nothing moved, nothing to show.
        std::thread::sleep(LOOK_EVERY + Duration::from_secs(2));
        assert_eq!(git(&main, &["rev-parse", "HEAD"]), before);
        assert_eq!(main_update::read(&store).looked_at, 0);
        drop(runtime);
        let _ = std::fs::remove_dir_all(&root);
    }
}
