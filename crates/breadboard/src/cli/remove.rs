//! `wt:remove <feature> [--keep-branch] [--close-space]` (commands/remove.mjs):
//! a worktree's own stack, directory and merged branch away, its removal
//! noted in its orchestration's record, its space in herdr released. The
//! overview's automatic clean-up (#227) and its ✕ run this.

use breadboard_core::conversations::{bound_session, left_in, left_text, running_claudes, session_file};
use breadboard_core::git::{self, dir_of, is_inside, main_dir, own_commits, worktrees};
use breadboard_core::herdr::{self, Tab, herdr, is_idle, is_in_herdr, panes_of, spaces, tabs_of};
use breadboard_core::local::{env_file_of, offset_of, read_env};
use breadboard_core::model::develop::VITA;
use breadboard_core::record::{self, mark_removed, origin_of};
use breadboard_core::setups::{live_setups_of, setup_at, setup_text};
use breadboard_core::time::{now_ms, to_iso};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// Why the removal stops, printed as proc.mjs `fail` does.
struct Fail(String);

pub fn main(args: &[String]) -> ExitCode {
    match remove(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail(message)) => {
            eprintln!("\n✗ {message}");
            ExitCode::from(1)
        }
    }
}

fn step(message: &str) {
    println!("\n▸ {message}");
}

fn remove(args: &[String]) -> Result<(), Fail> {
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    let Some(feature) = args.iter().find(|arg| !arg.starts_with("--")) else {
        return Err(Fail(
            "usage: pnpm wt:remove <feature> [--keep-branch] [--close-space]".into(),
        ));
    };
    let here = std::env::current_dir().map_err(|error| Fail(error.to_string()))?;
    let main = main_dir(&here).ok_or_else(|| Fail("not in a checkout of the repo".into()))?;
    let dir = dir_of(&main, feature);
    let Some(worktree) = worktrees(&main).into_iter().find(|tree| tree.dir == dir) else {
        return Err(Fail(format!("{} is not a worktree (pnpm wt:list)", dir.display())));
    };
    if is_inside(&here, &dir) {
        return Err(Fail(format!("run it from outside {}", dir.display())));
    }
    // A handed-over agent still in it: EnterWorktree fails once its directory is gone (#252).
    let left = left_in(&dir, &running_claudes(), bound_session(&dir).as_deref());
    if !left.is_empty() {
        return Err(Fail(format!(
            "{} still stands in {feature} (it enters its next worktree, or as a free agent the orchestration's checkout): again once it left",
            left_text(&left)
        )));
    }
    // A start sets it up or moves its agent out of it: git would pull the directory from under it (#272).
    let origin = origin_of(&dir);
    if let Some(origin) = &origin {
        let setups = live_setups_of(&main, &origin.name);
        let at = dir.to_string_lossy();
        if let Some(setup) = setup_at(&setups, &at) {
            let what = if setup.dir.as_deref() == Some(&*at) {
                "sets up"
            } else {
                "moves its agent out of"
            };
            return Err(Fail(format!(
                "{} {what} {feature}: again once it is done",
                setup_text(setup)
            )));
        }
    }
    if !git::git(&dir, &["status", "--porcelain"]).out().is_empty() {
        return Err(Fail(format!(
            "{feature} has uncommitted changes, commit or discard them first"
        )));
    }

    let branch = worktree.branch.clone();
    let mut is_merged = false;
    if let Some(branch) = &branch {
        run_quiet(&main, "git", &["fetch", "--prune"]);
        is_merged = own_commits(&dir, branch) == 0;
        if !is_merged && !has("--keep-branch") {
            return Err(Fail(format!(
                "{branch} has commits that are not in develop. Merge its MR first, or keep the branch with --keep-branch"
            )));
        }
    }

    if offset_of(read_env(&env_file_of(&dir)).as_ref()) != 0 {
        step(&format!("stop the own stack of {feature} and delete its data"));
        let compose = [
            "compose",
            "--env-file",
            ".env.local",
            "-f",
            "docker-compose.local.yml",
            "down",
            "-v",
        ];
        if !run_loud(&dir.join(VITA), "docker", &compose) {
            return Err(Fail(format!("docker compose down -v of {feature} failed")));
        }
    }
    step(&format!("git worktree remove {}", dir.display()));
    // Its binding goes first: the overview only rewrites a binding that is there.
    let _ = std::fs::remove_file(session_file(&dir));
    if !run_loud(&main, "git", &["worktree", "remove", &dir.to_string_lossy()]) {
        // git unregistered it and deleted the files, then failed on a directory
        // something wrote into meanwhile: nothing of anyone's work left, done.
        let is_gone = !worktrees(&main).iter().any(|tree| tree.dir == dir);
        let files = if is_gone && dir.exists() {
            own_files(&main, &dir)
        } else {
            Vec::new()
        };
        if !is_gone || !files.is_empty() {
            let left = if files.is_empty() {
                String::new()
            } else {
                format!(", left in it: {}", files.join(", "))
            };
            return Err(Fail(format!("git worktree remove {} failed{left}", dir.display())));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    // What the removal left behind once git let go: ignored folders, nothing of anyone's work (#268).
    if dir.exists() {
        let files = own_files(&main, &dir);
        if files.is_empty() {
            let _ = std::fs::remove_dir_all(&dir);
        } else {
            step(&format!("left in {}: {}", dir.display(), files.join(", ")));
        }
    }
    if let Some(branch) = branch.as_deref().filter(|_| is_merged && !has("--keep-branch")) {
        step(&format!("delete {branch} (all of it is in develop)"));
        run_loud(&main, "git", &["branch", "-D", branch]);
        // Its claim on origin too, while empty: the issue can start again (#272).
        let remote = format!("refs/remotes/origin/{branch}");
        let is_on_origin = git::git(&main, &["rev-parse", "--verify", "--quiet", &remote]).is_ok;
        if is_on_origin && own_commits(&main, &remote) == 0 && open_mrs_of(&main, branch) == Some(0) {
            step(&format!(
                "delete {branch} on origin (nothing on it develop lacks, no open MR)"
            ));
            run_quiet(&main, "git", &["push", "--quiet", "origin", "--delete", branch]);
        }
    }
    // The overview of the orchestration lists it under "done and removed", with the times.
    if let (Some(origin), Some(branch)) = (&origin, &branch) {
        let merged_at = if is_merged { merged_at_of(&main, branch) } else { None };
        if let Err(error) = mark_removed(
            &main,
            &origin.name,
            branch,
            feature,
            &to_iso(now_ms()),
            merged_at.as_deref(),
        ) {
            step(&format!("the record of {} stays as it was: {error}", origin.name));
        }
    }
    if let Some(said) = release_space(feature, has("--close-space")) {
        step(&format!("herdr: {said}"));
    }
    // Its origin went with the directory; the orchestration's space goes with its last worktree.
    if is_in_herdr() {
        for line in prune_orchestrators(&main) {
            step(&format!("herdr: {line}"));
        }
    }
    let kept = match &branch {
        Some(branch) if !is_merged => format!(", {branch} kept"),
        _ => String::new(),
    };
    println!("\n✓ {feature} removed{kept}");
    Ok(())
}

/// A command whose output goes to ours (proc.mjs `run`): whether it exited 0.
fn run_loud(cwd: &Path, program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A command run for its answer only; None when it fails.
fn run_quiet(cwd: &Path, program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The MRs of `branch` in a state, as GitLab lists them; None when it does not answer.
fn mrs_of(main: &Path, state: &str, branch: &str) -> Option<Vec<serde_json::Value>> {
    let endpoint = format!(
        "projects/:fullpath/merge_requests?state={state}&source_branch={}",
        encode(branch)
    );
    let out = run_quiet(main, "glab", &["api", &endpoint])?;
    serde_json::from_str::<Vec<serde_json::Value>>(&out).ok()
}

/// When the MR of `branch` was merged; None when it wasn't or GitLab does not answer.
fn merged_at_of(main: &Path, branch: &str) -> Option<String> {
    mrs_of(main, "merged", branch)?
        .first()?
        .get("merged_at")?
        .as_str()
        .map(String::from)
}

/// How many open MRs `branch` has; None when GitLab does not answer: then the claim stays.
fn open_mrs_of(main: &Path, branch: &str) -> Option<usize> {
    mrs_of(main, "opened", branch).map(|mrs| mrs.len())
}

/// `encodeURIComponent` for a branch name.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The files of a leftover folder that may be someone's work (prune.mjs
/// `ownFiles`): not the binding, symlinks, what .gitignore ignores, or a file
/// whose content git already has as a blob.
fn own_files(main: &Path, dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk(main, dir, Path::new(""), &mut files);
    if files.is_empty() {
        return files;
    }
    let paths: String = files
        .iter()
        .map(|name| format!("{}\n", dir.join(name).display()))
        .collect();
    let hashes = piped(main, &["hash-object", "--no-filters", "--stdin-paths"], &paths);
    let input: String = hashes.lines().map(|hash| format!("{hash}\n")).collect();
    let missing: Vec<String> = piped(main, &["cat-file", "--batch-check"], &input)
        .lines()
        .filter(|line| line.ends_with(" missing"))
        .filter_map(|line| line.split(' ').next().map(String::from))
        .collect();
    files
        .into_iter()
        .zip(hashes.lines().map(String::from).chain(std::iter::repeat(String::new())))
        .filter(|(_, hash)| hash.is_empty() || missing.contains(hash))
        .map(|(name, _)| name)
        .collect()
}

fn walk(main: &Path, dir: &Path, relative: &Path, files: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir.join(relative)) else {
        return;
    };
    let entries: Vec<_> = entries.flatten().collect();
    // A trailing slash, so `dir/` patterns match a folder the main checkout doesn't have.
    let names: Vec<(String, bool, bool)> = entries
        .iter()
        .map(|entry| {
            let kind = entry.file_type().ok();
            let is_dir = kind.is_some_and(|kind| kind.is_dir());
            let is_file = kind.is_some_and(|kind| kind.is_file());
            let name = relative.join(entry.file_name()).to_string_lossy().into_owned();
            (if is_dir { format!("{name}/") } else { name }, is_dir, is_file)
        })
        .collect();
    let input: String = names.iter().map(|(name, ..)| format!("{name}\n")).collect();
    let ignored = piped(main, &["check-ignore", "--no-index", "--stdin"], &input);
    let ignored: Vec<&str> = ignored.lines().collect();
    let binding = session_file(dir);
    for (name, is_dir, is_file) in &names {
        if ignored.contains(&name.as_str()) {
            continue;
        }
        if *is_dir {
            walk(main, dir, &PathBuf::from(name.trim_end_matches('/')), files);
        } else if *is_file && dir.join(name) != binding {
            files.push(name.clone());
        }
    }
}

/// `git <args>` in `cwd` with `input` on stdin, its output; empty when it fails to run.
fn piped(cwd: &Path, args: &[&str], input: &str) -> String {
    use std::io::Write;
    let child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return String::new() };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    child
        .wait_with_output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

/// What an agent in each of herdr's states holds its tab for; `idle` and `done` hold nothing.
fn agent_busy(status: Option<&str>) -> Option<&'static str> {
    match status.unwrap_or("unknown") {
        "working" => Some("an agent works there"),
        "blocked" => Some("an agent waits for an answer there"),
        "unknown" => Some("herdr can't tell whether an agent works there"),
        _ => None,
    }
}

/// Why a tab of a removed worktree has to stay (conversations.mjs `busyReason`).
fn busy_reason(tab: &Tab, panes: &[herdr::Pane], current: Option<&str>) -> Option<&'static str> {
    if Some(tab.id.as_str()) == current {
        return Some("it runs this command");
    }
    if tab.pane_count > 1 {
        return Some("it has several panes");
    }
    panes
        .iter()
        .filter(|pane| pane.tab == tab.id && pane.agent.is_some())
        .find_map(|pane| agent_busy(pane.status.as_deref()))
}

/// The tab this command runs in, when it has a pane of its own.
fn current_tab() -> Option<String> {
    std::env::var_os("HERDR_PANE_ID")?;
    let pane = herdr(&["pane", "current", "--current"])?;
    let pane = pane.get("pane")?;
    pane.get("tab_id")?.as_str().map(String::from)
}

/// After the removal the worktree's space goes (layout.mjs `releaseSpace`):
/// a tab an agent works in, with several panes, or running this stays;
/// `is_whole` (the ✕ of the overview, `--close-space`) closes all but this.
fn release_space(feature: &str, is_whole: bool) -> Option<String> {
    if !is_in_herdr() {
        return None;
    }
    let current = current_tab();
    let mut closed = 0;
    let mut kept: Vec<String> = Vec::new();
    for space in spaces() {
        let is_own = space.label == feature;
        let tabs: Vec<Tab> = tabs_of(&space.id)
            .into_iter()
            .filter(|tab| is_own || tab.label == feature || tab.label.starts_with(&format!("{feature} :")))
            .collect();
        if tabs.is_empty() {
            continue;
        }
        let panes = panes_of(&space.id);
        let mut busy = Vec::new();
        for tab in &tabs {
            let why = if is_whole {
                (Some(tab.id.as_str()) == current.as_deref()).then_some("it runs this command")
            } else {
                busy_reason(tab, &panes, current.as_deref())
            };
            if let Some(why) = why {
                kept.push(format!("{}: {why}", tab.label));
                busy.push(tab.id.clone());
            }
        }
        // Nothing to keep in its own space: the space goes as a whole.
        if is_own && busy.is_empty() && herdr(&["workspace", "close", &space.id]).is_some() {
            closed += tabs.len();
            continue;
        }
        for tab in &tabs {
            if !busy.contains(&tab.id) && herdr(&["tab", "close", &tab.id]).is_some() {
                closed += 1;
            }
        }
    }
    if closed == 0 && kept.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if closed > 0 {
        parts.push(if kept.is_empty() {
            format!("closed the space {feature}")
        } else {
            format!("closed {closed} tab{} of {feature}", if closed == 1 { "" } else { "s" })
        });
    }
    if !kept.is_empty() {
        let tabs = if kept.len() == 1 {
            "a tab".to_string()
        } else {
            format!("{} tabs", kept.len())
        };
        parts.push(format!("kept {tabs} of {feature} ({})", kept.join("; ")));
    }
    Some(parts.join(", "))
}

/// The space `⇶ <name>` of an orchestration without worktrees goes, its
/// checkout and record with it (origins.mjs `pruneOrchestrators`).
fn prune_orchestrators(main: &Path) -> Vec<String> {
    let live: Vec<String> = worktrees(main)
        .iter()
        .skip(1)
        .filter_map(|tree| origin_of(&tree.dir).map(|origin| origin.name))
        .collect();
    let titles = record::titles(main);
    let mut closed = Vec::new();
    let mut open = Vec::new();
    for space in spaces() {
        let Some(name) = record::orchestration_of_label(&space.label, &titles) else {
            continue;
        };
        if live.contains(&name) {
            open.push(name);
            continue;
        }
        let tabs = tabs_of(&space.id);
        let overview = tabs
            .iter()
            .find(|tab| tab.label == "overview")
            .map(|tab| tab.id.clone());
        let is_busy = panes_of(&space.id)
            .iter()
            .filter(|pane| Some(&pane.tab) != overview.as_ref())
            .any(|pane| pane.agent.is_some() || !is_idle(&pane.id));
        if is_busy || herdr(&["workspace", "close", &space.id]).is_none() {
            open.push(name);
            continue;
        }
        closed.push(format!("closed the space {}, its worktrees are gone", space.label));
        closed.extend(remove_orchestrator_checkout(main, &name));
    }
    for name in record::recorded(main) {
        if !live.contains(&name) && !open.contains(&name) {
            let _ = std::fs::remove_file(record::record_file(main, &name));
        }
    }
    // A checkout whose space is gone and whose worktrees are too.
    for dir in git::orchestrator_checkouts(main) {
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !live.contains(&name) && !open.contains(&name) {
            closed.extend(remove_orchestrator_checkout(main, &name));
        }
    }
    closed
}

/// The checkout of an orchestration away; what stays and why.
fn remove_orchestrator_checkout(main: &Path, name: &str) -> Option<String> {
    let dir = git::orchestrations_dir(main).join(name);
    if !git::orchestrator_checkouts(main).contains(&dir) {
        return None;
    }
    let is_removed = run_quiet(main, "git", &["worktree", "remove", &dir.to_string_lossy()]).is_some();
    (!is_removed).then(|| {
        format!(
            "kept the checkout of {}, it has changes: {}",
            record::orchestrator_label(main, name),
            dir.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_branch_as_a_query_value() {
        assert_eq!(encode("feature/303-x"), "feature%2F303-x");
    }

    #[test]
    fn a_tab_stays_while_its_agent_works() {
        let tab = Tab {
            id: "t1".into(),
            label: "303-x".into(),
            pane_count: 1,
        };
        let pane = |status: &str| herdr::Pane {
            id: "p1".into(),
            tab: "t1".into(),
            agent: Some("claude".into()),
            status: Some(status.into()),
            ..Default::default()
        };
        assert_eq!(busy_reason(&tab, &[pane("idle")], None), None);
        assert_eq!(busy_reason(&tab, &[pane("done")], None), None);
        assert_eq!(
            busy_reason(&tab, &[pane("working")], None),
            Some("an agent works there")
        );
        assert_eq!(busy_reason(&tab, &[], Some("t1")), Some("it runs this command"));
    }
}
