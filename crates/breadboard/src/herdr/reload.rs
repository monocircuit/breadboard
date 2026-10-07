//! `pnpm herdr --reload` / `prefix+shift+l` (reload.mjs): new workflow code
//! into the running herdrs, the session `platform` and your own, without
//! ending a conversation.

use super::own::{own_herdr, write_own_block};
use super::session::{self, SESSION, Session, Target};
use super::setup::{link_plugin, linked_file_of, manifest_hash, write_config, write_linked};
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
    // again from the pulled code.
    if !is_pulled && pull_develop(&session.main) {
        return Ok(rerun_pulled(&session.main));
    }
    for target in &targets {
        reload(session, target)?;
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
    let branch = git(main, &["branch", "--show-current"]).out().to_string();
    let is_dirty = !git(main, &["status", "--porcelain"]).out().is_empty();
    if branch != "develop" || is_dirty {
        step(&format!(
            "the main checkout stays as it is ({}{})",
            if branch.is_empty() { "detached" } else { &branch },
            if branch == "develop" { ", with changes" } else { "" }
        ));
        return false;
    }
    git(main, &["fetch", "--quiet", "origin", "develop"]);
    let before = git(main, &["rev-parse", "HEAD"]).out().to_string();
    let merged = git(main, &["merge", "--ff-only", "--quiet", "origin/develop"]);
    let after = git(main, &["rev-parse", "HEAD"]).out().to_string();
    if !merged.is_ok {
        step("develop can't fast-forward to origin/develop, it stays as it is");
        return false;
    }
    if before == after {
        step("develop is up to date");
    } else {
        step(&format!("develop moved to {}", after.get(..8).unwrap_or(&after)));
    }
    before != after
}

/// The rest of the reload in the main checkout's binary, built from what
/// the pull brought (cargo is incremental: seconds when the crates didn't
/// change). Without cargo or a build, this binary goes on: said, not hidden.
fn rerun_pulled(main: &Path) -> ExitCode {
    let package = main.join("packages/breadboard");
    let binary = package.join("target/release/breadboard");
    step("the pulled breadboard is built (cargo build --release)");
    let is_built = Command::new("cargo")
        .args(["build", "--release", "--quiet"])
        .current_dir(&package)
        .status()
        .is_ok_and(|status| status.success());
    let program = if is_built && binary.is_file() {
        binary
    } else {
        step("it didn't build: this binary goes on, pnpm build:rs in the main checkout builds the new one");
        std::env::current_exe().unwrap_or(binary)
    };
    let status = Command::new(program)
        .args(["herdr", "--reload", "--pulled"])
        .current_dir(main)
        .status();
    ExitCode::from(status.ok().and_then(|status| status.code()).unwrap_or(1).clamp(0, 255) as u8)
}

/// Whether a process's command line is an overview: Node's (`overview.mjs`)
/// or the binary's (`…/breadboard overview …`).
pub fn is_overview(cmdline: &Value) -> bool {
    let words: Vec<String> = match cmdline {
        Value::String(line) => line.split_whitespace().map(String::from).collect(),
        Value::Array(words) => words.iter().filter_map(Value::as_str).map(String::from).collect(),
        _ => return false,
    };
    words.iter().any(|word| word.contains("overview.mjs"))
        || words
            .windows(2)
            .any(|pair| (pair[0] == "breadboard" || pair[0].ends_with("/breadboard")) && pair[1] == "overview")
}

/// New workflow code into one herdr. The overviews quit (`q`) and the layout
/// starts them again from the current binary; claude, nvim, `pnpm dev` and
/// the logs keep running. The plugin is linked again last, and only when its
/// manifest changed: that could end the popup this runs in.
fn reload(session: &Session, target: &Target) -> Result<(), Fail> {
    let is_pinned = *target == Target::Pinned;
    let name = match target {
        Target::Pinned => format!("the session {SESSION}"),
        Target::Own(binary) => format!("your herdr ({})", binary.display()),
    };
    step(&format!("{name}:"));

    let is_changed = if is_pinned {
        write_config(session)?.is_changed
    } else {
        write_own_block(session, &session::own_config(), true)?
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
    step("layout built again, the overviews run the new code");

    let linked = fs::read_to_string(linked_file_of(session, target)).ok();
    if linked.as_deref().map(str::trim) != Some(manifest_hash(session).as_str()) {
        session.herdr(target, &["plugin", "unlink", PLUGIN]);
        link_plugin(session, target)?;
        write_linked(session, target);
        step(&format!("plugin {PLUGIN} linked again, its manifest changed"));
    }
    println!("\n✓ {name} runs the current workflow code; conversations, nvim and servers kept running\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_overviews_of_node_and_of_the_binary() {
        assert!(is_overview(&json!(
            "node /r/packages/breadboard/src/overview.mjs --herdr"
        )));
        assert!(is_overview(&json!(
            "/r/packages/breadboard/target/release/breadboard overview --herdr"
        )));
        assert!(is_overview(&json!(["breadboard", "overview", "--orchestration", "x"])));
        assert!(!is_overview(&json!("/r/target/release/breadboard wt:list")));
        assert!(!is_overview(&json!("nvim overview.md")));
        assert!(!is_overview(&json!(null)));
    }
}
