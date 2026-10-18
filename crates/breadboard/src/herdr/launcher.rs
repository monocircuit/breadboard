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
    /// (with `CARGO_SAYS_SOURCE`: what the checkout's crates/core/lib.rs
    /// says) into the target/ cargo.sh hands it (`--config build.target-dir`,
    /// else `CARGO_TARGET_DIR`, else the package's), carrying
    /// `BREADBOARD_SOURCE_KEY` as the real build does; each call logged with
    /// its jobs and that dir, a `clean` in `<log>.clean` only, and with
    /// `CARGO_ENV_LOG` (a file) what its environment holds for rustc. One that fails
    /// with `CARGO_FAIL` set, edits a source while it builds with
    /// `CARGO_EDITS` (a file), takes a second with `CARGO_SLOW`, copies what
    /// the store's lock says of it into `<log>.lock` with `CARGO_SHOWS_LOCK`
    /// (the lock). Only the
    /// tools the launcher has (no dirname: a short PATH).
    const CARGO: &str = r#"#!/bin/sh
dir=$CARGO_TARGET_DIR
jobs=$CARGO_BUILD_JOBS
while [ "$1" = --config ]; do
  case $2 in
    build.target-dir=*) dir=${2#*=\"} && dir=${dir%\"} ;;
    build.jobs=*) jobs=${2#*=} ;;
  esac
  shift 2
done
[ -n "$CARGO_ENV_LOG" ] && echo "wrapper=${RUSTC_WRAPPER-unset} target=${CARGO_TARGET_DIR-unset} jobs=${CARGO_BUILD_JOBS-unset}" >>"$CARGO_ENV_LOG"
if [ "$1" = clean ]; then
  echo "$* in ${dir:-} from $PWD" >>"$CARGO_LOG.clean"
  exit 0
fi
echo "$* jobs=${jobs:-} in ${dir:-}" >>"$CARGO_LOG"
[ "$1" = fmt ] && exit 0
[ -n "$CARGO_FAIL" ] && exit 101
for arg; do case $arg in */Cargo.toml) package=${arg%/*} ;; esac; done
[ -n "$CARGO_SLOW" ] && sleep 1
[ -n "$CARGO_SHOWS_LOCK" ] && cat "$CARGO_SHOWS_LOCK/checkout" "$CARGO_SHOWS_LOCK/what" >"$CARGO_LOG.lock" 2>/dev/null
says=${CARGO_SAYS:-ran}
[ -n "$CARGO_SAYS_SOURCE" ] && says=$(cat "$package/crates/core/lib.rs")
[ -n "$CARGO_EDITS" ] && echo edited >>"$CARGO_EDITS"
target=${dir:-$package/target}
mkdir -p "$target/release"
printf '#!/bin/sh\necho "%s $*"\n# breadboard-source-key:%s\n' "$says" "${BREADBOARD_SOURCE_KEY:-}" >"$target/release/breadboard"
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
                ("crates/core/Cargo.toml", "[package]\nname = \"core-x\"\n"),
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
                "getconf", "flock", "grep", "sed", "find", "head", "date",
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
    // instead of a cargo build. A build runs in the checkout's own target/
    // (#446).
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
                .ends_with(&format!("in {}", checkout.package.join("target").display())),
            "{calls}"
        );
        // A new worktree with the same sources: copied, no build, no word.
        let other = worktree(&checkout, "feature/1-x");
        let copied = run_in(&checkout, &other, &["wt:list"], &[]);
        assert_eq!(out(&copied), "ran wt:list");
        assert_eq!(err(&copied), "");
        assert_eq!(checkout.builds(), 1);
        assert!(other.join("target/release/breadboard.source").is_file());
        // Its own change builds once in its own target; the main checkout
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
        // Uncommitted changes to the sources build in its own target/ too
        // (#446), but never enter the store under HEAD's key.
        fs::write(&lib, "3\n").unwrap();
        fs::remove_file(other.join("target/release/breadboard.source")).unwrap();
        let dirty = run_in(&checkout, &other, &["x"], &[("CARGO_SAYS", "dirty")]);
        assert_eq!(out(&dirty), "dirty x");
        assert_eq!(checkout.builds(), 3);
        let calls = fs::read_to_string(checkout.root.join("cargo.log")).unwrap();
        assert!(
            calls
                .trim_end()
                .ends_with(&format!("in {}", other.join("target").display())),
            "{calls}"
        );
        assert_eq!(fs::read_dir(store.join("bin")).unwrap().count(), 2);
        assert_eq!(out(&checkout.run(&["x"], true, &[])), "new x");
    }

    /// `sh herdr/cargo.sh <args>` of the package at `package`, as pnpm lint:rs runs it.
    fn cargo_in(checkout: &Checkout, package: &Path, args: &[&str]) -> Output {
        cargo_with(checkout, package, args, &[])
    }

    /// cargo_in with the variables `env` set.
    fn cargo_with(checkout: &Checkout, package: &Path, args: &[&str], env: &[(&str, &Path)]) -> Output {
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
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    // #446: with sccache on the PATH it wraps rustc, its server started
    // before the lock; without it one line says so and cargo runs as before.
    // Either way the target/ and the jobs reach cargo as --config, never in
    // the environment rustc (and so sccache's key) gets.
    #[test]
    fn sccache_wraps_rustc_when_there_is_one_and_one_line_says_when_not() {
        let checkout = Checkout::new("sccache");
        let seen = checkout.root.join("env.log");
        let without = cargo_with(&checkout, &checkout.package, &["test"], &[("CARGO_ENV_LOG", &seen)]);
        assert!(without.status.success(), "{}", err(&without));
        let said = err(&without).lines().filter(|line| line.contains("no sccache")).count();
        assert_eq!(said, 1, "{}", err(&without));
        let env = fs::read_to_string(&seen).unwrap();
        let bare = "wrapper= target=unset jobs=unset";
        assert!(env.lines().all(|line| line == bare), "{env}");

        let sccache = checkout.bin.join("sccache");
        let started = checkout.root.join("started");
        fs::write(&sccache, format!("#!/bin/sh\necho \"$*\" >>{}\n", started.display())).unwrap();
        fs::set_permissions(&sccache, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_file(&seen).unwrap();
        let with = cargo_with(&checkout, &checkout.package, &["test"], &[("CARGO_ENV_LOG", &seen)]);
        assert!(with.status.success(), "{}", err(&with));
        assert!(!err(&with).contains("no sccache"), "{}", err(&with));
        let env = fs::read_to_string(&seen).unwrap();
        let wrapped = format!("wrapper={} target=unset jobs=unset", sccache.display());
        assert!(env.lines().all(|line| line == wrapped), "{env}");
        assert_eq!(fs::read_to_string(&started).unwrap(), "--start-server\n");
        let target = format!("in {}", checkout.package.join("target").display());
        let calls = fs::read_to_string(checkout.root.join("cargo.log")).unwrap();
        assert!(calls.lines().all(|call| call.ends_with(&target)), "{calls}");

        // fmt compiles nothing: no wrapper, no line.
        let fmt = cargo_with(&checkout, &checkout.package, &["fmt", "--check"], &[]);
        assert!(fmt.status.success(), "{}", err(&fmt));
        assert!(!err(&fmt).contains("sccache"), "{}", err(&fmt));
    }

    // #446: clippy and test of every worktree compile in its own target/,
    // on half the cores, and never wait for another worktree's run (the
    // dependencies come from sccache, not from a shared target/).
    #[test]
    fn every_worktree_runs_cargo_in_its_own_target_without_waiting() {
        let checkout = Checkout::new("own");
        let other = worktree(&checkout, "feature/2-vita");
        // The main checkout's cargo runs (holds its target): the other's doesn't wait.
        fs::create_dir_all(checkout.package.join("target")).unwrap();
        // -o: the lock goes with flock, not with its sleep.
        let mut busy = Command::new("flock")
            .arg("-o")
            .arg(checkout.package.join("target/.breadboard.lock"))
            .args(["sleep", "30"])
            .spawn()
            .unwrap();
        let held = checkout.package.join("target/.breadboard.lock");
        let since = Instant::now();
        while Command::new("flock")
            .args(["-n"])
            .arg(&held)
            .arg("true")
            .status()
            .unwrap()
            .success()
        {
            assert!(since.elapsed() < Duration::from_secs(5), "the lock was never taken");
            std::thread::sleep(Duration::from_millis(20));
        }
        let started = Instant::now();
        for args in [&["clippy", "--all-targets"][..], &["test", "--quiet"][..]] {
            let ran = cargo_in(&checkout, &other, args);
            assert!(ran.status.success(), "{}", err(&ran));
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        let _ = busy.kill();
        let _ = busy.wait();
        for args in [&["clippy", "--all-targets"][..], &["test", "--quiet"][..]] {
            let ran = cargo_in(&checkout, &checkout.package, args);
            assert!(ran.status.success(), "{}", err(&ran));
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
        let calls: Vec<_> = calls.lines().collect();
        assert_eq!(calls.len(), 4, "{calls:?}");
        for (call, package) in calls.iter().zip([&other, &other, &checkout.package, &checkout.package]) {
            let jobs = format!("jobs={} in {}", (cores / 2).max(1), package.join("target").display());
            assert!(call.ends_with(&jobs), "{call}");
        }
        assert!(!checkout.root.join("repo/.git/breadboard/target").exists());
    }

    /// What the binaries of the store say, by key: `key → output of "x"`.
    fn stored(checkout: &Checkout) -> Vec<(String, String)> {
        let bin = checkout.root.join("repo/.git/breadboard/bin");
        let mut found: Vec<(String, String)> = fs::read_dir(&bin)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| {
                        let said = Command::new(entry.path().join("breadboard")).arg("x").output().unwrap();
                        (entry.file_name().to_string_lossy().to_string(), out(&said))
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    }

    /// `git <args>` in the worktree whose package is `package`.
    fn git_in(checkout: &Checkout, package: &Path, args: &[&str]) {
        let done = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(package)
            .env("GIT_CEILING_DIRECTORIES", &checkout.root)
            .output()
            .unwrap();
        assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
    }

    // #339: a worktree's uncommitted edits are built for it alone; the store
    // keeps HEAD's binary, and the next checkout at HEAD gets that one.
    #[test]
    fn a_dirty_worktree_doesnt_poison_the_store() {
        let checkout = Checkout::new("dirty");
        assert!(checkout.run(&["--built"], true, &[]).status.success());
        let clean = stored(&checkout);
        assert_eq!(clean.len(), 1);
        assert_eq!(clean[0].1, "ran x");
        let other = worktree(&checkout, "feature/3-rust");
        fs::write(other.join("crates/core/lib.rs"), "edited\n").unwrap();
        let dirty = run_in(&checkout, &other, &["x"], &[("CARGO_SAYS", "dirty")]);
        assert_eq!(out(&dirty), "dirty x");
        assert_eq!(stored(&checkout), clean);
        let third = worktree(&checkout, "feature/4-vita");
        let copied = run_in(&checkout, &third, &["x"], &[("CARGO_SAYS", "never")]);
        assert_eq!(out(&copied), "ran x");
        assert_eq!(checkout.builds(), 2);
    }

    // #339: sources that change while cargo builds are built, not stored
    // under the key read before.
    #[test]
    fn sources_edited_during_the_build_are_not_stored() {
        let checkout = Checkout::new("edited");
        let lib = checkout.package.join("crates/core/lib.rs");
        let built = checkout.run(&["x"], true, &[("CARGO_EDITS", lib.to_str().unwrap())]);
        assert_eq!(out(&built), "ran x");
        assert!(stored(&checkout).is_empty());
    }

    // #339: a stored copy that doesn't carry its key (stored before the
    // check, or another build's) is removed, never run; a stamp from before
    // the check is none, so every checkout takes a checked binary once.
    #[test]
    fn a_stored_copy_without_its_key_is_removed_and_built_again() {
        let checkout = Checkout::new("poisoned");
        assert!(checkout.run(&["--built"], true, &[]).status.success());
        let (key, _) = stored(&checkout).remove(0);
        let kept = checkout.root.join(format!("repo/.git/breadboard/bin/{key}/breadboard"));
        fs::write(&kept, "#!/bin/sh\necho \"poison $*\"\n").unwrap();
        let other = worktree(&checkout, "feature/5-x");
        let rebuilt = run_in(&checkout, &other, &["x"], &[("CARGO_SAYS", "fresh")]);
        assert_eq!(out(&rebuilt), "fresh x");
        assert_eq!(checkout.builds(), 2);
        assert_eq!(stored(&checkout), vec![(key.clone(), "fresh x".to_string())]);
        // The main checkout's stamp from before #339: taken from the store again, no build.
        let now = fs::read_to_string(checkout.binary().with_extension("source")).unwrap();
        fs::write(checkout.binary().with_extension("source"), now.replace("#339\n", "")).unwrap();
        assert_eq!(out(&checkout.run(&["x"], true, &[])), "fresh x");
        assert_eq!(checkout.builds(), 2);
    }

    // #339: two worktrees with different sources building at once each get,
    // and store, the binary of their own sources.
    #[test]
    fn two_concurrent_builds_of_different_sources_store_the_right_binaries() {
        let checkout = Checkout::new("concurrent");
        let sides: Vec<PathBuf> = ["a", "b"]
            .iter()
            .map(|side| {
                let package = worktree(&checkout, &format!("feature/6-{side}"));
                fs::write(package.join("crates/core/lib.rs"), format!("{side}\n")).unwrap();
                git_in(&checkout, &package, &["commit", "--quiet", "-am", side]);
                package
            })
            .collect();
        let env = [("CARGO_SAYS_SOURCE", "1"), ("CARGO_SLOW", "1")];
        let runs: Vec<_> = sides
            .iter()
            .map(|package| {
                let checkout_root = checkout.root.clone();
                let bin = checkout.bin.clone();
                let package = package.clone();
                std::thread::spawn(move || {
                    Command::new("sh")
                        .arg("breadboard.sh")
                        .arg("x")
                        .current_dir(package.join("herdr"))
                        .env_clear()
                        .env(
                            "PATH",
                            format!("{}:{}", bin.display(), checkout_root.join("tools").display()),
                        )
                        .env("HOME", &checkout_root)
                        .env("CARGO_LOG", checkout_root.join("cargo.log"))
                        .env("GIT_CEILING_DIRECTORIES", &checkout_root)
                        .envs(env)
                        .stdin(Stdio::null())
                        .output()
                        .unwrap()
                })
            })
            .collect();
        let said: Vec<String> = runs.into_iter().map(|run| out(&run.join().unwrap())).collect();
        assert_eq!(said, vec!["a x", "b x"]);
        let mut kept: Vec<String> = stored(&checkout).into_iter().map(|(_, said)| said).collect();
        kept.sort();
        assert_eq!(kept, vec!["a x", "b x"]);
    }

    // #339: cargo judges freshness by mtimes, its hash leaves out the
    // worktree: a checkout that takes over a target/ another used (one set
    // with CARGO_TARGET_DIR, #446) cleans the workspace's own crates first,
    // dev and release; the same one again doesn't.
    #[test]
    fn a_checkout_that_takes_over_the_target_cleans_the_workspace_crates() {
        let checkout = Checkout::new("takeover");
        let other = worktree(&checkout, "feature/7-x");
        let shared = checkout.root.join("shared-target");
        let cargo_in = |_: &Checkout, package: &Path, args: &[&str]| {
            cargo_with(&checkout, package, args, &[("CARGO_TARGET_DIR", &shared)])
        };
        let cleans = || {
            fs::read_to_string(checkout.root.join("cargo.log.clean"))
                .map(|log| log.lines().map(String::from).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        assert!(cargo_in(&checkout, &checkout.package, &["clippy"]).status.success());
        assert_eq!(cleans().len(), 2);
        assert!(cleans()[0].starts_with("clean --quiet -p core-x"), "{:?}", cleans());
        assert!(
            cleans()[1].starts_with("clean --quiet --release -p core-x"),
            "{:?}",
            cleans()
        );
        assert!(cargo_in(&checkout, &checkout.package, &["test"]).status.success());
        assert!(
            cargo_in(&checkout, &checkout.package, &["fmt", "--check"])
                .status
                .success()
        );
        assert_eq!(cleans().len(), 2);
        assert!(cargo_in(&checkout, &other, &["clippy"]).status.success());
        assert_eq!(cleans().len(), 4);
        assert!(
            cleans()[3].ends_with(&format!("from {}", other.display())),
            "{:?}",
            cleans()
        );
        assert!(cargo_in(&checkout, &other, &["test"]).status.success());
        assert_eq!(cleans().len(), 4);
        // A cargo that keeps no record (a branch's older cargo.sh, a plain
        // cargo) built after this checkout's last run: cleaned again.
        let prints = shared.join("debug/.fingerprint");
        fs::create_dir_all(&prints).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        fs::write(prints.join("breadboard-core-123"), "").unwrap();
        assert!(cargo_in(&checkout, &other, &["test"]).status.success());
        assert_eq!(cleans().len(), 6);
        assert!(cargo_in(&checkout, &other, &["test"]).status.success());
        assert_eq!(cleans().len(), 6);
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
