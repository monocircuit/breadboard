//! What a click does, the one place the overview acts (rewrite rule 6): a
//! fold or a page on the view's thread; the answers to actions (actions.rs)
//! and the worktree view's buttons (worktree_view/acts.rs) where they are
//! drawn; everything else — two-click merges, removals and the end, tries,
//! lines to agents, hand-overs, the plan, the chat, dismissing a free agent,
//! resolving conflicts — through the actor (`actor/`), which also does what
//! the overview does by itself after each read (`after_read`).

use super::actor::{self, auto, browser, chat, herdr, merge, spawn::Run, tries};
use super::{Msg, Overview};
use crate::tui::{Flow, KeyEvent};
use breadboard_core::gitlab::Mr;
use breadboard_core::model::orchestration::{OrchestrationLocal, OrchestrationModel};
use breadboard_core::model::order::Go;
use breadboard_core::model::resolve::resolve_line;
use breadboard_core::model::{Paint, Tree};
use breadboard_core::reader::{Local, Model, RemoteOf, View};
use breadboard_core::record::write_plan;
use breadboard_core::time::now_ms;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a click on a block or a button of the overview runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// What core's model says a click does (a node, NEXT's row, an UP NEXT row).
    Go(Go),
    /// An action's block (actions.rs): a click does `go`; `key` names the
    /// action, so a Tab pick and its digit keys stay on that one action even
    /// where two share a `go` (two of one agent open its tab).
    Action {
        key: String,
        go: Go,
    },
    /// A page in the browser.
    Open(String),
    Fold(Fold),
    /// An answer to an action: a button or an option.
    Answer {
        file: PathBuf,
        id: i64,
        answer: String,
    },
    /// The orchestrator's chat: focused, or opened again (#287).
    Chat,
    /// Two clicks dismiss a free agent (#289).
    Dismiss(String),
    /// A free agent's id: `claude --resume <id>` into the clipboard (#345).
    CopyResume(String),
    /// A conflicted MR's agent merges develop in (#246).
    Resolve(i64),
    /// An MR waiting in the CI queue goes to its front (#357).
    RunNow(i64),
    /// Two clicks delete the worktree of the view.
    Delete,
    /// `stack:up` of a worktree (`u` too).
    StartStack(PathBuf),
    /// `pnpm pg:migrate` of a worktree.
    Migrate(PathBuf),
    /// A WORKTREES node's app (#332, WHERE's `app` row): its page in the
    /// space's tab group when it answers, else how to start it in the band.
    OpenApp {
        dir: PathBuf,
        port: i64,
        is_shared: bool,
    },
    /// A tool of the stack (Storybook) in a tab of the worktree's space, by name.
    Tool(PathBuf, String),
    /// A file in the worktree's nvim, at a line.
    OpenFile {
        dir: PathBuf,
        file: String,
        line: Option<u64>,
    },
    /// A file's changes inline under the row `at` of DIFF or DEVELOP, or closed again;
    /// `is_whole`: all of its lines, past the first screenful.
    Hunks {
        file: String,
        at: String,
        is_whole: bool,
    },
}

/// What folds: the removed worktrees, the queue, the merged MRs; in the
/// view of everything the open MRs and issues the pane cut; DONE's commands;
/// the boxes of an MR and of a worktree (#346).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Removed,
    Queued,
    Merged,
    Mrs,
    Issues,
    Done,
    /// The boxes under an MR's row, by its iid (#346).
    MrBoxes(i64),
    /// The boxes under a worktree's node, by its issue (#346).
    TreeBoxes(i64),
}

/// Opens what was folded, folds what was open.
fn toggle(open: &mut std::collections::BTreeSet<i64>, key: i64) {
    if !open.remove(&key) {
        open.insert(key);
    }
}

/// How long a first click keeps the end armed.
const END_ARM: Duration = Duration::from_millis(4000);
/// How long a first click keeps a dismiss armed.
const DISMISS_ARM: Duration = Duration::from_millis(4000);

pub fn act(app: &mut Overview, act: Act) -> Flow {
    match act {
        Act::Fold(fold) => {
            let is_removed_folded = app.is_removed_folded();
            // Whether the MR shows its open boxes before any click (#354).
            let has_open_boxes = |iid: i64| {
                app.orchestration()
                    .and_then(|model| model.merge_requests.as_ref().ok())
                    .is_some_and(|mrs| {
                        mrs.open
                            .iter()
                            .filter_map(|row| row.mr.as_ref())
                            .any(|mr| mr.iid == iid && !mr.open_boxes.is_empty())
                    })
            };
            let has_open = match fold {
                Fold::MrBoxes(iid) => has_open_boxes(iid),
                _ => false,
            };
            let folds = &mut app.folds;
            match fold {
                Fold::Removed => folds.removed = Some(!folds.removed.unwrap_or(is_removed_folded)),
                Fold::Queued => folds.is_queued_folded = !folds.is_queued_folded,
                Fold::Merged => folds.is_merged_folded = !folds.is_merged_folded,
                Fold::Mrs => folds.is_every_mr_shown = !folds.is_every_mr_shown,
                Fold::Issues => folds.is_every_issue_shown = !folds.is_every_issue_shown,
                Fold::Done => folds.is_every_command_shown = !folds.is_every_command_shown,
                Fold::MrBoxes(iid) => folds.boxes.fold_mr(iid, has_open),
                Fold::TreeBoxes(number) => toggle(&mut folds.boxes.worktrees, number),
            }
            if matches!(fold, Fold::MrBoxes(_) | Fold::TreeBoxes(_))
                && let Some(file) = &app.folds_file
            {
                app.folds.boxes.write(file);
            }
            return Flow::Redraw;
        }
        Act::Open(url) => {
            let main = app.actor.main.clone();
            browser::show_or_open(&main, &url, &herdr::space_label(&main, &app.view_dir()));
            return Flow::Same;
        }
        // The stack of the main checkout from the view of everything: the worktree view has its own.
        Act::StartStack(dir) if !matches!(app.model, Some(Model::Worktree(_))) => tries::start_stack(app, &dir, None),
        // A WORKTREES node's migrations and app (#332): the worktree view has its own.
        Act::Migrate(dir) if !matches!(app.model, Some(Model::Worktree(_))) => tries::migrate(app, &dir),
        Act::OpenApp { dir, port, is_shared } => open_app(app, &dir, port, is_shared),
        // A colliding file of a WORKTREES node (#332): in that worktree's nvim.
        Act::OpenFile { dir, file, line } if !matches!(app.model, Some(Model::Worktree(_))) => {
            open_file(app, &dir, &file, line)
        }
        // The view of a worktree's own: its delete, stack, migrations, Storybook, nvim.
        Act::Delete
        | Act::StartStack(_)
        | Act::Migrate(_)
        | Act::Tool(..)
        | Act::OpenFile { .. }
        | Act::Hunks { .. } => {
            return super::worktree_view::act(app, act);
        }
        // Stored, typed into the agent's pane, shown at once (actions.rs, answers.mjs).
        Act::Answer { file, id, answer } => return super::actions::answer(app, &file, id, &answer),
        Act::Go(go) | Act::Action { go, .. } => go_to(app, go),
        Act::Chat => chat::click(app),
        Act::Dismiss(id) => dismiss(app, &id),
        Act::CopyResume(id) => copy_resume(app, &id),
        Act::Resolve(iid) => resolve(app, iid),
        Act::RunNow(iid) => run_now(app, iid),
    }
    app.redraw();
    Flow::Redraw
}

/// A worktree's app (WHERE's row, #332): its page in the tab group of the
/// worktree's space when it answers there, else in the band how to start it.
/// Asked at the click: the dot on screen may be a couple of seconds old.
fn open_app(app: &mut Overview, dir: &Path, port: i64, is_shared: bool) {
    let main = app.actor.main.clone();
    let is_up = if is_shared {
        breadboard_core::local::is_port_taken(port)
    } else {
        breadboard_core::local::is_app_of(port, dir)
    };
    if is_up {
        browser::show_or_open(
            &main,
            &format!("http://localhost:{port}"),
            &herdr::space_label(&main, dir),
        );
        return;
    }
    // What to do first: the band shows the words before the first `: `.
    let dev = match breadboard_core::stack::of(&main) {
        Some(stack) if stack.dir != "." => format!("{} in {}", stack.app.dev, stack.dir),
        Some(stack) => stack.app.dev,
        None => "the app".into(),
    };
    let hint = if is_shared {
        format!("{dev} serves the shared :{port}: nothing answers there")
    } else {
        let who = breadboard_core::git::feature_of(&main, dir);
        format!("{dev} starts the app of {who}: nothing answers on :{port}")
    };
    app.actor.log(Paint::Dim, hint);
}

/// A file of a worktree in its nvim, from a view of more than it; why not in the band.
fn open_file(app: &mut Overview, dir: &Path, file: &str, line: Option<u64>) {
    let main = app.actor.main.clone();
    let feature = breadboard_core::git::feature_of(&main, dir);
    let checkout = super::worktree_view::editor::Checkout {
        dir,
        main: &main,
        is_main: false,
        feature: &feature,
    };
    if let Err(why) = super::worktree_view::editor::open_file(&checkout, file, line) {
        app.actor.log(Paint::Red, format!("nvim: {why}"));
    }
}

/// What a `Go` of the model does.
pub fn go_to(app: &mut Overview, go: Go) {
    let main = app.actor.main.clone();
    match go {
        Go::Nothing => {}
        Go::Show { url, dir } => {
            let label = herdr::space_label(&main, dir.as_deref().unwrap_or(&app.view_dir()));
            browser::show_or_open(&main, &url, &label);
        }
        Go::Merge { .. } => merge::click(app, &go),
        Go::Try { ref dir, id } => {
            let state = app.try_state_of(&go);
            tries::click(app, dir.clone(), id, state);
        }
        Go::FocusSpace(dir) => app.actor.queue(move || {
            herdr::focus_space(&main, &dir);
            None
        }),
        // A headless agent has no tab: the click opens it in one (#361).
        Go::FocusAgent(dir) if breadboard_core::runs::is_headless(&main, &dir) => app.actor.queue(move || {
            let opened = super::actor::open_agent::open(&main, &dir);
            Some(Box::new(move |app: &mut Overview| match opened {
                Ok(did) => app.actor.log(Paint::Green, did),
                Err(why) => app.actor.log(Paint::Red, format!("the agent stays headless: {why}")),
            }))
        }),
        Go::FocusAgent(dir) => {
            let tab = app.agent_tab_of(&dir);
            app.actor.queue(move || {
                herdr::focus_tab(&main, &dir, tab.as_deref());
                None
            });
        }
        Go::Send {
            dir,
            key,
            line,
            fallback,
        } => auto::send_once(app, &dir, &key, &line, *fallback),
        Go::Remove(dir) => auto::click_remove(app, &dir),
        Go::HandOver { .. } | Go::StartFresh(_) => auto::click_start(app, &go),
        Go::WritePlan(plan) => {
            let Some(name) = app.orchestration_name() else { return };
            let done = Go::WritePlan(plan.clone());
            match write_plan(&main, &name, Some(plan.work), Some(plan.merge.clone()), plan.why) {
                Ok(_) => {
                    // Its row goes at once, not with the next read (Node drops it on the click).
                    super::actions::did(&done);
                    let order: Vec<String> = plan.merge.iter().map(|number| format!("#{number}")).collect();
                    app.actor.log(
                        Paint::Green,
                        format!("wrote the merge order into the plan: {}", order.join(" → ")),
                    );
                }
                Err(error) => app.actor.log(Paint::Red, format!("the plan stays: {error}")),
            }
        }
        Go::End => end(app),
    }
}

/// A click on ✕ end orchestration or its UP NEXT row (end.mjs `clickEnd`):
/// the first arms it, the second runs `wt:orchestrate <name> --end` in the
/// background; an exit this overview lives to see is a refusal.
fn end(app: &mut Overview) {
    let Some(name) = app.orchestration_name() else { return };
    let state = app.actor.end.clone().unwrap_or_default();
    if state.is_running {
        return;
    }
    if !state.is_armed {
        app.actor.end = Some(Run::armed());
        app.actor.later(END_ARM, |app: &mut Overview| {
            if app.actor.end.as_ref().is_some_and(|end| end.is_armed) {
                app.actor.end = None;
                app.redraw();
            }
        });
        return;
    }
    let log = actor::spawn::log_file(&format!("orchestration-end-{name}.log"));
    let program = actor::spawn::breadboard();
    let poster = app.actor.poster();
    let started = actor::spawn::start(
        actor::spawn::Background {
            program: &program,
            args: vec!["wt:orchestrate".into(), name.clone(), "--end".into()],
            cwd: &app.actor.main,
            log: &log,
            env: vec![],
            is_paneless: true,
        },
        move |code, last| {
            poster.post(move |app: &mut Overview| {
                let error = actor::spawn::error_of(code, &last, "");
                match &error {
                    None => app.actor.log_as(
                        "end",
                        Paint::Green,
                        format!("ended: {}", actor::spawn::said(&last)),
                        None,
                    ),
                    Some(why) => app.actor.log_as("end", Paint::Red, format!("not ended: {why}"), None),
                }
                app.actor.end = Some(Run {
                    error,
                    ..Run::default()
                });
                app.redraw();
            });
        },
    );
    match started {
        Ok(()) => {
            app.actor.end = Some(Run::running(&log));
            app.actor.log_as(
                "end",
                Paint::Yellow,
                format!("ending ⇶ {name} …"),
                Some(format!(
                    "{run} wt:orchestrate {name} --end",
                    run = breadboard_core::config::run_word()
                )),
            );
        }
        Err(why) => {
            app.actor.log_as("end", Paint::Red, format!("not ended: {why}"), None);
            app.actor.end = Some(Run {
                error: Some(why),
                ..Run::default()
            });
        }
    }
}

/// The programs a copy tries, in turn: Wayland's, then X's two.
const CLIPBOARDS: [(&str, &[&str]); 3] = [
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

/// A click on a free agent's id (#345): `claude --resume <id>` into the
/// clipboard, the whole command in the log either way.
fn copy_resume(app: &mut Overview, id: &str) {
    let command = breadboard_core::model::free_agents::resume_command(id);
    let key = format!("copy:{id}");
    app.actor.queue(move || {
        let copied = CLIPBOARDS
            .iter()
            .find(|(program, args)| copy_with(program, args, &command))
            .map(|(program, _)| *program);
        Some(Box::new(move |app: &mut Overview| {
            let line = match copied {
                Some(program) => format!("copied {command} ({program}): paste it into a shell to resume or look in"),
                None => format!(
                    "no clipboard ({}): resume it with {command}",
                    CLIPBOARDS.map(|(program, _)| program).join(", ")
                ),
            };
            app.actor.log_as(&key, Paint::Green, line, None);
        }) as actor::Effect)
    });
}

/// `text` into `program`'s stdin: whether it took it.
fn copy_with(program: &str, args: &[&str], text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let is_written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    child.wait().is_ok_and(|status| status.success()) && is_written
}

/// A click on a free agent's row (free-agents.mjs `clickDismiss`): the first
/// arms it, the second drops it from the record (its tab closed, when one was
/// resumed by hand); its conversation stays in `/resume`.
fn dismiss(app: &mut Overview, id: &str) {
    let Some(name) = app.orchestration_name() else { return };
    let row = app
        .orchestration()
        .and_then(|model| model.free_agents.as_ref().ok())
        .and_then(|rows| rows.iter().find(|row| row.id == id).cloned());
    let Some(row) = row else { return };
    if !app.actor.dismissing.contains_key(id) {
        app.actor.dismissing.insert(id.to_string(), row.display.clone());
        let id = id.to_string();
        app.actor.later(DISMISS_ARM, move |app: &mut Overview| {
            if app.actor.dismissing.remove(&id).is_some() {
                app.redraw();
            }
        });
        return;
    }
    app.actor.dismissing.remove(id);
    let main = app.actor.main.clone();
    let id = id.to_string();
    let who = if row.display.is_empty() {
        row.name.clone()
    } else {
        row.display.clone()
    };
    let key = format!("dismiss:{id}");
    app.actor
        .log_as(&key, Paint::Yellow, format!("dismissing {who} …"), None);
    app.actor.queue(move || {
        if let Some(tab) = &row.tab {
            herdr::did(&["tab", "close", tab]);
        }
        let fields = serde_json::json!({ "free": null, "dismissed": breadboard_core::time::to_iso(now_ms()) });
        let result =
            breadboard_core::record::write_worker(&main, &name, &id, fields.as_object().cloned().unwrap_or_default());
        Some(Box::new(move |app: &mut Overview| {
            match result {
                Ok(()) => app.actor.log_as(
                    &key,
                    Paint::Green,
                    format!("dismissed {who}: its conversation stays in /resume"),
                    None,
                ),
                Err(error) => app
                    .actor
                    .log_as(&key, Paint::Red, format!("{who} not dismissed: {error}"), None),
            }
            app.redraw();
        }) as actor::Effect)
    });
}

/// A click on an MR's resolve button (resolve.mjs `clickResolve`): the order
/// typed into its agent's pane; nothing while the button isn't there.
/// An MR waiting in the CI queue to its front (#357): the daemon's queue
/// starts its pipeline as the next one, once the runner has a free slot.
fn run_now(app: &mut Overview, iid: i64) {
    use breadboard_core::ci::{RunNow, read_or_default, run_now_file, write_whole};
    let file = run_now_file(&app.actor.main);
    let mut run_now: RunNow = read_or_default(&file);
    run_now.at.insert(iid, now_ms());
    match write_whole(&file, &run_now) {
        Ok(()) => app
            .actor
            .log(Paint::Green, format!("CI: !{iid} to the front of the queue")),
        Err(error) => app.actor.log(Paint::Red, format!("CI: !{iid} not moved up: {error}")),
    }
}

fn resolve(app: &mut Overview, iid: i64) {
    let Some(RemoteOf::Orchestration(remote)) = &app.remote else {
        return;
    };
    let Some((mr, branch)) = remote.of.iter().find_map(|(branch, part)| {
        part.mr
            .as_ref()
            .filter(|mr| mr.iid == iid)
            .map(|mr| (mr.clone(), branch.clone()))
    }) else {
        return;
    };
    let tree = app.tree_of_branch(&branch);
    let agents = app
        .orchestration_local()
        .map(|local| local.agents.clone())
        .unwrap_or_default();
    let behind = tree.as_ref().map_or(0, |tree| tree.behind);
    let can_click = app
        .state
        .resolves
        .clone()
        .state_of(&mr, tree.as_ref(), &agents, behind, now_ms())
        .can_click();
    let Some(tree) = tree.filter(|_| can_click) else { return };
    let main = app.actor.main.clone();
    let line = resolve_line(&mr);
    app.actor.queue(move || {
        let is_sent = herdr::deliver_to(&main, &tree.dir, &line);
        let who = breadboard_core::git::feature_of(&main, &tree.dir);
        Some(Box::new(move |app: &mut Overview| {
            app.state.resolves.clicked(&mr, Some(&tree), is_sent, now_ms());
            let about = format!("to merge {} in and resolve the conflicts of !{}", mr.target(), mr.iid);
            if is_sent {
                app.actor.told(&who, &about, &line, true);
            } else {
                app.actor.log_as(
                    &format!("told:{who}:{line}"),
                    Paint::Red,
                    format!("{} didn't take the line: {about}", app.actor.shown(&who)),
                    Some(actor::herdr::overview_line(&line)),
                );
                // Its note goes after a while.
                app.actor
                    .later(Duration::from_millis(10_000), |app: &mut Overview| app.redraw());
            }
            app.redraw();
        }) as actor::Effect)
    });
}

/// A key the frame has no use for: a digit answers the action picked with
/// Tab, nothing while none is (actions.rs); `u` starts the stack whose
/// button shows, the default one's in the view of everything.
pub fn key(app: &mut Overview, key: KeyEvent, picked: Option<&Act>) -> Flow {
    super::actions::key(app, key, picked)
        .or_else(|| super::everything::key(app, key))
        .unwrap_or_else(|| super::worktree_view::key(app, key))
}

/// What a read sets off, by the view: the orchestration's own acts, the main space's `--herdr` sync.
pub fn after_read(app: &mut Overview, is_local: bool, is_remote: bool) {
    match &app.actor.view {
        View::Orchestration(_) => auto::after_read(app, is_local, is_remote),
        View::Everything => {
            if let Some(RemoteOf::Everything(remote)) = &app.remote
                && is_remote
                && app.gitlab.error.is_none()
                && !app.is_once
            {
                super::overall::keep(&app.actor.main, remote.head.as_deref());
            }
            actor::sync::after_read(app)
        }
        // Its worktree is gone (the ✕, or wt:remove elsewhere): nothing left to show (overview.mjs).
        View::Worktree(_) => {
            if app.actor.is_live
                && is_local
                && matches!(&app.local, Some(Local::Worktree(local)) if local.tree.is_none())
            {
                // SAFETY: a signal to this process: the frame's loop ends and restores the terminal.
                unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
            }
        }
    }
}

/// Whether this overview acts for its orchestration: set before the model is built from a read.
pub fn gate(app: &mut Overview) {
    if let View::Orchestration(name) = app.actor.view.clone() {
        app.state.is_deliverer = app.actor.delivers(&name);
    }
}

/// The view's channel, for the actor's effects.
pub fn start(app: &mut Overview, sender: tokio::sync::mpsc::UnboundedSender<Msg>) {
    app.actor.start(sender);
}

impl Overview {
    /// The model built again, with the actor's words for its rows.
    pub fn redraw(&mut self) {
        self.rebuild();
    }

    /// GitLab read again now and in full: after a merge, NEXT moves on.
    pub fn refresh_gitlab(&self) {
        if let Some(refresh) = &self.refresh {
            let _ = refresh.send(());
        }
    }

    pub fn orchestration(&self) -> Option<&OrchestrationModel> {
        match &self.model {
            Some(Model::Orchestration(model)) => Some(model),
            _ => None,
        }
    }

    pub fn orchestration_local(&self) -> Option<&OrchestrationLocal> {
        match &self.local {
            Some(Local::Orchestration(local)) => Some(local),
            _ => None,
        }
    }

    pub fn orchestration_remote(&self) -> Option<&breadboard_core::gitlab::Remote> {
        match &self.remote {
            Some(RemoteOf::Orchestration(remote)) => Some(remote),
            _ => None,
        }
    }

    pub fn orchestration_name(&self) -> Option<String> {
        match &self.actor.view {
            View::Orchestration(name) => Some(name.clone()),
            _ => None,
        }
    }

    /// The record's file and the orchestration's name.
    pub fn record_and_name(&self) -> (Option<PathBuf>, Option<String>) {
        (
            self.orchestration_local().map(|local| local.record_file.clone()),
            self.orchestration_name(),
        )
    }

    /// The checkout the view is about: a worktree's, else the main checkout.
    pub fn view_dir(&self) -> PathBuf {
        match &self.actor.view {
            View::Worktree(dir) => dir.clone(),
            _ => self.actor.main.clone(),
        }
    }

    /// A worktree as the last read has it.
    pub fn tree_of(&self, dir: &Path) -> Option<Tree> {
        match &self.local {
            Some(Local::Orchestration(local)) => local.trees.iter().find(|tree| tree.dir == dir).cloned(),
            Some(Local::Worktree(local)) => local.tree.clone().filter(|tree| tree.dir == dir),
            Some(Local::Everything(local)) => local.trees.iter().find(|tree| tree.dir == dir).cloned(),
            None => None,
        }
    }

    pub fn tree_of_branch(&self, branch: &str) -> Option<Tree> {
        self.orchestration_local()?
            .trees
            .iter()
            .find(|tree| tree.branch == branch)
            .cloned()
    }

    pub fn tree_dir_of(&self, branch: &str) -> Option<PathBuf> {
        self.tree_of_branch(branch).map(|tree| tree.dir)
    }

    /// The worktree of an issue of the orchestration.
    pub fn tree_dir_of_number(&self, number: i64) -> Option<PathBuf> {
        let local = self.orchestration_local()?;
        let record = local.record.as_ref();
        let branch = record
            .and_then(|record| record.issues.iter().find(|issue| issue.number == Some(number)))
            .and_then(|issue| issue.branch.clone());
        local
            .trees
            .iter()
            .find(|tree| {
                Some(&tree.branch) == branch.as_ref()
                    || breadboard_core::gitlab::number_of(&tree.branch) == Some(number)
            })
            .map(|tree| tree.dir.clone())
    }

    /// The MR of a worktree's branch, as GitLab last said.
    pub fn mr_of_dir(&self, dir: &Path) -> Option<Mr> {
        let tree = self.tree_of(dir)?;
        self.orchestration_remote()?.of(&tree.branch).mr.clone()
    }

    /// The tab of a worktree's agent, as herdr last said.
    fn agent_tab_of(&self, dir: &Path) -> Option<String> {
        let tree = self.tree_of(dir)?;
        let claude = tree.claude?;
        Some(self.orchestration_local()?.agents.get(&claude.id)?.tab.clone())
    }

    /// The try state of the item whose click is `go`.
    fn try_state_of(&self, go: &Go) -> Option<breadboard_core::model::try_state::TryState> {
        let section = self.orchestration()?.actions.as_ref().ok()?;
        section
            .actions
            .iter()
            .chain(&section.up_next)
            .find(|item| item.go == *go)
            .and_then(|item| item.try_state.clone())
    }
}
