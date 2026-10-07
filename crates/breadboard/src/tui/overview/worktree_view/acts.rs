//! What a click in the view of a worktree does, and what it started, kept
//! over rebuilds (actions.mjs, files.mjs, render-worktree.mjs): the delete
//! button's two clicks, `stack:up` (the button and `u`), `pnpm pg:migrate`,
//! Storybook's tab, a file in nvim. Never in a print (`--once`): it has no clicks.

use super::editor::{self, Checkout};
use super::hunks::{self, Shown};
use super::started::{self, Started};
use crate::tui::Flow;
use breadboard_core::model::worktree::{STORYBOOK_PORT, WorktreeModel};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How long the first click on delete keeps it armed (actions.mjs `ARM_MS`).
pub const ARM: Duration = Duration::from_millis(5000);
/// How often a view with something running is drawn again: its last line moves.
pub const WATCH: Duration = Duration::from_millis(500);
/// Where the vita app lives in a checkout, its `pnpm pg:migrate` runs there.
const VITA: &str = "products/vita/web";

/// A Storybook start: on its way until the row turns ●, or why it couldn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Storybook {
    Starting,
    Failed(String),
}

/// What the view started and how it went.
#[derive(Debug, Default)]
pub struct State {
    /// The first click on delete, until the second one or ARM.
    pub armed_at: Option<Instant>,
    pub removal: Option<Started>,
    pub stack: Option<Started>,
    pub migration: Option<Started>,
    pub storybook: Option<Storybook>,
    /// Why the last click on a file didn't open it; None once one did.
    pub nvim: Option<String>,
    /// The file whose hunks show in DIFF or DEVELOP, and under which row.
    pub hunks: Option<Shown>,
}

impl State {
    /// How long delete stays armed, None when it isn't.
    pub fn armed_for(&self) -> Option<Duration> {
        let left = ARM.checked_sub(self.armed_at?.elapsed())?;
        (!left.is_zero()).then_some(left)
    }

    /// Whether the stack's start shows: running, or failed with its error.
    pub fn stack_shown(&self) -> Option<&Started> {
        self.stack.as_ref().filter(|stack| !stack.is_done())
    }

    pub fn migration_shown(&self) -> Option<&Started> {
        self.migration.as_ref().filter(|migration| !migration.is_done())
    }

    /// What a read showed: Storybook answers, its start is over.
    pub fn saw(&mut self, model: &WorktreeModel) {
        if model.local.is_storybook_up {
            self.storybook = None;
        }
    }
}

/// Where the view's worktree is, for the editor and herdr.
fn checkout_of<'a>(model: &'a WorktreeModel, main: &'a Path) -> Option<Checkout<'a>> {
    let tree = model.local.tree.as_ref()?;
    Some(Checkout {
        dir: &model.local.dir,
        main,
        is_main: model.local.is_main,
        feature: &tree.name,
    })
}

/// A file in the worktree's nvim; the line under the dir says why not.
pub fn open_file(state: &mut State, model: &WorktreeModel, main: &Path, file: &str, line: Option<u64>) -> Flow {
    let Some(checkout) = checkout_of(model, main) else {
        return Flow::Same;
    };
    state.nvim = editor::open_file(&checkout, file, line).err();
    Flow::Redraw
}

/// A file's hunks under the row clicked; a second click on it closes them,
/// one on another file's row shows that one's instead (lazygit shows one).
/// `is_whole` (the `… N more lines` row): the shown ones, all their lines.
pub fn toggle_hunks(state: &mut State, file: String, at: String, is_whole: bool) -> Flow {
    // A line of several names, off its names or Enter: the next name, none after the last.
    if let Some(line) = at.strip_prefix(hunks::NEXT) {
        let next = match state.hunks.as_ref().filter(|shown| shown.at == line) {
            Some(shown) => shown.next_on_line().unwrap_or(Some(file)),
            None => Some(file),
        };
        state.hunks = next.map(|file| Shown::new(file, line.to_string()));
        return Flow::Redraw;
    }
    let open = state.hunks.as_mut().filter(|shown| shown.is_under(&at, &file));
    match open {
        Some(shown) if is_whole => shown.is_whole = true,
        Some(_) => state.hunks = None,
        None => {
            let mut shown = Shown::new(file, at);
            shown.is_whole = is_whole;
            state.hunks = Some(shown);
        }
    }
    Flow::Redraw
}

/// Esc: the hunks closed, if any show.
pub fn close_hunks(state: &mut State) -> Flow {
    match state.hunks.take() {
        Some(_) => Flow::Redraw,
        None => Flow::Same,
    }
}

/// Delete: the first click arms it, the second within ARM runs `wt:remove
/// <feature> --close-space` from the main checkout; a click while it runs does nothing.
pub fn delete(state: &mut State, model: &WorktreeModel, main: &Path) -> Flow {
    let Some(tree) = model.local.tree.as_ref() else {
        return Flow::Same;
    };
    if state.removal.as_ref().is_some_and(Started::is_running) || !model.local.is_done {
        return Flow::Same;
    }
    if state.armed_for().is_none() {
        state.armed_at = Some(Instant::now());
        state.removal = None;
        return Flow::Redraw;
    }
    state.armed_at = None;
    let log = started::log_of("remove", &model.local.dir);
    let mut command = Command::new(own_binary());
    // Without this tab's pane herdr closes the space with this tab in it: the click chose to delete it all.
    command
        .args(["wt:remove", &tree.name, "--close-space"])
        .current_dir(main)
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_TAB_ID");
    state.removal = Some(Started::start(command, log));
    Flow::Redraw
}

/// The stack of the worktree up: `stack:up` in it (the button, `u`), once at a time.
pub fn start_stack(state: &mut State, dir: &Path) -> Flow {
    if state.stack.as_ref().is_some_and(Started::is_running) {
        return Flow::Same;
    }
    let log = started::log_of("stack", dir);
    let mut command = Command::new(own_binary());
    command.arg("stack:up").current_dir(dir).env("INIT_CWD", dir);
    state.stack = Some(Started::start(command, log));
    Flow::Redraw
}

/// `pnpm pg:migrate` in the worktree's vita, once at a time.
pub fn migrate(state: &mut State, dir: &Path) -> Flow {
    if state.migration.as_ref().is_some_and(Started::is_running) {
        return Flow::Same;
    }
    let log = started::log_of("migrate", dir);
    let mut command = Command::new("pnpm");
    command.arg("pg:migrate").current_dir(dir.join(VITA));
    state.migration = Some(Started::start(command, log));
    Flow::Redraw
}

/// Storybook in the tab `storybook` of the worktree's space; the row says it is on its way.
pub fn start_storybook(state: &mut State, model: &WorktreeModel, main: &Path) -> Flow {
    let Some(checkout) = checkout_of(model, main) else {
        return Flow::Same;
    };
    if state.storybook == Some(Storybook::Starting) {
        return Flow::Same;
    }
    let offset = model.local.tree.as_ref().map_or(0, |tree| tree.offset);
    state.storybook = Some(if editor::start_storybook(&checkout, STORYBOOK_PORT + offset) {
        Storybook::Starting
    } else {
        Storybook::Failed("no space of the worktree in herdr to start it in".into())
    });
    Flow::Redraw
}

/// The stack `u` starts: the one whose `▶ start the stack` shows.
pub fn up_target(model: &WorktreeModel) -> Option<PathBuf> {
    let tree = model.local.tree.as_ref()?;
    let is_running = tree.stack.as_deref().is_some_and(|stack| stack.starts_with("running"));
    (tree.has_env && tree.has_own_port && model.local.has_docker && !is_running).then(|| model.local.dir.clone())
}

/// This program: `wt:remove` and `stack:up` are its own commands (rule 8, no Node).
fn own_binary() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("breadboard"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_shows_its_hunks_a_second_click_closes_them_more_shows_all() {
        let mut state = State::default();
        let shown = |state: &State| state.hunks.as_ref().map(|shown| (shown.file.clone(), shown.is_whole));
        toggle_hunks(&mut state, "a".into(), "file".into(), false);
        assert_eq!(shown(&state), Some(("a".into(), false)));
        toggle_hunks(&mut state, "a".into(), "file".into(), true);
        assert_eq!(shown(&state), Some(("a".into(), true)));
        toggle_hunks(&mut state, "b".into(), "file".into(), false);
        assert_eq!(shown(&state), Some(("b".into(), false)));
        toggle_hunks(&mut state, "b".into(), "file".into(), false);
        assert_eq!(shown(&state), None);
    }

    #[test]
    fn a_step_on_a_line_of_names_goes_name_by_name_then_closes() {
        let mut state = State::default();
        let shown = |state: &State| state.hunks.as_ref().map(|shown| (shown.file.clone(), shown.at.clone()));
        let step = |state: &mut State| toggle_hunks(state, "a".into(), format!("{}line", hunks::NEXT), false);
        step(&mut state);
        assert_eq!(shown(&state), Some(("a".into(), "line".into())));
        // The line was drawn with its names.
        let names = |state: &State| {
            state
                .hunks
                .as_ref()
                .unwrap()
                .on_line(vec!["a".into(), "b".into(), "c".into()])
        };
        names(&state);
        step(&mut state);
        assert_eq!(shown(&state), Some(("b".into(), "line".into())));
        names(&state);
        step(&mut state);
        names(&state);
        assert_eq!(shown(&state), Some(("c".into(), "line".into())));
        step(&mut state);
        assert_eq!(shown(&state), None);
        // A name's own click still toggles it.
        toggle_hunks(&mut state, "b".into(), "line".into(), false);
        names(&state);
        step(&mut state);
        assert_eq!(shown(&state), Some(("c".into(), "line".into())));
    }

    #[test]
    fn delete_stays_armed_for_five_seconds() {
        let mut state = State::default();
        assert_eq!(state.armed_for(), None);
        state.armed_at = Some(Instant::now());
        assert!(state.armed_for().is_some_and(|left| left > Duration::from_millis(4900)));
        state.armed_at = Instant::now().checked_sub(Duration::from_secs(6));
        assert_eq!(state.armed_for(), None);
    }
}
