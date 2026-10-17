//! The detail pane (#406): a click on an MR or a worktree splits the
//! overview's pane to the right and shows that worktree's nvim there. `e`,
//! `t`, `l` and `a` switch it to nvim, the try's terminal, the stack's logs
//! (its dev server while the app is down) and the agent's conversation; a
//! click on the row it shows, or Esc, closes it. One per overview: a click
//! on another row ends what runs there and runs its command in the same
//! pane. Its pane is kept in a file next to the overview's folds, so a
//! reload of the overview in its pane finds it again.
//!
//! Everything here runs on the actor's thread: it waits for herdr, nvim and
//! a headless run of the agent.

use super::herdr::quote;
use breadboard_core::git::feature_of;
use breadboard_core::herdr::{herdr_here, is_idle_now, is_in_herdr};
use breadboard_core::reader::View;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the detail pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shows {
    Editor,
    Try,
    Logs,
    Agent,
}

impl Shows {
    /// Its key in the overview.
    pub fn of_key(key: char) -> Option<Shows> {
        Some(match key {
            'e' => Shows::Editor,
            't' => Shows::Try,
            'l' => Shows::Logs,
            'a' => Shows::Agent,
            _ => return None,
        })
    }

    fn word(self) -> &'static str {
        match self {
            Shows::Editor => "nvim",
            Shows::Try => "its try",
            Shows::Logs => "its logs",
            Shows::Agent => "its agent",
        }
    }
}

/// The open detail pane: its herdr pane, the worktree and what runs there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
    /// The overview's own pane it was split from: another overview's file is no one's.
    pub overview: String,
    pub pane: String,
    pub dir: PathBuf,
    pub shows: Shows,
}

/// A file opened at a line in the detail pane's nvim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct At {
    pub file: String,
    pub line: u64,
}

/// The overview's detail pane, shared with the actor's thread.
#[derive(Clone, Default)]
pub struct Detail {
    /// Where it is kept: None for a print.
    file: Option<PathBuf>,
    shown: Arc<Mutex<Option<Shown>>>,
}

/// What the overview's keys say while the pane is open.
pub const KEYS: &str = "e nvim · t try · l logs/dev · a agent · Esc closes";

/// How long what runs in the pane gets to end before a command goes in.
const END_WAIT: Duration = Duration::from_secs(3);

impl Detail {
    /// The detail pane of an overview of `view`, as its file left it.
    pub fn new(main: &Path, view: &View, is_once: bool) -> Detail {
        let file = (!is_once).then(|| file_of(main, view));
        let shown = file
            .as_deref()
            .and_then(read)
            .filter(|shown| Some(&shown.overview) == here().as_ref());
        Detail {
            file,
            shown: Arc::new(Mutex::new(shown)),
        }
    }

    /// What it shows now, None while it is closed.
    pub fn shown(&self) -> Option<Shown> {
        self.shown.lock().ok().and_then(|shown| shown.clone())
    }

    /// A click on a row of `dir`: its nvim beside, or the pane closed when it shows `dir` already.
    pub fn click(&self, main: &Path, dir: &Path) -> Result<String, String> {
        if self.shown_alive().is_some_and(|shown| shown.dir == dir) {
            return self.close(main);
        }
        self.show(main, dir, Shows::Editor, None)
    }

    /// `shows` of `dir` in the detail pane: run in the open one, else in a split made for it.
    /// `at`: a file in nvim, through its socket where nvim of `dir` runs there already.
    pub fn show(&self, main: &Path, dir: &Path, shows: Shows, at: Option<At>) -> Result<String, String> {
        let feature = feature_of(main, dir);
        let open = self.shown_alive();
        if let Some(open) = open.as_ref().filter(|open| open.dir == dir && open.shows == shows) {
            let Some(at) = &at else {
                return Ok(format!("{feature}: {} shows beside already · {KEYS}", shows.word()));
            };
            if super::super::worktree_view::editor::open_on_socket(&open.dir, &at.file, at.line) {
                return Ok(format!("{feature}: {} in nvim beside", at.file));
            }
        }
        let command = command_of(main, dir, shows, at.as_ref())?;
        let pane = match open {
            Some(open) => {
                end(&open)?;
                open.pane
            }
            None => split(dir)?,
        };
        if herdr_here(&["pane", "run", &pane, &command]).is_none() {
            return Err(format!("herdr ran nothing in the detail pane {pane}"));
        }
        self.keep(Some(Shown {
            overview: here().unwrap_or_default(),
            pane,
            dir: dir.to_path_buf(),
            shows,
        }));
        Ok(format!("{feature}: {} beside · {KEYS}", shows.word()))
    }

    /// The pane closed, nvim quit first: nvim with unsaved changes keeps it open.
    pub fn close(&self, main: &Path) -> Result<String, String> {
        let Some(open) = self.shown_alive() else {
            return Err("no detail pane is open".into());
        };
        if open.shows == Shows::Editor {
            end(&open)?;
        }
        herdr_here(&["pane", "close", &open.pane]);
        self.keep(None);
        Ok(format!("{}: the detail pane closed", feature_of(main, &open.dir)))
    }

    /// What it shows while its pane is there; one herdr closed is forgotten.
    fn shown_alive(&self) -> Option<Shown> {
        let shown = self.shown()?;
        if is_alive(&shown.pane) {
            return Some(shown);
        }
        self.keep(None);
        None
    }

    fn keep(&self, shown: Option<Shown>) {
        if let Some(file) = &self.file {
            match &shown {
                Some(shown) => {
                    if let Ok(text) = serde_json::to_string(shown) {
                        let _ = std::fs::write(file, text + "\n");
                    }
                }
                None => {
                    let _ = std::fs::remove_file(file);
                }
            }
        }
        if let Ok(mut kept) = self.shown.lock() {
            *kept = shown;
        }
    }
}

/// Where an overview of `view` keeps its detail pane: an orchestration's
/// next to its folds, the others in the `.claude` of their checkout.
pub fn file_of(main: &Path, view: &View) -> PathBuf {
    match view {
        View::Orchestration(name) => breadboard_core::record::records_dir(main).join(format!("{name}.detail")),
        View::Everything => main.join(".claude").join("overview.detail"),
        View::Worktree(dir) => dir.join(".claude").join("overview.detail"),
    }
}

fn read(file: &Path) -> Option<Shown> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// The overview's own herdr pane, None outside herdr.
fn here() -> Option<String> {
    std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|pane| is_in_herdr() && !pane.is_empty())
}

/// Whether herdr still has the pane.
fn is_alive(pane: &str) -> bool {
    herdr_here(&["pane", "get", pane])
        .and_then(|result| {
            result
                .get("pane")
                .and_then(|pane| pane.get("pane_id"))
                .and_then(|id| id.as_str())
                .map(|id| id == pane)
        })
        .unwrap_or(false)
}

/// The overview's pane split to the right in `dir`, the overview keeping the focus: the new pane.
fn split(dir: &Path) -> Result<String, String> {
    let overview = here().ok_or("the overview runs in no herdr pane: nothing to split")?;
    let cwd = dir.to_string_lossy();
    let args = [
        "pane",
        "split",
        &overview,
        "--direction",
        "right",
        "--ratio",
        "0.5",
        "--cwd",
        &cwd,
        "--no-focus",
    ];
    herdr_here(&args)
        .and_then(|result| {
            result
                .get("pane")
                .and_then(|pane| pane.get("pane_id"))
                .and_then(|id| id.as_str())
                .map(String::from)
        })
        .ok_or_else(|| format!("herdr split no pane off {overview}"))
}

/// What runs in the pane ended, its shell idle again: nvim quits (one with
/// unsaved changes refuses, and that is said), anything else gets Ctrl-C
/// (twice: claude asks for a second).
fn end(open: &Shown) -> Result<(), String> {
    if is_idle_now(&open.pane) {
        return Ok(());
    }
    let wait_idle = |wait: Duration| {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if is_idle_now(&open.pane) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    };
    if open.shows == Shows::Editor {
        super::super::worktree_view::editor::remote(&open.dir, &["--remote-send", "<C-\\><C-N>:qa<CR>"]);
        if wait_idle(END_WAIT) {
            return Ok(());
        }
        return Err("nvim beside has unsaved changes: :w (or :qa!) there first".into());
    }
    for _ in 0..2 {
        herdr_here(&["pane", "send-keys", &open.pane, "ctrl+c"]);
        if wait_idle(END_WAIT / 2) {
            return Ok(());
        }
    }
    Err(format!(
        "{} still runs beside after two Ctrl-C: end it there first",
        open.shows.word()
    ))
}

/// The shell line that shows `shows` of `dir`.
fn command_of(main: &Path, dir: &Path, shows: Shows, at: Option<&At>) -> Result<String, String> {
    let cd = format!("cd {} && ", quote(&dir.to_string_lossy()));
    let feature = feature_of(main, dir);
    match shows {
        Shows::Editor => Ok(editor_command(main, dir, at)),
        Shows::Try => {
            let file = breadboard_core::actions::actions_file(dir);
            let run = breadboard_core::actions::actions_of(&file)
                .into_iter()
                .rev()
                .filter(|action| action.is_try() && action.answer.is_none())
                .find_map(|action| action.run)
                .ok_or_else(|| format!("{feature} has no open try with a terminal (--run)"))?;
            Ok(format!("{cd}{run}"))
        }
        Shows::Logs => {
            let stack = breadboard_core::stack::of(dir).ok_or_else(|| format!("{feature} has no stack"))?;
            let offset = crate::cli::lifecycle::repo::offset_of(dir);
            let stack_dir = quote(&breadboard_core::stack::dir_of(dir).to_string_lossy());
            let port = i64::from(stack.app.port) + offset;
            // The app up: the stack's logs; down: its dev server, as the space's `dev` tab ran it.
            if breadboard_core::local::is_app_of(port, dir) {
                let logs = breadboard_core::stack::compose_args(&stack, &["logs", "-f", "--tail", "200"]);
                Ok(format!("cd {stack_dir} && docker {}", logs.join(" ")))
            } else {
                Ok(format!("cd {stack_dir} && {}", stack.app.dev))
            }
        }
        Shows::Agent => {
            let id = breadboard_core::conversations::bound_session(dir)
                .ok_or_else(|| format!("{feature}'s worktree binds no conversation"))?;
            super::open_agent::wait_turn(main, dir, &id)?;
            Ok(super::open_agent::open_command(dir, &id, &feature))
        }
    }
}

/// The worktree's nvim (layout/nvim.rs), on `at` when given; Claude Code's
/// IDE plugin only when the layout cloned it already: a click clones nothing.
fn editor_command(main: &Path, dir: &Path, at: Option<&At>) -> String {
    use crate::cli::lifecycle::layout::nvim;
    let plugin = main.join(".claude/nvim/claudecode.nvim-v0.3.0");
    let ide = (nvim::is_ide_on(dir) && plugin.join("lua/claudecode").is_dir()).then_some(plugin);
    let command = nvim::editor_command(dir, None, None, ide.as_deref());
    match at {
        Some(at) => format!("{command} +{} {}", at.line.max(1), quote(&at.file)),
        None => command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_switch_what_it_shows() {
        assert_eq!(Shows::of_key('e'), Some(Shows::Editor));
        assert_eq!(Shows::of_key('t'), Some(Shows::Try));
        assert_eq!(Shows::of_key('l'), Some(Shows::Logs));
        assert_eq!(Shows::of_key('a'), Some(Shows::Agent));
        assert_eq!(Shows::of_key('x'), None);
    }

    #[test]
    fn nvim_opens_on_the_file_at_its_line() {
        let at = At {
            file: "src/a b.ts".into(),
            line: 7,
        };
        let command = editor_command(Path::new("/nowhere"), Path::new("/nowhere/p-1"), Some(&at));
        assert!(command.starts_with("cd '/nowhere/p-1'; nvim --server"), "{command}");
        assert!(command.ends_with(" +7 'src/a b.ts'"), "{command}");
    }

    #[test]
    fn kept_by_view() {
        let main = Path::new("/r/p");
        assert_eq!(
            file_of(main, &View::Orchestration("b".into())),
            Path::new("/r/p/.claude/orchestrations/b.detail")
        );
        assert_eq!(
            file_of(main, &View::Everything),
            Path::new("/r/p/.claude/overview.detail")
        );
        assert_eq!(
            file_of(main, &View::Worktree("/r/p-1".into())),
            Path::new("/r/p-1/.claude/overview.detail")
        );
    }

    #[test]
    fn a_kept_pane_reads_back() {
        let shown = Shown {
            overview: "w1:p1".into(),
            pane: "w1:p2".into(),
            dir: "/r/p-1".into(),
            shows: Shows::Logs,
        };
        let text = serde_json::to_string(&shown).unwrap();
        assert!(text.contains("\"shows\":\"logs\""), "{text}");
        assert_eq!(serde_json::from_str::<Shown>(&text).unwrap(), shown);
    }
}
