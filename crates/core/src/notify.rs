//! Desktop notifications of the repo's events (#449): an issue created, an
//! MR opened, merged, marked ready, its pipeline failed, a conflict, a
//! `decide` posted. The daemon reads GitLab and the actions of the repo on
//! its tick, and what changed since the last read (`events_of`) goes out
//! through `notify-send`, once: what was seen is kept in
//! `<main>/.claude/ci/notified.json`, so a restart repeats nothing, and one
//! daemon per checkout sends (`marker_file`). The first read only learns
//! what is there. Which events send is the settings panel's (core's
//! `settings`), the team's default breadboard.toml's `[notifications]`.

use crate::actions::{Action, actions_file, actions_of};
use crate::ci::{dir_of, read_or_default};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// An event that sends a notification: its key (breadboard.toml's
/// `events`, the setting `notify_<key>`), its label in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub key: &'static str,
    pub label: &'static str,
}

pub const ISSUE_CREATED: &str = "issue_created";
pub const MR_OPENED: &str = "mr_opened";
pub const MR_MERGED: &str = "mr_merged";
pub const MR_READY: &str = "mr_ready";
pub const PIPELINE_FAILED: &str = "pipeline_failed";
pub const MR_CONFLICT: &str = "mr_conflict";
pub const DECIDE: &str = "decide";

/// Every event, in the panel's order.
pub const EVENTS: [Event; 7] = [
    Event {
        key: ISSUE_CREATED,
        label: "issue created",
    },
    Event {
        key: MR_OPENED,
        label: "MR opened",
    },
    Event {
        key: MR_MERGED,
        label: "MR merged",
    },
    Event {
        key: MR_READY,
        label: "MR ready",
    },
    Event {
        key: PIPELINE_FAILED,
        label: "pipeline failed",
    },
    Event {
        key: MR_CONFLICT,
        label: "MR conflict",
    },
    Event {
        key: DECIDE,
        label: "asks you (decide)",
    },
];

/// The event of `key`.
pub fn event_of(key: &str) -> Option<&'static Event> {
    EVENTS.iter().find(|event| event.key == key)
}

/// The program that shows a desktop notification.
pub const PROGRAM: &str = "notify-send";

/// The app name every notification carries.
pub const APP: &str = "breadboard";

/// What is seen of the repo now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Now {
    /// The newest issues, number and title.
    pub issues: Vec<(i64, String)>,
    pub open: Vec<OpenMr>,
    /// The newest merged MRs.
    pub merged: Vec<MergedMr>,
    /// The open `decide` actions.
    pub decides: Vec<Asked>,
}

/// An open MR as a notification needs it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenMr {
    pub iid: i64,
    pub title: String,
    pub target: String,
    pub state: MrState,
}

/// What of an open MR a notification follows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MrState {
    pub is_draft: bool,
    pub is_conflict: bool,
    /// Its newest pipeline when it failed: the id.
    pub failed: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergedMr {
    pub iid: i64,
    pub title: String,
    pub target: String,
}

/// An open `decide`: who asks (a worktree's feature, `⇶ <orchestration>`), its key, its question.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Asked {
    pub key: String,
    pub who: String,
    pub text: String,
}

/// What the last read saw, kept between reads (and restarts).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Seen {
    /// The newest issue's number.
    pub issue: i64,
    /// The newest MR's number.
    pub mr: i64,
    pub open: BTreeMap<i64, MrState>,
    pub merged: BTreeSet<i64>,
    pub decides: BTreeSet<String>,
}

/// One notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub event: &'static str,
    pub title: String,
    pub body: String,
}

/// The notifications between `seen` and `now`, and what is seen from now
/// on. None seen (the first read): none, it only learns.
pub fn events_of(seen: Option<&Seen>, now: &Now) -> (Vec<Note>, Seen) {
    let next = Seen {
        issue: now
            .issues
            .iter()
            .map(|(iid, _)| *iid)
            .max()
            .unwrap_or(0)
            .max(seen.map_or(0, |seen| seen.issue)),
        mr: now
            .open
            .iter()
            .map(|mr| mr.iid)
            .chain(now.merged.iter().map(|mr| mr.iid))
            .max()
            .unwrap_or(0)
            .max(seen.map_or(0, |seen| seen.mr)),
        open: now.open.iter().map(|mr| (mr.iid, mr.state.clone())).collect(),
        merged: now.merged.iter().map(|mr| mr.iid).collect(),
        decides: now.decides.iter().map(|asked| asked.key.clone()).collect(),
    };
    let Some(seen) = seen else {
        return (Vec::new(), next);
    };
    let mut notes = Vec::new();
    let mut note = |event: &'static str, title: String, body: String| notes.push(Note { event, title, body });
    for (iid, title) in now.issues.iter().filter(|(iid, _)| *iid > seen.issue) {
        note(ISSUE_CREATED, format!("#{iid} {title}"), "issue created".into());
    }
    for mr in &now.open {
        let title = format!("!{} {}", mr.iid, mr.title);
        let before = match seen.open.get(&mr.iid) {
            Some(before) => before.clone(),
            None if mr.iid > seen.mr => {
                note(MR_OPENED, title.clone(), format!("MR opened into {}", mr.target));
                MrState {
                    is_draft: true,
                    ..Default::default()
                }
            }
            // Open before, out of the list since (a list of more than it holds): only what it is now counts.
            None => mr.state.clone(),
        };
        // A new MR opened ready is no `ready` of its own: its `opened` says it.
        if before.is_draft && !mr.state.is_draft && seen.open.contains_key(&mr.iid) {
            note(MR_READY, title.clone(), "marked ready".into());
        }
        if mr.state.failed.is_some() && mr.state.failed != before.failed {
            note(PIPELINE_FAILED, title.clone(), "its pipeline failed".into());
        }
        if mr.state.is_conflict && !before.is_conflict {
            note(MR_CONFLICT, title, format!("has a conflict with {}", mr.target));
        }
    }
    for mr in now.merged.iter().filter(|mr| !seen.merged.contains(&mr.iid)) {
        // A merged MR never seen open and older than the newest seen fell into the list from further down.
        if !seen.open.contains_key(&mr.iid) && mr.iid <= seen.mr {
            continue;
        }
        note(
            MR_MERGED,
            format!("!{} {}", mr.iid, mr.title),
            format!("merged into {}", mr.target),
        );
    }
    for asked in now.decides.iter().filter(|asked| !seen.decides.contains(&asked.key)) {
        note(DECIDE, asked.who.clone(), format!("asks you: {}", asked.text));
    }
    (notes, next)
}

/// The open `decide` actions of the repo: its worktrees' and its orchestrators'.
pub fn decides_of(main: &Path) -> Vec<Asked> {
    let open = |action: &&Action| action.kind == "decide" && action.answer.is_none();
    let first_line = |text: &str| text.lines().next().unwrap_or_default().to_string();
    let mut out = Vec::new();
    for tree in crate::git::worktrees(main).iter().filter(|tree| tree.dir != main) {
        let who = crate::git::feature_of(main, &tree.dir);
        for action in actions_of(&actions_file(&tree.dir)).iter().filter(open) {
            out.push(Asked {
                key: format!("{}#{}", tree.dir.display(), action.id),
                who: who.clone(),
                text: first_line(&action.text),
            });
        }
    }
    for name in crate::record::recorded(main) {
        let file = crate::record::record_file(main, &name);
        let raw = crate::json::read(&file).unwrap_or_default();
        for action in crate::actions::actions_in(&raw).iter().filter(open) {
            out.push(Asked {
                key: format!("⇶{name}#{}", action.id),
                who: format!("⇶ {name}"),
                text: first_line(&action.text),
            });
        }
    }
    out
}

pub fn seen_file(main: &Path) -> PathBuf {
    dir_of(main).join("notified.json")
}

/// The pid file of the daemon that notifies about `main`: one per checkout.
pub fn marker_file(main: &Path) -> PathBuf {
    dir_of(main).join("notify.pid")
}

/// What the last read saw, None before the first (or after the switch was off).
pub fn read_seen(main: &Path) -> Option<Seen> {
    seen_file(main).exists().then(|| read_or_default(&seen_file(main)))
}

/// `notify-send` on PATH: None without one.
pub fn program() -> Option<PathBuf> {
    crate::local::executable_on(&std::env::var_os("PATH").unwrap_or_default(), PROGRAM)
}

/// The notice when desktop notifications are on but nothing can show them.
pub const MISSING: &str = "desktop notifications are on, but notify-send isn't on this machine: none are sent";

/// One notification shown: `notify-send --app-name=breadboard <title> <body>`; true when it ran.
pub fn send(program: &Path, note: &Note) -> bool {
    Command::new(program)
        .arg(format!("--app-name={APP}"))
        .arg(&note.title)
        .arg(&note.body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(iid: i64, is_draft: bool, is_conflict: bool, failed: Option<i64>) -> OpenMr {
        OpenMr {
            iid,
            title: format!("mr {iid}"),
            target: "develop".into(),
            state: MrState {
                is_draft,
                is_conflict,
                failed,
            },
        }
    }

    fn merged(iid: i64) -> MergedMr {
        MergedMr {
            iid,
            title: format!("mr {iid}"),
            target: "develop".into(),
        }
    }

    fn kinds(notes: &[Note]) -> Vec<(&str, String)> {
        notes.iter().map(|note| (note.event, note.title.clone())).collect()
    }

    #[test]
    fn the_first_read_only_learns() {
        let now = Now {
            issues: vec![(10, "a".into())],
            open: vec![open(5, true, true, Some(3))],
            merged: vec![merged(4)],
            decides: vec![Asked {
                key: "k".into(),
                ..Default::default()
            }],
        };
        let (notes, seen) = events_of(None, &now);
        assert!(notes.is_empty());
        assert_eq!((seen.issue, seen.mr), (10, 5));
        // The same again: nothing.
        let (notes, again) = events_of(Some(&seen), &now);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(again, seen);
    }

    #[test]
    fn each_change_is_one_notification_once() {
        let before = Now {
            issues: vec![(10, "a".into())],
            open: vec![open(5, true, false, None), open(6, false, false, None)],
            merged: vec![merged(4)],
            decides: Vec::new(),
        };
        let (_, seen) = events_of(None, &before);
        let now = Now {
            issues: vec![(12, "new one".into()), (11, "other".into()), (10, "a".into())],
            open: vec![
                open(5, false, false, Some(9)),
                open(7, true, false, None),
                open(8, false, true, None),
            ],
            merged: vec![merged(6), merged(4)],
            decides: vec![Asked {
                key: "/w#1".into(),
                who: "449-x".into(),
                text: "which?".into(),
            }],
        };
        let (notes, seen) = events_of(Some(&seen), &now);
        assert_eq!(
            kinds(&notes),
            vec![
                (ISSUE_CREATED, "#12 new one".into()),
                (ISSUE_CREATED, "#11 other".into()),
                (MR_READY, "!5 mr 5".into()),
                (PIPELINE_FAILED, "!5 mr 5".into()),
                (MR_OPENED, "!7 mr 7".into()),
                (MR_OPENED, "!8 mr 8".into()),
                (MR_CONFLICT, "!8 mr 8".into()),
                (MR_MERGED, "!6 mr 6".into()),
                (DECIDE, "449-x".into()),
            ]
        );
        assert_eq!(notes[0].body, "issue created");
        assert_eq!(notes[7].body, "merged into develop");
        assert_eq!(notes[8].body, "asks you: which?");
        // Read again: nothing repeats.
        let (notes, _) = events_of(Some(&seen), &now);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_new_failed_pipeline_notifies_again_an_old_merge_never() {
        let (_, seen) = events_of(
            None,
            &Now {
                open: vec![open(5, false, false, Some(1))],
                merged: (1..=3).map(merged).collect(),
                ..Default::default()
            },
        );
        let now = Now {
            open: vec![open(5, false, false, Some(2))],
            // Out of the newest merged earlier, back in: it is old.
            merged: vec![merged(3), merged(2), merged(1), merged(0)],
            ..Default::default()
        };
        let (notes, _) = events_of(Some(&seen), &now);
        assert_eq!(kinds(&notes), vec![(PIPELINE_FAILED, "!5 mr 5".into())]);
    }

    #[test]
    fn an_mr_opened_and_merged_between_two_reads_says_merged() {
        let (_, seen) = events_of(
            None,
            &Now {
                merged: vec![merged(4)],
                ..Default::default()
            },
        );
        let now = Now {
            merged: vec![merged(9), merged(4)],
            ..Default::default()
        };
        assert_eq!(
            kinds(&events_of(Some(&seen), &now).0),
            vec![(MR_MERGED, "!9 mr 9".into())]
        );
    }
}
