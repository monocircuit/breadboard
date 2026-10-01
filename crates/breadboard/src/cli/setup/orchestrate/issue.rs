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
use std::process::{Command, Stdio};

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

pub fn brief_of(brief: &Brief) -> String {
    let main = repo::main_dir();
    let Item {
        number,
        title,
        dir,
        branch,
        ..
    } = brief.item;
    let dir = dir.display();
    let name = brief.name;
    let label = record::orchestrator_label(&main, name);
    let worktree = if brief.before.is_some() {
        format!(
            "Your worktree is `{dir}` on `{branch}`: it exists, its branch is pushed as the claim. You come from another worktree: `EnterWorktree(path: \"{dir}\")` once, if you don't stand in it yet; after that don't run `wt:new`, `EnterWorktree` or `ExitWorktree`."
        )
    } else {
        format!(
            "Your worktree is `{dir}` on `{branch}`: it exists, its branch is pushed as the claim, you already stand in it.\nDon't run `wt:new`, `EnterWorktree` or `ExitWorktree`."
        )
    };
    let peer = match brief.peer {
        Some(peer) => format!("The orchestrator is the session `{peer}` (`ListAgents`)."),
        None => format!(
            "The orchestrator messages you first: reply to the `from` of its message. Before that, `ListAgents` finds it (it works in `{}`).",
            main.display()
        ),
    };
    let before = brief
        .before
        .as_ref()
        .map(|before| format!("\n## Before this issue\n\n{before}\n"))
        .unwrap_or_default();
    let extra = if brief.extra.is_empty() {
        String::new()
    } else {
        format!("\n## From the orchestrator\n\n{}\n", brief.extra.trim())
    };
    format!(
        r#"# #{number} {title}

You work on issue #{number} of monocircuit/platform, started by the orchestration `{name}` (herdr space `{label}`).
{worktree}

1. Read the issue: `glab issue view {number} --comments`, then the AGENTS.md files and skills it touches.
2. Work as AGENTS.md says: commit, `pnpm wt:check`, push per step (green means wt:check passed on that commit: the pre-push hook refuses any other, never `--no-verify`; a failing test is fixed; one that fails on develop too is fixed once, in its own fix MR, and the others wait for it with `pnpm wt:wait`), the draft MR with the first commit (`pnpm wt:mr`), boxes ticked with `pnpm wt:tick`. Push every commit at once, never hold one back: the overview shows unpushed commits, and past 10 minutes they go to its NEXT.
   A change requested for your MR (by the user or the orchestrator): `pnpm wt:action change "<what>"` first, it makes the MR a draft and withdraws your merge action, then build it; once it is pushed and green, post the merge again. Merged meanwhile: the command says so, and the change gets its own issue, branch and draft MR at once.
   Your MR's pipeline failed: the overview types `[overview] the pipeline of !N failed: job …, <url>: …` into this tab by itself. Read the job's log (the url, or `glab ci trace <job id>`), fix what is your MR's doing and push. When the line says develop fails the same job, it isn't yours: say so in the MR (`glab mr note`) and go on, don't chase it.
   Your issue started at once, even when it builds on an open MR or its files move in one: when you have to wait for other MRs, run `pnpm wt:wait !N … --then "<what next>"` with `run_in_background` and stop; don't ask the orchestrator. Its line wakes you once they are merged (or why the wait ended).
3. **The overview shows your work, you don't report it.** Your node in its WORKTREES shows herdr's state of your agent and what you wait for, with commits, MR and pipeline; MERGE REQUESTS shows the boxes: no note, no status message.
   `SendMessage` to the orchestrator only for what needs it, one short message:
   - the plan, when it differs from the issue;
   - a decision that is the user's (options, your recommendation), then wait for the answer instead of guessing; with its `decide` action posted, the message is only context: the user answers on the action's buttons, and the orchestrator relays it only while the action is open;
   - what concerns another agent's work (an order, a conflict, a dependency).
   No message for a stage end or the end: the overview has them.
   {peer}
4. **Whenever you stop and the next step is the user's, leave an action for it** with `pnpm wt:action`. The overview of the orchestration answers "what now?" from these (its NEXT line and ACTIONS): the user must never have to ask you what is next.
   - `pnpm wt:action merge "<what is in it>"` only once the work is done: the pipeline of the MR passed, every box of the issue is ticked (Work and acceptance criteria), every `try` you posted was answered `looks good`, and it is its turn in the merge order (`wt:plan`). Then mark it ready (`glab mr update --ready`) and post it yourself, the overview's automatic one is only a fallback; when your permissions block `--ready`, post it anyway with "mark ready + merge" in the text. While a try is open the overview offers no merge. Never merge yourself;
   - `pnpm wt:action try "<what to check>" --run "<command>"` (a terminal, e.g. `pnpm overview --orchestration <name>` of your branch) or `--url <page>` (`/atlas/…` on your worktree's app, its stack started first) for something only a person can judge: the click opens that spot. Every box you leave open for the user's live check gets one, and the box names it (`… (try #<id>)`); tick it once the answer is `looks good`. A check that needs the deployment (HTTPS, the test instance's data) is `--on test --url <path>`, posted with the merge, its box marked `(after deploy, try #<id>)`: the only check that stays open across the merge; the row shows `waits for merge`, `deploying`, then `ready` by itself. A `broken` puts the MR back to draft (the overview does it when the MR was ready, else `glab mr update --draft`) until the fix is pushed, green and tried again with a new try;
   - `pnpm wt:action decide "<what is decided, one sentence>? <why the recommendation, one line>" --option "<label>" --why "<what happens>" --option "<label>" --why "<what happens>" --recommend <label>`: the user answers from the row, so the question says precisely what is decided, each option a short label (the answer you get) with what happens as its `--why` (or `--option "<label>: <what happens>"`), not `A`/`B`, and `--recommend` marks yours `(Recommended)`. Together with the SendMessage to the orchestrator that explains them;
   - `pnpm wt:action plan "<the plan in one line>"` before larger work, for an `ok` or `different`;
   - `pnpm wt:action look "<what>"` for anything else the user has to do.
   The user's click arrives typed into this tab as one line, `[overview] #<id> <kind> "<text>" → <answer>`: go on with it, don't wait or poll for it. A `broken` or `different` comes as `… → broken: <what is wrong>`, written by the user, maybe with images: until it arrives, wait for it instead of guessing from the bare word. The answered action goes by itself. Once one no longer applies (done by the user, overtaken), `pnpm wt:action --done <id>` (`--list` shows them). While you are working, leave none.
5. The user may type into this tab directly: their word wins over the orchestrator's.
6. Your next issue may come as one line, `[overview] next issue #<n>, your name is <n-feature> now: EnterWorktree(path: "<dir>"), then read .claude/brief.md`: do just that, the brief there is your task from then on. The `/rename` typed before it gave your tab and conversation that name: the orchestrator and the other agents reach you under it, a message to your old name is about your old issue.
{before}{extra}"#
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
    let made = Command::new(proc::entry())
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
        eprint!("{errors}");
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
        let _ = Command::new(proc::entry())
            .args(["wt:remove", &item.feature])
            .current_dir(repo::main_dir())
            .stdin(Stdio::null())
            .status();
    }
    let _ = record::write_record(&repo::main_dir(), name, &[start_entry(item, known, Some(why))]);
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
