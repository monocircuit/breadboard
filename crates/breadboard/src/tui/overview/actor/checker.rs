//! The try checker (#384): while its orchestration's auto mode is on, the
//! live overview that acts for it starts one `claude -p --model <checker>`
//! per open try and head, in the try's worktree, as far as the memory for
//! runs allows (#358's `room_for`). Its answer is read from its stream when
//! it ends: `looks good` and `broken` answer the try as a click does (a
//! `broken` with its reason reaches the agent, the MR goes back to draft),
//! `needs a human` leaves it to the user. The checks and their evidence stay
//! in the worktree (`try_check.rs`).

use super::super::actions::answer_as_checker;
use crate::tui::overview::Overview;
use breadboard_core::actions::{Action, actions_file};
use breadboard_core::config;
use breadboard_core::conversations::new_session_id;
use breadboard_core::git::git;
use breadboard_core::lock::is_alive;
use breadboard_core::model::Paint;
use breadboard_core::model::orchestration::OrchestrationLocal;
use breadboard_core::runs;
use breadboard_core::time::{now_ms, to_iso};
use breadboard_core::try_check::{
    self, Check, State, Verdict, answer_of, args_of, broken_note, checks_of, evidence_dir, prompt_of, verdict_or_human,
    write_check,
};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The claude a checker runs.
const CLAUDE: &str = "claude";

/// A try due for its checker: where, which, at which head.
struct Due {
    dir: PathBuf,
    action: Action,
    head: String,
    app: Option<String>,
}

/// The checks a read sets off: none unless the orchestration's auto mode is on.
pub fn after_read(app: &mut Overview, local: &OrchestrationLocal) {
    if !local.record.as_ref().is_some_and(|record| record.is_auto) {
        return;
    }
    let mut running = 0;
    let mut due: Vec<Due> = Vec::new();
    for tree in &local.trees {
        let checks = checks_of(&tree.dir);
        running += checks
            .iter()
            .filter(|check| check.state == State::Running && check.pid.is_some_and(|pid| is_alive(i64::from(pid))))
            .count();
        // A checker whose process is gone ended without an answer: the user's.
        for check in checks
            .iter()
            .filter(|check| check.state == State::Running && !check.pid.is_some_and(|pid| is_alive(i64::from(pid))))
        {
            let ended = Check {
                state: State::Done,
                verdict: Some(Verdict::Human("its checker ended without an answer".into())),
                pid: None,
                ..check.clone()
            };
            let _ = write_check(&tree.dir, &ended);
        }
        if !tree.actions.iter().any(try_check::is_checkable) {
            continue;
        }
        let head = git(&tree.dir, &["rev-parse", "HEAD"]).out().to_string();
        if head.is_empty() {
            continue;
        }
        let app_url = (tree.app_port > 0).then(|| format!("http://localhost:{}", tree.app_port + tree.offset));
        for action in try_check::due(&tree.actions, &checks, &head) {
            due.push(Due {
                dir: tree.dir.clone(),
                action: action.clone(),
                head: head.clone(),
                app: app_url.clone(),
            });
        }
    }
    let room = runs::room_for(runs::available(), running);
    for due in due.into_iter().take(room) {
        start(app, due);
    }
}

/// One checker started on a thread of its own; its end answers the try.
fn start(app: &mut Overview, due: Due) {
    let model = config::of(&due.dir).config.tries.checker.clone();
    let evidence = evidence_dir(&due.dir, due.action.id, &due.head);
    let _ = std::fs::create_dir_all(&evidence);
    let prompt = prompt_of(&due.action, &evidence, due.app.as_deref());
    let session = new_session_id();
    let spawned = Command::new(CLAUDE)
        .args(args_of(&model, &session, &prompt))
        .current_dir(&due.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut check = Check {
        id: due.action.id,
        head: due.head.clone(),
        model: model.clone(),
        state: State::Running,
        verdict: None,
        evidence: Some(evidence.clone()),
        at: to_iso(now_ms()),
        tokens: 0,
        pid: None,
    };
    let what = format!(
        "try #{} of {}",
        due.action.id,
        breadboard_core::git::feature_of(&app.actor.main, &due.dir)
    );
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            check.state = State::Done;
            check.verdict = Some(Verdict::Human(format!("its checker didn't start: {error}")));
            let _ = write_check(&due.dir, &check);
            app.actor.log(
                Paint::Red,
                format!("{what}: {model} didn't start ({error}), it waits for you"),
            );
            return;
        }
    };
    check.pid = Some(child.id());
    let _ = write_check(&due.dir, &check);
    app.actor.log(Paint::Yellow, format!("{model} checks {what} …"));
    let poster = app.actor.poster();
    std::thread::spawn(move || {
        let mut lines = Vec::new();
        if let Some(out) = child.stdout.take() {
            let mut log = std::fs::File::create(evidence.join("stream.jsonl")).ok();
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(log) = log.as_mut() {
                    let _ = writeln!(log, "{line}");
                }
                lines.push(line);
            }
        }
        let _ = child.wait();
        let (text, tokens) = answer_of(&lines);
        let _ = std::fs::write(evidence.join("answer.md"), &text);
        let verdict = verdict_or_human(&text);
        let done = Check {
            state: State::Done,
            verdict: Some(verdict.clone()),
            tokens,
            pid: None,
            ..check
        };
        let _ = write_check(&due.dir, &done);
        poster.post(move |app: &mut Overview| apply(app, &due.dir, &due.action, &due.head, &model, &verdict, &what));
    });
}

/// The verdict on its try: only while the try is open and unanswered and
/// the worktree still at the head it was checked at.
fn apply(app: &mut Overview, dir: &Path, action: &Action, head: &str, model: &str, verdict: &Verdict, what: &str) {
    let file = actions_file(dir);
    let is_open = breadboard_core::actions::actions_of(&file)
        .iter()
        .any(|open| open.id == action.id && open.answer.is_none());
    let is_same = git(dir, &["rev-parse", "HEAD"]).out() == head;
    let said = format!("{model} on {what}: {} {}", verdict.word(), verdict.reason())
        .trim_end()
        .to_string();
    match verdict.answer() {
        Some(answer) if is_open && is_same => {
            let note = (answer == "broken").then(|| broken_note(model, verdict.reason()));
            if answer_as_checker(app, &file, action.id, answer, note.as_deref()) {
                let paint = if answer == "broken" { Paint::Red } else { Paint::Green };
                app.actor.log(paint, said);
            }
        }
        Some(_) => app.actor.log(
            Paint::Dim,
            format!("{said} · not applied: the try or its head moved on"),
        ),
        None => app.actor.log(Paint::Yellow, format!("{said} · it waits for you")),
    }
    app.redraw();
}
