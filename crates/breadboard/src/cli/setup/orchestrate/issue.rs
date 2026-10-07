//! An issue of wt:orchestrate (orchestrator.mjs): read from GitLab with the
//! worktree it gets, its entry in the record, the brief its agent reads, its
//! worktree made by `wt:new --origin` in a child and its claude started.

use crate::cli::setup::conversations::session_file;
use crate::cli::setup::herdr;
use crate::cli::setup::new::claimed_elsewhere;
use crate::cli::setup::nvim::ide_flag;
use crate::cli::setup::proc::{self, quote};
use crate::cli::setup::repo;
use breadboard_core::json::str_of;
use breadboard_core::record;
use breadboard_core::time::{now_ms, to_iso};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Its first prompt: one line without quotes.
pub const BRIEF_PROMPT: &str = "Read .claude/brief.md and work on the issue it describes.";

const STOPWORDS: [&str; 15] = [
    "a", "an", "the", "of", "in", "on", "for", "to", "and", "or", "with", "by", "at", "from", "is",
];

/// A letter with its accent dropped (NFKD without the marks), for the slug.
fn fold(c: char) -> &'static str {
    match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' => "d",
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' => "h",
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò'..='ö' | 'ō' | 'ŏ' | 'ő' => "o",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' => "s",
        'ţ' | 'ť' => "t",
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => " ",
    }
}

/// `Orchestrated work: an orchestrator space, marked …` → `orchestrated-work-orchestrator-space-marked`:
/// up to five whole words, at most 40 characters.
pub fn slug_of(title: &str) -> String {
    let mut plain = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            plain.push(c);
        } else if c.is_ascii() {
            plain.push(' ');
        } else {
            plain.push_str(fold(c));
        }
    }
    let words: Vec<&str> = plain
        .split(' ')
        .filter(|word| !word.is_empty() && !STOPWORDS.contains(word))
        .collect();
    let Some(first) = words.first() else {
        return "issue".into();
    };
    let mut slug: String = first.chars().take(40).collect();
    for word in words.iter().skip(1).take(4) {
        if slug.len() + 1 + word.len() > 40 {
            break;
        }
        slug = format!("{slug}-{word}");
    }
    slug
}

/// An issue as wt:orchestrate starts it.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub number: String,
    pub title: String,
    pub description: String,
    pub feature: String,
    pub is_fix: bool,
    pub dir: PathBuf,
    /// Why it gets no worktree: closed, its worktree there, claimed on origin.
    pub why: Option<String>,
    pub branch: String,
}

impl Item {
    pub fn number(&self) -> i64 {
        self.number.parse().unwrap_or_default()
    }
}

/// The leading number of an argument (`12` of `12-x`).
pub fn number_of(arg: &str) -> String {
    arg.chars().take_while(char::is_ascii_digit).collect()
}

/// An issue (its number, or `<n>-<feature>`) as GitLab has it, None when it can't be read.
pub fn read_issue(arg: &str) -> Option<Item> {
    let number = number_of(arg);
    let found = proc::sh(
        "glab",
        &["issue", "view", &number, "-F", "json"],
        Some(&repo::main_dir()),
    );
    if !found.is_ok {
        return None;
    }
    let issue: Value = serde_json::from_str(found.text()).ok()?;
    let title = str_of(&issue, "title").unwrap_or_default().to_string();
    let feature = if arg.chars().all(|c| c.is_ascii_digit()) {
        format!("{number}-{}", slug_of(&title))
    } else {
        arg.to_string()
    };
    let is_fix = issue
        .get("labels")
        .and_then(Value::as_array)
        .is_some_and(|labels| labels.iter().any(|label| label.as_str() == Some("bug")));
    let dir = repo::dir_of(&feature);
    let state = str_of(&issue, "state").unwrap_or_default();
    let why = if state != "opened" {
        Some(format!("#{number} is {state}"))
    } else if dir.exists() {
        Some(format!("{} exists already", repo::base_of(&dir)))
    } else {
        claimed_elsewhere(&feature).map(|branch| format!("claimed by {branch}"))
    };
    Some(Item {
        branch: format!("{}/{feature}", if is_fix { "fix" } else { "feature" }),
        number,
        title,
        description: str_of(&issue, "description").unwrap_or_default().to_string(),
        feature,
        is_fix,
        dir,
        why,
    })
}

/// An issue as the record keeps it: `{ number, title, feature, branch, why }`.
pub fn record_entry(item: &Item, why: Option<&str>) -> Value {
    json!({ "number": item.number(), "title": item.title, "feature": item.feature, "branch": item.branch, "why": why })
}

/// The entry of an issue whose start runs or failed: a queued one stays queued at its place.
pub fn start_entry(item: &Item, known: Option<&Value>, why: Option<&str>) -> Value {
    let is_queued = known.is_some_and(|known| breadboard_core::json::is_truthy(known.get("queued")));
    if !is_queued {
        return record_entry(item, why.map(|why| format!("failed: {why}")).as_deref());
    }
    let mut entry = record_entry(item, None);
    let object = entry.as_object_mut().expect("an entry is an object");
    object.insert("queued".into(), json!(true));
    let queued_at = known
        .and_then(|known| str_of(known, "queuedAt"))
        .map(String::from)
        .unwrap_or_else(|| to_iso(now_ms()));
    object.insert("queuedAt".into(), json!(queued_at));
    if let Some(why) = why {
        object.insert("requeued".into(), json!(why));
    }
    entry
}

/// What an orchestrated agent is told (verbatim, agents read it).
pub struct Brief<'a> {
    pub item: &'a Item,
    pub name: &'a str,
    pub peer: Option<&'a str>,
    pub extra: &'a str,
    pub before: Option<String>,
}

/// The text is the lifecycle group's (handover.rs `brief_of`), one for both:
/// a fresh agent's and a handed-over one's must not drift.
pub fn brief_of(brief: &Brief) -> String {
    let main = repo::main_dir();
    let item = brief.item;
    let issue = crate::cli::lifecycle::handover::Issue {
        number: item.number(),
        title: item.title.clone(),
        feature: item.feature.clone(),
        is_fix: item.is_fix,
        dir: item.dir.clone(),
        why: item.why.clone(),
        branch: item.branch.clone(),
    };
    crate::cli::lifecycle::handover::brief_of(
        &issue,
        brief.name,
        &record::orchestrator_label(&main, brief.name),
        brief.peer,
        brief.extra,
        brief.before.as_deref(),
        &main,
    )
}

/// What `wt:new --origin` did: the worktree there, the reason it failed with (`✗ …`), its exit code.
pub struct Made {
    pub is_there: bool,
    pub failed: Option<String>,
    pub status: Option<i32>,
}

/// `wt:new <feature> --origin <name>` in a child, its output shown, its stderr read for `✗ …`.
pub fn make_worktree(item: &Item, name: &str, is_shared: bool) -> Made {
    let mut args = vec!["wt:new", item.feature.as_str(), "--origin", name];
    if item.is_fix {
        args.push("--fix");
    }
    if is_shared {
        args.push("--shared");
    }
    let made = proc::child()
        .args(&args)
        .current_dir(repo::main_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .output();
    let (errors, status) = match made {
        Ok(output) => (
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ),
        Err(error) => (format!("\n✗ {error}\n"), None),
    };
    if !errors.trim().is_empty() {
        proc::write_bytes(proc::Stream::Err, errors.as_bytes());
    }
    Made {
        is_there: repo::worktrees().iter().any(|tree| tree.dir == item.dir),
        failed: errors
            .lines()
            .find_map(|line| line.strip_prefix("✗ ").map(String::from)),
        status,
    }
}

/// A random conversation id (UUID v4).
pub fn new_session_id() -> String {
    let mut bytes = [0u8; 16];
    let is_read = fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok();
    if !is_read {
        let seed = now_ms() as u128 ^ (u128::from(std::process::id()) << 64);
        bytes = seed.to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// What starting an agent did.
pub struct Agent {
    pub is_started: bool,
    pub command: String,
    pub id: String,
}

/// The claude of an orchestrated worktree: its brief written, started in the
/// idle `<feature>` tab of its space (bound before it starts), else the command to run by hand.
pub fn start_agent(dir: &Path, brief: &str) -> Agent {
    let feature = repo::feature_of(dir);
    let _ = record::write_brief(dir, brief);
    let id = new_session_id();
    let command = format!(
        "cd {} && claude --session-id {id} --name {feature}{} {}",
        quote(&dir.to_string_lossy()),
        ide_flag(dir),
        quote(BRIEF_PROMPT)
    );
    let workspace = if herdr::is_in_herdr() {
        herdr::space_of(dir)
    } else {
        None
    };
    let pane = workspace.and_then(|workspace| herdr::idle_pane_of(&workspace, &feature, &herdr::tabs_of(&workspace)));
    let is_started = pane.is_some_and(|pane| herdr::herdr_did(&["pane", "run", &pane, &command]));
    if is_started {
        let _ = fs::write(session_file(dir), format!("{id}\n"));
    }
    Agent {
        is_started,
        command,
        id,
    }
}

/// A start that failed once its worktree was made: the worktree goes again
/// (wt:remove in a child), a queued issue back to the queue.
pub fn undo_start(name: &str, item: &Item, known: Option<&Value>, why: &str, claim: Option<&super::setups::Claim>) {
    if let Some(claim) = claim {
        claim.release();
    }
    if repo::worktrees().iter().any(|tree| tree.dir == item.dir) {
        let _ = proc::child()
            .args(["wt:remove", &item.feature])
            .current_dir(repo::main_dir())
            .stdin(Stdio::null())
            .status();
    }
    // Not write_record: it keeps the entry wt:new wrote (no why) over one
    // with a why, so the record said the worktree was removed but not why.
    // The undone start's entry replaces it, as the spec's undoStart wants.
    let main = repo::main_dir();
    let entry = start_entry(item, known, Some(why));
    let raw = breadboard_core::json::read(&record::record_file(&main, name));
    let issues = with_entry(
        raw.as_ref()
            .and_then(|raw| raw.get("issues"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        entry,
    );
    let mut fields = Map::new();
    fields.insert("issues".into(), Value::Array(issues));
    let _ = record::update_record(&main, name, fields);
}

/// `issues` with `entry` in place of the one of its number (or added).
fn with_entry(mut issues: Vec<Value>, entry: Value) -> Vec<Value> {
    match issues
        .iter()
        .position(|known| known.get("number") == entry.get("number"))
    {
        Some(at) => issues[at] = entry,
        None => issues.push(entry),
    }
    issues
}

/// The record's entry of `number` now.
pub fn known_entry(name: &str, number: i64) -> Option<Value> {
    breadboard_core::json::read(&record::record_file(&repo::main_dir(), name))?
        .get("issues")?
        .as_array()?
        .iter()
        .find(|issue| breadboard_core::json::int_of(issue, "number") == Some(number))
        .cloned()
}

/// `fields` as an object for the record writes.
pub fn object(fields: Value) -> Map<String, Value> {
    fields.as_object().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_a_slug_of_a_title() {
        assert_eq!(
            slug_of("Orchestrated work: an orchestrator space, marked …"),
            "orchestrated-work-orchestrator-space"
        );
        assert_eq!(slug_of("Café für Über-Größen"), "cafe-fur-uber-gro-en");
        assert_eq!(slug_of("the of"), "issue");
        assert_eq!(
            slug_of("overview: every drawn button works"),
            "overview-every-drawn-button-works"
        );
        assert_eq!(
            slug_of("a-very-long-word-that-goes-on-and-on-and-on-forever and more"),
            "very-long-word-that-goes"
        );
    }

    #[test]
    fn puts_an_undone_start_in_place_of_its_entry() {
        let issues = vec![
            json!({ "number": 5, "why": null, "removedAt": "x" }),
            json!({ "number": 6 }),
        ];
        let entry = json!({ "number": 5, "why": "failed: its claude did not start" });
        let issues = with_entry(issues, entry.clone());
        assert_eq!(issues, vec![entry, json!({ "number": 6 })]);
        assert_eq!(with_entry(vec![], json!({ "number": 7 })), vec![json!({ "number": 7 })]);
    }

    #[test]
    fn makes_a_uuid() {
        let id = new_session_id();
        assert!(breadboard_core::record::is_session_id(&id), "{id}");
        assert_eq!(&id[14..15], "4");
    }

    #[test]
    fn keeps_a_queued_entry_queued() {
        let item = Item {
            number: "5".into(),
            title: "t".into(),
            feature: "5-t".into(),
            branch: "feature/5-t".into(),
            ..Item::default()
        };
        let known = json!({ "number": 5, "queued": true, "queuedAt": "2026-10-01T00:00:00.000Z" });
        let entry = start_entry(&item, Some(&known), Some("no worktree"));
        assert_eq!(entry["queued"], json!(true));
        assert_eq!(entry["queuedAt"], json!("2026-10-01T00:00:00.000Z"));
        assert_eq!(entry["requeued"], json!("no worktree"));
        assert_eq!(start_entry(&item, None, Some("x"))["why"], json!("failed: x"));
    }
}
