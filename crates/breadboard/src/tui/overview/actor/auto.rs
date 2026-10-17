//! What the overview of an orchestration does by itself, from what it read
//! (auto-actions.mjs, workers.mjs, free.mjs, waits.mjs): the lines core's
//! model tells (conflicts with develop, failed pipelines), the hand-outs
//! and fresh starts it orders, the removal of merged worktrees, the waits
//! whose command is gone, the notifications of new actions, the plan kept in
//! step with the merge order (#376). Each guarded
//! against repeats: a key in flight or sent is never sent again.

use super::spawn::{self, Background, error_of};
use super::{Effect, herdr};
use crate::tui::overview::Overview;
use breadboard_core::epic::auto_merge_of;
use breadboard_core::git::{base_of, behind_base, feature_of, work_files};
use breadboard_core::gitlab::Remote;
use breadboard_core::herdr::{Agent, is_in_herdr};
use breadboard_core::json;
use breadboard_core::lock::with_dir_lock;
use breadboard_core::model::auto_actions::{Removal, Tell, sent_file};
use breadboard_core::model::clean_up::{CleanUp, clean_up_of};
use breadboard_core::model::display_names::names_of;
use breadboard_core::model::handovers::agent_name_of;
use breadboard_core::model::next::stamped;
use breadboard_core::model::orchestration::{OrchestrationLocal, OrchestrationModel};
use breadboard_core::model::order::Target;
use breadboard_core::model::order::{Go, Item};
use breadboard_core::model::workers::StartRun;
use breadboard_core::model::{Entry, Paint, Tree, issues_of};
use breadboard_core::record::{Worker, record_file, write_plan, write_worker};
use breadboard_core::runs::{self, Usage};
use breadboard_core::time::{now_ms, to_iso};
use breadboard_core::wait::{Awaited, Verdict, clear_wait, is_moot, is_waiting, line_of, verdict_of, wait_of};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the last line of a handover says when its agent got the issue but wt:new failed after the setup.
const HALF_SET_UP: &str = "wt:new failed after the setup";
/// How many sent keys a worktree keeps, the newest.
const SENT_KEPT: usize = 50;
/// How long a first click keeps a removal armed.
const REMOVE_ARM: Duration = Duration::from_millis(5000);
/// How long after writing the plan the overview writes it again at the
/// earliest (#376): its record read back first, so one change is one write.
const PLAN_EVERY: i64 = 5_000;
/// After a write that failed: the record's lock held, the file unwritable.
const PLAN_RETRY: i64 = 60_000;
/// How long the record's lock holds before it is taken over (as wt:plan's).
const PLAN_LOCK_STALE: Duration = Duration::from_millis(5000);

/// What the automatic side remembers between reads.
#[derive(Debug, Default)]
pub struct Auto {
    /// Lines on their way, by key: not queued twice.
    in_flight: HashSet<String>,
    /// Lines that got in, by key, until their cause goes: a read that lags never sends one again.
    sent: HashSet<String>,
    /// A failed pipeline's line: herdr's state of its agent at the last try.
    pipeline_status: HashMap<String, Option<String>>,
    /// The keys of the actions at the last read; None before GitLab first answered.
    seen: Option<HashSet<String>>,
    /// Worktrees whose agent is being freed before their removal.
    freeing: HashSet<PathBuf>,
    /// Worktrees whose ended wait is being delivered.
    waits_in_flight: HashSet<PathBuf>,
    /// Per worktree, since when each awaited MR has had conflicts.
    wait_conflicts: Arc<Mutex<HashMap<PathBuf, HashMap<i64, i64>>>>,
    /// The panes whose herdr record of a headless session was released, with it (#386).
    released: HashSet<(String, String)>,
    /// Which arming of a removal this is, by worktree.
    arms: HashMap<PathBuf, u64>,
    seq: u64,
    /// What the CI queue was last told of this orchestration (#357).
    published: Option<breadboard_core::ci::Published>,
    /// MRs into an epic merged by themselves, by iid: the head each was
    /// tried at. One that failed waits for a new push (#350).
    epic_tried: HashMap<i64, String>,
    /// When the plan may be written again at the earliest (#376).
    plan_next: i64,
}

/// `key` remembered as sent in the worktree's `.claude/overview-sent.json`, newest 50.
pub fn mark_sent(dir: &Path, key: &str) {
    let mut sent: Vec<String> = json::read(&sent_file(dir))
        .map(|value| json::strs_of(&value, "sent"))
        .unwrap_or_default();
    sent.push(key.to_string());
    let skip = sent.len().saturating_sub(SENT_KEPT);
    let sent: Vec<String> = sent.into_iter().skip(skip).collect();
    let _ = json::write_pretty(&sent_file(dir), &json!({ "sent": sent }));
}

/// `key` taken out of what `dir` remembers as sent: its cause went, a new one sends again.
pub fn forget_sent(dir: &Path, key: &str) {
    // In the order they were sent, as JavaScript's Set keeps them.
    let sent: Vec<String> = json::read(&sent_file(dir))
        .map(|value| json::strs_of(&value, "sent"))
        .unwrap_or_default()
        .into_iter()
        .filter(|known| known != key)
        .collect();
    let _ = json::write_pretty(&sent_file(dir), &json!({ "sent": sent }));
}

/// Why a failed pipeline's line didn't get in, by herdr's state of its agent.
fn pipeline_why(status: Option<&str>) -> &'static str {
    match status {
        Some("blocked") => "its agent is at a question, the line follows once it answers",
        Some(_) => "herdr didn't take the line: click to try again",
        None => "its agent isn't running: the line follows once it is",
    }
}

/// The issues of the orchestration, each with its worktree.
fn issues(local: &OrchestrationLocal) -> Vec<Entry> {
    issues_of(
        local.record.as_ref().map(|record| &record.issues[..]).unwrap_or(&[]),
        &local.trees,
    )
}

/// What a read of the orchestration sets off (render-orchestration.mjs,
/// view.mjs `retryDeliveries`): only by the live overview that acts for it.
pub fn after_read(app: &mut Overview, is_local: bool, is_remote: bool) {
    let Some(name) = app.orchestration_name() else { return };
    let Some(local) = app.orchestration_local().cloned() else {
        return;
    };
    let known = local.record.as_ref().map(|record| &record.workers[..]).unwrap_or(&[]);
    app.actor.names = names_of(&issues(&local), known);
    if !app.actor.is_live {
        return;
    }
    let remote = app.orchestration_remote().cloned();
    let Some(model) = app.orchestration().cloned() else {
        return;
    };
    let is_deliverer = app.state.is_deliverer;
    let mut is_changed = false;
    // Answers that didn't get in are actions.rs's (`retry_answers`); the lines after a merge are ours.
    if is_deliverer && is_local {
        let dirs: Vec<PathBuf> = local.trees.iter().map(|tree| tree.dir.clone()).collect();
        super::merge::retry_lines(&app.actor, dirs);
        // In auto mode a checker answers the open tries (#384).
        super::checker::after_read(app, &local);
    }
    // Every live overview computes the same order: whichever reads first writes it.
    if remote.is_some() {
        publish_ci(app, &name, &model);
    }
    if let Some(remote) = remote.as_ref().filter(|_| is_deliverer) {
        tell(app, &model.tells, &local.agents);
        // herdr's sidebar reads them: it knows neither the merge order nor the boxes (#344).
        if is_remote {
            for (dir, record) in &model.turn_records {
                let _ = breadboard_core::model::merge_turn::write_turn_record(dir, record);
            }
        }
        if is_remote {
            deliver_waits(app, &local, remote);
            is_changed |= merge_into_epic(app, &model, remote);
        }
        for order in &model.orders {
            is_changed |= run_order(app, &name, order);
        }
        is_changed |= follow_merge_order(app, &name, &model);
        if is_in_herdr() {
            is_changed |= remove_merged(app, &name, &local, remote, &model);
            tell_usage(app, &name, &model);
        }
    }
    if is_local && is_deliverer && is_in_herdr() {
        end_herdr_resumes(app, &local);
    }
    if is_deliverer {
        super::fresh::after_read(app, &name, &model);
    }
    notify_new(app, &model);
    if is_changed {
        app.redraw();
    }
}

/// A headless agent's claude herdr's restore resumed in a pane (#386, a bare
/// `claude --resume`, no click's) holds its lines: ended once herdr says it
/// idles, said in DONE. One at a time per claude, on the actor's thread.
fn end_herdr_resumes(app: &mut Overview, local: &OrchestrationLocal) {
    let main = app.actor.main.clone();
    let headless: HashMap<String, PathBuf> = local
        .trees
        .iter()
        .filter(|tree| breadboard_core::runs::is_headless(&main, &tree.dir))
        // Its binding, not its transcript: one herdr resumed may not have written yet.
        .filter_map(|tree| {
            Some((
                breadboard_core::conversations::bound_session(&tree.dir)?,
                tree.dir.clone(),
            ))
        })
        .collect();
    // herdr's record of a headless session on a pane that doesn't run it (one
    // the daemon's pane id put there before #386, the overview's tab too): a
    // restart would resume it there. Released once per pane and session.
    for pane in breadboard_core::herdr::all_panes() {
        let Some(session) = pane.session.clone().filter(|session| headless.contains_key(session)) else {
            continue;
        };
        if pane.agent.is_some() || !app.actor.auto.released.insert((pane.id.clone(), session.clone())) {
            continue;
        }
        let dir = headless[&session].clone();
        let main = main.clone();
        app.actor.queue(move || {
            let did = super::end_held::release_record(&main, &dir, &pane.id);
            Some(Box::new(move |app: &mut Overview| {
                if let Some(did) = did {
                    app.actor.log(Paint::Dim, did);
                }
            }))
        });
    }
    for (pid, id) in breadboard_core::conversations::unnamed_resumes() {
        let Some(dir) = headless.get(&id).cloned() else {
            continue;
        };
        let key = format!("herdr-resume:{pid}");
        if !app.actor.auto.in_flight.insert(key.clone()) {
            continue;
        }
        let main = main.clone();
        app.actor.queue(move || {
            let ended = super::end_held::end_herdr_resume(&main, &dir, &id, pid);
            Some(Box::new(move |app: &mut Overview| {
                app.actor.auto.in_flight.remove(&key);
                if let Some(ended) = ended {
                    app.actor.log(Paint::Yellow, ended);
                }
            }))
        });
    }
}

/// The orchestration's merge order and bars for the CI queue (#357), written when they changed.
fn publish_ci(app: &mut Overview, name: &str, model: &OrchestrationModel) {
    let Some(published) = model.ci_published.as_ref() else {
        return;
    };
    if app.actor.auto.published.as_ref() == Some(published) {
        return;
    }
    let file = breadboard_core::ci::order_file(&app.actor.main, name);
    if breadboard_core::ci::write_whole(&file, published).is_ok() {
        app.actor.auto.published = Some(published.clone());
    }
}

/// The computed merge order written into the stale plan (#376), by the
/// overview that delivers alone: no click, no orchestrator. The plan's why
/// kept, its line `updated from the merge order <time>` new.
fn follow_merge_order(app: &mut Overview, name: &str, model: &OrchestrationModel) -> bool {
    let Some(plan) = model.plan_update.as_ref() else {
        return false;
    };
    let now = now_ms();
    if now < app.actor.auto.plan_next {
        return false;
    }
    let iso = to_iso(now);
    let why = stamped(plan.why.as_deref(), &format!("{} {} UTC", &iso[..10], &iso[11..16]));
    let main = app.actor.main.clone();
    let written = with_dir_lock(&record_file(&main, name), PLAN_LOCK_STALE, || {
        write_plan(
            &main,
            name,
            Some(plan.work.clone()),
            Some(plan.merge.clone()),
            Some(why),
            None,
        )
    });
    match written {
        Ok(Ok(_)) => {
            app.actor.auto.plan_next = now + PLAN_EVERY;
            let order: Vec<String> = plan.merge.iter().map(|number| format!("#{number}")).collect();
            app.actor.log(
                Paint::Green,
                format!("the plan follows the merge order: {}", order.join(" → ")),
            );
            true
        }
        Ok(Err(error)) | Err(error) => {
            app.actor.auto.plan_next = now + PLAN_RETRY;
            app.actor
                .log(Paint::Red, format!("the plan stays behind the merge order: {error}"));
            true
        }
    }
}

/// A task's MR into its epic merges by itself once its turn offers it
/// (#350): the second click's merge, one at a time, never twice at one head;
/// into develop or main it stays the user's two clicks.
fn merge_into_epic(app: &mut Overview, model: &OrchestrationModel, remote: &Remote) -> bool {
    if app
        .actor
        .merges
        .of
        .values()
        .any(|merge| merge.phase == super::merge::Phase::Running)
    {
        return false;
    }
    let Ok(section) = model.actions.as_ref() else {
        return false;
    };
    let items: Vec<Item> = section.actions.iter().chain(&section.up_next).cloned().collect();
    let head_of = |iid: i64| {
        remote
            .of
            .values()
            .filter_map(|part| part.mr.as_ref())
            .find(|mr| mr.iid == iid)
            .and_then(|mr| mr.sha.clone())
    };
    let tried = &app.actor.auto.epic_tried;
    let found = auto_merge_of(
        &items,
        |branch| remote.of(branch).mr.as_ref().map(|mr| mr.target().to_string()),
        |iid| tried.get(&iid).is_some_and(|sha| Some(sha) == head_of(iid).as_ref()),
    );
    let Some((Go::Merge { branch, action }, Some(offer))) = found.map(|item| (item.go.clone(), item.merge.clone()))
    else {
        return false;
    };
    app.actor
        .auto
        .epic_tried
        .insert(offer.iid, head_of(offer.iid).unwrap_or_default());
    let target = remote
        .of(&branch)
        .mr
        .as_ref()
        .map(|mr| mr.target().to_string())
        .unwrap_or_default();
    app.actor.log(
        Paint::Yellow,
        format!("!{} is done: it merges into {target} by itself", offer.iid),
    );
    super::merge::start(app, &branch, &action, &offer);
    true
}

/// The lines core's model tells (`tellConflicts`, `tellFailedPipelines`): each
/// sent once; a failed pipeline's line that didn't get in only again once
/// herdr's state of its agent changed.
fn tell(app: &mut Overview, tells: &[Tell], agents: &HashMap<String, Agent>) {
    let due: HashSet<&str> = tells
        .iter()
        .filter_map(|tell| match tell {
            Tell::Send { key, .. } if key.starts_with("pipeline:") => Some(key.as_str()),
            _ => None,
        })
        .collect();
    app.actor
        .auto
        .pipeline_status
        .retain(|key, _| due.contains(key.as_str()));
    app.state.pipeline_tries.retain(|key, _| due.contains(key.as_str()));
    for tell in tells {
        match tell {
            Tell::Forget { dir, key } => {
                forget_sent(dir, key);
                app.actor.auto.sent.remove(key);
            }
            Tell::Send {
                dir,
                key,
                line,
                iid,
                id,
            } => {
                if app.actor.auto.in_flight.contains(key) || app.actor.auto.sent.contains(key) {
                    continue;
                }
                let is_pipeline = key.starts_with("pipeline:");
                let status = id
                    .as_ref()
                    .and_then(|id| agents.get(id))
                    .and_then(|agent| agent.status.clone());
                if is_pipeline && app.actor.auto.pipeline_status.get(key) == Some(&status) {
                    continue;
                }
                app.actor.auto.in_flight.insert(key.clone());
                let (main, dir, key, line, iid) =
                    (app.actor.main.clone(), dir.clone(), key.clone(), line.clone(), *iid);
                app.actor.queue(move || {
                    let is_in = herdr::deliver_to(&main, &dir, &line);
                    if is_in {
                        mark_sent(&dir, &key);
                    }
                    let who = feature_of(&main, &dir);
                    Some(Box::new(move |app: &mut Overview| {
                        app.actor.auto.in_flight.remove(&key);
                        if is_in {
                            app.actor.auto.sent.insert(key.clone());
                            app.actor.auto.pipeline_status.remove(&key);
                            app.state.pipeline_tries.remove(&key);
                            app.actor.told(&who, &about_of(&key), &line, true);
                            if key.starts_with("conflicts:") {
                                // Its MR row shows where the order stands, as after a click.
                                let mr = app.mr_of_dir(&dir).filter(|mr| mr.iid == iid);
                                let tree = app.tree_of(&dir);
                                if let Some(mr) = mr {
                                    app.state.resolves.note_told(&mr, tree.as_ref(), now_ms());
                                }
                            }
                        } else if is_pipeline {
                            app.actor.auto.pipeline_status.insert(key.clone(), status.clone());
                            app.state
                                .pipeline_tries
                                .insert(key.clone(), pipeline_why(status.as_deref()).into());
                        }
                        app.redraw();
                    }) as Effect)
                });
            }
        }
    }
}

/// A click that sends one line once (`sendOnce`), else its fallback. It goes
/// to herdr's agent of the worktree's tab whether or not its conversation is
/// written down (`.claude/herdr-session`): WORKTREES names that agent too, and
/// the row says a click tells it.
pub fn send_once(app: &mut Overview, dir: &Path, key: &str, line: &str, fallback: Go) {
    if app.actor.auto.in_flight.contains(key) {
        return;
    }
    app.actor.auto.in_flight.insert(key.to_string());
    let (main, dir, key, line) = (
        app.actor.main.clone(),
        dir.to_path_buf(),
        key.to_string(),
        line.to_string(),
    );
    app.actor.queue(move || {
        let is_in = herdr::deliver_to(&main, &dir, &line);
        if is_in {
            mark_sent(&dir, &key);
        }
        let who = feature_of(&main, &dir);
        Some(Box::new(move |app: &mut Overview| {
            app.actor.auto.in_flight.remove(&key);
            if is_in {
                app.actor.auto.sent.insert(key.clone());
                app.state.pipeline_tries.remove(&key);
                app.actor.told(&who, &about_of(&key), &line, true);
                app.redraw();
            } else {
                app.actor.log_as(
                    &format!("told:{who}:{line}"),
                    Paint::Yellow,
                    format!(
                        "{} didn't take the line: {}",
                        app.actor.shown(&who),
                        fallback_text(&fallback)
                    ),
                    Some(herdr::overview_line(&line)),
                );
                super::super::act::go_to(app, fallback);
            }
        }) as Effect)
    });
}

/// What a sent line told its agent, in a few words, by its key (auto-actions.mjs's keys).
fn about_of(key: &str) -> String {
    let iid = |rest: &str| {
        let digits: String = rest
            .trim_start_matches('!')
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        format!("!{digits}")
    };
    if let Some(rest) = key.strip_prefix("conflicts:") {
        breadboard_core::config::in_repo(&format!("to merge develop in ({} conflicts with it)", iid(rest)))
    } else if let Some(rest) = key.strip_prefix("pipeline:") {
        format!("that the pipeline of {} failed", iid(rest))
    } else if key.starts_with("unpushed:") {
        "to push its commits".into()
    } else if let Some(rest) = key.strip_prefix("boxes:") {
        format!("to tick the done boxes of {}'s issue", iid(rest))
    } else {
        String::new()
    }
}

fn fallback_text(go: &Go) -> &'static str {
    match go {
        Go::Show { .. } => "opened the page instead",
        Go::FocusAgent(_) => "its tab in front instead",
        Go::Detail(_) => "shown beside instead",
        _ => "",
    }
}

/// A hand-out or fresh start core ordered (workers.mjs): `wt:orchestrate` in
/// the background, once per issue while it runs. True when one started.
fn run_order(app: &mut Overview, name: &str, order: &Go) -> bool {
    match order {
        Go::HandOver { number, to, dir } => start_run(app, name, *number, &["--to", to], dir.clone(), false, Some(to)),
        Go::StartFresh(number) => start_run(app, name, *number, &["--now"], None, true, None),
        _ => false,
    }
}

/// A click on a hand-over or a start (`handOver`, `startFresh`).
pub fn click_start(app: &mut Overview, go: &Go) {
    let Some(name) = app.orchestration_name() else { return };
    if run_order(app, &name, go) {
        app.redraw();
    }
}

/// `wt:orchestrate <name> <number> <args…>` detached (`runInBackground`):
/// wt:new takes minutes, the overview goes on; without the overview's pane.
fn start_run(
    app: &mut Overview,
    name: &str,
    number: i64,
    args: &[&str],
    dir: Option<PathBuf>,
    is_fresh: bool,
    to: Option<&str>,
) -> bool {
    if app.state.runs.get(&number).is_some_and(|run| run.is_running) {
        return false;
    }
    let log = spawn::log_file(&format!("overview-start-{name}-{number}.log"));
    let program = spawn::breadboard();
    let mut argv = vec!["wt:orchestrate".to_string(), name.to_string(), number.to_string()];
    argv.extend(args.iter().map(|arg| arg.to_string()));
    let shown = format!("pnpm {}", argv.join(" "));
    let (what, done) = match to.map(|to| app.actor.shown(to)) {
        Some(to) => (
            format!("handing #{number} to {to}"),
            format!("handed #{number} to {to}"),
        ),
        None => (
            format!("starting a fresh agent for #{number}"),
            format!("a fresh agent took #{number}"),
        ),
    };
    let key = format!("start:{number}");
    let done_key = key.clone();
    // A half set up one says where to read why: its log, under the command.
    let half_detail = format!("{shown}  ·  its log: {}", log.display());
    let poster = app.actor.poster();
    let started = spawn::start(
        Background {
            program: &program,
            args: argv,
            cwd: &app.actor.main,
            log: &log,
            env: vec![],
            is_paneless: true,
        },
        move |code, last| {
            poster.post(move |app: &mut Overview| {
                let error = error_of(code, &last, "");
                if let Some(run) = app.state.runs.get_mut(&number) {
                    run.is_running = false;
                    run.ended = now_ms();
                    run.error = error.clone();
                }
                match error {
                    // The CLI goes on when wt:new failed after the setup (orchestrate.mjs): half set up, not done.
                    None if last.contains(HALF_SET_UP) => app.actor.log_as(
                        &done_key,
                        Paint::Yellow,
                        format!("{done}, half set up: {HALF_SET_UP}"),
                        Some(half_detail),
                    ),
                    None => app
                        .actor
                        .log_as(&done_key, Paint::Green, format!("{done}: {}", spawn::said(&last)), None),
                    Some(why) => {
                        app.actor
                            .log_as(&done_key, Paint::Red, format!("#{number} didn't start: {why}"), None)
                    }
                }
                app.redraw();
            });
        },
    );
    let run = StartRun {
        is_running: started.is_ok(),
        error: started.as_ref().err().cloned(),
        ended: if started.is_ok() { 0 } else { now_ms() },
        dir,
        is_fresh,
        to: to.map(String::from),
    };
    app.state.runs.insert(number, run);
    match started {
        Ok(()) => app.actor.log_as(&key, Paint::Yellow, format!("{what} …"), Some(shown)),
        Err(why) => app
            .actor
            .log_as(&key, Paint::Red, format!("#{number} didn't start: {why}"), Some(shown)),
    }
    true
}

/// The automatic clean-up (#227, `removeMerged`): wt:remove of each merged
/// worktree nothing stops, once per worktree; its agent freed first (#289),
/// then the removal waits until it left. Nothing while herdr told nothing:
/// a working agent would look idle.
fn remove_merged(
    app: &mut Overview,
    name: &str,
    local: &OrchestrationLocal,
    remote: &Remote,
    model: &OrchestrationModel,
) -> bool {
    if local.agents.is_empty() {
        return false;
    }
    let mut is_changed = false;
    let limit = breadboard_core::config::of_main(&app.actor.main).config.agents.limit();
    for issue in issues(local) {
        let Some(tree) = &issue.tree else { continue };
        if app.state.removals.contains_key(&tree.dir) || app.actor.auto.freeing.contains(&tree.dir) {
            continue;
        }
        let mr = remote.of(&issue.branch).mr.as_ref();
        let agent = tree.claude.as_ref().and_then(|claude| local.agents.get(&claude.id));
        let status = agent.and_then(|agent| agent.status.as_deref());
        let hold = model.holds.get(&tree.dir).map(String::as_str);
        if clean_up_of(Some(tree), mr, status, hold) != Some(CleanUp::Due) {
            continue;
        }
        // Its conversation is its agent (#361): a headless one that never ran has no row yet, and is freed too.
        if let (Some(claude), Some(mr)) = (&tree.claude, mr) {
            if claude.share() <= limit {
                free_then_remove(app, name, local, tree, claude.id.clone(), mr.iid);
            } else {
                // Full (#381): it retires with a handoff, its next issue goes to a fresh agent.
                retire_then_remove(app, name, tree, mr.iid);
            }
            continue;
        }
        remove_worktree(app, &tree.dir);
        is_changed = true;
    }
    is_changed
}

/// Its agent freed (`freeOnCleanUp`): the record keeps it (#345, #361);
/// then its worktree goes. Not freed, it goes with the space.
fn free_then_remove(app: &mut Overview, name: &str, local: &OrchestrationLocal, tree: &Tree, id: String, iid: i64) {
    let known: Vec<Worker> = local
        .record
        .as_ref()
        .map(|record| record.workers.clone())
        .unwrap_or_default();
    let worker = known.iter().find(|worker| worker.id == id);
    let free = Free {
        main: app.actor.main.clone(),
        name: name.to_string(),
        agent: agent_name_of(Some(&id), &tree.name, &known),
        known: worker.map(|worker| worker.files.clone()),
        mrs: worker.map(|worker| worker.mrs.clone()).unwrap_or_default(),
        id,
        dir: tree.dir.clone(),
        iid,
    };
    let dir = tree.dir.clone();
    app.actor.auto.freeing.insert(dir.clone());
    app.actor.queue(move || {
        let freed = free.run();
        Some(Box::new(move |app: &mut Overview| {
            app.actor.auto.freeing.remove(&dir);
            match freed {
                Ok(agent) => app.actor.log(
                    Paint::Green,
                    format!(
                        "{} is free: AGENTS of {} keeps its conversation, its worktree goes",
                        app.actor.shown(&agent),
                        free_label(app)
                    ),
                ),
                Err(why) => {
                    app.actor.log(
                        Paint::Dim,
                        format!(
                            "the agent of {} not freed: {why}, it goes with its worktree",
                            feature_of(&app.actor.main, &dir)
                        ),
                    );
                    remove_worktree(app, &dir);
                    app.redraw();
                }
            }
        }) as Effect)
    });
}

/// Its agent retired (#381): `wt:orchestrate <name> --retire` ends it, its
/// last run writes the handoff, the record keeps it; then its worktree goes,
/// whatever became of the handoff.
fn retire_then_remove(app: &mut Overview, name: &str, tree: &Tree, iid: i64) {
    let dir = tree.dir.clone();
    let main = app.actor.main.clone();
    let feature = feature_of(&main, &dir);
    let base = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let log = spawn::log_file(&format!("overview-retire-{base}.log"));
    let program = spawn::breadboard();
    let poster = app.actor.poster();
    app.actor.auto.freeing.insert(dir.clone());
    let done = dir.clone();
    let started = spawn::start(
        Background {
            program: &program,
            args: vec![
                "wt:orchestrate".into(),
                name.to_string(),
                "--retire".into(),
                feature.clone(),
                "--mr".into(),
                iid.to_string(),
            ],
            cwd: &main,
            log: &log,
            env: vec![],
            is_paneless: false,
        },
        move |code, last| {
            poster.post(move |app: &mut Overview| {
                app.actor.auto.freeing.remove(&done);
                let who = feature_of(&app.actor.main, &done);
                match error_of(code, &last, "wt:orchestrate --retire ") {
                    None => app.actor.log(
                        Paint::Green,
                        format!("{who}'s agent retired: its handoff is kept, its next issue goes to a fresh agent"),
                    ),
                    Some(why) => app.actor.log(
                        Paint::Dim,
                        format!("{who}'s agent not retired ({why}), it goes with its worktree"),
                    ),
                }
                remove_worktree(app, &done);
                app.redraw();
            });
        },
    );
    if let Err(why) = started {
        app.actor.auto.freeing.remove(&dir);
        app.actor.log(
            Paint::Dim,
            format!("{feature}'s agent not retired ({why}), it goes with its worktree"),
        );
        remove_worktree(app, &dir);
    }
}

fn free_label(app: &Overview) -> String {
    app.orchestration()
        .map(|model| model.label.clone())
        .unwrap_or_else(|| "its orchestration's space".into())
}

/// Freeing an agent (worktree/free.mjs `freeAgent`).
struct Free {
    main: PathBuf,
    name: String,
    id: String,
    agent: String,
    known: Option<Vec<String>>,
    /// The MRs it merged before this one.
    mrs: Vec<i64>,
    dir: PathBuf,
    iid: i64,
}

impl Free {
    /// Ok(its name): the record keeps it by its conversation's id (#345). A
    /// headless agent (#359, #361) has nothing to end or close: free is its
    /// record entry, and its next handover is the next run.
    fn run(&self) -> Result<String, String> {
        // One still in its tab from before the switch (#361) ends there and
        // its pane closes: the worktree's removal waits for every claude in it.
        let pane = breadboard_core::herdr::all_panes()
            .into_iter()
            .find(|pane| pane.session.as_deref() == Some(self.id.as_str()));
        for pid in breadboard_core::conversations::interactive_pids_of(&self.id) {
            if !crate::cli::lifecycle::handover::end_claude(pid) {
                return Err(format!("its claude in a tab (pid {pid}) did not end"));
            }
        }
        if let Some(pane) = pane {
            herdr::did(&["pane", "close", &pane.id]);
        }
        self.record();
        Ok(self.agent.clone())
    }

    /// The free agent in the record (its session, files, MRs, where it came from), unbound from its worktree.
    fn record(&self) {
        let mut files: Vec<String> = self.known.clone().unwrap_or_default();
        for file in work_files(&self.dir) {
            if !files.contains(&file) {
                files.push(file);
            }
        }
        let mut mrs = self.mrs.clone();
        if !mrs.contains(&self.iid) {
            mrs.push(self.iid);
        }
        let fields = json!({
            "name": self.agent,
            "files": files,
            "mrs": mrs,
            "free": { "at": to_iso(now_ms()), "from": feature_of(&self.main, &self.dir), "mr": self.iid },
        });
        let _ = write_worker(
            &self.main,
            &self.name,
            &self.id,
            fields.as_object().cloned().unwrap_or_default(),
        );
        // Unbound: no restart resumes it in the worktree, and wt:remove waits until it left.
        let _ = std::fs::remove_file(breadboard_core::conversations::session_file(&self.dir));
    }
}

/// `wt:remove <feature> --close-space`, detached (`removeWorktree`).
pub fn remove_worktree(app: &mut Overview, dir: &Path) {
    let main = app.actor.main.clone();
    let feature = feature_of(&main, dir);
    let base = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let log = spawn::log_file(&format!("overview-remove-{base}.log"));
    let program = spawn::breadboard();
    let poster = app.actor.poster();
    let done_dir = dir.to_path_buf();
    let done_feature = feature.clone();
    let started = spawn::start(
        Background {
            program: &program,
            args: vec!["wt:remove".into(), feature.clone(), "--close-space".into()],
            cwd: &main,
            log: &log,
            env: vec![],
            is_paneless: false,
        },
        move |code, last| {
            poster.post(move |app: &mut Overview| {
                let error = error_of(code, &last, "wt:remove ");
                if let Some(removal) = app.state.removals.get_mut(&done_dir) {
                    removal.is_running = false;
                    removal.error = error.clone();
                }
                let key = format!("remove:{done_feature}");
                match error {
                    None => app
                        .actor
                        .log_as(&key, Paint::Green, format!("removed {done_feature}"), None),
                    Some(why) => app
                        .actor
                        .log_as(&key, Paint::Red, format!("{done_feature} stays: {why}"), None),
                }
                app.redraw();
            });
        },
    );
    let removal = match &started {
        Ok(()) => Removal {
            is_armed: false,
            is_running: true,
            error: None,
        },
        Err(why) => Removal {
            is_armed: false,
            is_running: false,
            error: Some(why.clone()),
        },
    };
    app.state.removals.insert(dir.to_path_buf(), removal);
    let key = format!("remove:{feature}");
    let command = format!(
        "{run} wt:remove {feature} --close-space",
        run = breadboard_core::config::run_word()
    );
    match started {
        Ok(()) => app
            .actor
            .log_as(&key, Paint::Yellow, format!("removing {feature} …"), Some(command)),
        Err(why) => app
            .actor
            .log_as(&key, Paint::Red, format!("{feature} stays: {why}"), Some(command)),
    }
}

/// A click on a removal row (`clickRemove`): the first arms it, the second removes.
pub fn click_remove(app: &mut Overview, dir: &Path) {
    let state = app.state.removals.get(dir).cloned().unwrap_or_default();
    if state.is_running {
        return;
    }
    if !state.is_armed {
        app.actor.auto.seq += 1;
        let seq = app.actor.auto.seq;
        app.actor.auto.arms.insert(dir.to_path_buf(), seq);
        app.state.removals.insert(
            dir.to_path_buf(),
            Removal {
                is_armed: true,
                ..state
            },
        );
        let dir = dir.to_path_buf();
        app.actor.later(REMOVE_ARM, move |app: &mut Overview| {
            if app.actor.auto.arms.get(&dir) == Some(&seq)
                && let Some(removal) = app.state.removals.get_mut(&dir)
                && removal.is_armed
            {
                removal.is_armed = false;
                app.redraw();
            }
        });
        return;
    }
    remove_worktree(app, dir);
}

/// What a removal row says while armed or running (`removeText`).
pub fn removal_said(removal: &Removal, feature: &str) -> Option<Vec<breadboard_core::model::Span>> {
    use breadboard_core::model::Span;
    if removal.is_running {
        return Some(vec![Span::plain(format!("removing {feature} …"))]);
    }
    if removal.is_armed {
        return Some(vec![Span::new(
            format!("click again to remove {feature}: worktree, branch and its stack's data"),
            Paint::Red,
        )]);
    }
    removal.error.as_ref().map(|error| {
        vec![
            Span::plain(format!("remove the worktree {feature}  ")),
            Span::new(error.clone(), Paint::Red),
        ]
    })
}

/// The waits of the orchestration's agents whose command is gone (waits.mjs
/// `deliverWaits`): judged from what the refresh read (its MRs and the
/// awaited ones, no call of its own, #305), the line `wt:wait` would have
/// printed typed into the agent; delivered, the wait goes. One whose agent's
/// own MR is merged or closed is dropped instead (#421): its worktree is
/// about to go, and its line would only wake an agent with nothing left to do.
fn deliver_waits(app: &mut Overview, local: &OrchestrationLocal, remote: &Remote) {
    let mut found: HashMap<i64, Awaited> = remote
        .awaited
        .iter()
        .map(|(iid, state)| {
            (
                *iid,
                Awaited {
                    iid: *iid,
                    state: state.clone(),
                    has_conflicts: false,
                },
            )
        })
        .collect();
    for part in remote.of.values() {
        if let Some(mr) = &part.mr {
            found.insert(
                mr.iid,
                Awaited {
                    iid: mr.iid,
                    state: mr.state.clone(),
                    has_conflicts: mr.has_conflicts,
                },
            );
        }
    }
    for tree in &local.trees {
        let Some(wait) = &tree.wait else { continue };
        if is_waiting(wait) || app.actor.auto.waits_in_flight.contains(&tree.dir) {
            continue;
        }
        if let Some(own) = remote
            .of(&tree.branch)
            .mr
            .as_ref()
            .filter(|mr| is_moot(Some(&mr.state)))
        {
            clear_wait(&tree.dir);
            app.state.undelivered_waits.remove(&tree.dir);
            let who = feature_of(&app.actor.main, &tree.dir);
            app.actor.log(
                Paint::Dim,
                format!("dropped the wait of {who}: its own !{} is {}", own.iid, own.state),
            );
            continue;
        }
        let awaited: Vec<Awaited> = wait.mrs.iter().filter_map(|iid| found.get(iid).cloned()).collect();
        let (main, dir, wait) = (app.actor.main.clone(), tree.dir.clone(), wait.clone());
        let conflicts = app.actor.auto.wait_conflicts.clone();
        app.actor.auto.waits_in_flight.insert(dir.clone());
        app.actor.queue(move || {
            let delivered = deliver_wait(&main, &dir, &wait, &awaited, &conflicts);
            Some(Box::new(move |app: &mut Overview| {
                app.actor.auto.waits_in_flight.remove(&dir);
                if let Some(line) = delivered {
                    app.state.undelivered_waits.remove(&dir);
                    let who = feature_of(&app.actor.main, &dir);
                    app.actor.told(&who, "that its wait ended", &line, true);
                } else {
                    // Not in (its agent blocked): the row keeps `waits for` until it is.
                    app.state.undelivered_waits.insert(dir);
                }
            }) as Effect)
        });
    }
}

/// One ended wait delivered: the line when it got in.
fn deliver_wait(
    main: &Path,
    dir: &Path,
    wait: &breadboard_core::wait::Wait,
    found: &[Awaited],
    conflicts: &Mutex<HashMap<PathBuf, HashMap<i64, i64>>>,
) -> Option<String> {
    let verdict = {
        let mut conflicts = conflicts.lock().unwrap_or_else(|e| e.into_inner());
        verdict_of(wait, found, now_ms(), conflicts.entry(dir.to_path_buf()).or_default())
    };
    let behind = (verdict == Verdict::Merged).then(|| behind_base(dir));
    let line = line_of(wait, &verdict, &base_of(dir).local, behind)?;
    // A new wt:wait may have started meanwhile.
    if wait_of(dir).and_then(|now| now.since) != wait.since || !herdr::deliver_to(main, dir, &line) {
        return None;
    }
    clear_wait(dir);
    conflicts.lock().unwrap_or_else(|e| e.into_inner()).remove(dir);
    Some(herdr::overview_line(&line))
}

/// The orchestrator's chat told once that the Claude usage limit holds the
/// runs and once that they go on (#393), not each agent's: who was told is
/// in `usage.json`, so a restarted overview doesn't tell again.
fn tell_usage(app: &mut Overview, name: &str, model: &OrchestrationModel) {
    const KEY: &str = "usage-limit";
    let main = app.actor.main.clone();
    let usage: Usage = runs::read_or_default(&runs::usage_file(&main));
    let is_told = usage.told.iter().any(|told| told == name);
    let line = match (&model.usage, is_told) {
        (Some(until), false) => {
            format!("{until}: no run of your agents starts, the stopped ones go on by themselves after it")
        }
        (None, true) => "the usage limit reset: the runs it stopped go on".to_string(),
        _ => return,
    };
    if !app.actor.auto.in_flight.insert(KEY.into()) {
        return;
    }
    let (name, is_holding) = (name.to_string(), model.usage.is_some());
    app.actor.queue(move || {
        let pane = herdr::agent_pane_of(&main, &Target::Orchestration(name.clone()));
        let is_in = herdr::type_into(pane.as_ref(), &line);
        if is_in {
            let _ = runs::change_usage(&main, |usage| {
                usage.told.retain(|told| *told != name);
                if is_holding {
                    usage.told.push(name);
                }
            });
        }
        Some(Box::new(move |app: &mut Overview| {
            app.actor.auto.in_flight.remove(KEY);
            if is_in {
                app.actor.log(Paint::Yellow, format!("told the orchestrator: {line}"));
            }
        }))
    });
}

/// A herdr notification for each action new since the last read
/// (`notifyNewActions`): once GitLab answered, in herdr, a try once it is ready.
fn notify_new(app: &mut Overview, model: &OrchestrationModel) {
    if !model.has_remote || !is_in_herdr() {
        return;
    }
    let Ok(section) = &model.actions else { return };
    let items: Vec<&Item> = section
        .actions
        .iter()
        .chain(&section.up_next)
        .filter(|item| !item.is_waiting())
        .collect();
    let keys: HashSet<String> = items.iter().map(|item| item.key.clone()).collect();
    let fresh: Vec<(String, String)> = match &app.actor.auto.seen {
        Some(seen) => items
            .iter()
            .filter(|item| !seen.contains(&item.key))
            .map(|item| {
                let kind = if item.try_state.is_some() {
                    "try, ready".to_string()
                } else {
                    item.kind.clone()
                };
                let text = item
                    .action
                    .as_ref()
                    .map_or(item.text.as_str(), |action| action.text.as_str());
                (
                    item.key.clone(),
                    format!(
                        "{}: {kind} · {}",
                        item.who,
                        super::first_line(&spawn::strip_escapes(text))
                    ),
                )
            })
            .collect(),
        None => Vec::new(),
    };
    app.actor.auto.seen = Some(keys);
    if fresh.is_empty() {
        return;
    }
    let label = model.label.clone();
    app.actor.queue(move || {
        for (_, body) in &fresh {
            herdr::notify(&label, body, "request");
        }
        None
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("breadboard-auto-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn remembers_sent_lines_newest_fifty() {
        let dir = scratch("sent");
        for n in 0..55 {
            mark_sent(&dir, &format!("k{n}"));
        }
        let sent = breadboard_core::model::auto_actions::sent_of(&dir);
        assert_eq!(sent.len(), 50);
        assert!(sent.contains("k54") && !sent.contains("k4"));
        forget_sent(&dir, "k54");
        assert!(!breadboard_core::model::auto_actions::sent_of(&dir).contains("k54"));
        let text = std::fs::read_to_string(sent_file(&dir)).unwrap();
        assert!(
            text.starts_with("{\n  \"sent\": [\n    \"k5\"") && text.ends_with("]\n}\n"),
            "{text}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn says_what_a_line_told_in_a_few_words() {
        assert_eq!(
            about_of("conflicts:316"),
            "to merge develop in (!316 conflicts with it)"
        );
        assert_eq!(about_of("pipeline:!318:2901"), "that the pipeline of !318 failed");
        assert_eq!(about_of("unpushed:feature/x:2:none"), "to push its commits");
        assert_eq!(about_of("boxes:!303"), "to tick the done boxes of !303's issue");
    }

    #[test]
    fn says_why_a_pipeline_line_waits() {
        assert_eq!(
            pipeline_why(Some("blocked")),
            "its agent is at a question, the line follows once it answers"
        );
        assert_eq!(
            pipeline_why(Some("idle")),
            "herdr didn't take the line: click to try again"
        );
        assert_eq!(
            pipeline_why(None),
            "its agent isn't running: the line follows once it is"
        );
    }

    #[test]
    fn a_removal_row_says_where_it_stands() {
        let text =
            |removal: &Removal| removal_said(removal, "303-x").map(|spans| breadboard_core::model::text_of(&spans));
        assert_eq!(text(&Removal::default()), None);
        assert_eq!(
            text(&Removal {
                is_armed: true,
                ..Default::default()
            })
            .as_deref(),
            Some("click again to remove 303-x: worktree, branch and its stack's data")
        );
        assert_eq!(
            text(&Removal {
                is_running: true,
                ..Default::default()
            })
            .as_deref(),
            Some("removing 303-x …")
        );
        assert_eq!(
            text(&Removal {
                error: Some("it has changes".into()),
                ..Default::default()
            })
            .as_deref(),
            Some("remove the worktree 303-x  it has changes")
        );
    }
}
