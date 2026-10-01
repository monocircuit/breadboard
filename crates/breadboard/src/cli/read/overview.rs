//! `overview` printed once (overview.mjs `--once`, args.mjs): with `--once`,
//! or whenever stdout is no terminal (a pipe always prints once). It reads
//! the local state and GitLab side by side, prints the view and exits 0; a
//! GitLab failure is drawn in the view, not an exit code. It changes nothing:
//! no answers delivered, no merges, no herdr writes.
//!
//! What it shows is what core gives today (records, actions, git) and the
//! open and merged MRs in two GitLab calls; the live view (`tui`) draws the
//! full sections and will print this frame itself once it can.

mod clock;
mod everything;
mod frame;
mod orchestration;
mod paint;
mod theme;
mod worktree;

use super::args::{Takes, check, cwd, fail};
use breadboard_core::git;
use frame::Frame;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub const TAKES: Takes = Takes {
    valued: &["--orchestration"],
    lists: &[],
    optional: &["--worktree"],
    switches: &["--once", "--herdr"],
    words: 0,
    about: "The overview: everything, one orchestration (--orchestration <name>) or one worktree (--worktree [<dir>]). Live in a terminal; printed once with --once or into a pipe (FORCE_COLOR keeps its colours).",
    usage: "usage: pnpm overview [--orchestration <name> | --worktree [<dir>]] [--once] [--herdr]",
};

/// The command's exit code when it prints once (or its command line is
/// refused, or asks for help), None for the live view.
pub fn run(args: &[String]) -> Option<ExitCode> {
    let is_help = args.iter().any(|arg| arg == "--help" || arg == "-h");
    let is_once = args.iter().any(|arg| arg == "--once") || !std::io::stdout().is_terminal();
    if !is_once && !is_help {
        return None;
    }
    Some(check(args, &TAKES).unwrap_or_else(|| print_once(args)))
}

/// Which view: `--orchestration <name>`, `--worktree <dir>`, or everything.
#[derive(Debug, Clone, PartialEq)]
enum View {
    Orchestration(String),
    Worktree(PathBuf),
    Everything,
}

fn view_of(args: &[String], cwd: &Path) -> View {
    let after = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .map(|at| args.get(at + 1).cloned())
    };
    if let Some(dir) = after("--worktree") {
        return View::Worktree(absolute(cwd, dir.as_deref().unwrap_or(".")));
    }
    match after("--orchestration") {
        Some(Some(name)) => View::Orchestration(name),
        _ => View::Everything,
    }
}

/// `path.resolve(cwd, dir)`: `.` and `..` folded without touching the disk.
fn absolute(cwd: &Path, dir: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for part in cwd.join(dir).components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                path.pop();
            }
            other => path.push(other),
        }
    }
    path
}

fn print_once(args: &[String]) -> ExitCode {
    let cwd = cwd();
    let Some(main) = git::main_dir(&cwd) else {
        return fail("not in a git checkout of the platform repo: run it in the main checkout or one of its worktrees");
    };
    let here = std::env::var_os("INIT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.clone());
    paint::set_colour(is_colour());
    let mut frame = Frame::new(columns());
    match view_of(args, &cwd) {
        View::Orchestration(name) => orchestration::orchestration(&mut frame, &main, &name),
        View::Worktree(dir) => worktree::worktree(&mut frame, &main, &dir),
        View::Everything => everything::everything(&mut frame, &main, &here),
    }
    print!("{}", frame.text());
    ExitCode::SUCCESS
}

/// The width: the terminal's; piped, `$COLUMNS`; else 120 (tty.mjs `columns`).
fn columns() -> usize {
    let terminal = std::io::stdout()
        .is_terminal()
        .then(crossterm::terminal::size)
        .and_then(Result::ok);
    terminal
        .map(|(columns, _)| usize::from(columns))
        .filter(|columns| *columns > 0)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()?
                .trim()
                .parse()
                .ok()
                .filter(|columns: &usize| *columns > 0)
        })
        .unwrap_or(120)
}

/// Colour on a terminal or with `FORCE_COLOR` (the herdr plugin pipes into
/// `less -R`), never with `NO_COLOR` (tty.mjs `isColor`).
fn is_colour() -> bool {
    let is_set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    (std::io::stdout().is_terminal() || is_set("FORCE_COLOR")) && !is_set("NO_COLOR")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn picks_the_view_from_its_flags() {
        let cwd = Path::new("/r/platform/src");
        assert_eq!(
            view_of(&strings(&["--orchestration", "b", "--once"]), cwd),
            View::Orchestration("b".into())
        );
        assert_eq!(view_of(&strings(&["--orchestration"]), cwd), View::Everything);
        assert_eq!(
            view_of(&strings(&["--worktree", "../../platform-3-x"]), cwd),
            View::Worktree("/r/platform-3-x".into())
        );
        assert_eq!(
            view_of(&strings(&["--worktree"]), cwd),
            View::Worktree("/r/platform/src".into())
        );
        assert_eq!(view_of(&strings(&["--once", "--herdr"]), cwd), View::Everything);
    }
}
