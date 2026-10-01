//! Merging by two clicks (merge.mjs, #182, #242, #280): while anything blocks,
//! a click opens the MR; the first click arms the merge for 5 s, the second
//! reads the MR, its issue's boxes and its worktree's tries again, marks a
//! draft ready when it may, and merges as the user with the MR's squash
//! setting, the source branch deleted, at the commit whose pipeline passed.
//! Then the order carries on: its agent hears it, every open MR touching
//! the same files is told to merge develop in. With the redraft of a try
//! answered `broken` (actions.rs) these three PUTs are the overview's only
//! GitLab writes (rule 6).

use super::glab::{self, mr_endpoint, ready_title};
use super::{Actor, Poster, browser, herdr};
use crate::tui::overview::Overview;
use breadboard_core::actions::{Action, actions_file, actions_of, answered_of, done_action};
use breadboard_core::git::{git, work_files};
use breadboard_core::gitlab::{Mr, Pipeline, boxes_of};
use breadboard_core::model::merge::blockers_of;
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
/// How many shared files a carry-on line names.
const NAMED_FILES: usize = 3;
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
    pending: Arc<Mutex<Vec<(PathBuf, String)>>>,
    seq: u64,
}

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
    app.actor.log(Paint::Yellow, format!("{verb} !{iid} …"));
    let others: Vec<PathBuf> = offer
        .others
        .iter()
        .filter_map(|number| app.tree_dir_of_number(*number))
        .collect();
    let run = Running {
        main: app.actor.main.clone(),
        iid,
        number: breadboard_core::gitlab::number_of(branch),
        dir,
        can_mark_ready: offer.can_mark_ready || action.is_some(),
        action: action.clone(),
        others,
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
    others: Vec<PathBuf>,
    pending: Arc<Mutex<Vec<(PathBuf, String)>>>,
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
        self.poster.post(move |app: &mut Overview| {
            app.actor.merges.of.insert(
                iid,
                Merge {
                    phase,
                    error: error.clone(),
                    seq: 0,
                },
            );
            match &error {
                None => app.actor.log(Paint::Green, format!("merged !{iid}")),
                Some(why) => app.actor.log(Paint::Red, format!("!{iid} not merged: {why}")),
            }
            // GitLab now, not in half a minute: NEXT moves on, the clean-up shows.
            app.refresh_gitlab();
        });
        if let Ok(fresh) = result {
            let told = self.carry_on(&fresh);
            self.poster.post(move |app: &mut Overview| {
                for (who, line, is_in) in told {
                    if is_in {
                        app.actor
                            .log(Paint::Green, format!("typed into the agent of {who}: {line}"));
                    } else {
                        app.actor
                            .log(Paint::Yellow, format!("the agent of {who} gets once it can: {line}"));
                    }
                }
            });
        }
    }

    /// The MR fresh from GitLab, with its squash setting.
    fn read_mr(&self) -> Result<(Mr, bool), String> {
        glab::get(&self.main, &mr_endpoint(self.iid)).map(|value| {
            let squash = value.get("squash").and_then(Value::as_bool).unwrap_or(false);
            (mr_of_rest(&value), squash)
        })
    }

    /// Steps 1–5 of `runMerge`: Ok with the MR as merged at, Err why not.
    fn merge(&self) -> Result<Mr, String> {
        let boxes = match self.number {
            Some(number) => glab::get(&self.main, &format!("projects/:fullpath/issues/{number}"))
                .map(|issue| boxes_of(issue.get("description").and_then(Value::as_str).unwrap_or_default()))
                .map_err(|error| reason_of(&error))?,
            None => Vec::new(),
        };
        let (mut fresh, mut squash) = self.read_mr().map_err(|error| reason_of(&error))?;
        let (actions, answered) = self.tries();
        let blockers = blockers_of(Some(&fresh), &boxes, self.can_mark_ready, &actions, &answered);
        if !blockers.is_empty() {
            return Err(format!("can't merge: {}", blockers.join(" · ")));
        }
        if fresh.is_draft {
            (fresh, squash) = self.mark_ready(&fresh)?;
            let blockers = blockers_of(Some(&fresh), &boxes, false, &actions, &answered);
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
            (fresh, squash) = self.read_mr().map_err(|error| reason_of(&error))?;
            if fresh.merge_status.as_deref() != Some("mergeable") {
                let blockers = blockers_of(Some(&fresh), &boxes, false, &actions, &answered);
                let why = if blockers.is_empty() {
                    reason_of(&error)
                } else {
                    blockers.join(" · ")
                };
                return Err(format!("can't merge: {why}"));
            }
            self.put_merge(&fresh, squash).map_err(|error| reason_of(&error))?;
        }
        Ok(fresh)
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
        let mut ready = self.read_mr().map_err(|error| reason_of(&error))?;
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
            ready = self.read_mr().map_err(|error| reason_of(&error))?;
        }
        Ok(ready)
    }

    fn put_merge(&self, fresh: &Mr, squash: bool) -> Result<(), String> {
        glab::write(
            &self.main,
            "PUT",
            &format!("{}/merge", mr_endpoint(self.iid)),
            &[
                ("should_remove_source_branch", "true".into()),
                ("squash", squash.to_string()),
                ("sha", fresh.sha.clone().unwrap_or_default()),
            ],
        )
        .map(|_| ())
    }

    /// After the merge (`carryOn`): its posted merge done, its agent told, every
    /// other open MR whose branch lacks the merge and shares files with it too.
    /// What was typed: (feature, line, got in).
    fn carry_on(&self, fresh: &Mr) -> Vec<(String, String, bool)> {
        let mut told = Vec::new();
        if let Some((file, id)) = &self.action {
            let _ = done_action(file, *id);
        }
        let mut tell = |dir: &Path, line: String| {
            let is_in = herdr::deliver_to(&self.main, dir, &line);
            if !is_in {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((dir.to_path_buf(), line.clone()));
            }
            told.push((breadboard_core::git::feature_of(&self.main, dir), line, is_in));
        };
        if let Some(dir) = &self.dir {
            tell(dir, format!("[overview] !{} merged", self.iid));
        }
        let sha = fresh.sha.clone().unwrap_or_default();
        // A branch built on the merged one has its commits already.
        let dirs: Vec<&PathBuf> = self
            .others
            .iter()
            .filter(|dir| git(dir, &["merge-base", "--is-ancestor", &sha, "HEAD"]).status != 0)
            .collect();
        if dirs.is_empty() {
            return told;
        }
        let changes = glab::get(&self.main, &format!("{}/diffs?per_page=100", mr_endpoint(self.iid)))
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        let merged: Vec<String> = changes
            .iter()
            .flat_map(|change| {
                ["old_path", "new_path"]
                    .into_iter()
                    .filter_map(|key| change.get(key).and_then(Value::as_str).map(String::from))
            })
            .collect();
        for dir in dirs {
            let shared = shared_files(&merged, &work_files(dir));
            if !shared.is_empty() {
                tell(
                    dir,
                    format!(
                        "[overview] !{} is merged and touched your files too ({}): merge develop in (git fetch origin, git merge origin/develop)",
                        self.iid,
                        names_of(&shared)
                    ),
                );
            }
        }
        told
    }
}

/// The files of `mine` the merged MR touched too, sorted.
pub fn shared_files(merged: &[String], mine: &std::collections::BTreeSet<String>) -> Vec<String> {
    let mut shared: Vec<String> = merged.iter().filter(|file| mine.contains(*file)).cloned().collect();
    shared.sort();
    shared.dedup();
    shared
}

/// `a, b, c +2`: the first few files, how many more.
pub fn names_of(files: &[String]) -> String {
    let named = files.iter().take(NAMED_FILES).cloned().collect::<Vec<_>>().join(", ");
    if files.len() > NAMED_FILES {
        format!("{named} +{}", files.len() - NAMED_FILES)
    } else {
        named
    }
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
        for (dir, line) in lines {
            if !dirs.contains(&dir) {
                continue;
            }
            if herdr::deliver_to(&main, &dir, &line) {
                typed.push((breadboard_core::git::feature_of(&main, &dir), line));
            } else {
                left.push((dir, line));
            }
        }
        pending.lock().unwrap_or_else(|e| e.into_inner()).extend(left);
        (!typed.is_empty()).then(|| {
            Box::new(move |app: &mut Overview| {
                for (who, line) in typed {
                    app.actor
                        .log(Paint::Green, format!("typed into the agent of {who}: {line}"));
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
            blockers_of(Some(&mr), &[], false, &[], &[]),
            ["draft: its agent posts the merge"]
        );
        assert!(blockers_of(Some(&mr), &[], true, &[], &[]).is_empty());
    }

    #[test]
    fn names_the_files_two_merges_share() {
        let mine = ["b.rs", "a.rs", "c.rs", "d.rs"].map(String::from).into_iter().collect();
        let merged: Vec<String> = ["d.rs", "a.rs", "x.rs", "b.rs", "c.rs", "a.rs"]
            .map(String::from)
            .into();
        let shared = shared_files(&merged, &mine);
        assert_eq!(shared, ["a.rs", "b.rs", "c.rs", "d.rs"]);
        assert_eq!(names_of(&shared), "a.rs, b.rs, c.rs +1");
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
