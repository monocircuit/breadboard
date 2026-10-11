//! The daemon keeps the main checkouts of its overviews on origin/develop
//! (#348, core's `main_update`): one task per main checkout an overview
//! asked GitLab about, looking every LOOK_EVERY whether a look is due (EVERY
//! since the last, or a merge woke it), the steps after an update right
//! after it. The one thing the daemon writes besides its socket: a
//! fast-forward of a clean develop, which no overview's read could undo.

use breadboard_core::main_update::{self, Look};
use std::path::PathBuf;
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
        // Its repo gone (a gauntlet world removed): nothing to keep.
        if !main.is_dir() {
            return;
        }
        let (main, store) = (main.clone(), store.clone());
        // The steps run here too: the next look waits for the build of this one's sources.
        let _ = tokio::task::spawn_blocking(move || {
            if let Some(Look::Moved(_, files)) = main_update::look_if_due(&main, &store) {
                main_update::run_steps(&main, &store, &files);
            }
        })
        .await;
    }
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

    /// A fake remote, the main checkout on develop and a colleague's clone that pushes to it.
    fn world(root: &Path) -> (PathBuf, PathBuf) {
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
        let (main, other) = world(&root);
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
}
