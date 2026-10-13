//! What the `--herdr` sync made herdr do, in words, for the log under the
//! band (lazygit's command log): each act once, what it did and why, the
//! herdr commands under it word for word. The layout's own report lines
//! (`cli::lifecycle::layout`) say most acts; the writes no line names (the
//! #332 close of a worktree's `overview` tab, the tabs put back in order)
//! are said from the journal of the calls.

use super::herdr::quote;
use crate::cli::lifecycle::herdr::Journal;
use crate::cli::lifecycle::layout::tabs::OVERVIEW;
use breadboard_core::model::Paint;
use breadboard_core::record::{ORCHESTRATOR, SPACE};
use std::collections::HashMap;

/// One thing the sync did, for the log; `key` keeps it one entry while it repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub paint: Paint,
    pub text: String,
    pub detail: Option<String>,
}

impl Logged {
    pub fn new(paint: Paint, text: impl Into<String>, detail: Option<String>) -> Logged {
        Logged {
            paint,
            text: text.into(),
            detail,
        }
    }

    /// What makes it the same act at the next tick: its words and commands.
    pub fn key(&self) -> String {
        format!("sync:{}\u{1f}{}", self.text, self.detail.as_deref().unwrap_or_default())
    }
}

/// The step's report lines and its journal as entries of the log.
/// `spaces`: workspace id → its label, as read before the step.
pub fn said(lines: &[String], journal: &Journal, spaces: &HashMap<String, String>) -> Vec<Logged> {
    let writes: Vec<Write> = journal
        .writes
        .iter()
        .map(|args| Write::of(args, journal, spaces))
        .collect();
    let mut taken = vec![false; writes.len()];
    let mut out = Vec::new();
    for line in lines {
        let space = named_space(line, &writes, spaces);
        let mine: Vec<usize> = (0..writes.len())
            .filter(|at| !taken[*at])
            .filter(|at| match (&space, &writes[*at].kind) {
                (_, Kind::Sort) => line.contains("sorted"),
                // A space's marks are said on their own, not under what closed its tabs.
                (_, Kind::Mark(_)) => false,
                (Some(space), _) => writes[*at].space.as_deref() == Some(space.as_str()),
                (None, _) => false,
            })
            .collect();
        for at in &mine {
            taken[*at] = true;
        }
        let detail = commands(mine.iter().map(|at| &writes[*at]));
        out.push(Logged::new(paint_of(line), words_of(line), detail));
    }
    let left: Vec<&Write> = (0..writes.len())
        .filter(|at| !taken[*at])
        .map(|at| &writes[at])
        .collect();
    out.extend(unsaid(&left));
    out
}

/// A write as the log needs it: its words, the space it acts on, what kind of act.
#[derive(Debug, Clone)]
struct Write {
    args: Vec<String>,
    space: Option<String>,
    kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// A tab closed, by its label.
    Close(String),
    SpaceClose,
    /// A pane moved within its space: the tabs put back in order (`keepOrder`).
    Order,
    /// A pane moved into another space, by that space's label.
    MoveTo(String),
    /// A space made (`workspace create`, `worktree open`).
    Open,
    /// A tab made, by its label.
    TabOpen(String),
    /// A command run in a tab, by the tab's label.
    Run(String),
    /// A tab renamed, to its new label.
    Rename(String),
    /// The orchestration mark of a space (`workspace report-metadata`): the
    /// `⇶ <title>` it shows now, None when it was taken off.
    Mark(Option<String>),
    /// A tab in front.
    Focus,
    /// The spaces of the group sorted (`workspace.move_block`).
    Sort,
    Other,
}

impl Write {
    fn of(args: &[String], journal: &Journal, spaces: &HashMap<String, String>) -> Write {
        let arg = |at: usize| args.get(at).map(String::as_str).unwrap_or_default();
        let flag = |name: &str| {
            args.iter()
                .position(|arg| arg == name)
                .and_then(|at| args.get(at + 1))
                .map(String::as_str)
        };
        // Ids name their space in front: `w4P:t1`, `w46:p2`.
        let space_of_id = |id: &str| spaces.get(id.split(':').next().unwrap_or(id)).cloned();
        let tab = |id: &str| journal.tab_label(id).unwrap_or("a tab").to_string();
        let (space, kind) = match (arg(0), arg(1)) {
            ("socket", _) => (Some(SPACE.to_string()), Kind::Sort),
            ("workspace", "create") | ("worktree", "open") => (flag("--label").map(String::from), Kind::Open),
            ("workspace", "close") => (spaces.get(arg(2)).cloned(), Kind::SpaceClose),
            ("workspace", "report-metadata") => {
                let shown = args
                    .windows(2)
                    .find(|pair| pair[0] == "--token")
                    .and_then(|pair| pair[1].split_once('='))
                    .map(|(_, value)| value.to_string());
                (spaces.get(arg(2)).cloned(), Kind::Mark(shown))
            }
            ("tab", "close") => (space_of_id(arg(2)), Kind::Close(tab(arg(2)))),
            ("tab", "focus") => (space_of_id(arg(2)), Kind::Focus),
            ("tab", "rename") => (space_of_id(arg(2)), Kind::Rename(arg(3).to_string())),
            ("tab", "create") => (
                flag("--workspace").and_then(|id| spaces.get(id).cloned()),
                Kind::TabOpen(flag("--label").unwrap_or("a tab").to_string()),
            ),
            ("pane", "run") => (space_of_id(arg(2)), Kind::Run(tab(arg(2)))),
            ("pane", "move") => {
                let from = space_of_id(arg(2));
                let to = flag("--workspace").and_then(|id| spaces.get(id).cloned());
                match to {
                    Some(to) if Some(&to) != from.as_ref() => (from, Kind::MoveTo(to)),
                    _ => (from, Kind::Order),
                }
            }
            _ => (args.get(2).and_then(|id| space_of_id(id)), Kind::Other),
        };
        Write {
            args: args.to_vec(),
            space,
            kind,
        }
    }

    /// The call word for word: `herdr tab close w4P:t1`.
    fn command(&self) -> String {
        let words: Vec<String> = self.args.iter().map(|arg| quote(arg)).collect();
        format!("herdr {}", words.join(" "))
    }
}

/// The space a line is about: the longest label of a space it names.
fn named_space(line: &str, writes: &[Write], spaces: &HashMap<String, String>) -> Option<String> {
    spaces
        .values()
        .chain(writes.iter().filter_map(|write| write.space.as_ref()))
        .filter(|label| !label.is_empty() && line.contains(label.as_str()))
        .max_by_key(|label| label.len())
        .cloned()
}

/// The commands under an entry, one after the other.
fn commands<'a>(writes: impl Iterator<Item = &'a Write>) -> Option<String> {
    let all: Vec<String> = writes.map(Write::command).collect();
    (!all.is_empty()).then(|| all.join("; "))
}

/// Kept or left undone in yellow, failed in red, done in green.
fn paint_of(line: &str) -> Paint {
    if line.contains("not set up") || line.contains("opened empty") {
        Paint::Red
    } else if line.starts_with("kept") {
        Paint::Yellow
    } else {
        Paint::Green
    }
}

/// A report line of the layout as the log says it: the verb first, the
/// reason after ` · ` (the band shows what stands before it).
fn words_of(line: &str) -> String {
    if let Some(label) = line.strip_prefix("space ") {
        if let Some(feature) = label.strip_suffix(" opened") {
            return format!("opened the space {feature} · a worktree without one");
        }
        if let Some(feature) = label.strip_suffix(" moved into its group") {
            return format!("moved the space {feature} into the group of {SPACE}");
        }
    }
    if let Some(rest) = line.strip_prefix(&format!("spaces of {SPACE} sorted by orchestration")) {
        return format!("sorted the spaces of {SPACE} by orchestration{rest} · #255");
    }
    if let Some((feature, changes)) = line.split_once(": ")
        && !changes.is_empty()
        && changes
            .split(' ')
            .all(|word| word.starts_with('+') || word.starts_with('-'))
    {
        let names =
            |sign: char| -> Vec<&str> { changes.split(' ').filter_map(|word| word.strip_prefix(sign)).collect() };
        let mut said = Vec::new();
        for (sign, verb, why) in [('+', "opened", "its stack runs"), ('-', "closed", "its stack is down")] {
            let names = names(sign);
            if !names.is_empty() {
                said.push(format!(
                    "{verb} the tab{} {} of {feature} · {why}",
                    plural(names.len()),
                    names.join(", ")
                ));
            }
        }
        return said.join("; ");
    }
    // A release of `pruneSpaces` (in a tick or inside the layout) does not say why.
    let is_release = ["closed the space ", "kept a tab of ", "kept "]
        .iter()
        .any(|head| line.starts_with(head))
        || line.starts_with("closed ") && line.contains(" tab");
    if is_release && !line.contains("gone") && !line.contains("kept the checkout") {
        return format!("{line} · its worktree is gone");
    }
    line.to_string()
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// The writes no report line named, said by kind: the #332 closes of
/// several spaces as one entry, the tabs put in order as one.
fn unsaid(writes: &[&Write]) -> Vec<Logged> {
    let mut out = Vec::new();
    let spaces_of = |pick: &dyn Fn(&Write) -> bool| -> (Vec<String>, Vec<&Write>) {
        let picked: Vec<&Write> = writes.iter().copied().filter(|write| pick(write)).collect();
        let mut names: Vec<String> = Vec::new();
        for write in &picked {
            let name = write.space.clone().unwrap_or_else(|| "a space".into());
            if !names.contains(&name) {
                names.push(name);
            }
        }
        (names, picked)
    };
    let is_worktree = |space: &Option<String>| {
        space
            .as_deref()
            .is_some_and(|space| space != SPACE && !space.starts_with(ORCHESTRATOR))
    };
    // Spaces opened here: their first tab, its command and its order are part of the opening.
    let (opened, opens) = spaces_of(&|write| write.kind == Kind::Open);
    for (name, write) in opened.iter().zip(&opens) {
        let own = writes
            .iter()
            .copied()
            .filter(|other| other.space.as_ref() == Some(name) && other.kind != Kind::Open);
        out.push(Logged::new(
            Paint::Green,
            format!("opened the space {name}"),
            commands(std::iter::once(*write).chain(own)),
        ));
    }
    let rest: Vec<&Write> = writes
        .iter()
        .copied()
        .filter(|write| !write.space.as_ref().is_some_and(|space| opened.contains(space)))
        .collect();
    let pick = |kind: &dyn Fn(&Write) -> bool| -> (Vec<String>, Vec<&Write>) {
        let picked: Vec<&Write> = rest.iter().copied().filter(|write| kind(write)).collect();
        let mut names: Vec<String> = Vec::new();
        for write in &picked {
            let name = write.space.clone().unwrap_or_else(|| "a space".into());
            if !names.contains(&name) {
                names.push(name);
            }
        }
        (names, picked)
    };
    let is_332 = |write: &Write| write.kind == Kind::Close(OVERVIEW.into()) && is_worktree(&write.space);
    let (names, picked) = pick(&is_332);
    if !picked.is_empty() {
        out.push(Logged::new(
            Paint::Green,
            format!(
                "closed the {OVERVIEW} tab of {} · #332: the overview of its orchestration shows it",
                names.join(", ")
            ),
            commands(picked.into_iter()),
        ));
    }
    // A space's tabs put in order: its moves, the renames and focus that go with them.
    let (ordered, _) = pick(&|write| write.kind == Kind::Order);
    let in_order = |write: &Write| {
        write.space.as_ref().is_some_and(|space| ordered.contains(space))
            && matches!(write.kind, Kind::Order | Kind::Rename(_) | Kind::Focus)
    };
    let (names, picked) = pick(&in_order);
    if !picked.is_empty() {
        out.push(Logged::new(
            Paint::Green,
            format!("put the tabs of {} back in order", names.join(", ")),
            commands(picked.into_iter()),
        ));
    }
    // The rest one by one, by space.
    for write in rest
        .iter()
        .copied()
        .filter(|write| !is_332(write) && !in_order(write) && write.kind != Kind::Open)
    {
        let of = write.space.as_deref().unwrap_or("a space");
        let text = match &write.kind {
            Kind::Close(label) => format!("closed the tab {label} of {of}"),
            Kind::SpaceClose => format!("closed the space {of}"),
            Kind::MoveTo(to) => format!("moved a tab of {of} into the space {to}"),
            Kind::TabOpen(label) => format!("opened the tab {label} in {of}"),
            Kind::Run(label) => format!("ran a command in the tab {label} of {of}"),
            Kind::Rename(label) => format!("named a tab of {of} {label}"),
            Kind::Focus => format!("brought a tab of {of} to the front"),
            Kind::Mark(Some(shown)) => format!("marked the space {of} {shown} · it belongs to that orchestration"),
            Kind::Mark(None) => format!("took the orchestration's mark off the space {of} · it belongs to none now"),
            Kind::Sort => format!("sorted the spaces of {SPACE} by orchestration · #255"),
            _ => format!(
                "herdr {} in {of}",
                write.args.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
            ),
        };
        out.push(Logged::new(Paint::Green, text, Some(write.command())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(writes: &[&str], tabs: &[(&str, &str)]) -> Journal {
        Journal {
            writes: writes
                .iter()
                .map(|write| write.split(' ').map(String::from).collect())
                .collect(),
            tabs: tabs
                .iter()
                .map(|(id, label)| (id.to_string(), label.to_string()))
                .collect(),
            panes: HashMap::new(),
        }
    }

    fn spaces() -> HashMap<String, String> {
        [
            ("w1", SPACE.get().as_str()),
            ("w4P", "299-import-vita"),
            ("w46", "292-every-agent"),
            ("w48", "307-rust"),
        ]
        .iter()
        .map(|(id, label)| (id.to_string(), label.to_string()))
        .collect()
    }

    #[test]
    fn says_a_prune_with_its_commands_and_why() {
        let journal = journal(&["tab close w4P:t1", "tab close w4P:t2"], &[]);
        let lines = vec!["closed 2 tabs of 299-import-vita, kept a tab (an agent works there)".to_string()];
        let said = said(&lines, &journal, &spaces());
        assert_eq!(said.len(), 1);
        assert_eq!(
            said[0].text,
            "closed 2 tabs of 299-import-vita, kept a tab (an agent works there) · its worktree is gone"
        );
        assert_eq!(
            said[0].detail.as_deref(),
            Some("herdr tab close w4P:t1; herdr tab close w4P:t2")
        );
        assert_eq!(said[0].paint, Paint::Green);
    }

    #[test]
    fn says_what_no_line_names_by_kind() {
        let journal = journal(
            &[
                "tab close w46:t2",
                "pane move w46:p2 --workspace w46 --new-tab",
                "tab close w48:t2",
                "pane move w48:p2 --workspace w48 --new-tab",
            ],
            &[("w46:t2", "overview"), ("w48:t2", "overview")],
        );
        let said = said(&[], &journal, &spaces());
        let texts: Vec<&str> = said.iter().map(|logged| logged.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "closed the overview tab of 292-every-agent, 307-rust · #332: the overview of its orchestration shows it",
                "put the tabs of 292-every-agent, 307-rust back in order",
            ]
        );
        assert_eq!(
            said[0].detail.as_deref(),
            Some("herdr tab close w46:t2; herdr tab close w48:t2")
        );
    }

    #[test]
    fn says_a_mark_put_on_or_taken_off() {
        let mut journal = journal(&["workspace report-metadata w4P --source s --clear-token orch1"], &[]);
        let put = [
            "workspace",
            "report-metadata",
            "w48",
            "--source",
            "s",
            "--token",
            "orch0=⇶ bb",
        ];
        journal.writes.push(put.iter().map(|arg| arg.to_string()).collect());
        let texts: Vec<String> = said(&[], &journal, &spaces())
            .into_iter()
            .map(|said| said.text)
            .collect();
        assert_eq!(
            texts,
            [
                "took the orchestration's mark off the space 299-import-vita · it belongs to none now",
                "marked the space 307-rust ⇶ bb · it belongs to that orchestration",
            ]
        );
    }

    #[test]
    fn a_new_space_is_one_entry_with_its_tabs() {
        let journal = journal(
            &[
                "worktree open --workspace w1 --path /r/x --label 310-new --no-focus",
                "tab rename w9:t1 root",
            ],
            &[],
        );
        let mut spaces = spaces();
        spaces.insert("w9".into(), "310-new".into());
        let said = said(&["space 310-new opened".to_string()], &journal, &spaces);
        assert_eq!(said.len(), 1);
        assert_eq!(said[0].text, "opened the space 310-new · a worktree without one");
        assert!(
            said[0]
                .detail
                .as_deref()
                .unwrap()
                .ends_with("herdr tab rename w9:t1 root")
        );
    }

    #[test]
    fn says_a_stacks_tabs_with_the_verb() {
        assert_eq!(
            words_of("307-rust: +postgres +dev"),
            "opened the tabs postgres, dev of 307-rust · its stack runs"
        );
        assert_eq!(
            words_of("307-rust: -dev"),
            "closed the tab dev of 307-rust · its stack is down"
        );
        assert_eq!(paint_of("kept a tab of 3-x (an agent works there)"), Paint::Yellow);
    }

    #[test]
    fn an_act_is_the_same_at_the_next_tick() {
        let one = Logged::new(Paint::Green, "closed x", Some("herdr tab close w1:t1".into()));
        assert_eq!(one.key(), one.clone().key());
        assert_ne!(one.key(), Logged::new(Paint::Green, "closed x", None).key());
    }
}
