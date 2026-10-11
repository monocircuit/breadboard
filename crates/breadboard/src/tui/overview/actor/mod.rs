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
pub mod layout_log;
pub mod merge;
pub mod spawn;
pub mod sync;
pub mod tries;
pub mod view;

use super::{Msg, Overview};
use breadboard_core::lock::PidFile;
use breadboard_core::model::Paint;
use breadboard_core::model::display_names::Names;
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

/// How many things done the log keeps: a `--herdr` layout alone may say a dozen.
const DID_KEPT: usize = 40;

/// A thing done, for the log: when, how it went, what — in words, and
/// what exactly was run or typed, the way lazygit's command log puts the
/// command under its action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Did {
    pub at: i64,
    /// Green done, red failed, yellow under way or waiting.
    pub paint: Paint,
    pub text: String,
    /// The command run or the line typed, word for word.
    pub detail: Option<String>,
    /// One act's entry: its end replaces its start, never a second entry.
    pub key: Option<String>,
    /// How often it was done: an act done again, at any later tick, counts here.
    pub times: u32,
    /// When it was done the last time (ms): `at` stays its first.
    pub last: i64,
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
    /// `pnpm pg:migrate` started from WORKTREES (#332), by worktree.
    pub migrations: HashMap<PathBuf, Run>,
    pub tries: tries::Tries,
    pub sync: sync::Sync,
    /// What was done, newest first.
    pub did: VecDeque<Did>,
    /// The names the orchestration shows its agents by (#292), as of its last
    /// read: what the log says an agent was told or given goes by them.
    pub names: Names,
    /// The prompt a headless agent's `broken` / `different` opened (#359).
    pub prompt: Option<super::actions::Prompt>,
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
            migrations: HashMap::new(),
            tries: tries::Tries::default(),
            sync: sync::Sync::default(),
            did: VecDeque::new(),
            names: Names::default(),
            prompt: None,
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

    /// A thing done into the log. One that was under way with the same
    /// words before its ` · ` (an answer waiting for its agent) is replaced.
    pub fn log(&mut self, paint: Paint, text: impl Into<String>) {
        self.log_with(paint, text, None);
    }

    /// `log` with what was run or typed word for word under it (`detail`).
    pub fn log_with(&mut self, paint: Paint, text: impl Into<String>, detail: Option<String>) {
        let text = text.into();
        let head = head_of(&text).to_string();
        self.did
            .retain(|did| !(did.key.is_none() && did.paint == Paint::Yellow && head_of(&did.text) == head));
        let at = now_ms();
        self.push(Did {
            at,
            paint,
            text,
            detail,
            key: None,
            times: 1,
            last: at,
        });
    }

    /// One act into the log under `key`: its start (yellow) and its end
    /// (green or red) are one entry, the newest on top; `detail` the
    /// command or line, word for word.
    pub fn log_as(&mut self, key: &str, paint: Paint, text: impl Into<String>, detail: Option<String>) {
        let detail = detail.or_else(|| {
            let known = self.did.iter().find(|did| did.key.as_deref() == Some(key))?;
            known.detail.clone()
        });
        self.did.retain(|did| did.key.as_deref() != Some(key));
        let at = now_ms();
        self.push(Did {
            at,
            paint,
            text: text.into(),
            detail,
            key: Some(key.to_string()),
            times: 1,
            last: at,
        });
    }

    /// The act under `key` done once more: counted on its entry, which stays
    /// where it is, with when it was done last. False: no entry has it (any
    /// more), log it anew.
    pub fn again(&mut self, key: &str) -> bool {
        match self.did.iter_mut().find(|did| did.key.as_deref() == Some(key)) {
            Some(did) => {
                did.times += 1;
                did.last = now_ms();
                true
            }
            None => false,
        }
    }

    /// A line typed into an agent, or held until it can take it: in words
    /// what it was told (`about`), the line word for word under it.
    /// `who`: the agent's worktree or record name; the log names it as the overview shows it.
    pub fn told(&mut self, who: &str, about: &str, line: &str, is_in: bool) {
        let key = format!("told:{who}:{line}");
        let line = herdr::overview_line(line);
        let shown = self.names.shown(who);
        if is_in {
            self.log_as(&key, Paint::Green, format!("told {shown} {about}"), Some(line));
        } else {
            // What it will be told in front: the band shows what stands before ` · `.
            let whom = if shown == who {
                format!("the agent of {who}")
            } else {
                shown
            };
            self.log_as(
                &key,
                Paint::Yellow,
                format!("will tell {whom} {about} · it takes the line once it is free"),
                Some(line),
            );
        }
    }

    /// `who` (a worktree's or a record's name) as the overview shows its agent.
    pub fn shown(&self, who: &str) -> String {
        self.names.shown(who)
    }

    fn push(&mut self, did: Did) {
        self.did.push_front(did);
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

/// What a logged text says before its ` · `.
fn head_of(text: &str) -> &str {
    text.split(" · ").next().unwrap_or(text)
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
    fn an_act_keeps_one_entry_from_its_start_to_its_end() {
        let mut actor = Actor::new(&View::Orchestration("b".into()), Path::new("/r"), false, false);
        actor.log_as("remove:x", Paint::Yellow, "removing x", Some("pnpm wt:remove x".into()));
        actor.log(Paint::Green, "something else");
        actor.log_as("remove:x", Paint::Green, "removed x", None);
        let texts: Vec<&str> = actor.did.iter().map(|did| did.text.as_str()).collect();
        assert_eq!(texts, ["removed x", "something else"]);
        assert_eq!(actor.did[0].detail.as_deref(), Some("pnpm wt:remove x"));
        // An answer waiting for its agent, then typed in: one entry.
        actor.log(Paint::Yellow, "decide #2 of 307-x → “a” · waits for its agent");
        actor.log(Paint::Green, "decide #2 of 307-x → “a” · typed into its agent's pane");
        assert_eq!(actor.did.len(), 3);
        assert_eq!(actor.did[0].paint, Paint::Green);
        // Done again at the next tick: counted, in place.
        assert!(actor.again("remove:x"));
        assert_eq!(actor.did.len(), 3);
        assert_eq!(actor.did[1].times, 2);
        assert!(actor.did[1].last >= actor.did[1].at);
        assert!(!actor.again("gone"), "nothing to count it on");
    }

    #[test]
    fn a_held_line_says_what_it_will_tell_before_why() {
        let mut actor = Actor::new(&View::Orchestration("b".into()), Path::new("/r"), false, false);
        actor.told("307-x-y", "that !316 is merged", "!316 merged", false);
        assert_eq!(
            view::short_of(&actor.did[0].text).replace('\u{a0}', " "),
            "will tell the agent of #307 that !316 is merged"
        );
        actor.told("307-x-y", "that !316 is merged", "!316 merged", true);
        assert_eq!(actor.did.len(), 1);
        assert_eq!(actor.did[0].text, "told 307-x-y that !316 is merged");
    }

    #[test]
    fn takes_the_first_line_that_says_something() {
        assert_eq!(first_line("\n## Which stack?\nmore"), "Which stack?");
        assert_eq!(first_line("- > *x*"), "x*");
        assert_eq!(first_line(""), "");
    }
}
