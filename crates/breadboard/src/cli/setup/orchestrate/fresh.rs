//! `wt:orchestrate <name> --fresh` (#382): the orchestrator's chat starts
//! fresh. The old chat is told to write its journal entry and finishes its
//! turn, then its claude ends and the tab `orchestrator` runs a new claude
//! whose first prompt carries the journal and the record: nothing is lost
//! but the transcript, which stays in /resume. The overview runs it at a
//! quiet point (no question to the user open, the chat idle, no merge in
//! flight); never two claudes on one conversation.

use crate::cli::lifecycle::claude::running_claudes;
use crate::cli::lifecycle::handover::end_claude;
use crate::cli::lifecycle::herdr as lifecycle_herdr;
use crate::cli::lifecycle::layout::nvim::ide_flag;
use crate::cli::lifecycle::layout::orchestrations;
use crate::cli::setup::orchestrate::issue::new_session_id;
use crate::cli::setup::proc::{fail, outln, quote, step};
use crate::cli::setup::repo;
use breadboard_core::conversations::context_share_of;
use breadboard_core::journal;
use breadboard_core::record::{self, Record, is_session_id};
use breadboard_core::time::now_ms;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// How long the old chat gets for its journal entry.
const JOURNAL_WAIT: Duration = Duration::from_secs(300);
/// How long the old chat gets to start its answer to the line.
const START_WAIT: Duration = Duration::from_secs(5);
/// How long the tab's shell gets to come back once its claude ended.
const SHELL_WAIT: Duration = Duration::from_secs(10);
/// How many of the journal's lines the first prompt carries, the newest.
const PROMPT_LINES: usize = 80;

/// The line the old chat gets before it ends.
pub fn journal_line(name: &str) -> String {
    let run = breadboard_core::config::run_word();
    format!(
        "[overview] fresh start at a quiet point: this chat ends after this answer and a fresh one takes over from the journal. Write into it now what the next one needs and GitLab, the record and the overview don't hold: every open thread, who waits for what, decisions you took for the user. One line each: {run} wt:journal \"<line>\" --orchestration {name}. Nothing else, no answer to the user."
    )
}

/// The first prompt of the fresh chat: who it is, where its state lives, the journal and the record.
pub fn first_prompt(
    label: &str,
    name: &str,
    old: &str,
    share: Option<f64>,
    journal: &[String],
    record: Option<&Record>,
) -> String {
    let run = breadboard_core::config::run_word();
    let at = share
        .map(|share| format!(" at {} % of its context", (share * 100.0).round() as i64))
        .unwrap_or_default();
    let mut text = format!(
        "[overview] fresh start: you are the orchestrator of {label} (orchestration {name}) now. The chat before you ended{at} and stays in /resume ({old}). Read the skill parallel-work (orchestrate.md, \"Staying lean\" first), then go on from the journal and the record below: every open thread in them is yours. GitLab, the overview and the agents hold the rest; ask them, not the user, and keep this chat lean. Keep the journal with {run} wt:journal \"<line>\" --orchestration {name}.\n"
    );
    text += &format!("\nJournal (.claude/orchestrations/{name}.journal.md):\n");
    if journal.is_empty() {
        text += "  (empty)\n";
    }
    let from = journal.len().saturating_sub(PROMPT_LINES);
    for entry in &journal[from..] {
        text += &format!("  {entry}\n");
    }
    let Some(record) = record else { return text };
    text += "\nRecord:\n";
    for issue in &record.issues {
        let number = issue.number.map(|n| format!("#{n}")).unwrap_or_default();
        let feature = issue.feature.as_deref().unwrap_or_default();
        let state = if issue.merged_at.is_some() {
            "merged".to_string()
        } else if issue.removed_at.is_some() {
            "done, worktree removed".to_string()
        } else if let Some(why) = &issue.why {
            format!("skipped: {why}")
        } else if issue.is_queued {
            let after: Vec<String> = issue.after.iter().map(|iid| format!("!{iid}")).collect();
            if after.is_empty() {
                "queued".to_string()
            } else {
                format!("queued after {}", after.join(", "))
            }
        } else {
            "at work".to_string()
        };
        text += &format!("  {number} {feature}: {state}\n");
    }
    if let Some(plan) = &record.plan {
        let list = |numbers: &[i64]| numbers.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
        text += &format!("  plan: work {} · merge {}", list(&plan.work), list(&plan.merge));
        if let Some(why) = &plan.why {
            text += &format!(" · {why}");
        }
        text += "\n";
    }
    for handover in &record.handovers {
        let after: Vec<String> = handover.after.iter().map(|iid| format!("!{iid}")).collect();
        text += &format!(
            "  handover: #{} → {} after {}\n",
            handover.number,
            handover.to,
            after.join(", ")
        );
    }
    text
}

/// What a fresh start needs of the machine: herdr's in a run, a fake in the tests.
pub trait Host {
    /// `line` typed into the claude of `pane`: whether herdr took it.
    fn tell(&self, pane: &str, line: &str) -> bool;
    /// The pid of the claude that runs the conversation `id`, None when none does.
    fn pid_of(&self, id: &str) -> Option<i64>;
    /// The claude `pid` asked to end; whether it is gone within the wait.
    fn end(&self, pid: i64) -> bool;
    /// `command` run in the shell of `pane`, only while that shell idles.
    fn run(&self, pane: &str, command: &str) -> bool;
}

/// The machine as it is: herdr, the claudes' session files, signals.
struct Machine;

impl Host for Machine {
    fn tell(&self, pane: &str, line: &str) -> bool {
        lifecycle_herdr::did(&["agent", "prompt", pane, line])
    }
    fn pid_of(&self, id: &str) -> Option<i64> {
        running_claudes()
            .into_iter()
            .find(|claude| claude.id == id)
            .map(|claude| claude.pid)
    }
    fn end(&self, pid: i64) -> bool {
        end_claude(pid)
    }
    fn run(&self, pane: &str, command: &str) -> bool {
        // A shell line goes only into a shell, never into a running claude.
        lifecycle_herdr::is_idle(pane) && lifecycle_herdr::did(&["pane", "run", pane, command])
    }
}

/// What the fresh start needs to know.
pub struct Fresh<'a> {
    pub id: &'a str,
    /// The pane the old chat runs in: the new one runs there too.
    pub pane: &'a str,
    pub line: &'a str,
    /// The shell line that starts the new chat, made once the old one ended:
    /// from the journal as it left it.
    pub command: &'a dyn Fn() -> String,
}

/// The old chat told, waited for, ended; the new one started in its pane.
/// Err((why, has_ended)): nothing changed while `has_ended` is false.
pub fn fresh_start(host: &dyn Host, is_done: &dyn Fn(&str) -> bool, fresh: &Fresh) -> Result<(), (String, bool)> {
    if !host.tell(fresh.pane, fresh.line) {
        return Err(("herdr took no line".into(), false));
    }
    if !is_done(fresh.pane) {
        return Err((
            "the chat is still at work on its journal entry: it runs on, the next quiet point tries again".into(),
            false,
        ));
    }
    if let Some(pid) = host.pid_of(fresh.id)
        && !host.end(pid)
    {
        return Err((format!("its claude (pid {pid}) did not end"), false));
    }
    if host.pid_of(fresh.id).is_some() {
        return Err(("another claude runs its conversation".into(), false));
    }
    let command = (fresh.command)();
    let started = Instant::now();
    while !host.run(fresh.pane, &command) {
        if started.elapsed() >= SHELL_WAIT {
            return Err((format!("herdr ran no command in its tab: {command}"), true));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// Whether the agent of `pane` has finished its answer to the line (herdr's
/// agent status), waited for: its start first, a while, then its end.
fn is_done_on_herdr(pane: &str) -> bool {
    let status = || {
        lifecycle_herdr::all_panes()
            .into_iter()
            .find(|each| lifecycle_herdr::s(each, "pane_id") == pane)
            .map(|each| lifecycle_herdr::s(&each, "agent_status").to_string())
    };
    let started = Instant::now();
    while status().is_some_and(|now| now != "working") && started.elapsed() < START_WAIT {
        std::thread::sleep(Duration::from_millis(250));
    }
    let started = Instant::now();
    loop {
        match status() {
            None => return true,
            Some(now) if now != "working" => return true,
            Some(_) if started.elapsed() >= JOURNAL_WAIT => return false,
            Some(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

pub fn command(name: &str) -> ExitCode {
    let main = repo::main_dir();
    let label = record::orchestrator_label(&main, name);
    let Some(record) = record::record_of(&main, name) else {
        fail(&format!("no orchestration {name} is recorded"))
    };
    let Some(chat) = record.chat.clone().filter(|chat| is_session_id(&chat.session_id)) else {
        fail(&format!(
            "{label} has no orchestrator chat recorded: nothing to start fresh"
        ))
    };
    let old = chat.session_id.clone();
    let cwd = chat
        .cwd
        .as_deref()
        .filter(|cwd| !cwd.is_empty())
        .map_or_else(|| main.clone(), PathBuf::from);
    let Some(pane) = lifecycle_herdr::all_panes()
        .into_iter()
        .find(|pane| {
            pane.get("agent_session")
                .and_then(|session| session.get("value"))
                .and_then(serde_json::Value::as_str)
                == Some(old.as_str())
        })
        .map(|pane| lifecycle_herdr::s(&pane, "pane_id").to_string())
    else {
        fail(&format!("the orchestrator's chat {old} runs in no herdr pane"))
    };
    if std::env::var("HERDR_PANE_ID").is_ok_and(|here| here == pane) {
        fail("the orchestrator can't start itself fresh: the overview's ↻ fresh start does it");
    }
    let share = context_share_of(&old);
    let line = journal_line(name);
    step(&format!("the chat {} writes its journal entry", &old[..8]));
    let fresh_id = new_session_id();
    let command = || {
        let entries = journal::entries(&main, name);
        let prompt = first_prompt(&label, name, &old, share, &entries, Some(&record));
        start_command(&cwd, &fresh_id, name, &prompt)
    };
    let result = fresh_start(
        &Machine,
        &is_done_on_herdr,
        &Fresh {
            id: &old,
            pane: &pane,
            line: &line,
            command: &command,
        },
    );
    match result {
        Ok(()) => {
            orchestrations::record_chat(&repo::lifecycle(), name, Some((fresh_id.clone(), cwd)));
            let at = share
                .map(|share| format!(" at {} %", (share * 100.0).round() as i64))
                .unwrap_or_default();
            let _ = journal::append(
                &main,
                name,
                &format!("fresh start: the chat {old}{at} ended (claude --resume {old}), {fresh_id} took over"),
                now_ms(),
            );
            outln!(
                "✓ fresh start: {label} runs a new chat ({}), the old one stays in /resume",
                &fresh_id[..8]
            );
            ExitCode::SUCCESS
        }
        Err((why, has_ended)) => {
            if has_ended {
                fail(&format!("{why}; the old chat ended: claude --resume {old}"))
            }
            fail(&why)
        }
    }
}

/// The shell line of the new chat: in the old one's directory, a new conversation, the prompt its first.
pub fn start_command(cwd: &Path, id: &str, name: &str, prompt: &str) -> String {
    format!(
        "cd {} && claude --session-id {id} --name {}{} {}",
        quote(&cwd.to_string_lossy()),
        quote(&format!("orchestrator {name}")),
        ide_flag(cwd),
        quote(prompt)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::record::{IssueEntry, Plan};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        pid: RefCell<Option<i64>>,
        is_deaf: bool,
        did: RefCell<Vec<String>>,
    }

    impl Host for Fake {
        fn pid_of(&self, _: &str) -> Option<i64> {
            *self.pid.borrow()
        }
        fn end(&self, pid: i64) -> bool {
            self.did.borrow_mut().push(format!("end {pid}"));
            *self.pid.borrow_mut() = None;
            true
        }
        fn run(&self, pane: &str, command: &str) -> bool {
            assert!(self.pid.borrow().is_none(), "a second claude on one conversation");
            self.did.borrow_mut().push(format!("run {pane}: {command}"));
            true
        }
        fn tell(&self, pane: &str, line: &str) -> bool {
            if self.is_deaf {
                return false;
            }
            self.did.borrow_mut().push(format!("tell {pane}: {line}"));
            true
        }
    }

    fn fresh() -> Fresh<'static> {
        Fresh {
            id: "old",
            pane: "p1",
            line: "write your journal",
            command: &|| "claude go".to_string(),
        }
    }

    #[test]
    fn tells_then_ends_then_starts_in_the_same_pane() {
        let fake = Fake {
            pid: RefCell::new(Some(7)),
            ..Fake::default()
        };
        assert_eq!(fresh_start(&fake, &|_| true, &fresh()), Ok(()));
        assert_eq!(
            *fake.did.borrow(),
            vec![
                "tell p1: write your journal".to_string(),
                "end 7".into(),
                "run p1: claude go".into()
            ]
        );
    }

    #[test]
    fn a_chat_still_at_work_or_deaf_keeps_running() {
        let busy = Fake {
            pid: RefCell::new(Some(7)),
            ..Fake::default()
        };
        assert!(matches!(fresh_start(&busy, &|_| false, &fresh()), Err((_, false))));
        assert!(!busy.did.borrow().iter().any(|did| did.starts_with("end")));
        let deaf = Fake {
            is_deaf: true,
            pid: RefCell::new(Some(7)),
            ..Fake::default()
        };
        assert_eq!(
            fresh_start(&deaf, &|_| true, &fresh()),
            Err(("herdr took no line".into(), false))
        );
        assert!(deaf.did.borrow().is_empty());
    }

    #[test]
    fn the_first_prompt_carries_the_journal_and_the_record() {
        let record = Record {
            issues: vec![
                IssueEntry {
                    number: Some(3),
                    feature: Some("3-a".into()),
                    ..IssueEntry::default()
                },
                IssueEntry {
                    number: Some(4),
                    feature: Some("4-b".into()),
                    is_queued: true,
                    after: vec![12],
                    ..IssueEntry::default()
                },
            ],
            plan: Some(Plan {
                work: vec![3, 4],
                merge: vec![3, 4],
                why: Some("4 builds on 3".into()),
                ..Plan::default()
            }),
            ..Record::default()
        };
        let journal = vec!["2026-10-13 14:02 open: #4 waits for the user's answer on the API".to_string()];
        let prompt = first_prompt("⇶ b", "b", "old-id", Some(0.62), &journal, Some(&record));
        assert!(
            prompt.starts_with("[overview] fresh start: you are the orchestrator of ⇶ b"),
            "{prompt}"
        );
        assert!(prompt.contains("ended at 62 % of its context"), "{prompt}");
        assert!(
            prompt.contains("  2026-10-13 14:02 open: #4 waits for the user's answer on the API\n"),
            "{prompt}"
        );
        assert!(
            prompt.contains("  #3 3-a: at work\n  #4 4-b: queued after !12\n"),
            "{prompt}"
        );
        assert!(
            prompt.contains("  plan: work #3 #4 · merge #3 #4 · 4 builds on 3\n"),
            "{prompt}"
        );
        let command = start_command(Path::new("/r"), "id", "b", "it's");
        assert!(
            command.contains(" && claude --session-id id --name 'orchestrator b'"),
            "{command}"
        );
        assert!(command.ends_with(" 'it'\\''s'"), "{command}");
    }
}
