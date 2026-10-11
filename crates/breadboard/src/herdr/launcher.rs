//! herdr/breadboard.sh, the launcher everything starts the binary through:
//! the root package.json's scripts, the plugin's manifest, the tab bar's
//! status, the hooks' shims, `--reload` after a pull. It makes the binary of
//! its checkout first when it is missing or older than the checkout's HEAD
//! (from the repo's store when a checkout built those sources before, else
//! with cargo), so an upgrade or a pull by hand never leaves a script, a key,
//! the tab bar or a hook on a binary that isn't there or runs the code from
//! before (rewrite-log).

use std::path::Path;
use std::process::{Command, Stdio};

/// `sh <launcher> --built`: whether the binary beside it is that of its
/// checkout's HEAD now (built when it wasn't, its progress on this
/// terminal's stderr; its own lines say why not).
pub fn built(launcher: &Path) -> bool {
    launcher.is_file()
        && Command::new("sh")
            .arg(launcher)
            .arg("--built")
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};
    use std::time::{Duration, Instant};

    const LAUNCHER: &str = include_str!("../../../../herdr/breadboard.sh");
    const CARGO_SH: &str = include_str!("../../../../herdr/cargo.sh");

    /// A cargo that "builds" a binary saying `<CARGO_SAYS or ran> <args>`
    /// into `CARGO_TARGET_DIR` (else the package's target/), each call logged
    /// with its jobs and that dir; one that fails with `CARGO_FAIL` set. Only
    /// the tools the launcher has (no dirname: a short PATH).
    const CARGO: &str = r#"#!/bin/sh
echo "$* jobs=${CARGO_BUILD_JOBS:-} in ${CARGO_TARGET_DIR:-}" >>"$CARGO_LOG"
[ -n "$CARGO_FAIL" ] && exit 101
for arg; do case $arg in */Cargo.toml) package=${arg%/*} ;; esac; done
target=${CARGO_TARGET_DIR:-$package/target}
mkdir -p "$target/release"
printf '#!/bin/sh\necho "%s $*"\n' "${CARGO_SAYS:-ran}" >"$target/release/breadboard"
chmod +x "$target/release/breadboard"
"#;

    struct Checkout {
        root: PathBuf,
        package: PathBuf,
        bin: PathBuf,
    }

    impl Checkout {
        fn new(name: &str) -> Checkout {
            let root = std::env::temp_dir().join(format!("launcher-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let package = root.join("repo/packages/breadboard");
            let bin = root.join("bin");
            for (file, text) in [
                ("herdr/breadboard.sh", LAUNCHER),
                ("herdr/cargo.sh", CARGO_SH),
                ("Cargo.toml", "[workspace]\n"),
                ("Cargo.lock", ""),
                ("crates/core/lib.rs", "1\n"),
                ("hooks/pre-push", "#!/bin/sh\n"),
                ("docs/log.md", "1\n"),
            ] {
                fs::create_dir_all(package.join(file).parent().unwrap()).unwrap();
                fs::write(package.join(file), text).unwrap();
            }
            fs::create_dir_all(&bin).unwrap();
            // The tools the launcher runs, without a cargo (the system's may be in /usr/bin).
            let tools = root.join("tools");
            fs::create_dir_all(&tools).unwrap();
            for tool in [
                "sh", "git", "cat", "mkdir", "rm", "setsid", "chmod", "cp", "mv", "ls", "tail", "sleep", "nice",
                "getconf", "flock",
            ] {
                if let Some(found) = ["/usr/bin", "/bin"]
                    .iter()
                    .map(|dir| Path::new(dir).join(tool))
                    .find(|file| file.is_file())
                {
                    std::os::unix::fs::symlink(found, tools.join(tool)).unwrap();
                }
            }
            fs::write(bin.join("cargo"), CARGO).unwrap();
            fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
            let checkout = Checkout { root, package, bin };
            checkout.git(&["init", "--quiet", "--initial-branch=develop"]);
            fs::write(checkout.root.join("repo/.gitignore"), "target/\n").unwrap();
            checkout.commit();
            checkout
        }

        fn git(&self, args: &[&str]) {
            let done = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(self.root.join("repo"))
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env("GIT_CEILING_DIRECTORIES", &self.root)
                .output()
                .unwrap();
            assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
        }

        fn commit(&self) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "--quiet", "-m", "x"]);
        }

        fn change(&self, file: &str) {
            let path = self.package.join(file);
            let text = fs::read_to_string(&path).unwrap();
            fs::write(&path, format!("{text}1\n")).unwrap();
            self.commit();
        }

        fn binary(&self) -> PathBuf {
            self.package.join("target/release/breadboard")
        }

        /// `sh herdr/breadboard.sh <args>` from the plugin's directory, as
        /// herdr runs it; cargo only when `has_cargo`.
        fn run(&self, args: &[&str], has_cargo: bool, env: &[(&str, &str)]) -> Output {
            let tools = self.root.join("tools");
            let path = if has_cargo {
                format!("{}:{}", self.bin.display(), tools.display())
            } else {
                tools.display().to_string()
            };
            Command::new("sh")
                .arg("breadboard.sh")
                .args(args)
                .current_dir(self.package.join("herdr"))
                .env_clear()
                .env("PATH", path)
                // Nobody's ~/.cargo/bin: only the fake cargo builds.
                .env("HOME", &self.root)
                .env("CARGO_LOG", self.root.join("cargo.log"))
                .env("GIT_CEILING_DIRECTORIES", &self.root)
                .envs(env.iter().copied())
                .stdin(Stdio::null())
                .output()
                .unwrap()
        }

        fn builds(&self) -> usize {
            fs::read_to_string(self.root.join("cargo.log"))
                .map(|log| log.lines().count())
                .unwrap_or(0)
        }
    }

    impl Drop for Checkout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn out(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn err(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    }

    #[test]
    fn builds_a_missing_or_older_binary_before_it_runs_it_and_only_then() {
        let checkout = Checkout::new("fresh");
        // Missing (an upgrade from the Node plugin): built, then run.
        let first = checkout.run(&["herdr", "open", "overview"], true, &[]);
        assert!(first.status.success(), "{}", err(&first));
        assert_eq!(out(&first), "ran herdr open overview");
        assert!(err(&first).contains("▸ breadboard is built from"), "{}", err(&first));
        assert_eq!(checkout.builds(), 1);
        let calls = fs::read_to_string(checkout.root.join("cargo.log")).unwrap();
        assert!(calls.starts_with("build --release --manifest-path "), "{calls}");
        // Built from HEAD: no second build.
        assert_eq!(out(&checkout.run(&["herdr", "status"], true, &[])), "ran herdr status");
        assert_eq!(checkout.builds(), 1);
        // A pull that changes no source of it: none either.
        checkout.change("docs/log.md");
        checkout.run(&["x"], true, &[]);
        assert_eq!(checkout.builds(), 1);
        // A pull that changes the crates or the hooks it includes: built again.
        checkout.change("crates/core/lib.rs");
        assert_eq!(out(&checkout.run(&["x"], true, &[])), "ran x");
        assert_eq!(checkout.builds(), 2);
        checkout.change("hooks/pre-push");
        assert!(checkout.run(&["--built"], true, &[]).status.success());
        assert_eq!(checkout.builds(), 3);
        assert!(checkout.run(&["--built"], true, &[]).status.success());
        assert_eq!(checkout.builds(), 3);
        // A binary built by hand (no stamp): the store's of HEAD, no build.
        fs::remove_file(checkout.binary().with_extension("source")).unwrap();
        assert_eq!(out(&checkout.run(&["x"], true, &[])), "ran x");
        assert_eq!(checkout.builds(), 3);
    }

    /// A second worktree of the checkout's repo, at `branch`.
    fn worktree(checkout: &Checkout, branch: &str) -> PathBuf {
        let dir = checkout.root.join(branch);
        checkout.git(&["worktree", "add", "--quiet", "-b", branch, dir.to_str().unwrap()]);
        dir.join("packages/breadboard")
    }

    /// `sh herdr/breadboard.sh <args>` of the package at `package`, with cargo.
    fn run_in(checkout: &Checkout, package: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        Command::new("sh")
            .arg("breadboard.sh")
            .args(args)
            .current_dir(package.join("herdr"))
            .env_clear()
            .env(
                "PATH",
                format!("{}:{}", checkout.bin.display(), checkout.root.join("tools").display()),
            )
            .env("HOME", &checkout.root)
            .env("CARGO_LOG", checkout.root.join("cargo.log"))
            .env("GIT_CEILING_DIRECTORIES", &checkout.root)
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    // #314's cut-over: every worktree of a clone shares one store in its git
    // dir, so a new worktree copies the binary its sources were built to
    // instead of a cargo build and a target/ of its own (~1 GB).
    #[test]
    fn worktrees_share_one_store_a_build_of_the_same_sources_is_copied() {
        let checkout = Checkout::new("store");
        let store = checkout.root.join("repo/.git/breadboard");
        assert!(checkout.run(&["--built"], true, &[]).status.success());
        assert_eq!(checkout.builds(), 1);
        let calls = fs::read_to_string(checkout.root.join("cargo.log")).unwrap();
        assert!(
            calls
                .trim_end()
                .ends_with(&format!("in {}", store.join("target").display())),
            "{calls}"
        );
        assert!(!checkout.package.join("target/release/deps").exists());
        // A new worktree with the same sources: copied, no build, no word.
        let other = worktree(&checkout, "feature/1-x");
        let copied = run_in(&checkout, &other, &["wt:list"], &[]);
        assert_eq!(out(&copied), "ran wt:list");
        assert_eq!(err(&copied), "");
        assert_eq!(checkout.builds(), 1);
        assert!(other.join("target/release/breadboard.source").is_file());
        // Its own change builds once in the store's target; the main checkout
        // keeps its binary, and gets the change's when it pulls it.
        let lib = other.join("crates/core/lib.rs");
        fs::write(&lib, "2\n").unwrap();
        let git = |args: &[&str]| {
            let done = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(&other)
                .env("GIT_CEILING_DIRECTORIES", &checkout.root)
                .output()
                .unwrap();
            assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
        };
        git(&["commit", "--quiet", "-am", "y"]);
        let built = run_in(&checkout, &other, &["x"], &[("CARGO_SAYS", "new")]);
        assert_eq!(out(&built), "new x");
        assert_eq!(checkout.builds(), 2);
        assert_eq!(out(&checkout.run(&["x"], true, &[])), "ran x");
        checkout.git(&["merge", "--quiet", "feature/1-x"]);
        assert_eq!(out(&checkout.run(&["x"], true, &[])), "new x");
        assert_eq!(checkout.builds(), 2);
        assert_eq!(fs::read_dir(store.join("bin")).unwrap().count(), 2);
        // Uncommitted changes to the sources build in the store's target/
        // too (#336), but never enter the store under HEAD's key.
        fs::write(&lib, "3\n").unwrap();
        fs::remove_file(other.join("target/release/breadboard.source")).unwrap();
        let dirty = run_in(&checkout, &other, &["x"], &[("CARGO_SAYS", "dirty")]);
        assert_eq!(out(&dirty), "dirty x");
        assert_eq!(checkout.builds(), 3);
        let calls = fs::read_to_string(checkout.root.join("cargo.log")).unwrap();
        assert!(
            calls
                .trim_end()
                .ends_with(&format!("in {}", store.join("target").display())),
            "{calls}"
        );
        assert!(!other.join("target/release/deps").exists());
        assert_eq!(fs::read_dir(store.join("bin")).unwrap().count(), 2);
        assert_eq!(out(&checkout.run(&["x"], true, &[])), "new x");
    }

    /// `sh herdr/cargo.sh <args>` of the package at `package`, as pnpm lint:rs runs it.
    fn cargo_in(checkout: &Checkout, package: &Path, args: &[&str]) -> Output {
        Command::new("sh")
            .arg("herdr/cargo.sh")
            .args(args)
            .current_dir(package)
            .env_clear()
            .env(
                "PATH",
                format!("{}:{}", checkout.bin.display(), checkout.root.join("tools").display()),
            )
            .env("HOME", &checkout.root)
            .env("CARGO_LOG", checkout.root.join("cargo.log"))
            .env("GIT_CEILING_DIRECTORIES", &checkout.root)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    // #336: clippy and test of every worktree compile in the store's
    // target/, not in a target/ of their own (1.9 GB each), on half the cores.
    #[test]
    fn every_cargo_run_of_every_worktree_shares_the_stores_target() {
        let checkout = Checkout::new("shared");
        let store = checkout.root.join("repo/.git/breadboard/target");
        let other = worktree(&checkout, "feature/2-vita");
        for package in [&checkout.package, &other] {
            for args in [&["clippy", "--all-targets"][..], &["test", "--quiet"][..]] {
                let ran = cargo_in(&checkout, package, args);
                assert!(ran.status.success(), "{}", err(&ran));
            }
        }
        let cores: usize = String::from_utf8_lossy(
            &Command::new("getconf")
                .arg("_NPROCESSORS_ONLN")
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .parse()
        .unwrap();
        let calls = fs::read_to_string(checkout.root.join("cargo.log")).unwrap();
        assert_eq!(calls.lines().count(), 4, "{calls}");
        for call in calls.lines() {
            assert!(
                call.ends_with(&format!("jobs={} in {}", (cores / 2).max(1), store.display())),
                "{call}"
            );
        }
        assert!(!checkout.package.join("target").exists());
        assert!(!other.join("target").exists());
    }

    // #336: the target/ a worktree compiled in before is removed once; the
    // binary, its stamps and the build log stay. Not while a cargo holds it.
    #[test]
    fn a_worktrees_own_target_is_removed_once_unless_a_cargo_holds_it() {
        let checkout = Checkout::new("reclaim");
        assert!(checkout.run(&["--built"], true, &[]).status.success());
        let own = checkout.package.join("target");
        for dir in ["debug/deps", "release/deps", "release/.fingerprint"] {
            fs::create_dir_all(own.join(dir)).unwrap();
        }
        fs::write(own.join("debug/.cargo-lock"), "").unwrap();
        fs::write(own.join("CACHEDIR.TAG"), "").unwrap();
        fs::write(own.join("breadboard-build.log"), "").unwrap();
        // A cargo holds it (cargo flocks .cargo-lock): left as it is.
        let held = fs::File::open(own.join("debug/.cargo-lock")).unwrap();
        held.lock().unwrap();
        let busy = checkout.run(&["x"], true, &[]);
        assert_eq!(out(&busy), "ran x");
        assert!(own.join("debug/deps").exists());
        drop(held);
        // Free: removed, said once.
        let freed = checkout.run(&["x"], true, &[]);
        assert_eq!(out(&freed), "ran x");
        assert!(
            err(&freed).contains("removed this checkout's own cargo target/"),
            "{}",
            err(&freed)
        );
        for gone in ["debug", "release/deps", "release/.fingerprint", "CACHEDIR.TAG"] {
            assert!(!own.join(gone).exists(), "{gone}");
        }
        for kept in [
            "release/breadboard",
            "release/breadboard.source",
            "breadboard-build.log",
        ] {
            assert!(own.join(kept).exists(), "{kept}");
        }
        assert_eq!(err(&checkout.run(&["x"], true, &[])), "");
    }

    #[test]
    fn without_cargo_the_binary_from_before_runs_and_a_missing_one_says_what_to_do() {
        let checkout = Checkout::new("nocargo");
        checkout.run(&["--built"], true, &[]);
        checkout.change("crates/core/lib.rs");
        let stale = checkout.run(&["hook", "stop-answers"], false, &[]);
        assert!(stale.status.success());
        assert_eq!(out(&stale), "ran hook stop-answers");
        assert!(err(&stale).contains("there is no cargo to build it"), "{}", err(&stale));
        assert!(
            err(&stale).contains("! the breadboard from before runs"),
            "{}",
            err(&stale)
        );
        assert!(!checkout.run(&["--built"], false, &[]).status.success());
        fs::remove_file(checkout.binary()).unwrap();
        let missing = checkout.run(&["herdr", "open", "overview"], false, &[]);
        assert!(!missing.status.success());
        assert_eq!(out(&missing), "");
        assert!(
            err(&missing).contains("✗ breadboard can't run until it is built: pnpm build:rs in"),
            "{}",
            err(&missing)
        );
        // The tab bar: a line, never an error every 10 s.
        let status = checkout.run(&["--no-wait", "herdr", "status"], false, &[]);
        assert!(status.status.success());
        assert_eq!(out(&status), "breadboard isn't built: pnpm build:rs");
    }

    #[test]
    fn the_tab_bar_never_waits_the_build_runs_behind_it() {
        let checkout = Checkout::new("nowait");
        // Missing: the line says so, the build runs on after it.
        let status = checkout.run(&["--no-wait", "herdr", "status"], true, &[]);
        assert!(status.status.success());
        assert_eq!(out(&status), "breadboard builds…");
        let until = Instant::now() + Duration::from_secs(10);
        while !(checkout.binary().is_file() && checkout.binary().with_extension("source").is_file()) {
            assert!(Instant::now() < until, "the build behind finished");
            std::thread::sleep(Duration::from_millis(20));
        }
        while checkout.binary().with_extension("building").exists() {
            assert!(Instant::now() < until, "its lock went with it");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(checkout.builds(), 1);
        assert_eq!(
            out(&checkout.run(&["--no-wait", "herdr", "status"], true, &[])),
            "ran herdr status"
        );
        // Older: the binary from before answers at once.
        checkout.change("crates/core/lib.rs");
        let stale = checkout.run(&["--no-wait", "herdr", "status"], true, &[("CARGO_FAIL", "1")]);
        assert_eq!(out(&stale), "ran herdr status");
        let until = Instant::now() + Duration::from_secs(10);
        while !checkout.binary().with_extension("failed").is_file()
            || checkout.binary().with_extension("building").exists()
        {
            assert!(Instant::now() < until, "the failed build is written down");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(checkout.builds(), 2);
        // Sources it failed on aren't built again behind the tab bar.
        checkout.run(&["--no-wait", "herdr", "status"], true, &[("CARGO_FAIL", "1")]);
        fs::remove_file(checkout.binary()).unwrap();
        let failed = checkout.run(&["--no-wait", "herdr", "status"], true, &[("CARGO_FAIL", "1")]);
        assert_eq!(out(&failed), "breadboard didn't build: pnpm build:rs");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(checkout.builds(), 2);
        // A key tries them in front again.
        let key = checkout.run(&["herdr", "open", "overview"], true, &[("CARGO_FAIL", "1")]);
        assert!(!key.status.success());
        assert!(err(&key).contains("✗ cargo build --release failed in"), "{}", err(&key));
        assert_eq!(checkout.builds(), 3);
    }

    /// Node on this PATH, for the cut-over's shims; a clean environment has none.
    fn node() -> Option<PathBuf> {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("node"))
                .find(|node| node.is_file())
        })
    }

    // #314's cut-over: the Node plugin's reload pulls develop and runs the
    // pulled `node src/herdr.mjs --reload --pulled`; the old manifest and a
    // tab bar status written before still run `node run.mjs …`. Both reach
    // the binary through the launcher, built first.
    #[test]
    fn the_cut_overs_node_shims_run_the_binary_through_the_launcher() {
        let Some(node) = node() else {
            eprintln!("no node on PATH: the cut-over's shims are not run");
            return;
        };
        let checkout = Checkout::new("cutover");
        for (from, to) in [("herdr/run.mjs", "herdr/run.mjs"), ("src/herdr.mjs", "src/herdr.mjs")] {
            let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(from));
            fs::create_dir_all(checkout.package.join(to).parent().unwrap()).unwrap();
            fs::write(checkout.package.join(to), text.unwrap()).unwrap();
        }
        let run = |cwd: &Path, args: &[&str]| {
            let path = format!(
                "{}:{}:/usr/bin:/bin",
                checkout.bin.display(),
                node.parent().unwrap().display()
            );
            Command::new(&node)
                .args(args)
                .current_dir(cwd)
                .env_clear()
                .env("PATH", path)
                .env("HOME", &checkout.root)
                .env("CARGO_LOG", checkout.root.join("cargo.log"))
                .env("GIT_CEILING_DIRECTORIES", &checkout.root)
                .stdin(Stdio::null())
                .output()
                .unwrap()
        };
        let main = checkout.root.join("repo");
        let reload = run(&main, &["packages/breadboard/src/herdr.mjs", "--reload", "--pulled"]);
        assert!(reload.status.success(), "{}", err(&reload));
        assert_eq!(out(&reload), "ran herdr --reload --pulled");
        assert!(err(&reload).contains("▸ breadboard is built from"), "{}", err(&reload));
        assert_eq!(checkout.builds(), 1);
        let herdr = checkout.package.join("herdr");
        assert_eq!(
            out(&run(&herdr, &["run.mjs", "open", "overview"])),
            "ran herdr open overview"
        );
        assert_eq!(
            out(&run(&herdr, &["run.mjs", "restore", "--startup"])),
            "ran herdr restore --startup"
        );
        // The status never waits: older than HEAD, the binary from before answers.
        checkout.change("crates/core/lib.rs");
        let status = run(Path::new("/"), &[herdr.join("run.mjs").to_str().unwrap(), "status"]);
        assert_eq!(out(&status), "ran herdr status");
        let until = Instant::now() + Duration::from_secs(10);
        while checkout.builds() < 2 || checkout.binary().with_extension("building").exists() {
            assert!(Instant::now() < until, "the build behind the status finished");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Its exit code is the binary's.
        fs::write(checkout.binary(), "#!/bin/sh\nexit 3\n").unwrap();
        assert_eq!(run(&herdr, &["run.mjs", "pane", "reload"]).status.code(), Some(3));
    }

    #[test]
    fn a_first_build_that_fails_is_written_down_so_the_tab_bar_stops_trying() {
        let checkout = Checkout::new("firstfail");
        // No target/ yet: the failure still lands next to where the binary goes.
        let key = checkout.run(&["herdr", "open", "overview"], true, &[("CARGO_FAIL", "1")]);
        assert!(!key.status.success());
        assert!(!err(&key).contains("No such file"), "{}", err(&key));
        assert!(checkout.binary().with_extension("failed").is_file());
        let status = checkout.run(&["--no-wait", "herdr", "status"], true, &[("CARGO_FAIL", "1")]);
        assert_eq!(out(&status), "breadboard didn't build: pnpm build:rs");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(checkout.builds(), 1);
    }

    #[test]
    fn a_package_without_sources_runs_the_binary_it_has() {
        let checkout = Checkout::new("nosources");
        fs::remove_file(checkout.package.join("Cargo.toml")).unwrap();
        let missing = checkout.run(&["x"], true, &[]);
        assert!(!missing.status.success());
        assert!(
            err(&missing).contains("has no Cargo.toml to build it from"),
            "{}",
            err(&missing)
        );
        fs::create_dir_all(checkout.binary().parent().unwrap()).unwrap();
        fs::write(checkout.binary(), "#!/bin/sh\necho \"linked $*\"\n").unwrap();
        fs::set_permissions(checkout.binary(), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            out(&checkout.run(&["herdr", "status"], true, &[])),
            "linked herdr status"
        );
        assert_eq!(checkout.builds(), 0);
        assert!(super::built(&checkout.package.join("herdr/breadboard.sh")));
        assert!(!super::built(Path::new("/nonexistent/breadboard.sh")));
    }
}
