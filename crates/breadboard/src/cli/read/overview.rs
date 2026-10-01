//! `overview` printed once (overview.mjs `--once`, args.mjs): with `--once`,
//! or whenever stdout is no terminal (a pipe always prints once). It reads
//! the local state and GitLab side by side, prints the view and exits 0; a
//! GitLab failure is drawn in the view, not an exit code. It changes nothing:
//! no answers delivered, no merges, no herdr writes.
//!
//! What it shows is what core gives today (records, actions, git) and the
//! open and merged MRs in two GitLab calls; the live view (`tui`) draws the
//! full sections and will print this frame itself once it can.

mod frame;
mod views;

use super::args::{cwd, fail};
use breadboard_core::git;
use frame::Frame;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The command's exit code when it prints once, None for the live view.
pub fn run(args: &[String]) -> Option<ExitCode> {
    let is_once = args.iter().any(|arg| arg == "--once") || !std::io::stdout().is_terminal();
    is_once.then(|| print_once(args))
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
    let mut frame = Frame::new(columns(), is_colour());
    match view_of(args, &cwd) {
        View::Orchestration(name) => views::orchestration(&mut frame, &main, &name),
        View::Worktree(dir) => views::worktree(&mut frame, &main, &dir),
        View::Everything => views::everything(&mut frame, &main, &here),
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
