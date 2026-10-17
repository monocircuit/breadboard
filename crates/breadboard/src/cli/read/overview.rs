//! `overview`'s command line (overview/args.mjs), held before the view
//! starts: `--help`, an unknown flag, `--orchestration` with `--worktree`, a
//! `--worktree` directory in no worktree are refused here; everything else
//! is the TUI's (`tui::overview`), live or printed once (`--once`, or stdout
//! no terminal): one way to draw the overview (rewrite rule 4).

use super::args::{Takes, check, cwd, fail, flag_value};
use breadboard_core::git;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

pub const TAKES: Takes = Takes {
    valued: &["--orchestration", "--product"],
    lists: &[],
    optional: &["--worktree"],
    switches: &["--once", "--herdr"],
    words: 0,
    about: "The overview: everything, one orchestration (--orchestration <name>), every orchestration of a product (--product <name>) or one worktree (--worktree [<dir>]). Live in a terminal; printed once with --once or into a pipe.",
    usage: "usage: pnpm overview [--orchestration <name> | --product <name> | --worktree [<dir>]] [--once] [--herdr]",
};

/// The exit code of a command line the overview doesn't run, None to run it.
pub fn refused(args: &[String]) -> Option<ExitCode> {
    check(args, &TAKES)
        .or_else(|| exclusive(args))
        .or_else(|| no_worktree(args))
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter()
        .any(|arg| arg == flag || arg.starts_with(&format!("{flag}=")))
}

/// One view at a time: two of `--orchestration`, `--product` and `--worktree` refused together.
fn exclusive(args: &[String]) -> Option<ExitCode> {
    let given: Vec<&str> = ["--orchestration", "--product", "--worktree"]
        .into_iter()
        .filter(|flag| has(args, flag))
        .collect();
    (given.len() > 1).then(|| {
        fail(&format!(
            "{} exclude each other: one view at a time\n{}",
            given.join(" and "),
            TAKES.usage
        ))
    })
}

/// `--worktree <dir>` of a directory in no worktree of the repo: said, not
/// drawn (Node printed `null` and went on).
fn no_worktree(args: &[String]) -> Option<ExitCode> {
    if !has(args, "--worktree") {
        return None;
    }
    let cwd = cwd();
    let dir = absolute(&cwd, &flag_value(args, "--worktree").unwrap_or_else(|| ".".into()));
    let main = git::main_dir(&cwd)?;
    let is_known = git::worktrees(&main).iter().any(|tree| git::is_inside(&dir, &tree.dir));
    (!is_known).then(|| {
        fail(&format!(
            "{} is not a worktree of the repo: {run} wt:list shows them",
            dir.display(),
            run = breadboard_core::config::run_word()
        ))
    })
}

/// `path.resolve(cwd, dir)`: `.` and `..` folded without touching the disk.
fn absolute(cwd: &Path, dir: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for part in cwd.join(dir).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                path.pop();
            }
            other => path.push(other),
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn refuses_two_views_and_unknown_flags() {
        assert!(exclusive(&strings(&["--orchestration", "b", "--worktree"])).is_some());
        assert!(exclusive(&strings(&["--orchestration=b", "--once"])).is_none());
        assert!(exclusive(&strings(&["--product", "vita", "--orchestration", "b"])).is_some());
        assert!(check(&strings(&["--product", "vita", "--once"]), &TAKES).is_none());
        assert!(check(&strings(&["--onec"]), &TAKES).is_some());
        assert!(check(&strings(&["--orchestration", "b", "--once", "--herdr"]), &TAKES).is_none());
    }

    #[test]
    fn folds_a_relative_dir() {
        assert_eq!(
            absolute(Path::new("/r/platform/src"), "../../platform-3-x"),
            PathBuf::from("/r/platform-3-x")
        );
        assert_eq!(absolute(Path::new("/r/platform"), "."), PathBuf::from("/r/platform"));
    }
}
