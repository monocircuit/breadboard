//! `arch:deltas [--worktree [<dir>] | --mr <iid> | --epic [<branch>]] [--out <file>]`
//! (#439): which elements of the repo's LikeC4 model (`[architecture]`) a
//! change adds, changes or removes, as JSON for the architecture view
//! (#440). A worktree is its work against its base, uncommitted changes
//! included; an MR is GitLab's diff of it; an epic or batch branch is what it
//! has that the default branch lacks.

use super::args::{Takes, cwd, fail, flag_value};
use super::gitlab;
use breadboard_core::architecture::{self, Deltas, Model};
use breadboard_core::git::{self, Base, git};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: pnpm arch:deltas [--worktree [<dir>] | --mr <iid> | --epic [<branch>]] [--out <file>]

  --worktree [<dir>]   a worktree's work against its base, uncommitted changes included (the default: the one you stand in)
  --mr <iid>           an MR's changes, as GitLab diffs them
  --epic [<branch>]    an epic or batch branch against the default branch (alone: your worktree's base)
  --out <file>         the JSON into <file> instead of stdout";

pub const TAKES: Takes = Takes {
    valued: &["--mr", "--out"],
    lists: &[],
    optional: &["--worktree", "--epic"],
    switches: &[],
    words: 0,
    about: "Which elements of the LikeC4 model ([architecture] of breadboard.toml) an MR, a worktree or an epic adds, changes or removes, as JSON for the architecture view.",
    usage: USAGE,
};

/// What `arch:deltas` writes.
#[derive(Debug, Serialize)]
struct Written {
    /// `worktree`, `mr` or `epic`.
    scope: &'static str,
    /// The worktree's branch, `!<iid>`, the epic's branch.
    name: String,
    /// The commit compared against.
    base: String,
    /// The commit compared; a worktree's HEAD, its uncommitted changes counted too.
    head: String,
    /// The model's folder.
    model: String,
    #[serde(flatten)]
    deltas: Deltas,
}

/// The two sides of a change: its base, its head (a commit, or a worktree's
/// files as they are) and the files it changed.
struct Change {
    scope: &'static str,
    name: String,
    dir: PathBuf,
    base: String,
    head: String,
    is_worktree: bool,
    files: BTreeSet<String>,
}

pub fn run(args: &[String]) -> ExitCode {
    let loaded = breadboard_core::config::here();
    let Some(architecture) = loaded.config.architecture.clone() else {
        return fail(
            "breadboard.toml has no [architecture]: name the folder of the LikeC4 model there (model = \"architecture\")",
        );
    };
    let scopes = ["--worktree", "--mr", "--epic"]
        .iter()
        .filter(|flag| has_flag(args, flag))
        .count();
    if scopes > 1 {
        return fail(&format!("one of --worktree, --mr and --epic at a time\n{USAGE}"));
    }
    let change = if let Some(iid) = flag_value(args, "--mr") {
        mr_change(&iid)
    } else if has_flag(args, "--epic") {
        epic_change(flag_value(args, "--epic"))
    } else {
        worktree_change(flag_value(args, "--worktree").map_or_else(cwd, PathBuf::from))
    };
    let change = match change {
        Ok(change) => change,
        Err(message) => return fail(&message),
    };
    let mut before = architecture::model_at(&change.dir, &change.base, &architecture.model);
    let mut after: Model = if change.is_worktree {
        architecture::model_in(&change.dir, &architecture.model)
    } else {
        architecture::model_at(&change.dir, &change.head, &architecture.model)
    };
    // A head from before the model: its files mapped with the model of the checkout you stand in.
    if after.elements.is_empty()
        && let Ok(here) = top_of(&cwd())
    {
        after = architecture::model_in(&here, &architecture.model);
        before = after.clone();
    }
    let written = Written {
        scope: change.scope,
        name: change.name,
        base: change.base,
        head: change.head,
        model: architecture.model,
        deltas: architecture::deltas(&before, &after, &change.files),
    };
    let json = serde_json::to_string_pretty(&written).unwrap_or_default() + "\n";
    match flag_value(args, "--out") {
        Some(out) => match std::fs::write(&out, json) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => fail(&format!("could not write {out}: {error}")),
        },
        None => {
            print!("{json}");
            ExitCode::SUCCESS
        }
    }
}

/// Whether `flag` is given, alone or with its value.
fn has_flag(args: &[String], flag: &str) -> bool {
    let prefix = format!("{flag}=");
    args.iter().any(|arg| arg == flag || arg.starts_with(&prefix))
}

/// The root of the checkout `dir` stands in.
fn top_of(dir: &Path) -> Result<PathBuf, String> {
    let out = git(dir, &["rev-parse", "--show-toplevel"]);
    if !out.is_ok || out.out().is_empty() {
        return Err(format!("{} is no git checkout", dir.display()));
    }
    Ok(PathBuf::from(out.out()))
}

/// The commit `rev` names in `dir`, None where it has none.
fn commit_of(dir: &Path, rev: &str) -> Option<String> {
    let out = git(dir, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]);
    Some(out.out().to_string()).filter(|sha| out.is_ok && !sha.is_empty())
}

fn worktree_change(dir: PathBuf) -> Result<Change, String> {
    let dir = top_of(&dir)?;
    let base = git::merge_base(&dir).ok_or_else(|| {
        format!(
            "{} has no merge base with {}: fetch it (git fetch origin)",
            dir.display(),
            git::base_of(&dir).remote
        )
    })?;
    let head = commit_of(&dir, "HEAD").unwrap_or_default();
    Ok(Change {
        scope: "worktree",
        name: git(&dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .out()
            .to_string(),
        files: git::work_files(&dir),
        dir,
        base,
        head,
        is_worktree: true,
    })
}

fn epic_change(branch: Option<String>) -> Result<Change, String> {
    let here = top_of(&cwd())?;
    let branch = branch.unwrap_or_else(|| git::base_of(&here).local);
    let branch = branch.strip_prefix("origin/").unwrap_or(&branch).to_string();
    let develop = Base::develop();
    if branch == develop.local {
        return Err(format!(
            "{branch} is the default branch: name an epic or batch branch (--epic batch/<product>-<n>)"
        ));
    }
    let head = commit_of(&here, &format!("origin/{branch}"))
        .ok_or_else(|| format!("origin has no {branch} (as of the last fetch: git fetch origin)"))?;
    let out = git(&here, &["merge-base", &develop.remote, &head]);
    if !out.is_ok || out.out().is_empty() {
        return Err(format!("{branch} has no merge base with {}", develop.remote));
    }
    let base = out.out().to_string();
    Ok(Change {
        scope: "epic",
        name: branch,
        files: architecture::files_between(&here, &base, &head),
        dir: here,
        base,
        head,
        is_worktree: false,
    })
}

fn mr_change(iid: &str) -> Result<Change, String> {
    if iid.is_empty() || !iid.trim_start_matches('!').chars().all(|char| char.is_ascii_digit()) {
        return Err(format!("{iid} is no MR number\n{USAGE}"));
    }
    let iid = iid.trim_start_matches('!');
    let main = git::main_dir(&cwd())
        .ok_or("not in a git checkout of the repo: run it in the main checkout or one of its worktrees")?;
    let mr = gitlab::get(&main, &format!("projects/:fullpath/merge_requests/{iid}"))?;
    let refs = mr.get("diff_refs");
    let sha = |key: &str| {
        refs.and_then(|refs| refs.get(key))
            .and_then(|sha| sha.as_str())
            .map(String::from)
            .ok_or_else(|| format!("GitLab has no diff of !{iid} yet ({key} missing)"))
    };
    let (base, head) = (sha("base_sha")?, sha("head_sha")?);
    // The MR's head and its target may not be fetched here yet.
    if commit_of(&main, &head).is_none() {
        git(
            &main,
            &["fetch", "--quiet", "origin", &format!("refs/merge-requests/{iid}/head")],
        );
    }
    if commit_of(&main, &base).is_none()
        && let Some(target) = mr.get("target_branch").and_then(|target| target.as_str())
    {
        git(&main, &["fetch", "--quiet", "origin", target]);
    }
    for sha in [&base, &head] {
        if commit_of(&main, sha).is_none() {
            return Err(format!("{sha} of !{iid} is not here, even after a fetch"));
        }
    }
    Ok(Change {
        scope: "mr",
        name: format!("!{iid}"),
        files: architecture::files_between(&main, &base, &head),
        dir: main,
        base,
        head,
        is_worktree: false,
    })
}
