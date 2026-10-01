//! The overview as actor: everything it does by itself or on a click, the
//! one place it writes (rewrite rule 6). What it types into agents, the
//! files it changes, its glab and herdr calls and what it starts.
//!
//! - Who acts: only a live overview (never `--once` or a print), and of the
//!   orchestration's live overviews the one holding its pid file
//!   (`$TMPDIR/platform-overview-<uid>-<name>.pid`, answers.mjs
//!   `deliversAnswers`): it delivers answers, tells agents, hands out
//!   issues, removes merged worktrees. Node's overview and this one exclude
//!   each other through that same file.
//! - Where: a click's state changes on the view's thread at once (an
//!   answer is stored before the next frame); what waits for herdr, git or
//!   GitLab runs on the actor's thread, one job after the other, and comes
//!   back as an `Effect` on the view; merges and background commands have
//!   threads of their own.
//! - What it says: every row keeps saying where its act stands (armed,
//!   running, failed, as Node's), and the log under the band says what was
//!   done, newest first, like lazygit's command log (`view.rs`).

pub mod auto;
pub mod browser;
pub mod chat;
pub mod glab;
pub mod herdr;
pub mod merge;
pub mod spawn;
pub mod sync;
pub mod tries;
pub mod view;

use super::{Msg, Overview};
use breadboard_core::lock::PidFile;
use breadboard_core::model::Paint;
use breadboard_core::reader::View;
use breadboard_core::time::now_ms;
use spawn::Run;
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// What comes back to the view from the actor's thread: a change of the overview's state.
pub type Effect = Box<dyn FnOnce(&mut Overview) + Send>;
/// Work for the actor's thread: herdr, git, GitLab; its effect, if any, goes to the view.
type Job = Box<dyn FnOnce() -> Option<Effect> + Send>;

/// How many things done the log keeps.
const DID_KEPT: usize = 20;

/// A thing done, for the log: when, how it went, what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Did {
    pub at: i64,
    /// Green done, red failed, yellow under way or waiting.
    pub paint: Paint,
    pub text: String,
}

/// The actor of one overview.
pub struct Actor {
    pub main: PathBuf,
    pub view: View,
    /// A live overview: a print never acts.
    pub is_live: bool,
    /// `--herdr` in herdr: the main space's overview keeps herdr in step.
    pub is_sync: bool,
    sender: Option<UnboundedSender<Msg>>,
    jobs: Option<std::sync::mpsc::Sender<Job>>,
    /// The pid file that makes this the one that acts for its orchestration.
    holder: Option<PidFile>,
    pub merges: merge::Merges,
    pub auto: auto::Auto,
    /// The end of the orchestration: armed, running, failed.
    pub end: Option<Run>,
    /// What the last click on the orchestrator's chat did, shown for a while.
    pub chat: Option<String>,
    /// Free agents whose dismiss is armed: conversation → name.
    pub dismissing: HashMap<String, String>,
    /// `stack:up` started for a try or the view of everything, by checkout
    /// (the worktree view keeps its own, worktree_view/acts.rs).
    pub stacks: HashMap<PathBuf, Run>,
    pub tries: tries::Tries,
    pub sync: sync::Sync,
    /// What was done, newest first.
    pub did: VecDeque<Did>,
}

impl Actor {
    pub fn new(view: &View, main: &Path, is_once: bool, is_herdr: bool) -> Actor {
        Actor {
            main: main.to_path_buf(),
            view: view.clone(),
            is_live: !is_once,
            is_sync: is_herdr && !is_once && *view == View::Everything && breadboard_core::herdr::is_in_herdr(),
            sender: None,
            jobs: None,
            holder: None,
            merges: merge::Merges::default(),
            auto: auto::Auto::default(),
            end: None,
            chat: None,
            dismissing: HashMap::new(),
            stacks: HashMap::new(),
            tries: tries::Tries::default(),
            sync: sync::Sync::default(),
            did: VecDeque::new(),
        }
    }

    /// The view's channel: effects come back on it; the actor's thread starts.
    pub fn start(&mut self, sender: UnboundedSender<Msg>) {
        let (jobs, queue) = std::sync::mpsc::channel::<Job>();
        let back = sender.clone();
        std::thread::spawn(move || {
            for job in queue {
                // A job that panics costs that job, not the actor.
                if let Ok(Some(effect)) = catch_unwind(AssertUnwindSafe(job))
                    && back.send(Msg::Acted(effect)).is_err()
                {
                    return;
                }
            }
        });
        self.sender = Some(sender);
        self.jobs = Some(jobs);
    }

    /// Whether this overview acts for the orchestration `name`: live, and
    /// holding its pid file (taken over once the one holding it is gone).
    pub fn delivers(&mut self, name: &str) -> bool {
        if !self.is_live || self.sender.is_none() {
            return false;
        }
        if self.holder.is_none() {
            self.holder = PidFile::hold(&pid_file(name));
        }
        self.holder.is_some()
    }

    /// Work for the actor's thread; dropped in a print.
    pub fn queue(&self, job: impl FnOnce() -> Option<Effect> + Send + 'static) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Box::new(job));
        }
    }

    /// An effect on the view from another thread (a merge, a background run's end).
    pub fn poster(&self) -> Poster {
        Poster(self.sender.clone())
    }

    /// `effect` on the view after `after`: an armed button disarms, a said text goes.
    pub fn later(&self, after: Duration, effect: impl FnOnce(&mut Overview) + Send + 'static) {
        let poster = self.poster();
        if poster.0.is_none() {
            return;
        }
        std::thread::spawn(move || {
            std::thread::sleep(after);
            poster.post(effect);
        });
    }

    /// A thing done into the log.
    pub fn log(&mut self, paint: Paint, text: impl Into<String>) {
        self.did.push_front(Did {
            at: now_ms(),
            paint,
            text: text.into(),
        });
        self.did.truncate(DID_KEPT);
    }
}

/// Sends effects to the view from any thread.
#[derive(Clone)]
pub struct Poster(Option<UnboundedSender<Msg>>);

impl Poster {
    pub fn post(&self, effect: impl FnOnce(&mut Overview) + Send + 'static) {
        if let Some(sender) = &self.0 {
            let _ = sender.send(Msg::Acted(Box::new(effect)));
        }
    }
}

/// An actor that never acts and has nothing to say: for a section drawn on its own (tests).
#[cfg(test)]
pub fn idle() -> &'static Actor {
    static IDLE: std::sync::LazyLock<Actor> =
        std::sync::LazyLock::new(|| Actor::new(&View::Everything, Path::new(""), true, false));
    &IDLE
}

/// The pid file of the one overview that acts for `name` (lock.mjs `holds`, answers.mjs).
pub fn pid_file(name: &str) -> PathBuf {
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    // JavaScript's `[^\w-]` per UTF-16 unit: one `_` for each.
    let name: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch.to_string()
            } else {
                "_".repeat(ch.len_utf16())
            }
        })
        .collect();
    std::env::temp_dir().join(format!("platform-overview-{uid}-{name}.pid"))
}

/// The first line of a text that says something: `#>*-` and spaces in front gone (auto-actions.mjs `firstLine`).
pub fn first_line(text: &str) -> String {
    text.lines()
        .map(|line| line.trim_start_matches(['#', '>', '*', ' ', '\t', '-']).trim())
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_pid_file_as_node_does() {
        let file = pid_file("one codebase/2");
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("platform-overview-"), "{name}");
        assert!(name.ends_with("-one_codebase_2.pid"), "{name}");
    }

    #[test]
    fn a_print_never_acts() {
        let mut actor = Actor::new(&View::Orchestration("b".into()), Path::new("/r"), true, false);
        assert!(!actor.delivers("b"));
        actor.log(Paint::Green, "x");
        assert_eq!(actor.did.len(), 1);
    }

    #[test]
    fn takes_the_first_line_that_says_something() {
        assert_eq!(first_line("\n## Which stack?\nmore"), "Which stack?");
        assert_eq!(first_line("- > *x*"), "x*");
        assert_eq!(first_line(""), "");
    }
}
