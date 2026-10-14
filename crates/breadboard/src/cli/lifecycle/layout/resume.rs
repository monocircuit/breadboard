//! `claude --resume <id>` typed into a pane after a restart (#368): only
//! into a shell that idles, never twice. A second layout (the startup's
//! restore, a wt:herdr) that came while the first claude still started saw
//! the shell idle and typed the line again, into that claude: the agent got
//! a shell command as its prompt, glued to the next line the overview typed.

use super::super::claude::is_conversation_running;
use super::super::herdr;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// What a machine behind herdr does for a resume; faked in the tests.
pub trait Host {
    /// Whether the pane's shell has the terminal to itself, asked afresh.
    fn is_idle(&self, pane: &str) -> bool;
    /// Whether a claude runs the conversation already, anywhere on the machine.
    fn is_running(&self, id: &str) -> bool;
    /// Whether the conversation was typed into a pane a moment ago, by any process.
    fn was_typed(&self, id: &str) -> bool;
    /// Notes that the conversation was typed just now.
    fn mark_typed(&self, id: &str);
    /// The line typed into the pane and submitted; whether herdr took it.
    fn run(&self, pane: &str, line: &str) -> bool;
    /// A moment to wait for the shell to start the command.
    fn pause(&self);
}

/// What [`resume_in`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resumed {
    /// Typed, and the shell gave its terminal to the command.
    Started,
    /// Typed, but the shell still had the terminal when the wait ended.
    Typed,
    /// Nothing typed: the pane's foreground is not its shell (a claude runs there).
    Busy,
    /// Nothing typed: a claude runs the conversation, or it was typed a moment ago.
    Running,
    /// Nothing typed: herdr refused the line.
    Refused,
}

/// How often the shell is asked whether the command took the terminal.
const TRIES: u32 = 30;

/// How long a typed conversation counts as starting: no second line meanwhile.
const STARTING: Duration = Duration::from_secs(60);

/// `line`, one command that resumes `id`, typed into `pane` as its own line,
/// if the pane's foreground is its shell and the conversation runs nowhere.
/// Then held until the shell gave the terminal away, so a layout after this
/// one (under the same lock) sees a busy pane instead of an idle shell.
pub fn resume_in(host: &impl Host, pane: &str, id: &str, line: &str) -> Resumed {
    debug_assert!(!line.contains(['\n', '\r']), "one line: {line:?}");
    if host.is_running(id) || host.was_typed(id) {
        return Resumed::Running;
    }
    if !host.is_idle(pane) {
        return Resumed::Busy;
    }
    if !host.run(pane, line) {
        return Resumed::Refused;
    }
    host.mark_typed(id);
    for _ in 0..TRIES {
        host.pause();
        if !host.is_idle(pane) {
            return Resumed::Started;
        }
    }
    Resumed::Typed
}

/// [`resume_in`] on the real herdr; whether the line was typed.
pub fn resume(pane: &str, id: &str, line: &str) -> bool {
    matches!(resume_in(&Herdr, pane, id, line), Resumed::Started | Resumed::Typed)
}

/// The machine: herdr, claude's sessions and a mark per typed conversation in `$TMPDIR`.
struct Herdr;

impl Host for Herdr {
    fn is_idle(&self, pane: &str) -> bool {
        // Under the layout lock herdr's reads are remembered: this one must not be.
        herdr::forget();
        herdr::is_idle(pane)
    }
    fn is_running(&self, id: &str) -> bool {
        is_conversation_running(id)
    }
    fn was_typed(&self, id: &str) -> bool {
        fs::metadata(mark_of(id))
            .and_then(|meta| meta.modified())
            .is_ok_and(|at| SystemTime::now().duration_since(at).unwrap_or_default() < STARTING)
    }
    fn mark_typed(&self, id: &str) {
        let mark = mark_of(id);
        if let Some(dir) = mark.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(mark, "");
    }
    fn run(&self, pane: &str, line: &str) -> bool {
        herdr::did(&["pane", "run", pane, line])
    }
    fn pause(&self) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `$TMPDIR/platform-resumed-<uid>/<id>`: when the conversation was last typed into a pane.
fn mark_of(id: &str) -> PathBuf {
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    std::env::temp_dir().join(format!("platform-resumed-{uid}")).join(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// A pane behind a fake herdr: a shell that starts what is typed after
    /// `delay` pauses, or a claude in it from the start.
    #[derive(Default)]
    struct Fake {
        has_claude: Cell<bool>,
        is_running: bool,
        is_refused: bool,
        /// Pauses until the typed command takes the terminal; None: never.
        delay: Option<u32>,
        pauses: Cell<u32>,
        typed: RefCell<Vec<String>>,
        marked: RefCell<Vec<String>>,
    }

    impl Host for Fake {
        fn is_idle(&self, _: &str) -> bool {
            let started = !self.typed.borrow().is_empty() && self.delay.is_some_and(|delay| self.pauses.get() >= delay);
            !self.has_claude.get() && !started
        }
        fn is_running(&self, _: &str) -> bool {
            self.is_running
        }
        fn was_typed(&self, id: &str) -> bool {
            self.marked.borrow().iter().any(|marked| marked == id)
        }
        fn mark_typed(&self, id: &str) {
            self.marked.borrow_mut().push(id.to_string());
        }
        fn run(&self, pane: &str, line: &str) -> bool {
            assert!(!self.has_claude.get(), "typed into a running claude");
            if self.is_refused {
                return false;
            }
            self.typed.borrow_mut().push(format!("{pane}: {line}"));
            true
        }
        fn pause(&self) {
            self.pauses.set(self.pauses.get() + 1);
        }
    }

    const LINE: &str = "claude --resume abc";

    #[test]
    fn nothing_is_typed_into_a_pane_where_claude_runs() {
        let fake = Fake {
            has_claude: Cell::new(true),
            ..Fake::default()
        };
        assert_eq!(resume_in(&fake, "p1", "abc", LINE), Resumed::Busy);
        assert!(fake.typed.borrow().is_empty());
    }

    #[test]
    fn an_idle_shell_gets_one_line_and_is_held_until_claude_has_the_terminal() {
        let fake = Fake {
            delay: Some(3),
            ..Fake::default()
        };
        assert_eq!(resume_in(&fake, "p1", "abc", LINE), Resumed::Started);
        assert_eq!(*fake.typed.borrow(), ["p1: claude --resume abc"]);
        assert_eq!(fake.pauses.get(), 3);
    }

    #[test]
    fn a_second_layout_while_claude_starts_types_nothing() {
        // The shell never shows the command taking over within the wait: the
        // mark still keeps the next layout from typing into the starting claude.
        let fake = Fake::default();
        assert_eq!(resume_in(&fake, "p1", "abc", LINE), Resumed::Typed);
        assert_eq!(resume_in(&fake, "p1", "abc", LINE), Resumed::Running);
        assert_eq!(fake.typed.borrow().len(), 1);
    }

    #[test]
    fn a_conversation_that_runs_elsewhere_is_not_resumed() {
        let fake = Fake {
            is_running: true,
            ..Fake::default()
        };
        assert_eq!(resume_in(&fake, "p1", "abc", LINE), Resumed::Running);
        assert!(fake.typed.borrow().is_empty());
    }

    #[test]
    fn a_refused_line_is_not_marked() {
        let fake = Fake {
            is_refused: true,
            ..Fake::default()
        };
        assert_eq!(resume_in(&fake, "p1", "abc", LINE), Resumed::Refused);
        assert!(fake.marked.borrow().is_empty());
    }
}
