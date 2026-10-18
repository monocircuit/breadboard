//! Merging by two clicks (merge.mjs, #182, #242, #280): while anything blocks,
//! a click opens the MR; the first click arms the merge for 5 s, the second
//! reads the MR, its issue's boxes and its worktree's tries again, marks a
//! draft ready when it may, and merges as the user with the MR's squash
//! setting, the source branch deleted, at the commit whose pipeline passed
//! (or, into the default branch without one, #443, wt:check passed at).
//! Then the order carries on: its agent hears it. The other open MRs are
//! not told to merge develop in; one that conflicts gets that from its own
//! conflict (#392). With the redraft of a try
//! answered `broken` (actions.rs) these three PUTs are the overview's only
//! GitLab writes (rule 6).

use super::glab::{self, mr_endpoint, ready_title};
use super::{Actor, Poster, browser, herdr};
use crate::tui::overview::Overview;
use breadboard_core::actions::{Action, actions_file, actions_of, answered_of, done_action};
use breadboard_core::check::{CheckState, check_of};
use breadboard_core::gitlab::{IssueBox, Mr, Pipeline, boxes_of};
use breadboard_core::model::merge::{blockers_of, is_same_commit, needs_pipeline};
use breadboard_core::model::order::{Go, MergeOffer};
use breadboard_core::model::{Paint, Span};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the first click keeps a merge armed.
const ARM: Duration = Duration::from_millis(5000);
/// How often, and how far apart, the MR is read again after it was marked ready.
const SETTLE_TRIES: usize = 10;
const SETTLE: Duration = Duration::from_millis(1000);
/// How long a merge GitLab refused for now waits before its one retry.
const RETRY: Duration = Duration::from_millis(5000);
/// The project's open MRs, asking GitLab to recompute their mergeability.
const RECHECK_OPEN: &str = "projects/:fullpath/merge_requests?state=opened&per_page=100&with_merge_status_recheck=true";
/// GitLab still working out whether an MR can be merged.
const CHECKING: [&str; 4] = ["checking", "unchecked", "preparing", "approvals_syncing"];

/// Where a merge stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Armed,
    Running,
    Merged,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    pub phase: Phase,
    pub error: Option<String>,
    /// Which arming this is: an older one's expiry leaves a newer one alone.
    pub seq: u64,
}

/// The merges of this overview, by iid; the lines after them that didn't get in yet.
#[derive(Debug, Default)]
pub struct Merges {
    pub of: HashMap<i64, Merge>,
    pending: Arc<Mutex<Vec<Pending>>>,
    seq: u64,
}

/// A line after a merge that didn't get in yet: its worktree, what it tells in words, the line.
type Pending = (PathBuf, String, String);

/// Whether `text` holds one of `codes` as a word of its own (`\b405\b`).
fn has_code(text: &str, codes: &[&str]) -> bool {
    let is_word = |ch: Option<char>| ch.is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
    codes.iter().any(|code| {
        text.match_indices(code)
            .any(|(at, _)| !is_word(text[..at].chars().next_back()) && !is_word(text[at + code.len()..].chars().next()))
    })
}

/// Whether GitLab refused the merge the way it does while it still checks the MR (after another merge).
pub fn is_not_yet(error: &str) -> bool {
    let lower = error.to_lowercase();
    has_code(error, &["405", "406", "409", "422"])
        || ["not allowed", "cannot be merged", "unprocessable"]
            .iter()
            .any(|words| lower.contains(words))
}

/// `(HTTP 422)`, ` HTTP 500` taken out of a line.
fn without_http(line: &str) -> String {
    let mut out = line.to_string();
    while let Some(at) = out.find("HTTP ") {
        let code = &out[at + 5..];
        if code.len() < 3 || !code[..3].chars().all(|ch| ch.is_ascii_digit()) {
            break;
        }
        let mut from = at;
        let mut to = at + 8;
        if out[..from].ends_with('(') {
            from -= 1;
        }
        if out[to..].starts_with(')') {
            to += 1;
        }
        from = out[..from].trim_end().len();
        out.replace_range(from..to, "");
    }
    out.trim().to_string()
}

/// What went wrong, one line in words: no bare `HTTP 422` (`reasonOf`).
pub fn reason_of(error: &str) -> String {
    let text = error.trim();
    if is_not_yet(text) {
        return "GitLab is still checking the MR (it refused the merge for now)".into();
    }
    if has_code(text, &["401", "403"]) {
        return "GitLab refuses: no right to merge it as you".into();
    }
    if has_code(text, &["404"]) {
        return "GitLab doesn't find the MR".into();
    }
    let last = without_http(text.lines().last().unwrap_or_default());
    if last.is_empty() { "GitLab refused".into() } else { last }
}

/// Why a read before the merge failed, in words: what it names is the
/// caller's (`#305 not read: …`), never `the MR` for an issue.
pub fn read_reason(error: &str) -> String {
    let text = error.trim();
    if has_code(text, &["401", "403"]) {
        return "GitLab refuses to show it to you".into();
    }
    if has_code(text, &["404"]) {
        return "GitLab doesn't find it".into();
    }
    let last = without_http(text.lines().last().unwrap_or_default());
    if last.is_empty() {
        "GitLab did not answer".into()
    } else {
        last
    }
}

/// An MR as GitLab's REST answers it, in the shape the blockers read.
pub fn mr_of_rest(value: &Value) -> Mr {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(String::from);
    let pipeline = value
        .get("head_pipeline")
        .or_else(|| value.get("pipeline"))
        .filter(|pipeline| pipeline.is_object())
        .map(|pipeline| Pipeline {
            id: pipeline.get("id").and_then(Value::as_i64).unwrap_or(0),
            status: pipeline
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            web_url: pipeline.get("web_url").and_then(Value::as_str).map(String::from),
            sha: pipeline.get("sha").and_then(Value::as_str).map(String::from),
            ..Pipeline::default()
        });
    Mr {
        iid: value.get("iid").and_then(Value::as_i64).unwrap_or(0),
        title: ready_title(&text("title").unwrap_or_default()),
        is_draft: value
            .get("draft")
            .or_else(|| value.get("work_in_progress"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        state: text("state").unwrap_or_default(),
        merged_at: text("merged_at"),
        has_conflicts: value.get("has_conflicts").and_then(Value::as_bool).unwrap_or(false),
        merge_status: text("detailed_merge_status"),
        merge_check: text("merge_status"),
        sha: text("sha"),
        url: text("web_url").unwrap_or_default(),
        pipeline,
        branch: text("source_branch").unwrap_or_default(),
        target: text("target_branch").unwrap_or_default(),
        ..Mr::default()
    }
}

/// The merge offer of the item whose click is `go`, from the model on screen.
fn offer_of(app: &Overview, go: &Go) -> Option<MergeOffer> {
    let model = app.orchestration()?;
    let section = model.actions.as_ref().ok()?;
    section
        .actions
        .iter()
        .chain(&section.up_next)
        .find(|item| item.go == *go && item.merge.is_some())
        .and_then(|item| item.merge.clone())
}

/// A click on a merge (`clickMerge`): opens the MR while anything blocks; the
/// first click arms it, the second merges; nothing while one runs or is done.
pub fn click(app: &mut Overview, go: &Go) {
    let Go::Merge { branch, action } = go else { return };
    let Some(offer) = offer_of(app, go) else {
        app.actor
            .log(Paint::Dim, "that merge is gone from the view: nothing to do");
        return;
    };
    let iid = offer.iid;
    let state = app.actor.merges.of.get(&iid).cloned();
    if state
        .as_ref()
        .is_some_and(|state| matches!(state.phase, Phase::Running | Phase::Merged))
    {
        return;
    }
    let dir = app.tree_dir_of(branch);
    if !offer.blockers.is_empty() {
        let main = app.actor.main.clone();
        browser::open_popup(
            &main,
            &offer.url,
            &herdr::space_label(&main, dir.as_deref().unwrap_or(&main)),
        );
        app.actor.log(
            Paint::Yellow,
            format!("!{iid} can't merge yet ({}): opened the MR", offer.blockers.join(" · ")),
        );
        return;
    }
    if state.as_ref().map(|state| state.phase) != Some(Phase::Armed) {
        app.actor.merges.seq += 1;
        let seq = app.actor.merges.seq;
        app.actor.merges.of.insert(
            iid,
            Merge {
                phase: Phase::Armed,
                error: None,
                seq,
            },
        );
        app.actor.later(ARM, move |app: &mut Overview| {
            if app
                .actor
                .merges
                .of
                .get(&iid)
                .is_some_and(|merge| merge.phase == Phase::Armed && merge.seq == seq)
            {
                app.actor.merges.of.remove(&iid);
                app.redraw();
            }
        });
        return;
    }
    start(app, branch, action, &offer);
}

/// A merge under way at once: after a second click, or by itself into an
/// epic (#350); on a thread of its own.
pub fn start(app: &mut Overview, branch: &str, action: &Option<(PathBuf, i64)>, offer: &MergeOffer) {
    let iid = offer.iid;
    let dir = app.tree_dir_of(branch);
    app.actor.merges.of.insert(
        iid,
        Merge {
            phase: Phase::Running,
            error: None,
            seq: 0,
        },
    );
    let verb = if offer.is_draft {
        "marking ready and merging"
    } else {
        "merging"
    };
    app.actor
        .log_as(&format!("merge:{iid}"), Paint::Yellow, format!("{verb} !{iid} …"), None);
    let run = Running {
        main: app.actor.main.clone(),
        iid,
        number: breadboard_core::gitlab::number_of(branch),
        dir,
        can_mark_ready: offer.can_mark_ready || action.is_some(),
        action: action.clone(),
        pending: app.actor.merges.pending.clone(),
        poster: app.actor.poster(),
    };
    std::thread::spawn(move || run.run());
}

/// A merge after its second click, on a thread of its own.
struct Running {
    main: PathBuf,
    iid: i64,
    number: Option<i64>,
    dir: Option<PathBuf>,
    can_mark_ready: bool,
    action: Option<(PathBuf, i64)>,
    pending: Arc<Mutex<Vec<Pending>>>,
    poster: Poster,
}

impl Running {
    fn run(self) {
        let iid = self.iid;
        let result = self.merge();
        let (phase, error) = match &result {
            Ok(_) => (Phase::Merged, None),
            Err(why) => (Phase::Failed, Some(why.clone())),
        };
        // The call that merged, word for word, as lazygit's log shows its command.
        let call = result
            .as_ref()
            .ok()
            .map(|(fresh, squash)| merge_call(iid, *squash, fresh));
        self.poster.post(move |app: &mut Overview| {
            app.actor.merges.of.insert(
                iid,
                Merge {
                    phase,
                    error: error.clone(),
                    seq: 0,
                },
            );
            let key = format!("merge:{iid}");
            match &error {
                None => app.actor.log_as(&key, Paint::Green, format!("merged !{iid}"), call),
                Some(why) => app
                    .actor
                    .log_as(&key, Paint::Red, format!("!{iid} not merged: {why}"), None),
            }
            // GitLab now, not in half a minute: NEXT moves on, the clean-up shows.
            app.refresh_gitlab();
        });
        if let Ok((fresh, _)) = result {
            // The main checkout takes it now, not at its next look (#348).
            breadboard_core::main_update::wake(&self.main);
            let told = self.carry_on(&fresh);
            self.poster.post(move |app: &mut Overview| {
                for (who, about, line, is_in) in told {
                    app.actor.told(&who, &about, &line, is_in);
                }
            });
        }
    }

    /// The MR fresh from GitLab, with its squash setting; Err: `!316 not read: <why>`.
    fn read_mr(&self) -> Result<(Mr, bool), String> {
        glab::get(&self.main, &mr_endpoint(self.iid))
            .map(|value| {
                let squash = value.get("squash").and_then(Value::as_bool).unwrap_or(false);
                (mr_of_rest(&value), squash)
            })
            .map_err(|error| format!("!{} not read: {}", self.iid, read_reason(&error)))
    }

    /// The boxes of its issue fresh from GitLab, none without one; Err: `#305 not read: <why>`.
    fn read_boxes(&self) -> Result<Vec<IssueBox>, String> {
        let Some(number) = self.number else {
            return Ok(Vec::new());
        };
        glab::get(&self.main, &format!("projects/:fullpath/issues/{number}"))
            .map(|issue| boxes_of(issue.get("description").and_then(Value::as_str).unwrap_or_default()))
            .map_err(|error| format!("#{number} not read: {}", read_reason(&error)))
    }

    /// Steps 1–5 of `runMerge`: Ok with the MR as merged at and its squash setting, Err why not.
    fn merge(&self) -> Result<(Mr, bool), String> {
        // The MR and its issue at once, as Node's Promise.all: one round trip before the PUT.
        let (boxes, first) = both(|| self.read_boxes(), || self.read_mr());
        let (boxes, mut fresh, mut squash) = fresh_of(boxes, first)?;
        let cant = |why: String| format!("can't merge: {why}");
        let (actions, answered) = self.tries();
        let merges_on_check = !breadboard_core::config::of_main(&self.main).config.merge.needs_pipeline;
        let needs = needs_pipeline(&fresh, merges_on_check);
        let mut blockers = blockers_of(Some(&fresh), &boxes, self.can_mark_ready, &actions, &answered, needs);
        if !needs && let Some(why) = self.check_lack(&fresh) {
            blockers.push(why);
        }
        if !blockers.is_empty() {
            return Err(format!("can't merge: {}", blockers.join(" · ")));
        }
        if fresh.is_draft {
            (fresh, squash) = self.mark_ready(&fresh)?;
            let blockers = blockers_of(Some(&fresh), &boxes, false, &actions, &answered, needs);
            if !blockers.is_empty() {
                return Err(format!("marked ready, can't merge: {}", blockers.join(" · ")));
            }
        }
        if let Err(error) = self.put_merge(&fresh, squash) {
            // Right after another merge GitLab still checks: once more when it says mergeable.
            if !is_not_yet(&error) {
                return Err(reason_of(&error));
            }
            let iid = self.iid;
            let why = reason_of(&error);
            self.poster.post(move |app: &mut Overview| {
                if let Some(merge) = app.actor.merges.of.get_mut(&iid) {
                    merge.error = Some(why);
                }
                app.redraw();
            });
            std::thread::sleep(RETRY);
            (fresh, squash) = self.read_mr().map_err(cant)?;
            if fresh.merge_status.as_deref() != Some("mergeable") {
                let blockers = blockers_of(Some(&fresh), &boxes, false, &actions, &answered, needs);
                let why = if blockers.is_empty() {
                    reason_of(&error)
                } else {
                    blockers.join(" · ")
                };
                return Err(format!("can't merge: {why}"));
            }
            self.put_merge(&fresh, squash).map_err(|error| reason_of(&error))?;
        }
        Ok((fresh, squash))
    }

    /// Without a pipeline of its own (#443): None once wt:check passed at the
    /// head GitLab merges, in its worktree; else why not.
    fn check_lack(&self, fresh: &Mr) -> Option<String> {
        let Some(dir) = &self.dir else {
            return Some("wt:check not known (no worktree here)".into());
        };
        match check_of(dir) {
            CheckState::Tested { sha, .. } if fresh.sha.as_deref().is_some_and(|head| is_same_commit(&sha, head)) => {
                None
            }
            _ => Some("wt:check not passed at the pushed head".into()),
        }
    }

    /// The worktree's tries fresh from its file: the overview's copy may be old.
    fn tries(&self) -> (Vec<Action>, Vec<Action>) {
        match &self.dir {
            Some(dir) => (actions_of(&actions_file(dir)), answered_of(&actions_file(dir))),
            None => (Vec::new(), Vec::new()),
        }
    }

    /// A draft marked ready, then read again until GitLab worked out whether it can merge.
    fn mark_ready(&self, fresh: &Mr) -> Result<(Mr, bool), String> {
        glab::write(
            &self.main,
            "PUT",
            &mr_endpoint(self.iid),
            &[("title", ready_title(&fresh.title))],
        )
        .map_err(|error| reason_of(&error))?;
        let marked = |why: String| format!("marked ready, can't merge: {why}");
        let mut ready = self.read_mr().map_err(marked)?;
        for _ in 1..SETTLE_TRIES {
            let is_checking = ready
                .0
                .merge_status
                .as_deref()
                .is_some_and(|status| CHECKING.contains(&status));
            if !ready.0.is_draft && !is_checking {
                break;
            }
            std::thread::sleep(SETTLE);
            ready = self.read_mr().map_err(marked)?;
        }
        Ok(ready)
    }

    fn put_merge(&self, fresh: &Mr, squash: bool) -> Result<(), String> {
        glab::write(
            &self.main,
            "PUT",
            &merge_endpoint(self.iid),
            &merge_fields(squash, fresh),
        )
        .map(|_| ())
    }

    /// After the merge (`carryOn`): its posted merge done, its agent told. The
    /// other open MRs are not told to merge the target in (#392): a branch
    /// that conflicts gets that line from its own conflict (`conflicts_of`),
    /// the others keep their green pipeline.
    /// What was typed: (feature, in words, line, got in).
    fn carry_on(&self, _fresh: &Mr) -> Vec<(String, String, String, bool)> {
        let mut told = Vec::new();
        if let Some((file, id)) = &self.action {
            let _ = done_action(file, *id);
        }
        let mut tell = |dir: &Path, about: String, line: String| {
            let is_in = herdr::deliver_to(&self.main, dir, &line);
            if !is_in {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).push((
                    dir.to_path_buf(),
                    about.clone(),
                    line.clone(),
                ));
            }
            told.push((breadboard_core::git::feature_of(&self.main, dir), about, line, is_in));
        };
        if let Some(dir) = &self.dir {
            tell(
                dir,
                format!("that !{} is merged", self.iid),
                format!("[overview] !{} merged", self.iid),
            );
        }
        // The merge moved the target: GitLab resets every open MR's mergeability and
        // recomputes it only when asked, so a real conflict shows within a tick (#391, #392).
        let _ = glab::get(&self.main, RECHECK_OPEN);
        told
    }
}

/// Two reads at once (Node's `Promise.all`): `first` on a thread of its
/// own, `second` on this one; a read whose thread died says so.
fn both<A: Send, B>(
    first: impl FnOnce() -> Result<A, String> + Send,
    second: impl FnOnce() -> B,
) -> (Result<A, String>, B) {
    std::thread::scope(|scope| {
        let first = scope.spawn(first);
        let second = second();
        (
            first.join().unwrap_or_else(|_| Err("its issue not read".into())),
            second,
        )
    })
}

/// The fresh reads before a merge, or why it can't merge: a read that
/// failed ends it as a blocker does, `can't merge: #305 not read: …`.
fn fresh_of(
    boxes: Result<Vec<IssueBox>, String>,
    first: Result<(Mr, bool), String>,
) -> Result<(Vec<IssueBox>, Mr, bool), String> {
    let cant = |why: String| format!("can't merge: {why}");
    let boxes = boxes.map_err(cant)?;
    let (fresh, squash) = first.map_err(cant)?;
    Ok((boxes, fresh, squash))
}

fn merge_endpoint(iid: i64) -> String {
    format!("{}/merge", mr_endpoint(iid))
}

/// The fields of the merge: the source branch deleted, the MR's squash, at
/// the commit whose pipeline passed. Into a batch never squashed (#411):
/// each task keeps its commits under its own merge commit.
fn merge_fields(squash: bool, fresh: &Mr) -> [(&'static str, String); 3] {
    let squash = squash && !breadboard_core::batch::is_batch(fresh.target());
    [
        ("should_remove_source_branch", "true".into()),
        ("squash", squash.to_string()),
        ("sha", fresh.sha.clone().unwrap_or_default()),
    ]
}

/// The merge's call as a command line, for the log.
fn merge_call(iid: i64, squash: bool, fresh: &Mr) -> String {
    let fields: Vec<String> = merge_fields(squash, fresh)
        .iter()
        .map(|(key, value)| format!(" --field {key}={value}"))
        .collect();
    format!("glab api --method PUT {}{}", merge_endpoint(iid), fields.concat())
}

/// On a local read (`retryMergeLines`): the lines after a merge that didn't
/// get in, tried again; one for a worktree that is gone goes.
pub fn retry_lines(actor: &Actor, dirs: Vec<PathBuf>) {
    let pending = actor.merges.pending.clone();
    if pending.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
        return;
    }
    let main = actor.main.clone();
    actor.queue(move || {
        let lines = std::mem::take(&mut *pending.lock().unwrap_or_else(|e| e.into_inner()));
        let mut typed = Vec::new();
        let mut left = Vec::new();
        for (dir, about, line) in lines {
            if !dirs.contains(&dir) {
                continue;
            }
            if herdr::deliver_to(&main, &dir, &line) {
                typed.push((breadboard_core::git::feature_of(&main, &dir), about, line));
            } else {
                left.push((dir, about, line));
            }
        }
        pending.lock().unwrap_or_else(|e| e.into_inner()).extend(left);
        (!typed.is_empty()).then(|| {
            Box::new(move |app: &mut Overview| {
                for (who, about, line) in typed {
                    app.actor.told(&who, &about, &line, true);
                }
            }) as super::Effect
        })
    });
}

/// What a merge row says while its merge is under way or done (`mergeText`),
/// in place of its text; None: the row as the model has it.
pub fn said(actor: &Actor, offer: &MergeOffer, text: &str) -> Option<Vec<Span>> {
    let state = actor.merges.of.get(&offer.iid)?;
    let verb = if offer.is_draft {
        "mark ready and merge"
    } else {
        "merge"
    };
    let mut spans = vec![
        Span::link(format!("!{}", offer.iid), Some(&offer.url)),
        Span::plain(" "),
    ];
    match state.phase {
        Phase::Merged => spans.push(Span::new("merged", Paint::Green)),
        Phase::Running => {
            spans.push(Span::plain(format!(
                "{}merging …",
                if offer.is_draft { "marking ready, " } else { "" }
            )));
            if let Some(error) = &state.error {
                spans.push(Span::dim(format!(" {error}")));
            }
        }
        Phase::Armed => spans.push(Span::new(format!("click again to {verb} !{}", offer.iid), Paint::Red)),
        Phase::Failed => {
            spans.push(Span::plain(format!("{text} ")));
            spans.push(Span::new(state.error.clone().unwrap_or_default(), Paint::Red));
        }
    }
    Some(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn says_why_gitlab_refused_in_words() {
        assert_eq!(
            reason_of("glab: 405 {\"message\":\"Method Not Allowed\"}"),
            "GitLab is still checking the MR (it refused the merge for now)"
        );
        assert_eq!(
            reason_of("glab: 403 Forbidden"),
            "GitLab refuses: no right to merge it as you"
        );
        assert_eq!(reason_of("x\nglab: 404 not found"), "GitLab doesn't find the MR");
        assert_eq!(reason_of("first\nboom (HTTP 500)"), "boom");
        assert_eq!(reason_of("HTTP 500"), "GitLab refused");
        // A read names what it read: an issue's 404 is not the MR's.
        assert_eq!(read_reason("glab: 404 Not Found"), "GitLab doesn't find it");
        assert_eq!(read_reason("glab: 403 Forbidden"), "GitLab refuses to show it to you");
        assert_eq!(read_reason("connection refused (HTTP 502)"), "connection refused");
    }

    #[test]
    fn a_merge_into_a_batch_is_never_squashed() {
        let mr = |target: &str| mr_of_rest(&json!({ "iid": 4, "sha": "abc", "target_branch": target }));
        let squash = |mr: &Mr| merge_fields(true, mr)[1].1.clone();
        assert_eq!(squash(&mr("batch/breadboard-1")), "false");
        assert_eq!(squash(&mr("develop")), "true", "elsewhere the MR's setting");
        assert_eq!(merge_fields(false, &mr("develop"))[1].1, "false");
    }

    #[test]
    fn reads_a_rest_mr_as_the_blockers_do() {
        let mr = mr_of_rest(&json!({
            "iid": 316, "title": "Draft: breadboard: x", "draft": true, "state": "opened",
            "has_conflicts": false, "detailed_merge_status": "draft_status", "merge_status": "can_be_merged",
            "sha": "abc", "web_url": "u", "head_pipeline": { "id": 9, "status": "success" }
        }));
        assert_eq!(mr.title, "breadboard: x");
        assert!(mr.is_draft);
        assert_eq!(mr.pipeline_status(), Some("success"));
        assert_eq!(mr.merge_status.as_deref(), Some("draft_status"));
        assert_eq!(
            blockers_of(Some(&mr), &[], false, &[], &[], true),
            ["draft: its agent posts the merge"]
        );
        assert!(blockers_of(Some(&mr), &[], true, &[], &[], true).is_empty());
    }

    #[test]
    fn reads_the_mr_and_its_issue_at_once() {
        use std::sync::mpsc::channel;
        // Each read waits for the other to have started: one after the other, neither sees the other.
        let (to_second, at_second) = channel();
        let (to_first, at_first) = channel();
        let wait = Duration::from_secs(5);
        let (first, second) = both(
            move || {
                to_second.send(()).unwrap();
                Ok(at_first.recv_timeout(wait).is_ok())
            },
            move || {
                to_first.send(()).unwrap();
                at_second.recv_timeout(wait).is_ok()
            },
        );
        assert_eq!((first, second), (Ok(true), true));
    }

    #[test]
    fn a_read_that_fails_ends_the_merge_with_cant_merge() {
        let mr = || Ok((Mr::default(), false));
        assert_eq!(
            fresh_of(Err("#305 not read: GitLab doesn't find it".into()), mr()).unwrap_err(),
            "can't merge: #305 not read: GitLab doesn't find it"
        );
        assert_eq!(
            fresh_of(Ok(Vec::new()), Err("!316 not read: GitLab did not answer".into())).unwrap_err(),
            "can't merge: !316 not read: GitLab did not answer"
        );
        assert!(fresh_of(Ok(Vec::new()), mr()).is_ok());
    }

    #[test]
    fn says_where_a_merge_stands_on_its_row() {
        let mut actor = Actor::new(
            &breadboard_core::reader::View::Everything,
            Path::new("/r"),
            false,
            false,
        );
        let offer = MergeOffer {
            iid: 7,
            url: "u".into(),
            ..Default::default()
        };
        assert_eq!(said(&actor, &offer, "ready"), None);
        actor.merges.of.insert(
            7,
            Merge {
                phase: Phase::Armed,
                error: None,
                seq: 1,
            },
        );
        let text = breadboard_core::model::text_of(&said(&actor, &offer, "ready").unwrap());
        assert_eq!(text, "!7 click again to merge !7");
        actor.merges.of.get_mut(&7).unwrap().phase = Phase::Failed;
        actor.merges.of.get_mut(&7).unwrap().error = Some("GitLab refused".into());
        let text = breadboard_core::model::text_of(&said(&actor, &offer, "ready").unwrap());
        assert_eq!(text, "!7 ready GitLab refused");
    }
}
