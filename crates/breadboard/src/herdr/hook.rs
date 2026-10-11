//! `breadboard hook stop-answers`: Claude Code's Stop hook
//! (.claude/hooks/stop-answers.mjs). An answer from the orchestration
//! overview that did not reach the agent's pane (it was blocked, herdr was
//! away) is handed over at the agent's next stop. Only reads files and
//! writes one: silent and quick without an answer, and never in the way of
//! a stop: whatever fails, it exits 0 (#306).
//!
//! `breadboard hook pre-edit`: Claude Code's PreToolUse hook on edits
//! (.claude/hooks/pre-edit-worktree.mjs before #342): every task works in
//! its own worktree, so an edit in the main checkout is blocked with the
//! command that starts one.
//!
//! Also the git pre-push shim of the binary (`PRE_PUSH_SHIM`,
//! packages/breadboard/hooks/pre-push): wt:new and wt:mr install it
//! (`cli::write::mr::SHIM`). Its sibling hooks/stop-answers is what
//! settings.json's Stop hook runs at the cut-over (`sh "$CLAUDE_PROJECT_DIR/packages/breadboard/hooks/stop-answers"`).

use breadboard_core::actions::{Action, actions_file, answer_line, take_answers};
use serde_json::json;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The push guard (#184, #298) for the shared hooks folder once breadboard
/// is the binary: the pushing branch's own binary, the Node hook of an
/// older branch, the main checkout's binary, else a refusal that says so.
/// The marker line stays, so the installers recognise it as theirs.
pub const PRE_PUSH_SHIM: &str = include_str!("../../../../hooks/pre-push");

pub const USAGE: &str = "usage: breadboard hook stop-answers   (Claude Code runs it at a stop, its JSON on stdin)
       breadboard hook pre-edit       (before an edit, its JSON on stdin: exit 2 blocks it)";

/// `breadboard hook pre-edit`: the edit of `tool_input.file_path` (or
/// `notebook_path`) in the main checkout is blocked (exit 2, why on
/// stderr). Allowed there: what git ignores, files inside another worktree
/// (claude -w puts them below the main checkout), and everything with
/// `CLAUDE_ALLOW_MAIN_CHECKOUT=1`. Outside a repo, or on input it can't
/// read, it lets the edit through.
pub fn pre_edit(args: &[String]) -> ExitCode {
    if !args.is_empty() {
        let _ = writeln!(std::io::stderr(), "{USAGE}");
        return ExitCode::FAILURE;
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if std::env::var("CLAUDE_ALLOW_MAIN_CHECKOUT").as_deref() == Ok("1") {
        return ExitCode::SUCCESS;
    }
    let value: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let tool = value.get("tool_input");
    let Some(file) = ["file_path", "notebook_path"]
        .iter()
        .find_map(|key| tool.and_then(|tool| tool.get(key)).and_then(|file| file.as_str()))
    else {
        return ExitCode::SUCCESS;
    };
    let target = std::env::current_dir().unwrap_or_default().join(file);
    // The closest folder that exists: git needs one, a new file's may not be there yet.
    let Some(dir) = target.ancestors().skip(1).find(|dir| dir.is_dir()) else {
        return ExitCode::SUCCESS;
    };
    let trees: Vec<PathBuf> = breadboard_core::git::all_worktrees(dir)
        .into_iter()
        .map(|tree| tree.dir)
        .collect();
    let is_ignored = || {
        breadboard_core::git::git(
            dir,
            &["check-ignore", "--quiet", "--no-index", &target.to_string_lossy()],
        )
        .is_ok
    };
    match blocked(&target, &trees, is_ignored) {
        Some(main) => {
            let loaded = breadboard_core::config::of(&main);
            let relative = target.strip_prefix(&main).unwrap_or(&target).display().to_string();
            let _ = write!(
                std::io::stderr(),
                "Blocked: {relative} is in the main checkout, and every task works in its own worktree.

Start one and make the change there:

  {} <feature>          # from {}; --fix for a fix

then use the paths below {}-<feature>/. If the user explicitly wants this edit in the main checkout, they can set CLAUDE_ALLOW_MAIN_CHECKOUT=1 for the session.
",
                loaded.command("wt:new"),
                main.display(),
                main.display()
            );
            ExitCode::from(2)
        }
        None => ExitCode::SUCCESS,
    }
}

/// The main checkout when `target` is in it and not in another worktree
/// below it nor ignored by git; `trees` the main checkout first.
fn blocked(target: &Path, trees: &[PathBuf], is_ignored: impl FnOnce() -> bool) -> Option<PathBuf> {
    let main = trees.first()?;
    // The worktree a file belongs to is the deepest one holding it.
    let owner = trees
        .iter()
        .filter(|tree| target.starts_with(tree))
        .max_by_key(|tree| tree.components().count())?;
    (owner == main && !is_ignored()).then(|| main.clone())
}

pub fn stop_answers(args: &[String]) -> ExitCode {
    if !args.is_empty() {
        let _ = writeln!(std::io::stderr(), "{USAGE}");
        return ExitCode::FAILURE;
    }
    // Claude Code's input says nothing it needs; read so the pipe drains.
    if !std::io::stdin().is_terminal() {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
    }
    let dir = std::env::var_os("CLAUDE_PROJECT_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let pane = std::env::var("HERDR_PANE_ID").ok().filter(|pane| !pane.is_empty());
    let answers = answers_of(&dir, pane.as_deref());
    if let Some(reply) = reply_of(&answers) {
        let mut out = std::io::stdout();
        let _ = out.write_all(reply.as_bytes()).and_then(|()| out.flush());
    }
    ExitCode::SUCCESS
}

/// Taken out of their files: a worktree's agent gets every answer of the
/// worktree; the orchestrator in the main checkout those it asked from its
/// pane, in every record.
fn answers_of(dir: &Path, pane: Option<&str>) -> Vec<Action> {
    let take = |file: &Path, pane: Option<Option<&str>>| {
        if file.is_file() {
            take_answers(file, pane).unwrap_or_default()
        } else {
            Vec::new()
        }
    };
    let mut answers = take(&actions_file(dir), None);
    let records = dir.join(".claude").join("orchestrations");
    if let Some(pane) = pane
        && let Ok(entries) = std::fs::read_dir(&records)
    {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|file| file.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort();
        for file in files {
            answers.extend(take(&file, Some(Some(pane))));
        }
    }
    answers
}

/// What blocks the stop: `{"decision":"block","reason":…}`, None without answers.
fn reply_of(answers: &[Action]) -> Option<String> {
    if answers.is_empty() {
        return None;
    }
    let lines: Vec<String> = answers.iter().map(answer_line).collect();
    let reply = json!({
        "decision": "block",
        "reason": format!("The user answered in the orchestration overview, go on with it:\n{}", lines.join("\n")),
    });
    Some(reply.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_an_edit_in_the_main_checkout_only() {
        let trees = [
            PathBuf::from("/r/app"),
            PathBuf::from("/r/app-1-x"),
            PathBuf::from("/r/app/.claude/worktrees/w"),
        ];
        assert_eq!(
            blocked(Path::new("/r/app/src/a.rs"), &trees, || false),
            Some(PathBuf::from("/r/app"))
        );
        assert_eq!(
            blocked(Path::new("/r/app/src/a.rs"), &trees, || true),
            None,
            "ignored by git"
        );
        assert_eq!(blocked(Path::new("/r/app-1-x/src/a.rs"), &trees, || false), None);
        assert_eq!(
            blocked(Path::new("/r/app/.claude/worktrees/w/a.rs"), &trees, || false),
            None
        );
        assert_eq!(blocked(Path::new("/elsewhere/a.rs"), &trees, || false), None);
        assert_eq!(blocked(Path::new("/r/app/a.rs"), &[], || false), None);
    }
    use serde_json::Value;
    use std::fs;

    #[test]
    fn hands_over_the_answers_once_and_leaves_drafts_and_claims() {
        let root = std::env::temp_dir().join(format!("stop-answers-{}", std::process::id()));
        let claude = root.join(".claude");
        fs::create_dir_all(claude.join("orchestrations")).unwrap();
        let recent = breadboard_core::time::to_iso(breadboard_core::time::now_ms());
        fs::write(
            claude.join("actions.json"),
            json!({
                "change": { "kept": true },
                "actions": [
                    { "id": 1, "kind": "decide", "text": "A or B", "answer": "B" },
                    { "id": 2, "kind": "try", "text": "the app", "answer": "broken" },
                    { "id": 3, "kind": "look", "text": "x", "answer": "seen", "deliveringAt": recent },
                    { "id": 4, "kind": "merge", "text": "!12" },
                ],
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            claude.join("orchestrations/one.json"),
            json!({ "actions": [
                { "id": 7, "kind": "plan", "text": "go?", "answer": "go", "pane": "w1:p2" },
                { "id": 8, "kind": "decide", "text": "other", "answer": "x", "pane": "w9:p1" },
            ] })
            .to_string(),
        )
        .unwrap();
        fs::write(claude.join("orchestrations/broken.json"), "{ not json").unwrap();

        let answers = answers_of(&root, Some("w1:p2"));
        let reply: Value = serde_json::from_str(&reply_of(&answers).unwrap()).unwrap();
        assert_eq!(reply["decision"], "block");
        assert_eq!(
            reply["reason"],
            "The user answered in the orchestration overview, go on with it:\n[overview] #1 decide \"A or B\" → B\n[overview] #7 plan \"go?\" → go"
        );
        let file: Value = serde_json::from_str(&fs::read_to_string(claude.join("actions.json")).unwrap()).unwrap();
        let left: Vec<i64> = file["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_i64().unwrap())
            .collect();
        assert_eq!(left, [2, 3, 4]);
        assert_eq!(file["answered"][0]["id"], 1);
        assert!(file["answered"][0]["deliveredAt"].is_string());
        assert_eq!(file["change"]["kept"], true);
        assert!(answers_of(&root, Some("w1:p2")).is_empty(), "taken once");
        assert_eq!(
            fs::read_to_string(claude.join("orchestrations/broken.json")).unwrap(),
            "{ not json"
        );
        assert!(answers_of(&root.join("nowhere"), None).is_empty());
        assert!(!root.join("nowhere").exists(), "nothing made where nothing was");
        fs::remove_dir_all(&root).unwrap();
    }

    const STOP_SHIM: &str = include_str!("../../../../hooks/stop-answers");

    /// `sh -c <shim>` run as git runs the hook, in `dir`, with `path` as PATH.
    fn run_shim(dir: &Path, path: &str) -> std::process::Output {
        std::process::Command::new("sh")
            .args(["-c", PRE_PUSH_SHIM, "pre-push", "origin", "git@x:p.git"])
            .current_dir(dir)
            .env("PATH", path)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }

    fn script(file: &Path, says: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, format!("#!/bin/sh\necho \"{says} $*\"\n")).unwrap();
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_pre_push_shim_runs_the_guard_of_the_branch_and_refuses_without_one() {
        assert!(
            PRE_PUSH_SHIM.contains("breadboard push guard (#184)"),
            "the installers' marker"
        );
        let root = std::env::temp_dir().join(format!("pre-push-shim-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (main, tree, bin) = (root.join("main"), root.join("tree"), root.join("bin"));
        fs::create_dir_all(&main).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let done = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env("GIT_CEILING_DIRECTORIES", &root)
                .output()
                .unwrap();
            assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
        };
        git(&main, &["init", "--quiet", "--initial-branch=develop"]);
        git(
            &main,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "x",
            ],
        );
        git(
            &main,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "feature/1-x",
                tree.to_str().unwrap(),
            ],
        );
        script(&bin.join("node"), "node");
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        let said = |dir: &Path| {
            let output = run_shim(dir, &path);
            let text = String::from_utf8_lossy(if output.status.success() {
                &output.stdout
            } else {
                &output.stderr
            });
            (output.status.success(), text.trim().to_string())
        };
        let binary = "packages/breadboard/target/release/breadboard";

        // The worktree's own binary first.
        script(&tree.join(binary), "own");
        script(&main.join(binary), "main");
        assert_eq!(said(&tree), (true, "own hook pre-push origin git@x:p.git".to_string()));
        // A branch from before the rewrite: its Node hook.
        fs::remove_file(tree.join(binary)).unwrap();
        let node_hook = tree.join("packages/breadboard/src/git-hooks/pre-push.mjs");
        script(&node_hook, "unused");
        assert_eq!(
            said(&tree),
            (true, format!("node {} origin git@x:p.git", node_hook.display()))
        );
        // A branch of the binary that isn't built here: the main checkout's.
        fs::remove_file(&node_hook).unwrap();
        assert_eq!(said(&tree), (true, "main hook pre-push origin git@x:p.git".to_string()));
        assert_eq!(said(&main), (true, "main hook pre-push origin git@x:p.git".to_string()));
        // The main checkout's through its launcher where it has one (it builds
        // when the binary is older than HEAD: a pull), in the main checkout too.
        let launcher = main.join("packages/breadboard/herdr/breadboard.sh");
        script(&launcher, "launcher");
        assert_eq!(
            said(&tree),
            (true, "launcher hook pre-push origin git@x:p.git".to_string())
        );
        assert_eq!(
            said(&main),
            (true, "launcher hook pre-push origin git@x:p.git".to_string())
        );
        // The Stop hook's shim the same way, a worktree's own binary as built.
        let stop = |dir: &Path| {
            let output = std::process::Command::new("sh")
                .args(["-c", STOP_SHIM])
                .current_dir(&root)
                .env("PATH", &path)
                .env("CLAUDE_PROJECT_DIR", dir)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        assert_eq!(stop(&tree), "launcher --no-wait hook stop-answers");
        assert_eq!(stop(&main), "launcher --no-wait hook stop-answers");
        script(&tree.join(binary), "own");
        assert_eq!(stop(&tree), "own hook stop-answers");
        fs::remove_file(tree.join(binary)).unwrap();
        fs::remove_file(&launcher).unwrap();
        assert_eq!(stop(&tree), "main hook stop-answers");
        // None at all: refused, never a guard that is off in silence.
        fs::remove_file(main.join(binary)).unwrap();
        let (is_ok, text) = said(&tree);
        assert_eq!(stop(&tree), "", "no binary: the stop goes on");
        assert!(!is_ok);
        assert!(text.starts_with("✗ push refused: no push guard to run"), "{text}");
        fs::remove_dir_all(&root).unwrap();
    }
}
