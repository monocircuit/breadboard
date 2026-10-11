//! An agent moves by a restart of its session (#335): Claude Code keeps a
//! session inside the directory it was resumed from, so `EnterWorktree` out
//! of an orchestration's checkout fails, and a `/rename` and a line typed
//! into a running claude arrive merged. So nothing is typed: the claude that
//! runs the conversation ends, then `claude --resume <id> --name <name>
//! '<line>'` starts it again in the tab `<label>` of its new space, standing
//! in its new directory, the line its first prompt. Never two processes on
//! one session: the resume starts only once the old one is gone.

use super::super::claude::running_claudes;
use super::super::herdr::{self, call, did, id_at, s};
use super::super::layout::tabs::{drop_duplicates, idle_pane_of};
use super::super::proc::{quote, text};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a claude gets to end once asked (SIGTERM).
const END_WAIT: Duration = Duration::from_secs(15);
/// How long a new tab's shell gets to idle.
const SHELL_WAIT: Duration = Duration::from_secs(5);

/// What a restart needs of the machine: herdr's in a run, a fake in the tests.
pub trait Host {
    /// The idle shell of the tab `label` in `workspace`, the tab made in `cwd` when missing.
    fn shell(&self, workspace: &str, label: &str, cwd: &Path) -> Option<String>;
    /// The pid of the claude that runs the conversation `id`, None when none does.
    fn pid_of(&self, id: &str) -> Option<i64>;
    /// The claude `pid` asked to end; whether it is gone within the wait.
    fn end(&self, pid: i64) -> bool;
    /// `command` run in the shell of `pane`.
    fn run(&self, pane: &str, command: &str) -> bool;
    /// The pane closed (its tab with it when it was its last).
    fn close(&self, pane: &str);
}

/// Where and as what the conversation `id` runs again.
pub struct Restart<'a> {
    pub id: &'a str,
    pub workspace: &'a str,
    pub label: &'a str,
    pub cwd: &'a Path,
    /// Its session's name (`--name`).
    pub name: &'a str,
    /// Its first prompt.
    pub prompt: &'a str,
    /// The pane it runs in now: closed once it runs elsewhere.
    pub from: Option<&'a str>,
    /// ` --ide` when nvim's plugin is on for `cwd` (`layout::nvim::ide_flag`), else "".
    pub ide: &'a str,
}

/// The shell line that resumes the conversation.
pub fn resume_command(restart: &Restart) -> String {
    format!(
        "cd {} && claude --resume {} --name {}{} {}",
        quote(&text(restart.cwd)),
        restart.id,
        quote(restart.name),
        restart.ide,
        quote(restart.prompt)
    )
}

/// How a restart failed.
#[derive(Debug, PartialEq)]
pub enum Failed {
    /// Nothing changed: the old claude runs on.
    Before(String),
    /// The old claude ended, the conversation runs nowhere: the command resumes it.
    After { why: String, command: String },
}

/// The conversation ended where it runs and resumed in the tab `label`:
/// Ok(the pane it runs in). The new tab is found before anything ends.
pub fn restart(host: &dyn Host, restart: &Restart) -> Result<String, Failed> {
    let Some(pane) = host.shell(restart.workspace, restart.label, restart.cwd) else {
        return Err(Failed::Before(format!(
            "no idle shell in a tab {} of its space",
            restart.label
        )));
    };
    if let Some(pid) = host.pid_of(restart.id)
        && !host.end(pid)
    {
        return Err(Failed::Before(format!("its claude (pid {pid}) did not end")));
    }
    let command = resume_command(restart);
    if host.pid_of(restart.id).is_some() {
        return Err(Failed::Before("another claude runs its conversation".into()));
    }
    let from = restart.from.filter(|from| *from != pane);
    if host.run(&pane, &command) {
        if let Some(from) = from {
            host.close(from);
        }
        return Ok(pane);
    }
    // Its old shell is free now: there, rather than nowhere.
    if let Some(from) = from.filter(|from| host.run(from, &command)) {
        return Ok(from.to_string());
    }
    Err(Failed::After {
        why: "herdr ran no command".into(),
        command,
    })
}

/// The machine as it is: herdr, the claudes' session files, signals.
pub struct Machine;

impl Host for Machine {
    fn shell(&self, workspace: &str, label: &str, cwd: &Path) -> Option<String> {
        let tabs = herdr::tabs_of(workspace);
        if tabs.iter().any(|tab| s(tab, "label") == label) {
            return idle_pane_of(workspace, label, &tabs);
        }
        let created = call(&[
            "tab",
            "create",
            "--workspace",
            workspace,
            "--cwd",
            &text(cwd),
            "--label",
            label,
            "--no-focus",
        ])?;
        drop_duplicates(workspace, label);
        let pane = id_at(&created, &["root_pane", "pane_id"])?;
        let started = Instant::now();
        while !herdr::is_idle(&pane) {
            if started.elapsed() >= SHELL_WAIT {
                return None;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Some(pane)
    }

    fn pid_of(&self, id: &str) -> Option<i64> {
        running_claudes()
            .into_iter()
            .find(|claude| claude.id == id)
            .map(|claude| claude.pid)
    }

    fn end(&self, pid: i64) -> bool {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: a signal to one process this user runs.
        unsafe { libc::kill(pid, libc::SIGTERM) };
        let started = Instant::now();
        while breadboard_core::lock::is_alive(i64::from(pid)) {
            if started.elapsed() >= END_WAIT {
                return false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        true
    }

    fn run(&self, pane: &str, command: &str) -> bool {
        // A shell line goes only into a shell, never into a running claude.
        herdr::is_idle(pane) && did(&["pane", "run", pane, command])
    }

    fn close(&self, pane: &str) {
        did(&["pane", "close", pane]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A machine behind a fake herdr: what it was asked, in order.
    #[derive(Default)]
    struct Fake {
        has_shell: bool,
        /// The pid that runs the conversation, None when none.
        pid: RefCell<Option<i64>>,
        is_stubborn: bool,
        refused: Vec<&'static str>,
        did: RefCell<Vec<String>>,
    }

    impl Host for Fake {
        fn shell(&self, _: &str, label: &str, _: &Path) -> Option<String> {
            self.did.borrow_mut().push(format!("shell {label}"));
            self.has_shell.then(|| "new".to_string())
        }
        fn pid_of(&self, _: &str) -> Option<i64> {
            *self.pid.borrow()
        }
        fn end(&self, pid: i64) -> bool {
            self.did.borrow_mut().push(format!("end {pid}"));
            if !self.is_stubborn {
                *self.pid.borrow_mut() = None;
            }
            !self.is_stubborn
        }
        fn run(&self, pane: &str, command: &str) -> bool {
            assert!(self.pid.borrow().is_none(), "a second claude on one session");
            if self.refused.contains(&pane) {
                return false;
            }
            self.did.borrow_mut().push(format!("run {pane}: {command}"));
            true
        }
        fn close(&self, pane: &str) {
            self.did.borrow_mut().push(format!("close {pane}"));
        }
    }

    fn fake() -> Fake {
        Fake {
            has_shell: true,
            pid: RefCell::new(Some(41)),
            ..Fake::default()
        }
    }

    fn handover() -> Restart<'static> {
        Restart {
            id: "0123abcd-0000-0000-0000-000000000000",
            workspace: "w5C",
            label: "7-x",
            cwd: Path::new("/r/p-7-x"),
            name: "7-x",
            prompt: "[overview] next issue #7, your name is 7-x now: read .claude/brief.md",
            from: Some("old"),
            ide: "",
        }
    }

    #[test]
    fn ends_the_claude_then_resumes_it_in_its_tab() {
        let fake = fake();
        assert_eq!(restart(&fake, &handover()), Ok("new".into()));
        assert_eq!(
            *fake.did.borrow(),
            vec![
                "shell 7-x".to_string(),
                "end 41".into(),
                "run new: cd '/r/p-7-x' && claude --resume 0123abcd-0000-0000-0000-000000000000 --name '7-x' '[overview] next issue #7, your name is 7-x now: read .claude/brief.md'".into(),
                "close old".into(),
            ]
        );
    }

    #[test]
    fn without_its_tab_nothing_ends() {
        let fake = Fake {
            has_shell: false,
            ..fake()
        };
        assert_eq!(
            restart(&fake, &handover()),
            Err(Failed::Before("no idle shell in a tab 7-x of its space".into()))
        );
        assert_eq!(*fake.did.borrow(), vec!["shell 7-x".to_string()]);
    }

    #[test]
    fn a_claude_that_does_not_end_gets_no_second_one() {
        let fake = Fake {
            is_stubborn: true,
            ..fake()
        };
        assert_eq!(
            restart(&fake, &handover()),
            Err(Failed::Before("its claude (pid 41) did not end".into()))
        );
        assert!(!fake.did.borrow().iter().any(|did| did.starts_with("run")));
    }

    #[test]
    fn a_refused_tab_falls_back_to_its_old_shell() {
        let one = Fake {
            refused: vec!["new"],
            ..fake()
        };
        assert_eq!(restart(&one, &handover()), Ok("old".into()));
        let refused = Fake {
            refused: vec!["new", "old"],
            ..fake()
        };
        let Err(Failed::After { command, .. }) = restart(&refused, &handover()) else {
            panic!("a resume that ran nowhere counted")
        };
        assert!(command.starts_with("cd '/r/p-7-x' && claude --resume "));
    }

    #[test]
    fn a_name_and_a_prompt_are_one_word_each() {
        let command = resume_command(&Restart {
            name: "free a",
            prompt: "it's done",
            ..handover()
        });
        assert!(command.ends_with(" --name 'free a' 'it'\\''s done'"), "{command}");
    }
}
