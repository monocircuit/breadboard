//! The record of an orchestration, `.claude/orchestrations/<name>.json` in the
//! main checkout (origins.mjs): its issues, the plan of `wt:plan`, its title,
//! the workers (#217), the announced handovers (#227), the orchestrator's own
//! actions and chat; and a worktree's `.claude/origin`, the orchestration
//! that started it. Read leniently: a field of the wrong type is left out,
//! never the whole record (#306). Written as values, so what Rust doesn't
//! know stays (rewrite rule 5).

use crate::actions::{Action, actions_in};
use crate::json::{self, int_of, ints_of, str_of, strs_of};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The mark of an orchestration: its space is `⇶ <title>`.
pub const ORCHESTRATOR: &str = "⇶";
/// The main herdr space of the repo.
pub const SPACE: &str = "Monocircuit/Platform";
/// One colour per concurrent orchestration (origins.mjs `ORIGIN_SLOTS`).
pub const SLOTS: u32 = 3;

/// A name of a feature or an orchestration: kebab-case (constants.mjs `NAME`).
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").unwrap());

pub fn is_name(text: &str) -> bool {
    NAME.is_match(text)
}

// ---------------------------------------------------------------------------
// .claude/origin
// ---------------------------------------------------------------------------

/// The orchestration that started a worktree and its colour slot: `{ "name", "slot" }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub name: String,
    pub slot: u32,
}

pub fn origin_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("origin")
}

/// The origin of a worktree, None without one or with a broken one; the slot
/// taken modulo the slots there are.
pub fn origin_of(dir: &Path) -> Option<Origin> {
    let value = json::read(&origin_file(dir))?;
    let name = str_of(&value, "name")?;
    let slot = value.get("slot")?.as_u64()?;
    is_name(name).then(|| Origin {
        name: name.to_string(),
        slot: (slot % u64::from(SLOTS)) as u32,
    })
}

/// Records who started a worktree, as `JSON.stringify(origin)` and a newline.
pub fn write_origin(dir: &Path, origin: &Origin) -> io::Result<()> {
    json::write_compact(&origin_file(dir), &json!({ "name": origin.name, "slot": origin.slot }))
}

/// The brief of an orchestrated agent, `.claude/brief.md` of its worktree.
pub fn brief_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("brief.md")
}

pub fn read_brief(dir: &Path) -> Option<String> {
    fs::read_to_string(brief_file(dir)).ok()
}

pub fn write_brief(dir: &Path, brief: &str) -> io::Result<()> {
    let file = brief_file(dir);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, brief)
}

// ---------------------------------------------------------------------------
// The record
// ---------------------------------------------------------------------------

/// Where the records are: `.claude/orchestrations/` of the main checkout.
pub fn records_dir(main: &Path) -> PathBuf {
    main.join(".claude").join("orchestrations")
}

pub fn record_file(main: &Path, name: &str) -> PathBuf {
    records_dir(main).join(format!("{name}.json"))
}

/// What the orchestration got done (#248), next to its record.
pub fn changelog_file(main: &Path, name: &str) -> PathBuf {
    records_dir(main).join(format!("{name}.changelog.md"))
}

/// Every orchestration with a record, by name.
pub fn recorded(main: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(records_dir(main)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| {
            entry
                .ok()?
                .file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(String::from)
        })
        .collect();
    names.sort();
    names
}

/// An issue as the record keeps it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssueEntry {
    pub number: Option<i64>,
    pub title: Option<String>,
    pub feature: Option<String>,
    pub branch: Option<String>,
    /// Why it got no worktree (skipped, failed), None for one that did.
    pub why: Option<String>,
    pub removed_at: Option<String>,
    pub merged_at: Option<String>,
    /// Waits for an agent (`--queue`, #217).
    pub is_queued: bool,
    pub queued_at: Option<String>,
    /// Why it is back in the queue: its start failed (#272).
    pub requeued: Option<String>,
    /// The MRs it starts after (`--queue --after`, #272).
    pub after: Vec<i64>,
}

impl IssueEntry {
    pub fn from_value(value: &Value) -> IssueEntry {
        IssueEntry {
            number: int_of(value, "number"),
            title: str_of(value, "title").map(String::from),
            feature: str_of(value, "feature").map(String::from),
            branch: str_of(value, "branch").map(String::from),
            why: str_of(value, "why").filter(|why| !why.is_empty()).map(String::from),
            removed_at: str_of(value, "removedAt").map(String::from),
            merged_at: str_of(value, "mergedAt").filter(|at| !at.is_empty()).map(String::from),
            is_queued: json::is_truthy(value.get("queued")),
            queued_at: str_of(value, "queuedAt").map(String::from),
            requeued: str_of(value, "requeued").map(String::from),
            after: ints_of(value, "after"),
        }
    }
}

/// The order the orchestrator set (`pnpm wt:plan`): issue numbers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub work: Vec<i64>,
    pub merge: Vec<i64>,
    pub why: Option<String>,
}

/// An issue an agent started on (history.mjs): `how` it got it, `context`
/// the share of its context used then.
#[derive(Debug, Clone, PartialEq)]
pub struct Start {
    pub number: i64,
    pub how: String,
    pub at: Option<String>,
    pub context: Option<f64>,
}

/// A handover to an agent that failed (history.mjs `withFailure`).
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub number: i64,
    pub reason: String,
    pub at: Option<String>,
    pub times: i64,
}

/// A free agent's whence (#289): since when, the worktree it came from, its merged MR.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Freed {
    pub at: Option<String>,
    pub from: Option<String>,
    pub mr: Option<i64>,
}

/// An agent of the orchestration (#217), by its conversation's id.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Worker {
    pub id: String,
    pub name: Option<String>,
    /// The person's name it is shown by (#292): `Mira Albrecht`.
    pub display_name: Option<String>,
    pub issues: Vec<i64>,
    pub files: Vec<String>,
    /// Its name before the last handover (#251), and when it was renamed.
    pub was: Option<String>,
    pub renamed: Option<String>,
    /// None for one recorded before the history: read from `issues` (historyOf).
    pub history: Option<Vec<Start>>,
    pub failed: Vec<Failure>,
    pub free: Option<Freed>,
}

impl Worker {
    pub fn from_value(id: &str, value: &Value) -> Worker {
        let history = value.get("history").and_then(Value::as_array).map(|starts| {
            starts
                .iter()
                .filter_map(|start| {
                    Some(Start {
                        number: int_of(start, "number")?,
                        how: str_of(start, "how").unwrap_or("fresh").to_string(),
                        at: str_of(start, "at").map(String::from),
                        context: start.get("context").and_then(Value::as_f64),
                    })
                })
                .collect()
        });
        let failed = value
            .get("failed")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|failure| {
                        Some(Failure {
                            number: int_of(failure, "number")?,
                            reason: str_of(failure, "reason").unwrap_or_default().to_string(),
                            at: str_of(failure, "at").map(String::from),
                            times: int_of(failure, "times").unwrap_or(1),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let free = value.get("free").filter(|free| free.is_object()).map(|free| Freed {
            at: str_of(free, "at").map(String::from),
            from: str_of(free, "from").map(String::from),
            mr: int_of(free, "mr"),
        });
        Worker {
            id: id.to_string(),
            name: str_of(value, "name").map(String::from),
            display_name: str_of(value, "displayName").map(String::from),
            issues: ints_of(value, "issues"),
            files: strs_of(value, "files"),
            was: str_of(value, "was").map(String::from),
            renamed: str_of(value, "renamed").map(String::from),
            history,
            failed,
            free,
        }
    }
}

/// An announced handover (#227): the agent `to` takes `number` once the MRs `after` are merged.
#[derive(Debug, Clone, PartialEq)]
pub struct Handover {
    pub number: i64,
    pub to: String,
    pub after: Vec<i64>,
    pub at: Option<String>,
}

/// The orchestrator's conversation (#287): `orchestrator: { sessionId, cwd }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Chat {
    pub session_id: String,
    pub cwd: Option<String>,
}

/// The record of an orchestration, as read.
#[derive(Debug, Clone, Default)]
pub struct Record {
    /// The file as it was, for writes that keep what Rust doesn't know.
    pub raw: Value,
    pub issues: Vec<IssueEntry>,
    pub plan: Option<Plan>,
    pub title: Option<String>,
    pub workers: Vec<Worker>,
    pub handovers: Vec<Handover>,
    /// The orchestrator's own actions (`wt:action --orchestration`).
    pub actions: Vec<Action>,
    pub chat: Option<Chat>,
    pub brief: Option<String>,
    pub peer: Option<String>,
}

static SESSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f-]{36}$").unwrap());

/// Whether `id` looks like a claude conversation's id.
pub fn is_session_id(id: &str) -> bool {
    SESSION.is_match(id)
}

impl Record {
    /// A record from its file's value; None unless it lists `issues` (origins.mjs `recordOf`).
    pub fn from_value(raw: Value) -> Option<Record> {
        let issues = raw
            .get("issues")?
            .as_array()?
            .iter()
            .map(IssueEntry::from_value)
            .collect();
        let plan = raw.get("plan").filter(|plan| plan.is_object()).map(|plan| Plan {
            work: ints_of(plan, "work"),
            merge: ints_of(plan, "merge"),
            why: str_of(plan, "why").map(String::from),
        });
        let workers = raw
            .get("workers")
            .and_then(Value::as_object)
            .map(|workers| {
                workers
                    .iter()
                    .map(|(id, worker)| Worker::from_value(id, worker))
                    .collect()
            })
            .unwrap_or_default();
        let handovers = raw
            .get("handovers")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|handover| {
                        Some(Handover {
                            number: int_of(handover, "number")?,
                            to: str_of(handover, "to")?.to_string(),
                            after: ints_of(handover, "after"),
                            at: str_of(handover, "at").map(String::from),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let chat = raw.get("orchestrator").and_then(|chat| {
            let session_id = str_of(chat, "sessionId").filter(|id| is_session_id(id))?;
            Some(Chat {
                session_id: session_id.to_string(),
                cwd: str_of(chat, "cwd").filter(|cwd| !cwd.is_empty()).map(String::from),
            })
        });
        Some(Record {
            issues,
            plan,
            title: str_of(&raw, "title")
                .filter(|title| !title.is_empty())
                .map(String::from),
            workers,
            handovers,
            actions: actions_in(&raw),
            chat,
            brief: str_of(&raw, "brief").map(String::from),
            peer: str_of(&raw, "peer").map(String::from),
            raw,
        })
    }

    /// The worker of a conversation's id.
    pub fn worker(&self, id: &str) -> Option<&Worker> {
        self.workers.iter().find(|worker| worker.id == id)
    }

    /// The free agents (#289, free.mjs `freeAgentsOf`).
    pub fn free_agents(&self) -> Vec<&Worker> {
        self.workers.iter().filter(|worker| worker.free.is_some()).collect()
    }
}

/// The record of `name`, None without one (or one without `issues`).
pub fn record_of(main: &Path, name: &str) -> Option<Record> {
    Record::from_value(json::read(&record_file(main, name))?)
}

/// What an orchestration shows (#234): its record's title, else its name.
pub fn title_of(main: &Path, name: &str) -> String {
    record_of(main, name)
        .and_then(|record| record.title)
        .unwrap_or_else(|| name.to_string())
}

/// The label of an orchestration's space: `⇶ <title>`.
pub fn orchestrator_label(main: &Path, name: &str) -> String {
    format!("{ORCHESTRATOR} {}", title_of(main, name))
}

/// The orchestration of a space's label `⇶ <title>` (also `⇶ <name>` and an
/// older `Monocircuit/Platform ⇶ <name>`), None for any other. `titles`:
/// `(name, title)` of every record.
pub fn orchestration_of_label(label: &str, titles: &[(String, String)]) -> Option<String> {
    let shown = label
        .strip_prefix(&format!("{SPACE} "))
        .unwrap_or(label)
        .strip_prefix(&format!("{ORCHESTRATOR} "))?;
    if shown.is_empty() {
        return None;
    }
    let by_title = titles.iter().find(|(name, title)| name != shown && title == shown);
    Some(
        by_title
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| shown.to_string()),
    )
}

/// The `(name, title)` of every record, for `orchestration_of_label`.
pub fn titles(main: &Path) -> Vec<(String, String)> {
    recorded(main)
        .into_iter()
        .map(|name| (name.clone(), title_of(main, &name)))
        .collect()
}

// ---------------------------------------------------------------------------
// Writes, as origins.mjs and handovers.mjs make them
// ---------------------------------------------------------------------------

fn object_of(value: Option<Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(object)) => object,
        _ => Map::new(),
    }
}

/// The record's file as an object, `{ issues: [] }` without one.
fn raw_of(main: &Path, name: &str) -> Map<String, Value> {
    let raw =
        object_of(json::read(&record_file(main, name)).filter(|raw| raw.get("issues").is_some_and(Value::is_array)));
    if raw.is_empty() {
        json!({ "issues": [] }).as_object().cloned().unwrap_or_default()
    } else {
        raw
    }
}

fn write_raw(main: &Path, name: &str, raw: Map<String, Value>) -> io::Result<()> {
    json::write_pretty(&record_file(main, name), &Value::Object(raw))
}

/// `issues` into the record of `name`, by number: a second run adds to it; an
/// issue it skips because the first run started it keeps that entry
/// (origins.mjs `writeRecord`).
pub fn write_record(main: &Path, name: &str, issues: &[Value]) -> io::Result<()> {
    let mut raw = raw_of(main, name);
    let mut known: Vec<Value> = raw.get("issues").and_then(Value::as_array).cloned().unwrap_or_default();
    for issue in issues {
        let number = issue.get("number");
        let at = known.iter().position(|entry| entry.get("number") == number);
        let has_why = json::is_truthy(issue.get("why"));
        if let Some(at) = at {
            if has_why && !json::is_truthy(known[at].get("why")) {
                continue;
            }
            known[at] = issue.clone();
        } else {
            known.push(issue.clone());
        }
    }
    raw.insert("issues".into(), Value::Array(known));
    write_raw(main, name, raw)
}

/// `record` with the worktree of `branch` marked removed: `removedAt` and the
/// MR's `mergedAt` on its entry; one started by hand gets an entry
/// (origins.mjs `withRemoved`).
pub fn with_removed(
    record: Map<String, Value>,
    branch: &str,
    feature: &str,
    removed_at: &str,
    merged_at: Option<&str>,
) -> Map<String, Value> {
    let mut record = record;
    let issues: Vec<Value> = record
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let is_known = issues.iter().any(|issue| str_of(issue, "branch") == Some(branch));
    let issues = if is_known {
        issues
            .into_iter()
            .map(|mut issue| {
                if str_of(&issue, "branch") == Some(branch) {
                    let merged = merged_at
                        .map(|at| Value::String(at.into()))
                        .or_else(|| issue.get("mergedAt").filter(|at| !at.is_null()).cloned())
                        .unwrap_or(Value::Null);
                    if let Some(entry) = issue.as_object_mut() {
                        entry.insert("removedAt".into(), json!(removed_at));
                        entry.insert("mergedAt".into(), merged);
                    }
                }
                issue
            })
            .collect()
    } else {
        let number = crate::git::issue_of_branch(branch).filter(|n| *n > 0);
        let mut issues = issues;
        issues.push(json!({
            "number": number, "title": null, "feature": feature, "branch": branch,
            "why": null, "removedAt": removed_at, "mergedAt": merged_at,
        }));
        issues
    };
    record.insert("issues".into(), Value::Array(issues));
    record
}

/// Notes in the record of `name` that the worktree of `branch` was removed (wt:remove).
pub fn mark_removed(
    main: &Path,
    name: &str,
    branch: &str,
    feature: &str,
    removed_at: &str,
    merged_at: Option<&str>,
) -> io::Result<()> {
    let raw = object_of(json::read(&record_file(main, name)));
    write_raw(main, name, with_removed(raw, branch, feature, removed_at, merged_at))
}

/// The plan of `wt:plan`: a list left out keeps the one there.
pub fn write_plan(
    main: &Path,
    name: &str,
    work: Option<Vec<i64>>,
    merge: Option<Vec<i64>>,
    why: Option<String>,
) -> io::Result<Plan> {
    let mut raw = raw_of(main, name);
    let old = Record::from_value(Value::Object(raw.clone()))
        .and_then(|record| record.plan)
        .unwrap_or_default();
    let old_why = raw
        .get("plan")
        .and_then(|plan| plan.get("why"))
        .cloned()
        .unwrap_or(Value::Null);
    let plan = Plan {
        work: work.unwrap_or(old.work),
        merge: merge.unwrap_or(old.merge),
        why: why.or(old.why),
    };
    let why_value = match &plan.why {
        Some(why) => json!(why),
        None => old_why,
    };
    raw.insert(
        "plan".into(),
        json!({ "work": plan.work, "merge": plan.merge, "why": why_value }),
    );
    write_raw(main, name, raw)?;
    Ok(plan)
}

/// An agent of the orchestration as a worker: `fields` merged into what the record has.
pub fn write_worker(main: &Path, name: &str, id: &str, fields: Map<String, Value>) -> io::Result<()> {
    // Inserted, never removed and added again: a key keeps its place in the file.
    let mut raw = raw_of(main, name);
    let mut workers = object_of(raw.get("workers").cloned());
    let mut worker = object_of(workers.get(id).cloned());
    json::merge(&mut worker, fields);
    workers.insert(id.into(), Value::Object(worker));
    raw.insert("workers".into(), Value::Object(workers));
    write_raw(main, name, raw)
}

/// Fields of the whole record set (origins.mjs `updateRecord`); the record made
/// when there is none.
pub fn update_record(main: &Path, name: &str, fields: Map<String, Value>) -> io::Result<()> {
    let mut raw = raw_of(main, name);
    json::merge(&mut raw, fields);
    write_raw(main, name, raw)
}

/// `record` with `handover` in it, one per issue (handovers.mjs `withHandover`).
pub fn with_handover(mut record: Map<String, Value>, handover: Value) -> Map<String, Value> {
    let number = handover.get("number").cloned();
    let mut list: Vec<Value> = record
        .get("handovers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|known| known.get("number").cloned() != number)
        .collect();
    list.push(handover);
    record.insert("handovers".into(), Value::Array(list));
    record
}

/// `record` without the handover of the issue `number`.
pub fn without_handover(mut record: Map<String, Value>, number: i64) -> Map<String, Value> {
    let list: Vec<Value> = record
        .get("handovers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|known| int_of(known, "number") != Some(number))
        .collect();
    record.insert("handovers".into(), Value::Array(list));
    record
}

/// Records that the agent `to` takes the issue `number` once the MRs `after` are merged.
pub fn announce_handover(main: &Path, name: &str, number: i64, to: &str, after: &[i64], at: &str) -> io::Result<()> {
    let raw = raw_of(main, name);
    write_raw(
        main,
        name,
        with_handover(raw, json!({ "number": number, "to": to, "after": after, "at": at })),
    )
}

/// The handover of the issue `number` done: out of the record.
pub fn drop_handover(main: &Path, name: &str, number: i64) -> io::Result<()> {
    let raw = raw_of(main, name);
    let has = raw
        .get("handovers")
        .and_then(Value::as_array)
        .is_some_and(|list| list.iter().any(|h| int_of(h, "number") == Some(number)));
    if has {
        write_raw(main, name, without_handover(raw, number))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::scratch::Scratch;

    /// A record as wt:orchestrate, wt:plan and the overview write it.
    const RECORD: &str = r#"{
  "issues": [
    {
      "number": 234,
      "title": "overview: an orchestration shows its own title",
      "feature": "234-overview-orchestration-shows-its-own",
      "branch": "feature/234-overview-orchestration-shows-its-own",
      "why": null,
      "removedAt": "2026-09-30T15:27:58.866Z",
      "mergedAt": "2026-09-30T15:27:33.321Z"
    },
    {
      "number": 296,
      "title": "overview: hover",
      "feature": "296-overview-hover",
      "branch": "feature/296-overview-hover",
      "why": null,
      "queued": true,
      "queuedAt": "2026-10-01T10:00:00.000Z",
      "after": [303]
    }
  ],
  "brief": "This is research",
  "plan": {
    "work": [306, 303],
    "merge": [234, 296],
    "why": ""
  },
  "title": "@monocircuit/breadboard",
  "handovers": [
    {
      "number": 296,
      "to": "303-overview-buttons",
      "after": [303],
      "at": "2026-10-01T10:00:00.000Z"
    }
  ],
  "actions": [],
  "change": null,
  "workers": {
    "d2419ba2-0884-42ab-9522-a592c3e3cfc4": {
      "name": "235-overview-actions-text-under-action",
      "issues": [235, 240],
      "files": ["packages/breadboard/src/overview/answers.mjs"]
    },
    "7a1c4b21-68c3-4a3b-b351-fb0001d05e9c": {
      "name": "272-orchestration",
      "history": [{ "number": 272, "how": "fresh", "at": "2026-10-01T09:59:03.983Z", "context": 0 }],
      "issues": [272],
      "failed": [{ "number": 292, "reason": "it is working", "at": "2026-10-01T11:03:42.004Z", "times": 1 }],
      "free": { "at": "2026-10-01T12:00:00.000Z", "from": "272-orchestration", "mr": 289 }
    }
  },
  "orchestrator": { "sessionId": "62eabed2-3a26-46a3-88f2-d50d0cd6f88d", "cwd": "/r/platform" }
}"#;

    fn scratch_record() -> (Scratch, PathBuf) {
        let scratch = Scratch::new();
        let main = scratch.join("platform");
        let file = record_file(&main, "breadboard");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, format!("{RECORD}\n")).unwrap();
        (scratch, main)
    }

    #[test]
    fn reads_the_record_node_writes() {
        let (_scratch, main) = scratch_record();
        let record = record_of(&main, "breadboard").unwrap();
        assert_eq!(record.issues.len(), 2);
        assert_eq!(record.issues[0].merged_at.as_deref(), Some("2026-09-30T15:27:33.321Z"));
        assert!(record.issues[1].is_queued);
        assert_eq!(record.issues[1].after, vec![303]);
        assert_eq!(record.plan.as_ref().unwrap().merge, vec![234, 296]);
        assert_eq!(record.title.as_deref(), Some("@monocircuit/breadboard"));
        assert_eq!(record.workers.len(), 2);
        assert_eq!(record.workers[1].history.as_ref().unwrap()[0].context, Some(0.0));
        assert_eq!(record.workers[1].failed[0].reason, "it is working");
        assert_eq!(record.free_agents().len(), 1);
        assert_eq!(record.handovers[0].to, "303-overview-buttons");
        assert_eq!(record.chat.as_ref().unwrap().cwd.as_deref(), Some("/r/platform"));
        assert_eq!(title_of(&main, "breadboard"), "@monocircuit/breadboard");
        assert_eq!(title_of(&main, "other"), "other");
        assert_eq!(recorded(&main), vec!["breadboard".to_string()]);
    }

    #[test]
    fn a_record_without_issues_is_none() {
        assert!(Record::from_value(json!({ "plan": {} })).is_none());
        assert!(Record::from_value(json!({ "issues": "no" })).is_none());
        let record = Record::from_value(json!({ "issues": [{ "number": "x" }], "plan": 3 })).unwrap();
        assert_eq!(record.issues[0].number, None);
        assert!(record.plan.is_none());
    }

    #[test]
    fn a_write_keeps_what_rust_does_not_know() {
        let (_scratch, main) = scratch_record();
        write_plan(&main, "breadboard", Some(vec![296]), None, None).unwrap();
        let text = fs::read_to_string(record_file(&main, "breadboard")).unwrap();
        let mut expected: Value = serde_json::from_str(RECORD).unwrap();
        expected["plan"]["work"] = json!([296]);
        assert_eq!(text, format!("{}\n", json::pretty(&expected)));
    }

    #[test]
    fn round_trips_unchanged() {
        let (_scratch, main) = scratch_record();
        update_record(&main, "breadboard", Map::new()).unwrap();
        let text = fs::read_to_string(record_file(&main, "breadboard")).unwrap();
        let expected: Value = serde_json::from_str(RECORD).unwrap();
        assert_eq!(text, format!("{}\n", json::pretty(&expected)));
    }

    #[test]
    fn write_record_keeps_a_started_entry_over_a_skipped_one() {
        let (_scratch, main) = scratch_record();
        write_record(
            &main,
            "breadboard",
            &[
                json!({ "number": 234, "why": "exists already" }),
                json!({ "number": 300, "why": null }),
            ],
        )
        .unwrap();
        let record = record_of(&main, "breadboard").unwrap();
        assert_eq!(record.issues.len(), 3);
        assert_eq!(
            record.issues[0].title.as_deref(),
            Some("overview: an orchestration shows its own title")
        );
        assert_eq!(record.issues[2].number, Some(300));
    }

    #[test]
    fn marks_a_removed_worktree() {
        let raw: Value = serde_json::from_str(RECORD).unwrap();
        let marked = with_removed(
            raw.as_object().unwrap().clone(),
            "feature/296-overview-hover",
            "296-overview-hover",
            "T",
            None,
        );
        assert_eq!(marked["issues"][1]["removedAt"], json!("T"));
        assert_eq!(marked["issues"][1]["mergedAt"], Value::Null);
        let added = with_removed(marked, "fix/301-by-hand", "301-by-hand", "T2", Some("M"));
        assert_eq!(added["issues"][2]["number"], json!(301));
        assert_eq!(added["issues"][2]["mergedAt"], json!("M"));
        assert_eq!(added["issues"][2]["title"], Value::Null);
    }

    #[test]
    fn handovers_one_per_issue() {
        let (_scratch, main) = scratch_record();
        announce_handover(&main, "breadboard", 296, "other", &[310], "T").unwrap();
        let record = record_of(&main, "breadboard").unwrap();
        assert_eq!(record.handovers.len(), 1);
        assert_eq!(record.handovers[0].to, "other");
        drop_handover(&main, "breadboard", 296).unwrap();
        assert!(record_of(&main, "breadboard").unwrap().handovers.is_empty());
    }

    #[test]
    fn workers_merge_their_fields() {
        let (_scratch, main) = scratch_record();
        let fields = json!({ "free": null, "dismissed": "T" }).as_object().unwrap().clone();
        write_worker(&main, "breadboard", "7a1c4b21-68c3-4a3b-b351-fb0001d05e9c", fields).unwrap();
        let record = record_of(&main, "breadboard").unwrap();
        assert!(record.free_agents().is_empty());
        assert_eq!(record.workers[1].name.as_deref(), Some("272-orchestration"));
        assert_eq!(record.workers.len(), 2);
    }

    #[test]
    fn reads_origins_and_space_labels() {
        let scratch = Scratch::new();
        write_origin(
            scratch.path(),
            &Origin {
                name: "breadboard".into(),
                slot: 4,
            },
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(origin_file(scratch.path())).unwrap(),
            "{\"name\":\"breadboard\",\"slot\":4}\n"
        );
        assert_eq!(
            origin_of(scratch.path()),
            Some(Origin {
                name: "breadboard".into(),
                slot: 1
            })
        );
        fs::write(origin_file(scratch.path()), "{\"name\":\"Bad Name\",\"slot\":1}").unwrap();
        assert_eq!(origin_of(scratch.path()), None);
        let titles = vec![("breadboard".to_string(), "@monocircuit/breadboard".to_string())];
        assert_eq!(
            orchestration_of_label("⇶ @monocircuit/breadboard", &titles).as_deref(),
            Some("breadboard")
        );
        assert_eq!(
            orchestration_of_label("⇶ breadboard", &titles).as_deref(),
            Some("breadboard")
        );
        assert_eq!(
            orchestration_of_label("Monocircuit/Platform ⇶ other", &titles).as_deref(),
            Some("other")
        );
        assert_eq!(orchestration_of_label("Monocircuit/Platform", &titles), None);
    }
}
