//! The review run (#374): an MR whose row waits for `review due` (its bar
//! full, its pipeline green, `[review] enabled`) gets one `claude -p --model
//! <[review] model>` in its worktree, started by the live overview that
//! delivers for its orchestration, as far as the memory for runs allows and
//! never while the daily limit or a pause holds the runs. A diff not worth a
//! review is skipped without a run. Its end writes the findings, an MR
//! comment, and serious ones go to the MR's agent (`review.rs`).

use super::glab;
use super::herdr::deliver_to;
use crate::tui::overview::Overview;
use breadboard_core::config;
use breadboard_core::conversations::new_session_id;
use breadboard_core::git::git;
use breadboard_core::gitlab::Remote;
use breadboard_core::lock::is_alive;
use breadboard_core::model::Paint;
use breadboard_core::model::orchestration::{OrchestrationLocal, OrchestrationModel};
use breadboard_core::review::{
    self, DUE, Issue, Review, State, agent_line, agents_files_of, args_of, comment_of, diff_of, dollars_of,
    findings_of, prompt_of, reviews_of, skip_of, write_review,
};
use breadboard_core::runs;
use breadboard_core::time::{now_ms, parse_iso, to_iso};
use breadboard_core::try_check::answer_of;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The claude a review runs.
const CLAUDE: &str = "claude";

/// A review written `Running` whose claude never got a pid is gone after this.
const STARTING_FOR_MS: i64 = 5 * 60_000;

/// An MR due for its review: where, which, at which head, into what.
struct Due {
    dir: PathBuf,
    iid: i64,
    issue: Option<i64>,
    head: String,
    target: String,
}

/// The reviews a read sets off: the MRs whose row waits for `review due`.
pub fn after_read(app: &mut Overview, local: &OrchestrationLocal, model: &OrchestrationModel, remote: &Remote) {
    let now = now_ms();
    let mut running = 0;
    for tree in &local.trees {
        for found in reviews_of(&tree.dir)
            .iter()
            .filter(|found| found.state == State::Running)
        {
            let is_starting = found.pid.is_none() && parse_iso(&found.at).is_some_and(|at| now - at < STARTING_FOR_MS);
            if found.pid.is_some_and(|pid| is_alive(i64::from(pid))) || is_starting {
                running += 1;
                continue;
            }
            // Its claude is gone without an answer: the user reads the MR.
            let ended = Review {
                state: State::Done,
                failed: Some("its run ended without an answer".into()),
                pid: None,
                ..found.clone()
            };
            let _ = write_review(&tree.dir, &ended);
        }
    }
    let main = app.actor.main.clone();
    if runs::held_now(&main, now).is_some() || runs::paused_of(&main).is_some() {
        return;
    }
    let Ok(mrs) = model.merge_requests.as_ref() else {
        return;
    };
    let mut due: Vec<Due> = Vec::new();
    for facts in mrs.open.iter().filter_map(|row| row.mr.as_ref()) {
        let Some(dir) = facts.dir.as_ref() else { continue };
        if !facts.reasons.iter().any(|reason| reason.text == DUE) {
            continue;
        }
        let Some(mr) = remote.of(&facts.branch).mr.as_ref() else {
            continue;
        };
        let Some(sha) = mr.sha.clone().filter(|sha| !sha.is_empty()) else {
            continue;
        };
        // The worktree must stand at the head GitLab has: that is what is reviewed.
        let head = git(dir, &["rev-parse", "HEAD"]).out().to_string();
        if !head.starts_with(&sha) && !sha.starts_with(&head) {
            continue;
        }
        due.push(Due {
            dir: dir.clone(),
            iid: mr.iid,
            issue: facts.issue,
            head,
            target: mr.target().to_string(),
        });
    }
    let room = runs::room_for(runs::available(), running);
    for due in due.into_iter().take(room) {
        start(app, due);
    }
}

/// One review started: written `Running` at once (the next read starts no
/// second), the diff, the issue and the run on a thread of its own.
fn start(app: &mut Overview, due: Due) {
    let main = app.actor.main.clone();
    let config = config::of(&due.dir).config.review.clone();
    let session = new_session_id();
    let review = Review {
        iid: due.iid,
        head: due.head.clone(),
        model: config.model.clone(),
        state: State::Running,
        findings: Vec::new(),
        skipped: None,
        lines: 0,
        tokens: 0,
        dollars: 0.0,
        at: to_iso(now_ms()),
        pid: None,
        session: Some(session.clone()),
        failed: None,
    };
    if write_review(&due.dir, &review).is_err() {
        return;
    }
    let what = format!("!{} of {}", due.iid, breadboard_core::git::feature_of(&main, &due.dir));
    let poster = app.actor.poster();
    std::thread::spawn(move || {
        let (review, said) = run(&main, &due, review, &config, &session);
        let _ = write_review(&due.dir, &review);
        if review.state != State::Running {
            let _ = review::log(&main, due.issue, &review);
        }
        poster.post(move |app: &mut Overview| {
            let (paint, text) = said;
            app.actor.log(paint, format!("{} on {what}: {text}", review.model));
            app.redraw();
        });
    });
}

/// The review of `due`: skipped, or run and its answer read, its comment
/// posted, serious findings sent to the agent. What the log says of it.
fn run(
    main: &Path,
    due: &Due,
    mut review: Review,
    config: &config::Review,
    session: &str,
) -> (Review, (Paint, String)) {
    let Some(diff) = diff_of(&due.dir, &due.target, &config.exclude) else {
        review.state = State::Done;
        review.failed = Some(format!("no diff against origin/{}", due.target));
        return (review, (Paint::Red, "no diff to review".into()));
    };
    review.lines = diff.lines;
    if let Some(why) = skip_of(&diff, config) {
        review.state = State::Skipped;
        review.skipped = Some(why.clone());
        review.session = None;
        return (review, (Paint::Dim, format!("review skipped: {why}")));
    }
    let issue = due.issue.and_then(|number| issue_of(main, number));
    let prompt = prompt_of(issue.as_ref(), &diff, &agents_files_of(&due.dir, &diff.files));
    let evidence = review::evidence_dir(&due.dir, &due.head);
    let _ = std::fs::create_dir_all(&evidence);
    // The overview's pane isn't the review's: herdr would resume it there (#425).
    let spawned = breadboard_core::herdr::off_pane(&mut Command::new(CLAUDE))
        .args(args_of(&config.model, session, &prompt, config.max_dollars))
        .current_dir(&due.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            review.state = State::Done;
            review.failed = Some(format!("its run didn't start: {error}"));
            return (review, (Paint::Red, format!("didn't start ({error})")));
        }
    };
    review.pid = Some(child.id());
    let _ = write_review(&due.dir, &review);
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
    review.state = State::Done;
    review.pid = None;
    review.tokens = tokens;
    review.dollars = dollars_of(&lines);
    let Some(findings) = findings_of(&text) else {
        review.failed = Some("its answer named no findings".into());
        return (
            review,
            (Paint::Yellow, "its answer named no findings, the MR is yours".into()),
        );
    };
    review.findings = findings;
    let note = format!("{}/notes", glab::mr_endpoint(due.iid));
    let comment = glab::write(main, "POST", &note, &[("body", comment_of(&review))]).err();
    let serious = review.serious();
    let mut said = review::row_text(&review);
    if serious > 0 {
        let line = agent_line(due.iid, &review);
        if !deliver_to(main, &due.dir, &line) {
            said.push_str(" · its line didn't get to the agent");
        }
    }
    if let Some(error) = comment {
        said.push_str(&format!(" · no MR comment ({error})"));
    }
    let paint = match (serious, review.findings.len()) {
        (0, 0) => Paint::Green,
        (0, _) => Paint::Yellow,
        _ => Paint::Red,
    };
    (review, (paint, said))
}

/// The issue `number` as the review is told it: GitLab's title and description.
fn issue_of(main: &Path, number: i64) -> Option<Issue> {
    let found = glab::get(main, &format!("projects/:fullpath/issues/{number}")).ok()?;
    Some(Issue {
        number,
        title: found.get("title")?.as_str()?.to_string(),
        description: found
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}
