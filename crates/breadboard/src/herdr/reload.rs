//! `pnpm herdr --reload` / `prefix+shift+l` (reload.mjs): new workflow code
//! into the running herdrs, the session `platform` and your own, without
//! ending a conversation.

use super::launcher::built;
use super::own::{own_herdr, write_own_block, write_own_status};
use super::session::{self, SESSION, Session, Target};
use super::setup::{ensure_breadboard, link_plugin, linked_file_of, manifest_hash, write_config, write_linked};
use super::{Done, Fail, PLUGIN, SPACE, step};
use breadboard_core::git::git;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::Duration;

/// `--reload [--pulled]`.
pub fn command(session: &Session, is_pulled: bool) -> Done {
    let targets = targets(session);
    if targets.is_empty() {
        return Err(format!(
            "neither the session {SESSION} nor your herdr runs with the space {SPACE}: pnpm herdr, or pnpm herdr:install"
        )
        .into());
    }
    // The pull can bring new workflow code, but this process runs the binary
    // built before it: the rest goes on in the main checkout's binary, built
    // again from the pulled code. Without a build it goes on here, and says
    // the old code runs on.
    let mut is_stale = false;
    if !is_pulled && pull_develop(&session.main) {
        match rerun_pulled(&session.main) {
            Some(code) => return Ok(code),
            None => is_stale = true,
        }
    }
    ensure_breadboard(session);
    for target in &targets {
        reload(session, target, is_stale)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// The herdrs it acts on. Inside herdr (the key): the session it runs in,
/// `platform` or your own. From a plain terminal: each that runs with the
/// space, so a pull reaches both.
fn targets(session: &Session) -> Vec<Target> {
    let own = own_herdr(session).map(Target::Own);
    if session::is_in_herdr() {
        return if session::is_in_platform() {
            vec![Target::Pinned]
        } else {
            own.into_iter().collect()
        };
    }
    let pinned = (session.bin.exists() && session.is_running(&Target::Pinned)).then_some(Target::Pinned);
    let own = own.filter(|own| session.is_running(own) && session.has_space(own));
    pinned.into_iter().chain(own).collect()
}

/// The main checkout to origin/develop when it can without a question:
/// clean, on develop, fast-forward. Whether develop moved.
fn pull_develop(main: &Path) -> bool {
    let develop = breadboard_core::git::branch(main);
    let branch = git(main, &["branch", "--show-current"]).out().to_string();
    let is_dirty = !git(main, &["status", "--porcelain"]).out().is_empty();
    if branch != develop || is_dirty {
        step(&format!(
            "the main checkout stays as it is ({}{})",
            if branch.is_empty() { "detached" } else { &branch },
            if branch == develop { ", with changes" } else { "" }
        ));
        return false;
    }
    git(main, &["fetch", "--quiet", "origin", &develop]);
    let before = git(main, &["rev-parse", "HEAD"]).out().to_string();
    let merged = git(main, &["merge", "--ff-only", "--quiet", &format!("origin/{develop}")]);
    let after = git(main, &["rev-parse", "HEAD"]).out().to_string();
    if !merged.is_ok {
        step(&format!(
            "{develop} can't fast-forward to origin/{develop}, it stays as it is"
        ));
        return false;
    }
    if before == after {
        step(&format!("{develop} is up to date"));
    } else {
        step(&format!("{develop} moved to {}", after.get(..8).unwrap_or(&after)));
    }
    before != after
}

/// The rest of the reload in the main checkout's binary, built from what
/// the pull brought by its launcher (herdr/breadboard.sh `--built`; cargo is
/// incremental: seconds when the crates didn't change); its exit code. None
/// without a launcher, cargo or a build: this process goes on with the code
/// from before the pull, said, not hidden.
fn rerun_pulled(main: &Path) -> Option<ExitCode> {
    let package = main.join("packages/breadboard");
    let binary = package.join("target/release/breadboard");
    if !built(&package.join("herdr/breadboard.sh")) || !binary.is_file() {
        step(
            "the pulled breadboard didn't build: the code from before the pull goes on, pnpm build:rs in the main checkout builds the new one",
        );
        return None;
    }
    let status = Command::new(binary)
        .args(["herdr", "--reload", "--pulled"])
        .current_dir(main)
        .status();
    Some(ExitCode::from(
        status.ok().and_then(|status| status.code()).unwrap_or(1).clamp(0, 255) as u8,
    ))
}

/// Whether a process's command line is an overview: Node's (`overview.mjs`)
/// or the binary's (`…/breadboard overview …`). A line herdr gives as one
/// string is not split on spaces: a checkout's path may hold one.
pub fn is_overview(cmdline: &Value) -> bool {
    let is_binary = |word: &str| word == "breadboard" || word.ends_with("/breadboard");
    match cmdline {
        Value::String(line) => {
            line.contains("overview.mjs")
                || line.match_indices(" overview").any(|(at, _)| {
                    let rest = &line[at + " overview".len()..];
                    is_binary(&line[..at]) && (rest.is_empty() || rest.starts_with(' '))
                })
        }
        Value::Array(words) => {
            let words: Vec<&str> = words.iter().filter_map(Value::as_str).collect();
            words.iter().any(|word| word.contains("overview.mjs"))
                || words.windows(2).any(|pair| is_binary(pair[0]) && pair[1] == "overview")
        }
        _ => false,
    }
}

/// New workflow code into one herdr. The overviews quit (`q`) and the layout
/// starts them again from the current binary; claude, nvim, `pnpm dev` and
/// the logs keep running. The plugin is linked again last, and only when its
/// manifest changed: that could end the popup this runs in. `is_stale`: the
/// pull brought code that didn't build, so the overviews run the old binary.
fn reload(session: &Session, target: &Target, is_stale: bool) -> Result<(), Fail> {
    let is_pinned = *target == Target::Pinned;
    let name = match target {
        Target::Pinned => format!("the session {SESSION}"),
        Target::Own(binary) => format!("your herdr ({})", binary.display()),
    };
    step(&format!("{name}:"));

    let is_changed = if is_pinned {
        write_config(session)?.is_changed
    } else {
        let is_block_changed = write_own_block(session, &session::own_config(), true)?;
        write_own_status(session, &session::own_config())? || is_block_changed
    };
    if is_changed && session.herdr(target, &["server", "reload-config"]).is_ok {
        step("config written again and reloaded");
    }

    // A plugin whose directory moved (a pull took the old one) is linked from
    // where it is now first: herdr can't run restore from a directory that is gone.
    if link_plugin(session, target)? {
        write_linked(session, target);
        step(&format!(
            "plugin {PLUGIN} linked again from {}, it moved",
            session.plugin_dir.display()
        ));
    }

    let process_info = |pane: &str| {
        session
            .ask(target, &["pane", "process-info", "--pane", pane])
            .and_then(|result| result.get("process_info").cloned())
    };
    let panes = session
        .ask(target, &["pane", "list"])
        .and_then(|result| result.get("panes").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let overviews: Vec<String> = panes
        .iter()
        .filter_map(|pane| pane.get("pane_id").and_then(Value::as_str))
        .filter(|pane| {
            process_info(pane)
                .and_then(|info| info.get("foreground_processes").and_then(Value::as_array).cloned())
                .is_some_and(|processes| {
                    processes
                        .iter()
                        .any(|process| process.get("cmdline").is_some_and(is_overview))
                })
        })
        .map(String::from)
        .collect();
    for pane in &overviews {
        session.herdr(target, &["pane", "send-keys", pane, "q"]);
    }
    // Until their shells idle: the layout only runs a command again in an idle tab.
    for _ in 0..50 {
        let is_running_still = overviews.iter().any(|pane| {
            process_info(pane).is_some_and(|info| info.get("foreground_process_group_id") != info.get("shell_pid"))
        });
        if !is_running_still {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    step(&format!(
        "{} overview{} quit",
        overviews.len(),
        if overviews.len() == 1 { "" } else { "s" }
    ));

    // Through the plugin, like pnpm herdr: it runs inside the session, from wherever this was typed.
    let restored = session.herdr(target, &["plugin", "action", "invoke", &format!("{PLUGIN}.restore")]);
    if !restored.is_ok {
        let again = if is_pinned {
            "pnpm herdr --no-attach"
        } else {
            "pnpm herdr:install"
        };
        return Err(format!(
            "the layout failed: {}\n  The overviews quit; {again} links the plugin and builds the layout again.",
            restored.out
        )
        .into());
    }
    step(if is_stale {
        "layout built again, the overviews run the binary from before the pull"
    } else {
        "layout built again, the overviews run the new code"
    });

    let linked = fs::read_to_string(linked_file_of(session, target)).ok();
    if linked.as_deref().map(str::trim) != Some(manifest_hash(session).as_str()) {
        session.herdr(target, &["plugin", "unlink", &PLUGIN.get()]);
        link_plugin(session, target)?;
        write_linked(session, target);
        step(&format!("plugin {PLUGIN} linked again, its manifest changed"));
    }
    println!("\n{}\n", closing(&name, is_stale));
    Ok(())
}

/// The last line of a reload: ✓ only when the overviews run the current code.
fn closing(name: &str, is_stale: bool) -> String {
    if is_stale {
        format!(
            "! {name} runs the code from before the pull until pnpm build:rs in the main checkout and a reload (prefix+shift+l); conversations, nvim and servers kept running"
        )
    } else {
        format!("✓ {name} runs the current workflow code; conversations, nvim and servers kept running")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn a_reload_without_the_pulled_build_says_the_old_code_runs() {
        assert!(closing("the session platform", false).starts_with("✓ the session platform runs the current"));
        let stale = closing("the session platform", true);
        assert!(stale.starts_with("! the session platform runs the code from before the pull until pnpm build:rs"));
        assert!(!stale.contains('✓'));
    }

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let done = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=develop",
            ])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
        String::from_utf8_lossy(&done.stdout).trim().to_string()
    }

    /// A main checkout on develop with an origin, and a second clone that pushes to it.
    fn checkouts(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("herdr-reload-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git_in(&root, &["init", "--quiet", "--bare", "origin.git"]);
        git_in(&root, &["clone", "--quiet", "origin.git", "main"]);
        let main = root.join("main");
        fs::write(main.join("a"), "1\n").unwrap();
        git_in(&main, &["add", "a"]);
        git_in(&main, &["commit", "--quiet", "-m", "a"]);
        git_in(&main, &["push", "--quiet", "origin", "develop"]);
        // origin's HEAD names the default branch where no breadboard.toml does.
        git_in(&main, &["remote", "set-head", "origin", "develop"]);
        git_in(&root, &["clone", "--quiet", "origin.git", "other"]);
        (root.clone(), main, root.join("other"))
    }

    fn push_from(other: &Path) {
        fs::write(
            other.join("a"),
            format!("{}1\n", fs::read_to_string(other.join("a")).unwrap()),
        )
        .unwrap();
        git_in(other, &["commit", "--quiet", "-am", "more"]);
        git_in(other, &["push", "--quiet", "origin", "develop"]);
    }

    #[test]
    fn pulls_develop_only_clean_and_fast_forward() {
        let (root, main, other) = checkouts("pull");
        assert!(!pull_develop(&main), "up to date");
        push_from(&other);
        let head = git_in(&other, &["rev-parse", "HEAD"]);
        // Changes in the main checkout: it stays.
        fs::write(main.join("b"), "").unwrap();
        assert!(!pull_develop(&main));
        fs::remove_file(main.join("b")).unwrap();
        // Another branch: it stays.
        git_in(&main, &["switch", "--quiet", "-c", "feature/1-x"]);
        assert!(!pull_develop(&main));
        git_in(&main, &["switch", "--quiet", "develop"]);
        assert_ne!(git_in(&main, &["rev-parse", "HEAD"]), head);
        // Clean on develop: fast-forwarded, develop moved.
        assert!(pull_develop(&main));
        assert_eq!(git_in(&main, &["rev-parse", "HEAD"]), head);
        // Diverged: no merge commit, it stays.
        push_from(&other);
        fs::write(main.join("c"), "").unwrap();
        git_in(&main, &["add", "c"]);
        git_in(&main, &["commit", "--quiet", "-m", "local"]);
        let local = git_in(&main, &["rev-parse", "HEAD"]);
        assert!(!pull_develop(&main));
        assert_eq!(git_in(&main, &["rev-parse", "HEAD"]), local);
        fs::remove_dir_all(&root).unwrap();
    }

    // After a pull that moved develop the rest runs in the main checkout's
    // binary, built by its launcher first; without a build it goes on here.
    #[test]
    fn the_rest_of_a_reload_runs_the_pulled_build() {
        let (root, main, _) = checkouts("rerun");
        let package = main.join("packages/breadboard");
        let launcher = package.join("herdr/breadboard.sh");
        let binary = package.join("target/release/breadboard");
        assert!(rerun_pulled(&main).is_none(), "no launcher");
        fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        // The launcher "builds" the binary on --built.
        fs::write(
            &launcher,
            format!(
                "[ \"$1\" = --built ] || exit 9\nmkdir -p {dir}\nprintf '#!/bin/sh\\necho \"$PWD $*\" >ran\\nexit 4\\n' >{bin}\nchmod +x {bin}\n",
                dir = binary.parent().unwrap().display(),
                bin = binary.display()
            ),
        )
        .unwrap();
        assert_eq!(rerun_pulled(&main), Some(ExitCode::from(4)));
        assert_eq!(
            fs::read_to_string(main.join("ran")).unwrap().trim(),
            format!("{} herdr --reload --pulled", main.display())
        );
        // A build that fails: this process goes on (None), even with a binary from before.
        let failing = fs::read_to_string(&launcher).unwrap().replace("|| exit 9", "&& exit 1");
        fs::write(&launcher, failing).unwrap();
        fs::remove_file(main.join("ran")).unwrap();
        assert!(rerun_pulled(&main).is_none());
        assert!(!main.join("ran").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_the_overviews_of_node_and_of_the_binary() {
        assert!(is_overview(&json!(
            "node /r/packages/breadboard/src/overview.mjs --herdr"
        )));
        assert!(is_overview(&json!(
            "/r/packages/breadboard/target/release/breadboard overview --herdr"
        )));
        assert!(is_overview(&json!(["breadboard", "overview", "--orchestration", "x"])));
        // A checkout whose path holds a space: one string, one array.
        assert!(is_overview(&json!(
            "/home/me/my repos/platform/packages/breadboard/target/release/breadboard overview"
        )));
        assert!(is_overview(&json!([
            "/a b/target/release/breadboard",
            "overview",
            "--herdr"
        ])));
        assert!(!is_overview(&json!("/a b/target/release/breadboard overviews")));
        assert!(!is_overview(&json!("/a b/target/release/breadboard wt:list overview")));
        assert!(!is_overview(&json!("/r/target/release/breadboard wt:list")));
        assert!(!is_overview(&json!("nvim overview.md")));
        assert!(!is_overview(&json!(null)));
    }
}
