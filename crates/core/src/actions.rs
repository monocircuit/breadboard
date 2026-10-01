//! What an agent asks the human to do (`pnpm wt:action`), in
//! `.claude/actions.json` of its worktree, the orchestrator's own in the
//! record of the orchestration (actions.mjs). Both hold them under `actions`:
//! `[{ id, kind, text, url, run, on, options, descriptions, recommend, pane, mr, at }]`,
//! an answered one with `answer` and `answeredAt` until it is delivered, then
//! in `answered`. Every change runs under the `<file>.lock` directory: the
//! overview, the Stop hook and wt:action write it from processes of their own.

use crate::json::{self, int_of, str_of, strs_of};
use crate::lock::with_dir_lock;
use crate::time::{now_ms, parse_iso, to_iso};
use serde_json::{Map, Value, json};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The kinds of action (actions.mjs `ACTION_KINDS`).
pub const ACTION_KINDS: [&str; 5] = ["merge", "try", "decide", "look", "plan"];
/// How long a `<file>.lock` holds before another takes it over.
const LOCK_STALE: Duration = Duration::from_millis(5000);
/// How many delivered answers `answered` keeps, the newest.
const ANSWERED_KEPT: usize = 50;
/// How long a claim on an answer holds: a deliverer that died frees it after this.
const CLAIM_MS: i64 = 30_000;

/// An action as stored, its fields typed; `raw` the stored object.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Action {
    pub id: i64,
    pub kind: String,
    pub text: String,
    pub url: Option<String>,
    pub run: Option<String>,
    /// `test` for a try on the test instance.
    pub on: Option<String>,
    /// A decide's labels, what a click answers.
    pub options: Vec<String>,
    /// What each option does (#273).
    pub descriptions: Vec<String>,
    pub recommend: Option<String>,
    /// `$HERDR_PANE_ID` of whoever asked.
    pub pane: Option<String>,
    /// The MR of the orchestrator's merge (`--mr !N`).
    pub mr: Option<i64>,
    pub at: String,
    pub answer: Option<String>,
    pub answered_at: Option<String>,
    pub delivering_at: Option<String>,
    pub delivered_at: Option<String>,
    /// Why the draft of its answer waits (draftWaits).
    pub waits: Option<String>,
    pub raw: Value,
}

impl Action {
    pub fn from_value(value: &Value) -> Option<Action> {
        if !value.is_object() {
            return None;
        }
        Some(Action {
            id: int_of(value, "id").unwrap_or(0),
            kind: str_of(value, "kind").unwrap_or_default().to_string(),
            text: value.get("text").map(text_of).unwrap_or_default(),
            url: str_of(value, "url").map(String::from),
            run: str_of(value, "run").map(String::from),
            on: str_of(value, "on").map(String::from),
            options: strs_of(value, "options"),
            descriptions: strs_of(value, "descriptions"),
            recommend: value.get("recommend").filter(|v| !v.is_null()).map(text_of),
            pane: str_of(value, "pane").map(String::from),
            mr: int_of(value, "mr"),
            at: str_of(value, "at").unwrap_or_default().to_string(),
            answer: str_of(value, "answer").map(String::from),
            answered_at: str_of(value, "answeredAt").map(String::from),
            delivering_at: str_of(value, "deliveringAt").map(String::from),
            delivered_at: str_of(value, "deliveredAt").map(String::from),
            waits: str_of(value, "waits").map(String::from),
            raw: value.clone(),
        })
    }

    /// Whether it is a try (`browser-check` is one, stored before `try` came).
    pub fn is_try(&self) -> bool {
        self.kind == "try" || self.kind == "browser-check"
    }
}

/// A value as text, the way a template string shows it.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The actions of a file's value under `key`.
fn list_in(value: &Value, key: &str) -> Vec<Action> {
    value.get(key).and_then(Value::as_array).map(|list| list.iter().filter_map(Action::from_value).collect()).unwrap_or_default()
}

/// The open actions of a record's or actions file's value.
pub fn actions_in(value: &Value) -> Vec<Action> {
    list_in(value, "actions")
}

/// Where a worktree keeps its actions.
pub fn actions_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("actions.json")
}

/// The open actions in `file`, oldest first; none without a file.
pub fn actions_of(file: &Path) -> Vec<Action> {
    actions_in(&Value::Object(json::read_object(file)))
}

/// The answered actions of `file` once delivered, oldest first.
pub fn answered_of(file: &Path) -> Vec<Action> {
    list_in(&Value::Object(json::read_object(file)), "answered")
}

/// A change requested for a worktree's MR (`wt:action change`), in the same
/// file: `{ text, at, mr, head }`, `head` origin's commit when it was asked for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Change {
    pub text: String,
    pub at: Option<String>,
    pub mr: Option<i64>,
    pub head: Option<String>,
}

pub fn change_in(value: &Value) -> Option<Change> {
    let change = value.get("change").filter(|change| change.is_object())?;
    Some(Change {
        text: change.get("text").map(text_of).unwrap_or_default(),
        at: str_of(change, "at").map(String::from),
        mr: int_of(change, "mr"),
        head: str_of(change, "head").map(String::from),
    })
}

/// The change requested in `file`, None when none is open.
pub fn change_of(file: &Path) -> Option<Change> {
    change_in(&Value::Object(json::read_object(file)))
}

/// The answers the overview offers per kind; a decide offers its options.
/// None (a merge, a decide without options): only ✓, which ticks it off.
pub fn answers_of(action: &Action) -> Vec<String> {
    let fixed: &[&str] = match action.kind.as_str() {
        "decide" => return action.options.clone(),
        "try" | "browser-check" => &["looks good", "broken"],
        "look" => &["done"],
        "plan" => &["ok", "different"],
        _ => &[],
    };
    fixed.iter().map(|answer| answer.to_string()).collect()
}

/// Whether the answer of `action` is completed by the user before it is sent:
/// a try's `broken`, a plan's `different`.
pub fn is_draft(action: &Action) -> bool {
    let draft = match action.kind.as_str() {
        "try" | "browser-check" => "broken",
        "plan" => "different",
        _ => return false,
    };
    action.answer.as_deref() == Some(draft)
}

/// An option of a decide as shown (#273): `label` the answer, short,
/// `description` what happens; given apart, or split at the first `: `.
#[derive(Debug, Clone, PartialEq)]
pub struct DecideOption {
    /// The stored option, what a click answers.
    pub answer: String,
    pub label: String,
    pub description: String,
}

/// `(label, description)` of an option and its `--why` (actions.mjs `splitOption`).
pub fn split_option(option: &str, description: &str) -> (String, String) {
    if !description.is_empty() {
        return (option.trim().to_string(), description.to_string());
    }
    match option.find(": ") {
        Some(at) if at > 0 => (option[..at].trim().to_string(), option[at + 2..].trim().to_string()),
        _ => (option.trim().to_string(), String::new()),
    }
}

/// The options of a decide as shown.
pub fn options_of(action: &Action) -> Vec<DecideOption> {
    action
        .options
        .iter()
        .enumerate()
        .map(|(i, answer)| {
            let (label, description) = split_option(answer, action.descriptions.get(i).map(String::as_str).unwrap_or(""));
            DecideOption { answer: answer.clone(), label, description }
        })
        .collect()
}

/// Why a decide can't be answered from its row, None when it can
/// (actions.mjs `decideProblem`). `options`: `(label, description)`.
pub fn decide_problem(text: &str, options: &[(String, String)], recommend: Option<&str>) -> Option<String> {
    if !text.contains('?') {
        return Some(r#"a decide asks one question: what is decided, in one sentence, ending in "?""#.into());
    }
    if options.len() < 2 {
        return Some(r#"a decide offers at least two options: --option "<label>" --why "<what happens>""#.into());
    }
    let bare: Vec<String> = options
        .iter()
        .filter(|(label, description)| format!("{label} {description}").split_whitespace().count() < 2)
        .map(|(label, _)| format!("\"{label}\""))
        .collect();
    if !bare.is_empty() {
        return Some(format!(
            r#"each option says what happens, not just {}: --option "Go" --why "phases 2-4 build the painter""#,
            bare.join(", ")
        ));
    }
    let Some(recommend) = recommend.filter(|r| !r.is_empty()) else {
        return Some("a decide names its recommendation: --recommend <label or number>".into());
    };
    if !options.iter().any(|(label, _)| label == recommend) {
        let listed: Vec<String> =
            options.iter().enumerate().map(|(i, (label, _))| format!("{} \"{label}\"", i + 1)).collect();
        return Some(format!("--recommend {recommend} is none of the options: {}", listed.join(", ")));
    }
    None
}

/// How an answer reaches its agent: `[overview] #3 decide "A or B" → B`.
pub fn answer_line(action: &Action) -> String {
    format!("[overview] #{} {} \"{}\" → {}", action.id, action.kind, action.text, action.answer.as_deref().unwrap_or(""))
}

/// The draft the user completes: `… → broken: `.
pub fn draft_line(action: &Action) -> String {
    format!("{}: ", answer_line(action))
}

fn is_claimed(action: &Value, now: i64) -> bool {
    str_of(action, "deliveringAt").and_then(parse_iso).is_some_and(|at| now - at < CLAIM_MS)
}

/// What a change of the file does: the actions to write (None: none), the
/// fields to write next to them.
struct Changed<T> {
    actions: Option<Vec<Value>>,
    fields: Option<Map<String, Value>>,
    result: T,
}

/// `change(actions)` as one change of `file`, under its lock (actions.mjs `changeActions`).
fn change_actions<T>(file: &Path, change: impl FnOnce(Vec<Value>) -> Changed<T>) -> io::Result<T> {
    with_dir_lock(file, LOCK_STALE, || {
        let read = json::read_object(file);
        let actions = read.get("actions").and_then(Value::as_array).cloned().unwrap_or_default();
        let Changed { actions, fields, result } = change(actions);
        if let Some(actions) = actions {
            let mut object = json::read_object(file);
            object.insert("actions".into(), Value::Array(actions));
            json::write_pretty(file, &Value::Object(object))?;
        }
        if let Some(fields) = fields {
            let mut object = json::read_object(file);
            json::merge(&mut object, fields);
            json::write_pretty(file, &Value::Object(object))?;
        }
        Ok(result)
    })?
}

/// `delivered` as the field that appends them to `answered`.
fn answered_fields(file: &Path, delivered: &[Value]) -> Map<String, Value> {
    let delivered_at = to_iso(now_ms());
    let read = json::read_object(file);
    let mut answered: Vec<Value> = read.get("answered").and_then(Value::as_array).cloned().unwrap_or_default();
    for action in delivered {
        let mut action = action.as_object().cloned().unwrap_or_default();
        action.shift_remove("deliveringAt");
        action.insert("deliveredAt".into(), json!(delivered_at));
        answered.push(Value::Object(action));
    }
    let skip = answered.len().saturating_sub(ANSWERED_KEPT);
    let mut fields = Map::new();
    fields.insert("answered".into(), Value::Array(answered.into_iter().skip(skip).collect()));
    fields
}

/// A new action, as wt:action gives it.
#[derive(Debug, Clone, Default)]
pub struct NewAction {
    pub kind: String,
    pub text: String,
    pub url: Option<String>,
    pub run: Option<String>,
    pub on: Option<String>,
    pub options: Vec<String>,
    pub descriptions: Vec<String>,
    pub recommend: Option<String>,
    pub pane: Option<String>,
    pub mr: Option<i64>,
}

/// A new action in `file`, numbered after the highest there (the delivered
/// too). The same kind and text again replaces the older one; a merge ends
/// the requested change (actions.mjs `addAction`).
pub fn add_action(file: &Path, new: NewAction) -> io::Result<Action> {
    change_actions(file, |actions| {
        // After the delivered ones too: an answer in `answered` keeps its id.
        let answered = answered_of(file);
        let highest = actions.iter().filter_map(|action| int_of(action, "id")).chain(answered.iter().map(|a| a.id)).max();
        let mut action = Map::new();
        action.insert("id".into(), json!(highest.unwrap_or(0).max(0) + 1));
        action.insert("kind".into(), json!(new.kind));
        action.insert("text".into(), json!(new.text));
        action.insert("url".into(), json!(new.url));
        action.insert("run".into(), json!(new.run));
        if let Some(on) = &new.on {
            action.insert("on".into(), json!(on));
        }
        action.insert("options".into(), json!(new.options));
        if new.descriptions.iter().any(|description| !description.is_empty()) {
            action.insert("descriptions".into(), json!(new.descriptions));
        }
        action.insert("recommend".into(), json!(new.recommend));
        action.insert("pane".into(), json!(new.pane));
        if let Some(mr) = new.mr {
            action.insert("mr".into(), json!(mr));
        }
        action.insert("at".into(), json!(to_iso(now_ms())));
        let action = Value::Object(action);
        let mut kept: Vec<Value> = actions
            .into_iter()
            .filter(|other| str_of(other, "kind") != Some(new.kind.as_str()) || str_of(other, "text") != Some(new.text.as_str()))
            .collect();
        kept.push(action.clone());
        let fields = (new.kind == "merge").then(|| json!({ "change": null }).as_object().cloned().unwrap_or_default());
        Changed { actions: Some(kept), fields, result: Action::from_value(&action).unwrap_or_default() }
    })
}

/// The action `id` done: it goes, an answered one into `answered`. False when there is none.
pub fn done_action(file: &Path, id: i64) -> io::Result<bool> {
    change_actions(file, |actions| {
        let Some(at) = actions.iter().position(|action| int_of(action, "id") == Some(id)) else {
            return Changed { actions: None, fields: None, result: false };
        };
        let mut actions = actions;
        let action = actions.remove(at);
        let fields = json::is_truthy(action.get("answer")).then(|| answered_fields(file, &[action]));
        Changed { actions: Some(actions), fields, result: true }
    })
}

/// The action `id` answered, until it is delivered. False when there is none.
pub fn answer_action(file: &Path, id: i64, answer: &str) -> io::Result<bool> {
    change_actions(file, |actions| {
        if !actions.iter().any(|action| int_of(action, "id") == Some(id)) {
            return Changed { actions: None, fields: None, result: false };
        }
        let answered_at = to_iso(now_ms());
        let actions = actions
            .into_iter()
            .map(|mut action| {
                if int_of(&action, "id") == Some(id)
                    && let Some(object) = action.as_object_mut()
                {
                    object.insert("answer".into(), json!(answer));
                    object.insert("answeredAt".into(), json!(answered_at));
                }
                action
            })
            .collect();
        Changed { actions: Some(actions), fields: None, result: true }
    })
}

/// Sets or clears a field of the action `id`: a None takes it out, as
/// `{ ...action, field: undefined }` does.
fn set_field(file: &Path, id: i64, field: &str, value: Option<Value>) -> io::Result<()> {
    change_actions(file, |actions| {
        let actions = actions
            .into_iter()
            .map(|mut action| {
                if int_of(&action, "id") == Some(id)
                    && let Some(object) = action.as_object_mut()
                {
                    match &value {
                        Some(value) => {
                            object.insert(field.into(), value.clone());
                        }
                        None => {
                            object.shift_remove(field);
                        }
                    }
                }
                action
            })
            .collect();
        Changed { actions: Some(actions), fields: None, result: () }
    })
}

/// Why the draft of action `id` waits, shown on its row; None: it no longer does.
pub fn draft_waits(file: &Path, id: i64, waits: Option<&str>) -> io::Result<()> {
    set_field(file, id, "waits", waits.map(|waits| json!(waits)))
}

/// The answer of action `id` claimed for typing it into its agent's pane: no
/// one else takes it meanwhile. None when it has no answer or another holds it.
pub fn claim_answer(file: &Path, id: i64) -> io::Result<Option<Action>> {
    change_actions(file, |actions| {
        let now = now_ms();
        let found = actions.iter().find(|action| int_of(action, "id") == Some(id));
        if found.is_none_or(|action| !json::is_truthy(action.get("answer")) || is_claimed(action, now)) {
            return Changed { actions: None, fields: None, result: None };
        }
        let mut claimed = None;
        let actions = actions
            .into_iter()
            .map(|mut action| {
                if int_of(&action, "id") == Some(id)
                    && let Some(object) = action.as_object_mut()
                {
                    object.insert("deliveringAt".into(), json!(to_iso(now)));
                    claimed = Action::from_value(&action);
                }
                action
            })
            .collect();
        Changed { actions: Some(actions), fields: None, result: claimed }
    })
}

/// The claim on the answer of action `id` given back: it didn't get in.
pub fn release_answer(file: &Path, id: i64) -> io::Result<()> {
    set_field(file, id, "deliveringAt", None)
}

/// The answered actions of `file` taken into `answered`: whoever takes them
/// delivers them, so each arrives once. A claimed one stays, and a draft (the
/// overview writes it into the agent's input). `pane`: Some(p) takes only those
/// asked from `p` (None as p: those asked from no pane).
pub fn take_answers(file: &Path, pane: Option<Option<&str>>) -> io::Result<Vec<Action>> {
    change_actions(file, |actions| {
        let now = now_ms();
        let is_taken = |action: &Value| {
            let typed = Action::from_value(action).unwrap_or_default();
            typed.answer.as_deref().is_some_and(|answer| !answer.is_empty())
                && !is_draft(&typed)
                && !is_claimed(action, now)
                && pane.is_none_or(|pane| str_of(action, "pane") == pane)
        };
        let (taken, kept): (Vec<Value>, Vec<Value>) = actions.into_iter().partition(is_taken);
        if taken.is_empty() {
            return Changed { actions: None, fields: None, result: Vec::new() };
        }
        let fields = answered_fields(file, &taken);
        Changed { actions: Some(kept), fields: Some(fields), result: taken.iter().filter_map(Action::from_value).collect() }
    })
}

/// `change` requested in `file`: its merge actions withdrawn, returned.
pub fn request_change(file: &Path, change: Value) -> io::Result<Vec<Action>> {
    change_actions(file, |actions| {
        let (merges, kept): (Vec<Value>, Vec<Value>) =
            actions.into_iter().partition(|action| str_of(action, "kind") == Some("merge"));
        let mut fields = Map::new();
        fields.insert("change".into(), change);
        Changed { actions: Some(kept), fields: Some(fields), result: merges.iter().filter_map(Action::from_value).collect() }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::scratch::Scratch;
    use std::fs;

    fn file_of(scratch: &Scratch) -> PathBuf {
        actions_file(scratch.path())
    }

    fn add(file: &Path, kind: &str, text: &str) -> Action {
        add_action(file, NewAction { kind: kind.into(), text: text.into(), ..Default::default() }).unwrap()
    }

    #[test]
    fn has_none_without_a_file() {
        let scratch = Scratch::new();
        assert!(actions_of(&file_of(&scratch)).is_empty());
    }

    #[test]
    fn keeps_the_mr_of_the_orchestrators_merge() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add_action(&file, NewAction { kind: "merge".into(), text: "#242 rows".into(), mr: Some(256), ..Default::default() })
            .unwrap();
        add(&file, "look", "the logo");
        assert_eq!(actions_of(&file).iter().map(|a| a.mr).collect::<Vec<_>>(), vec![Some(256), None]);
    }

    #[test]
    fn numbers_new_actions_after_the_highest_and_writes_them_as_node() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add(&file, "merge", "!1 ready");
        let second = add_action(
            &file,
            NewAction {
                kind: "browser-check".into(),
                text: "pinch-zoom".into(),
                url: Some("http://localhost:3001".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(second.id, 2);
        let raw = json::read(&file).unwrap();
        let keys: Vec<&String> = raw["actions"][1].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["id", "kind", "text", "url", "run", "options", "recommend", "pane", "at"]);
        assert!(fs::read_to_string(&file).unwrap().ends_with("}\n"));
    }

    #[test]
    fn replaces_the_same_kind_and_text_announced_again() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add(&file, "decide", "A or B");
        add(&file, "decide", "A or B");
        assert_eq!(actions_of(&file).iter().map(|a| a.id).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn drops_an_action_that_is_done() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add(&file, "look", "the log");
        assert!(done_action(&file, 1).unwrap());
        assert!(!done_action(&file, 1).unwrap());
        assert!(actions_of(&file).is_empty());
    }

    #[test]
    fn keeps_what_else_the_file_holds() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, r#"{"issues":[{"number":1}]}"#).unwrap();
        add(&file, "decide", "merge order");
        let raw = json::read(&file).unwrap();
        assert_eq!(raw["issues"], json!([{ "number": 1 }]));
        assert_eq!(raw["actions"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn offers_the_options_of_a_decision() {
        let decide = Action { kind: "decide".into(), options: vec!["A".into(), "B".into()], ..Default::default() };
        assert_eq!(answers_of(&decide), vec!["A", "B"]);
        assert!(answers_of(&Action { kind: "decide".into(), ..Default::default() }).is_empty());
        assert_eq!(answers_of(&Action { kind: "plan".into(), ..Default::default() }), vec!["ok", "different"]);
        assert!(answers_of(&Action { kind: "merge".into(), ..Default::default() }).is_empty());
    }

    #[test]
    fn hands_an_answer_over_once_as_one_line() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add_action(
            &file,
            NewAction { kind: "decide".into(), text: "A or B".into(), options: vec!["A".into(), "B".into()], ..Default::default() },
        )
        .unwrap();
        add(&file, "look", "the log");
        assert!(take_answers(&file, None).unwrap().is_empty());
        assert!(answer_action(&file, 1, "B").unwrap());
        let taken = take_answers(&file, None).unwrap();
        assert_eq!(answer_line(&taken[0]), "[overview] #1 decide \"A or B\" → B");
        assert!(take_answers(&file, None).unwrap().is_empty());
        assert_eq!(actions_of(&file).iter().map(|a| a.id).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn leaves_a_broken_or_different_as_an_unsent_draft() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add(&file, "try", "the chip");
        add(&file, "plan", "the plan");
        add_action(&file, NewAction { kind: "decide".into(), text: "A or B".into(), options: vec!["broken".into()], ..Default::default() })
            .unwrap();
        answer_action(&file, 1, "broken").unwrap();
        answer_action(&file, 2, "different").unwrap();
        answer_action(&file, 3, "broken").unwrap();
        let all = actions_of(&file);
        assert!(is_draft(&all[0]));
        assert!(is_draft(&all[1]));
        assert!(!is_draft(&all[2]));
        assert_eq!(draft_line(&all[0]), "[overview] #1 try \"the chip\" → broken: ");
        assert_eq!(take_answers(&file, None).unwrap().iter().map(|a| a.id).collect::<Vec<_>>(), vec![3]);
        draft_waits(&file, 1, Some("at a question")).unwrap();
        assert_eq!(actions_of(&file)[0].waits.as_deref(), Some("at a question"));
        draft_waits(&file, 1, None).unwrap();
        assert_eq!(actions_of(&file)[0].waits, None);
        assert!(json::read(&file).unwrap()["actions"][0].get("waits").is_none());
    }

    #[test]
    fn takes_only_the_answers_asked_from_a_pane() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add_action(&file, NewAction { kind: "plan".into(), text: "mine".into(), pane: Some("w1:p1".into()), ..Default::default() })
            .unwrap();
        add_action(&file, NewAction { kind: "plan".into(), text: "theirs".into(), pane: Some("w2:p1".into()), ..Default::default() })
            .unwrap();
        answer_action(&file, 1, "ok").unwrap();
        answer_action(&file, 2, "ok").unwrap();
        assert_eq!(take_answers(&file, Some(Some("w1:p1"))).unwrap().iter().map(|a| a.id).collect::<Vec<_>>(), vec![1]);
        assert_eq!(actions_of(&file).len(), 1);
    }

    #[test]
    fn gives_a_claimed_answer_to_no_one_else_until_it_is_released() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add_action(
            &file,
            NewAction { kind: "decide".into(), text: "A or B".into(), options: vec!["A".into(), "B".into()], ..Default::default() },
        )
        .unwrap();
        answer_action(&file, 1, "A").unwrap();
        assert_eq!(claim_answer(&file, 1).unwrap().unwrap().answer.as_deref(), Some("A"));
        assert!(claim_answer(&file, 1).unwrap().is_none());
        assert!(take_answers(&file, None).unwrap().is_empty());
        release_answer(&file, 1).unwrap();
        assert_eq!(actions_of(&file)[0].delivering_at, None);
        assert_eq!(take_answers(&file, None).unwrap().iter().map(|a| a.id).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn frees_the_claim_of_a_deliverer_that_died() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add(&file, "look", "the log");
        answer_action(&file, 1, "done").unwrap();
        let mut raw = json::read(&file).unwrap();
        raw["actions"][0]["deliveringAt"] = json!(to_iso(now_ms() - 60_000));
        fs::write(&file, raw.to_string()).unwrap();
        assert!(claim_answer(&file, 1).unwrap().is_some());
    }

    #[test]
    fn keeps_a_delivered_answer_readable() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add_action(
            &file,
            NewAction { kind: "decide".into(), text: "A or B".into(), options: vec!["A".into(), "B".into()], ..Default::default() },
        )
        .unwrap();
        add(&file, "plan", "the plan");
        add(&file, "look", "the log");
        answer_action(&file, 1, "B").unwrap();
        answer_action(&file, 2, "ok").unwrap();
        claim_answer(&file, 1).unwrap();
        done_action(&file, 1).unwrap();
        take_answers(&file, None).unwrap();
        done_action(&file, 3).unwrap();
        assert!(actions_of(&file).is_empty());
        let answered = answered_of(&file);
        assert_eq!(answered.iter().map(|a| (a.id, a.answer.clone().unwrap())).collect::<Vec<_>>(), vec![
            (1, "B".to_string()),
            (2, "ok".to_string())
        ]);
        assert!(answered.iter().all(|a| a.delivered_at.is_some() && a.delivering_at.is_none() && a.answered_at.is_some()));
        assert_eq!(add(&file, "look", "more").id, 3);
    }

    #[test]
    fn a_merge_ends_the_requested_change_and_a_change_withdraws_the_merge() {
        let scratch = Scratch::new();
        let file = file_of(&scratch);
        add(&file, "merge", "ready");
        let withdrawn = request_change(&file, json!({ "text": "rename it", "at": "T", "mr": 3, "head": "abc" })).unwrap();
        assert_eq!(withdrawn.len(), 1);
        assert_eq!(change_of(&file).unwrap().text, "rename it");
        assert!(actions_of(&file).is_empty());
        add(&file, "merge", "ready again");
        assert_eq!(change_of(&file), None);
    }

    #[test]
    fn takes_a_question_options_saying_what_happens_and_a_recommendation() {
        let options: Vec<(String, String)> =
            ["keep both lines", "drop the old line"].iter().map(|label| split_option(label, "")).collect();
        assert_eq!(decide_problem("Which line stays? The old one is shorter.", &options, Some("drop the old line")), None);
        let short = vec![split_option("Go", "phases 2-4"), split_option("No-go: pixi stays", "")];
        assert_eq!(decide_problem("Skia?", &short, Some("Go")), None);
        assert!(decide_problem("A or B", &options, Some("keep both lines")).unwrap().contains("question"));
        assert!(decide_problem("Which?", &options[..1], None).unwrap().contains("two options"));
        let bare = vec![split_option("A", ""), split_option("B", "")];
        assert!(decide_problem("Which?", &bare, Some("A")).unwrap().contains("\"A\", \"B\""));
        assert!(decide_problem("Which?", &options, None).unwrap().contains("--recommend"));
        assert!(decide_problem("Which?", &options, Some("C")).unwrap().contains("none of the options"));
    }

    #[test]
    fn shows_a_stored_option_split_its_answer_the_whole_text() {
        let action = Action {
            options: vec!["Go".into(), "No-go: pixi stays".into()],
            descriptions: vec!["phases 2-4".into(), String::new()],
            ..Default::default()
        };
        assert_eq!(options_of(&action), vec![
            DecideOption { answer: "Go".into(), label: "Go".into(), description: "phases 2-4".into() },
            DecideOption { answer: "No-go: pixi stays".into(), label: "No-go".into(), description: "pixi stays".into() },
        ]);
    }
}
