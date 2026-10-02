//! `wt:remove <feature> [--keep-branch] [--close-space]`
//! (worktree/commands/remove.mjs): a worktree's own stack, the worktree and
//! its merged branch away, its empty claim on origin too (#272), the record
//! of its orchestration told, its herdr space released. Refused while
//! anything of anyone's is still in it.

use super::args::{Args, Takes};
use super::claude::{bound_session, left_in, left_text, running_claudes, session_file};
use super::gitlab;
use super::herdr;
use super::layout::{orchestrations::prune_orchestrators, release_space};
use super::plugin;
use super::proc::{Done, Fail, exe, fail, git, run_inherit, step, text};
use super::prune::{has_own_commits, own_files};
use super::repo::{Repo, VITA, compose_args, offset_of};
use super::setups::{setup_at, setup_text, setups_of};
use breadboard_core::git::{is_inside, own_commits};
use breadboard_core::json::str_of;
use breadboard_core::record::{self, origin_of};
use breadboard_core::time::{now_ms, to_iso};
use serde_json::Value;
use std::cell::OnceCell;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const USAGE: &str = "usage: pnpm wt:remove <feature> [--keep-branch] [--close-space]";
const TAKES: Takes = Takes {
    switches: &["--keep-branch", "--close-space"],
    valued: &[],
    lists: &[],
    words: 1,
    about: "Removes the worktree of <feature> (or its folder, platform-<feature>): its own stack and its data, the worktree, its branch once all of it is in develop, the empty claim on origin, its herdr tab. Refused while anything of anyone's is still in it. --keep-branch keeps the branch, merged or not; --close-space closes its whole herdr space, conversations too.",
    usage: USAGE,
};

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `--keep-branch`: the branch stays, merged or not.
    pub is_keeping_branch: bool,
    /// `--close-space`: the whole herdr space goes, conversations too.
    pub is_closing_space: bool,
    /// wt:prune fetched a moment ago.
    pub is_fetched: bool,
}

pub fn command(args: &Args) -> Done {
    if args.check(&TAKES)? {
        return Ok(());
    }
    // The feature in any place: its flags all stand alone (`--keep-branch 303-x` too).
    let Some(given) = args.word() else {
        return fail(USAGE);
    };
    let repo = Repo::here()?;
    let feature = &feature_given(&repo.main, given);
    let options = Options {
        is_keeping_branch: args.has("--keep-branch"),
        is_closing_space: args.has("--close-space"),
        is_fetched: false,
    };
    // The plugin is checked once nothing refused it (#230), in remove().
    remove(&repo, feature, &options).map(|_| ())
}

/// The feature `given` names: itself, or the folder of a worktree as a path
/// (`../platform-303-x`, `platform-303-x/`) or its name.
pub fn feature_given(main: &Path, given: &str) -> String {
    let base = Path::new(given.trim_end_matches('/'))
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| given.to_string());
    let main_name = main.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    base.strip_prefix(&format!("{main_name}-"))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(&base)
        .to_string()
}

/// Whether the claim of `branch` on origin can go with its worktree (#272):
/// nothing on it develop lacks and no open MR builds on it; the reason it goes.
pub fn empty_claim(is_on_origin: bool, has_own: bool, open_mrs: Option<usize>) -> Option<&'static str> {
    (is_on_origin && !has_own && open_mrs == Some(0)).then_some("nothing on it develop lacks, no open MR")
}

/// `rm -rf`, again if a writer strikes once more in between.
fn remove_dir(dir: &Path) {
    for _ in 0..10 {
        if fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Every MR of `branch`, None when GitLab does not answer: one read for
/// both questions of a removal, an open MR on its claim and when it was
/// merged (Node asked twice, #305).
fn mrs_of(repo: &Repo, branch: &str) -> Option<Vec<Value>> {
    let endpoint = format!(
        "projects/:fullpath/merge_requests?state=all&per_page=100&source_branch={}",
        gitlab::encode(branch)
    );
    gitlab::api(&repo.main, &endpoint)?.as_array().cloned()
}

/// The uncommitted files a refusal lists.
const SHOWN: usize = 10;

/// Why a branch with `own` commits develop lacks stays, and what to do,
/// by its MRs (None: GitLab didn't answer).
fn not_in_develop(branch: &str, own: usize, mrs: Option<&[Value]>) -> String {
    let commits = if own == 1 {
        "1 commit that is".to_string()
    } else {
        format!("{own} commits that are")
    };
    let iid = |mr: &Value| mr.get("iid").and_then(Value::as_i64).unwrap_or_default();
    let open = mrs.and_then(|mrs| mrs.iter().find(|mr| str_of(mr, "state") == Some("opened")));
    let merged = mrs.and_then(|mrs| mrs.iter().find(|mr| str_of(mr, "state") == Some("merged")));
    let keep = "or keep the branch with --keep-branch";
    match (mrs, open, merged) {
        (_, Some(mr), _) => format!(
            "{branch} has {commits} not in develop, its MR !{} is open. Merge its MR first, {keep}",
            iid(mr)
        ),
        (_, None, Some(mr)) => format!(
            "{branch} has {commits} not in develop, its MR !{} is merged without them. An MR of their own (pnpm wt:mr), {keep}",
            iid(mr)
        ),
        (Some(_), None, None) => {
            format!("{branch} has {commits} not in develop and no MR. pnpm wt:mr, {keep}")
        }
        (None, ..) => format!("{branch} has {commits} not in develop. Merge its MR first, {keep}"),
    }
}

/// How many of `mrs` are open.
fn open_count(mrs: &[Value]) -> usize {
    mrs.iter().filter(|mr| str_of(mr, "state") == Some("opened")).count()
}

/// The last merged one of `mrs`: the one its changelog lists.
fn last_merged(mrs: &[Value]) -> Option<&Value> {
    mrs.iter()
        .filter(|mr| str_of(mr, "state") == Some("merged"))
        .filter(|mr| str_of(mr, "merged_at").is_some_and(|at| !at.is_empty()))
        .max_by(|a, b| str_of(a, "merged_at").cmp(&str_of(b, "merged_at")))
}

/// When the last merged one of `mrs` was merged.
fn merged_at(mrs: &[Value]) -> Option<String> {
    last_merged(mrs)
        .and_then(|mr| str_of(mr, "merged_at"))
        .map(String::from)
}

/// Whether the changelog `text` lists the MR `iid` (changelog.mjs `listedIn`).
fn lists_mr(text: &str, iid: i64) -> bool {
    text.contains(&format!("[!{iid}]("))
}

/// Every MR the changelog `text` lists (`[!<iid>](`), in order.
fn listed_mrs(text: &str) -> Vec<i64> {
    text.split("[!")
        .skip(1)
        .filter_map(|rest| rest.split_once("](").and_then(|(iid, _)| iid.parse().ok()))
        .collect()
}

/// What a removal does about its orchestration's changelog (#305, #306).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Merge {
    /// It lists the merge already (the overview or an earlier removal wrote it): left as it is.
    Listed,
    /// It misses the merge (`!iid`; None: no merged MR to name): rewritten.
    Missing(Option<i64>),
    /// GitLab did not answer: a rewrite would get nothing either and blank it, so it stays.
    Unanswered,
}

/// What a removal of a merged worktree of orchestration `name` does about
/// its changelog, by the MRs of its branch (None: GitLab did not answer).
fn merge_of(main: &Path, name: &str, mrs: Option<&[Value]>) -> Merge {
    let Some(mrs) = mrs else {
        return Merge::Unanswered;
    };
    let iid = last_merged(mrs).and_then(|mr| mr.get("iid")).and_then(Value::as_i64);
    let text = fs::read_to_string(record::changelog_file(main, name)).unwrap_or_default();
    match iid {
        Some(iid) if lists_mr(&text, iid) => Merge::Listed,
        iid => Merge::Missing(iid),
    }
}

/// The step that says an orchestration's changelog stays as it is.
pub fn not_rewritten(name: &str, reason: &str) -> String {
    format!("the changelog of {name} is not rewritten: {reason}; pnpm wt:orchestrate {name} --changelog tries again")
}

/// Why a changelog stays: GitLab gave nothing to rewrite it from.
pub const UNANSWERED: &str = "GitLab did not answer";

/// An orchestration's changelog a removal leaves to wt:prune: rewritten once
/// after all removals, or said to stay when GitLab didn't answer one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub name: String,
    pub is_answered: bool,
}

/// Writes `text` through a temporary file: the tab reading it never sees half of it.
fn write_atomic(file: &Path, text: &str) -> std::io::Result<()> {
    let temporary = format!("{}.{}.tmp", file.display(), std::process::id());
    fs::write(&temporary, text)?;
    fs::rename(&temporary, file)
}

/// The merges the changelog `before` listed that `after` lost: a merge never
/// becomes unmerged, so GitLab did not answer for them.
fn lost_merges(before: &str, after: &str) -> Vec<i64> {
    let kept = listed_mrs(after);
    listed_mrs(before)
        .into_iter()
        .filter(|iid| !kept.contains(iid))
        .collect()
}

/// A rewrite of an orchestration's changelog (`wt:orchestrate <name>
/// --changelog`, it asks GitLab per issue) running beside the rest of a
/// removal, waited for before the command exits, as today's awaited it: a
/// process left running would outlive the command, race the next removal's
/// rewrite of the same file and drop its errors.
pub struct Rewrite {
    name: String,
    file: std::path::PathBuf,
    /// The changelog before: put back when the rewrite loses a merge it listed.
    before: Option<String>,
    child: std::io::Result<Child>,
}

impl Rewrite {
    /// Starts the rewrite of the changelog of `name`.
    pub fn start(repo: &Repo, name: &str) -> Rewrite {
        let file = record::changelog_file(&repo.main, name);
        let before = fs::read_to_string(&file).ok();
        let child = Command::new(exe())
            .args(["wt:orchestrate", name, "--changelog"])
            .current_dir(&repo.main)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        Rewrite {
            name: name.to_string(),
            file,
            before,
            child,
        }
    }

    /// Waits for it: quiet when it worked, a failure said with the way to
    /// retry. A rewrite that lost merges the changelog listed got nothing
    /// from GitLab for them: the changelog before is put back (#306).
    pub fn wait(self) {
        let name = &self.name;
        let reason = match self.child.and_then(Child::wait_with_output) {
            Ok(done) if done.status.success() => {
                let Some(before) = &self.before else {
                    return;
                };
                let after = fs::read_to_string(&self.file).unwrap_or_default();
                if lost_merges(before, &after).is_empty() {
                    return;
                }
                if let Err(error) = write_atomic(&self.file, before) {
                    step(&format!(
                        "the changelog of {name} lost merges GitLab did not answer for, and could not be put back: {error}; pnpm wt:orchestrate {name} --changelog tries again"
                    ));
                    return;
                }
                UNANSWERED.to_string()
            }
            Ok(done) => String::from_utf8_lossy(&done.stderr)
                .lines()
                .find_map(|line| line.strip_prefix("✗ "))
                .unwrap_or("it failed")
                .to_string(),
            Err(error) => error.to_string(),
        };
        step(&not_rewritten(name, &reason));
    }
}

/// Removes the worktree of `feature` (remove.mjs), its steps said as they
/// run. The orchestration whose changelog misses the merge: rewritten here,
/// or by wt:prune (is_fetched, it gets the name) once after all removals.
pub fn remove(repo: &Repo, feature: &str, options: &Options) -> Result<Option<Pending>, Fail> {
    let dir = repo.dir_of(feature);
    let dir_text = text(&dir);
    let trees = repo.trees();
    let Some(worktree) = trees.iter().find(|tree| tree.dir == dir).cloned() else {
        let names: Vec<String> = trees.iter().map(|tree| repo.feature_of(&tree.dir)).collect();
        return fail(format!(
            "{dir_text} is not a worktree (pnpm wt:list); the worktrees: {}",
            if names.is_empty() {
                "none".into()
            } else {
                names.join(", ")
            }
        ));
    };
    if std::env::current_dir().is_ok_and(|here| is_inside(&here, &dir)) {
        return fail(format!("run it from outside {dir_text}"));
    }
    // A handed-over agent still in it: EnterWorktree fails once its directory is gone (#252).
    let left = left_in(&dir, &running_claudes(), bound_session(&dir).as_deref());
    if !left.is_empty() {
        return fail(format!(
            "{} still stands in {feature} (it enters its next worktree, or as a free agent the orchestration's checkout): again once it left",
            left_text(&left)
        ));
    }
    // A start sets it up or moves its agent out of it: git would pull the directory from under it (#272).
    let origin = origin_of(&dir);
    if let Some(origin) = &origin {
        let setups = setups_of(&repo.main, &origin.name);
        if let Some(setup) = setup_at(&setups, &dir_text) {
            let what = if setup.dir.as_deref() == Some(dir_text.as_str()) {
                "sets up"
            } else {
                "moves its agent out of"
            };
            return fail(format!("{} {what} {feature}: again once it is done", setup_text(setup)));
        }
    }
    let status = git(&dir, &["status", "--porcelain"], None);
    if !status.out().is_empty() {
        // Which ones, above the ✗ (that stays the last line, other processes read it).
        let lines: Vec<&str> = status.stdout.lines().filter(|line| !line.is_empty()).collect();
        step(&format!("uncommitted in {feature}:"));
        for line in lines.iter().take(SHOWN) {
            println!("  {line}");
        }
        if lines.len() > SHOWN {
            println!("  and {} more (git status)", lines.len() - SHOWN);
        }
        return fail(format!(
            "{feature} has uncommitted changes, commit or discard them first"
        ));
    }

    let branch = worktree.branch.clone();
    let mut is_merged = false;
    if let Some(branch) = &branch {
        if !options.is_fetched {
            git(&repo.main, &["fetch", "--prune"], None);
        }
        let own = own_commits(&dir, branch);
        is_merged = own == 0;
        if !is_merged && !options.is_keeping_branch {
            return fail(not_in_develop(branch, own, mrs_of(repo, branch).as_deref()));
        }
    }

    if offset_of(&dir) != 0 {
        step(&format!("stop the own stack of {feature} and delete its data"));
        let args = compose_args(&["down", "-v"]);
        if !run_inherit("docker", &args, &dir.join(VITA)) {
            return fail(format!("docker {} failed", args.join(" ")));
        }
    }
    step(&format!("git worktree remove {dir_text}"));
    // Its binding goes first: the overview only rewrites a binding that is there.
    let _ = fs::remove_file(session_file(&dir));
    if !run_inherit("git", &["worktree", "remove", &dir_text], &repo.main) {
        // git unregisters it and deletes the files, then fails on a directory something wrote
        // into meanwhile. Nothing of anyone's work left: done; anything else stops here.
        let is_gone = !repo.worktrees().iter().any(|tree| tree.dir == dir);
        let files = if is_gone && dir.exists() {
            own_files(&repo.main, &dir)
        } else {
            Vec::new()
        };
        if !is_gone || !files.is_empty() {
            let left = if files.is_empty() {
                String::new()
            } else {
                format!(", left in it: {}", files.join(", "))
            };
            return fail(format!("git worktree remove {dir_text} failed{left}"));
        }
        remove_dir(&dir);
    }
    // What the removal left behind once git let go: ignored folders a process still wrote into (#268).
    if dir.exists() {
        let files = own_files(&repo.main, &dir);
        if files.is_empty() {
            remove_dir(&dir);
        } else {
            step(&format!("left in {dir_text}: {}", files.join(", ")));
        }
    }
    let mrs = OnceCell::new();
    if let Some(branch) = branch.as_deref().filter(|_| is_merged && !options.is_keeping_branch) {
        step(&format!("delete {branch} (all of it is in develop)"));
        if !run_inherit("git", &["branch", "-D", branch], &repo.main) {
            return fail(format!("git branch -D {branch} failed"));
        }
        // Its claim on origin too, while empty: the issue can start again (#272).
        let remote = format!("refs/remotes/origin/{branch}");
        let is_on_origin = git(&repo.main, &["rev-parse", "--verify", "--quiet", &remote], None).is_ok;
        if is_on_origin && !has_own_commits(&remote, &repo.main) {
            let open_mrs = mrs.get_or_init(|| mrs_of(repo, branch)).as_deref().map(open_count);
            if let Some(claim) = empty_claim(true, false, open_mrs) {
                step(&format!("delete {branch} on origin ({claim})"));
                run_inherit("git", &["push", "--quiet", "origin", "--delete", branch], &repo.main);
            } else if open_mrs.is_none() {
                // Not deleted blind: an open MR may build on it.
                step(&format!(
                    "kept {branch} on origin: GitLab did not answer whether an open MR builds on it; without one: git push origin --delete {branch}"
                ));
            }
        }
    }
    // The overview of the orchestration lists it under "done and removed", with the times.
    if let (Some(origin), Some(branch)) = (&origin, &branch) {
        let merged_at = if is_merged {
            mrs.get_or_init(|| mrs_of(repo, branch)).as_deref().and_then(merged_at)
        } else {
            None
        };
        let _ = record::mark_removed(
            &repo.main,
            &origin.name,
            branch,
            feature,
            &to_iso(now_ms()),
            merged_at.as_deref(),
        );
    }
    // Its changelog lists the merge, unless it does already (#305): started
    // now, beside herdr's calls, waited for after the ✓ (remove.mjs).
    // GitLab silent: a rewrite would get nothing either and blank it (#306).
    let changelog = origin
        .as_ref()
        .filter(|_| is_merged)
        .map(|origin| {
            let mrs = mrs.get().and_then(Option::as_deref);
            (origin.name.clone(), merge_of(&repo.main, &origin.name, mrs))
        })
        .filter(|(_, merge)| *merge != Merge::Listed);
    let rewrite = match &changelog {
        Some((name, Merge::Missing(iid))) if !options.is_fetched => {
            let merge = iid.map(|iid| format!(" (!{iid} is not in it yet)")).unwrap_or_default();
            step(&format!("rewrite the changelog of {name}{merge}"));
            Some(Rewrite::start(repo, name))
        }
        Some((name, Merge::Unanswered)) if !options.is_fetched => {
            step(&not_rewritten(name, UNANSWERED));
            None
        }
        _ => None,
    };
    // One stretch of herdr calls, no child process in between: its reads are remembered.
    herdr::remembering(|| {
        if let Some(line) = release_space(feature, options.is_closing_space) {
            step(&format!("herdr: {line}"));
        }
        // Its origin went with the directory; the orchestration's space goes with its last worktree.
        if herdr::is_in() {
            for line in prune_orchestrators(repo) {
                step(&format!("herdr: {line}"));
            }
        }
    });
    // The plugin may have been linked from the tree that just went (#230);
    // wt:prune (is_fetched) checks once after all its removals.
    if !options.is_fetched {
        plugin::heal(&repo.main);
    }
    let kept = match &branch {
        Some(branch) if !is_merged || options.is_keeping_branch => format!(", {branch} kept"),
        _ => String::new(),
    };
    println!("\n✓ {feature} removed{kept}");
    if let Some(rewrite) = rewrite {
        rewrite.wait();
    }
    // wt:prune rewrites each orchestration's once, after all its removals.
    Ok(changelog.filter(|_| options.is_fetched).map(|(name, merge)| Pending {
        name,
        is_answered: merge != Merge::Unanswered,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_read_answers_both_questions() {
        let mrs = vec![
            serde_json::json!({ "state": "closed", "merged_at": null }),
            serde_json::json!({ "state": "merged", "merged_at": "2026-10-01T10:00:00Z" }),
            serde_json::json!({ "state": "opened" }),
        ];
        assert_eq!(open_count(&mrs), 1);
        assert_eq!(merged_at(&mrs).as_deref(), Some("2026-10-01T10:00:00Z"));
        assert_eq!(merged_at(&mrs[..1]), None);
    }

    #[test]
    fn the_changelog_is_rewritten_only_for_a_merge_it_misses() {
        let mrs = vec![
            serde_json::json!({ "iid": 7, "state": "merged", "merged_at": "2026-09-01T10:00:00Z" }),
            serde_json::json!({ "iid": 9, "state": "merged", "merged_at": "2026-10-01T10:00:00Z" }),
            serde_json::json!({ "iid": 11, "state": "opened" }),
        ];
        assert_eq!(last_merged(&mrs).and_then(|mr| mr["iid"].as_i64()), Some(9));
        let text = "- [!9](https://x/merge_requests/9) [#4](https://x/issues/4) a · 10:00";
        assert!(lists_mr(text, 9));
        assert!(!lists_mr(text, 7));
        assert!(!lists_mr("[!90](u)", 9));
        let main = Path::new("/nowhere/platform");
        // No changelog yet, no merged MR: rewritten; GitLab silent: left as it is (#306).
        assert_eq!(merge_of(main, "o", Some(&mrs)), Merge::Missing(Some(9)));
        assert_eq!(merge_of(main, "o", None), Merge::Unanswered);
        assert_eq!(merge_of(main, "o", Some(&mrs[2..])), Merge::Missing(None));
    }

    #[test]
    fn a_rewrite_that_loses_a_merge_got_nothing_from_gitlab() {
        let before = "## 2026-10-01\n\n- [!9](u/9) [#4](u/4) a · 10:00\n- [!12](u/12) b · 11:00\n";
        assert_eq!(listed_mrs(before), vec![9, 12]);
        assert_eq!(listed_mrs("[!x](u) [!](u) [!7]"), Vec::<i64>::new());
        assert_eq!(
            lost_merges(before, "# o: changelog\n\nNothing merged yet.\n"),
            vec![9, 12]
        );
        assert_eq!(lost_merges(before, "- [!12](u/12) b\n"), vec![9]);
        assert!(lost_merges(before, &format!("- [!15](u/15) c\n{before}")).is_empty());
        assert!(lost_merges("", "Nothing merged yet.").is_empty());
        assert_eq!(
            not_rewritten("breadboard", UNANSWERED),
            "the changelog of breadboard is not rewritten: GitLab did not answer; pnpm wt:orchestrate breadboard --changelog tries again"
        );
    }

    #[test]
    fn says_why_the_commits_keep_the_branch() {
        let mr = |iid: i64, state: &str| serde_json::json!({ "iid": iid, "state": state });
        let b = "feature/4-x";
        assert_eq!(
            not_in_develop(b, 2, Some(&[mr(9, "opened"), mr(7, "merged")])),
            "feature/4-x has 2 commits that are not in develop, its MR !9 is open. Merge its MR first, or keep the branch with --keep-branch"
        );
        assert!(
            not_in_develop(b, 1, Some(&[mr(7, "merged")]))
                .contains("1 commit that is not in develop, its MR !7 is merged without them")
        );
        assert!(not_in_develop(b, 1, Some(&[])).contains("and no MR. pnpm wt:mr"));
        assert!(
            not_in_develop(b, 3, None).ends_with(
                "3 commits that are not in develop. Merge its MR first, or keep the branch with --keep-branch"
            )
        );
    }

    #[test]
    fn takes_a_feature_or_its_folder() {
        let main = Path::new("/r/platform");
        assert_eq!(feature_given(main, "303-x"), "303-x");
        assert_eq!(feature_given(main, "platform-303-x"), "303-x");
        assert_eq!(feature_given(main, "../platform-303-x/"), "303-x");
        assert_eq!(feature_given(main, "/r/platform-303-x"), "303-x");
        assert_eq!(feature_given(main, "platform-"), "platform-");
    }

    #[test]
    fn an_empty_claim_goes_only_without_work_and_mrs() {
        assert_eq!(
            empty_claim(true, false, Some(0)),
            Some("nothing on it develop lacks, no open MR")
        );
        assert_eq!(empty_claim(true, true, Some(0)), None);
        assert_eq!(empty_claim(true, false, Some(1)), None);
        // GitLab did not answer: the claim stays.
        assert_eq!(empty_claim(true, false, None), None);
        assert_eq!(empty_claim(false, false, Some(0)), None);
    }
}
