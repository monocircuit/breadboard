//! What each view draws, derived from what was read (disk, git, herdr,
//! GitLab): plain structs, no terminal. The logic of src/overview/*.mjs,
//! ported with its tests. Each section of a view is derived on its own
//! (#306): one that fails costs that section, which says so in its place.

pub mod agent_state;
pub mod auto_actions;
pub mod boxes;
pub mod change;
pub mod clean_up;
pub mod develop;
pub mod diff;
pub mod display_names;
pub mod everything;
pub mod files;
pub mod free_agents;
pub mod handovers;
pub mod history;
pub mod memory;
pub mod merge;
pub mod merge_order;
pub mod merge_turn;
pub mod mr_blockers;
pub mod next;
pub mod numbering;
pub mod orchestration;
pub mod order;
pub mod resolve;
pub mod tree;
pub mod try_state;
pub mod waiting;
pub mod workers;
pub mod worktree;

use crate::actions::{Action, Change};
use crate::check::CheckState;
use crate::conversations::{Claude, Running};
use crate::record::{IssueEntry, Origin};
use crate::wait::Wait;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

/// The colour of a piece of text, as the Node overview paints it; the view
/// maps it to its theme. `Wait` is whom an agent waits for (herdr-tones.mjs
/// `paintOf`), `State` herdr's state of an agent (`toneOf`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Paint {
    #[default]
    Plain,
    Dim,
    Bold,
    Cyan,
    Green,
    Yellow,
    Red,
    /// The pink of herdr's sidebar for a worktree's name (names.mjs).
    Pink,
    Wait(Tone),
    State(String),
}

/// Whom an agent waits for (agent-state.mjs): each in a colour of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// At work: yellow.
    Work,
    /// GitLab (the pipeline, another MR): blue.
    Gitlab,
    /// You: red.
    You,
    /// Nothing to do: teal.
    Free,
}

/// A piece of a line: its text, its paint, the page a click on it opens.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub paint: Paint,
    pub url: Option<String>,
}

impl Span {
    pub fn new(text: impl Into<String>, paint: Paint) -> Span {
        Span {
            text: text.into(),
            paint,
            url: None,
        }
    }
    pub fn plain(text: impl Into<String>) -> Span {
        Span::new(text, Paint::Plain)
    }
    pub fn dim(text: impl Into<String>) -> Span {
        Span::new(text, Paint::Dim)
    }
    pub fn link(text: impl Into<String>, url: Option<&str>) -> Span {
        Span {
            text: text.into(),
            paint: Paint::Plain,
            url: url.filter(|url| !url.is_empty()).map(String::from),
        }
    }
}

/// A fact: spans one after the other (`#296 open`), facts are parted by ` · `.
pub type Fact = Vec<Span>;

/// The plain text of spans, as a person reads them.
pub fn text_of(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

/// A section's derivation, or why it failed in its place (#306).
pub type Section<T> = Result<T, String>;

/// `derive()` as a section: a panic in it is that section's error, not the view's.
pub fn section<T>(name: &str, derive: impl FnOnce() -> T) -> Section<T> {
    catch_unwind(AssertUnwindSafe(derive)).map_err(|panic| {
        let why = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
            .unwrap_or_else(|| "it panicked".into());
        format!("{name} failed to draw: {why}")
    })
}

/// What docker said of the stacks when a worktree's was read (#331, #332).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Docker {
    /// It answered: no `stack` is a stack that is down.
    #[default]
    Answered,
    /// Not yet (a slow docker, another process reading it): the stack is unknown, not down.
    NotYet,
    /// It didn't answer in time: unknown too.
    Silent,
}

/// A worktree as the views read it (read.mjs `readLocal`, view.mjs).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tree {
    pub dir: PathBuf,
    /// Its feature, `292-x`; the main checkout's folder name for it.
    pub name: String,
    /// `(detached)` without a branch.
    pub branch: String,
    /// The orchestration that started it.
    pub origin: Option<Origin>,
    /// The cwd of the overview is in it.
    pub is_here: bool,
    pub is_main: bool,
    pub has_env: bool,
    pub env: BTreeMap<String, String>,
    /// The offset of its own stack's ports, 0 for the shared one.
    pub offset: i64,
    pub has_own_port: bool,
    /// Its app answers.
    pub is_up: bool,
    /// Its compose project, `vita` for the shared one.
    pub project: String,
    /// docker's status of its project, None while it is down or unknown.
    pub stack: Option<String>,
    /// Whether docker answered when `stack` was set (#331: it may be slow to).
    pub docker: Docker,
    /// When docker's answer `stack` comes from was read (ms), 0 without one:
    /// a stack started after it is not told yet.
    pub docker_at: i64,
    /// Files changed, new or deleted.
    pub changed: usize,
    /// Commits develop doesn't have.
    pub ahead: u64,
    /// Whether wt:check passed on its HEAD (#298).
    pub checked: CheckState,
    // What an orchestration's read adds (view.mjs `readView`).
    /// The conversation bound to its space.
    pub claude: Option<Claude>,
    /// claudes that stand in it without being its agent (#252).
    pub left: Vec<Running>,
    pub actions: Vec<Action>,
    /// The tries answered and delivered: a `broken` holds the merge (#280).
    pub answered: Vec<Action>,
    /// What its work touches, for the merge order.
    pub files: BTreeSet<String>,
    /// Of `files`, the ones develop changed too since the merge base (#332).
    pub on_develop: Vec<String>,
    /// The files a `git merge origin/develop` would stop at (#332, DEVELOP's
    /// `git merge-tree`), asked only where `on_develop` has any: empty, it
    /// merges cleanly; None where git can't tell or nothing was asked.
    pub conflicts: Option<Vec<String>>,
    /// The migrations its database lacks (#332, STACK's line): an
    /// orchestration's read asks where its database runs (`develop::migrations_held`,
    /// apart from git's cached facts); None without psql or a journal.
    pub migrations: Option<develop::Migrations>,
    /// When `migrations` was asked (ms), 0 never: a `pnpm pg:migrate` that ended after it isn't in it.
    pub migrations_at: i64,
    /// When its facts were read (ms): an act that ended after it isn't in them yet.
    pub read_at: i64,
    /// What the commits of its branch touched (git.rs `branch_files`): what its agent knows.
    pub branch_files: Vec<String>,
    /// Commits of develop it lacks.
    pub behind: u64,
    /// Commits develop doesn't have by patch (`git cherry`).
    pub own_commits: usize,
    pub wait: Option<Wait>,
    /// Its MR's merge turn as the orchestration's overview left it, for herdr's sidebar (#344).
    pub merge_turn: Option<merge_turn::TurnRecord>,
    pub change: Option<Change>,
    /// The commit of `origin/<branch>`.
    pub pushed: Option<String>,
    pub unpushed: u64,
    pub unpushed_at: Option<i64>,
    pub has_remote_branch: bool,
    /// The automatic lines that got in to its agent (`.claude/overview-sent.json`).
    pub sent: BTreeSet<String>,
    /// The tool call its agent's last answer ends with, unanswered (auto-actions.mjs `pendingOf`).
    pub pending: Option<String>,
}

/// An issue of an orchestration with its worktree while that is there
/// (gitlab.mjs `issuesOf`): the record's entries, and worktrees started for
/// it by hand.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Entry {
    pub number: Option<i64>,
    pub title: Option<String>,
    pub feature: String,
    pub branch: String,
    /// Why it got no worktree (skipped, failed).
    pub why: Option<String>,
    pub removed_at: Option<String>,
    pub merged_at: Option<String>,
    /// Waits for an agent (`--queue`, #217).
    pub is_queued: bool,
    pub queued_at: Option<String>,
    /// Why it is back in the queue (#272).
    pub requeued: Option<String>,
    /// The MRs it starts after (#272).
    pub after: Vec<i64>,
    pub tree: Option<Tree>,
}

/// The issues of an orchestration, each with its worktree while that is there,
/// by number; a worktree the record doesn't list is one of its own.
pub fn issues_of(record: &[IssueEntry], trees: &[Tree]) -> Vec<Entry> {
    let mut used = vec![false; trees.len()];
    let mut issues: Vec<Entry> = record
        .iter()
        .map(|issue| {
            let branch = issue.branch.clone().unwrap_or_default();
            let at = trees
                .iter()
                .position(|tree| !branch.is_empty() && tree.branch == branch);
            if let Some(at) = at {
                used[at] = true;
            }
            Entry {
                number: issue.number,
                title: issue.title.clone(),
                feature: issue.feature.clone().unwrap_or_default(),
                branch,
                why: issue.why.clone(),
                removed_at: issue.removed_at.clone(),
                merged_at: issue.merged_at.clone(),
                is_queued: issue.is_queued,
                queued_at: issue.queued_at.clone(),
                requeued: issue.requeued.clone(),
                after: issue.after.clone(),
                tree: at.map(|at| trees[at].clone()),
            }
        })
        .collect();
    for (tree, _) in trees.iter().zip(&used).filter(|(_, is_used)| !**is_used) {
        issues.push(Entry {
            number: crate::gitlab::number_of(&tree.branch),
            feature: tree.name.clone(),
            branch: tree.branch.clone(),
            tree: Some(tree.clone()),
            ..Default::default()
        });
    }
    issues.sort_by_key(|issue| issue.number.unwrap_or(i64::MAX));
    issues
}

/// `3 commits`, `1 commit`.
pub fn counted(count: u64, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_that_panics_says_so_in_its_place() {
        assert_eq!(section("NEXT", || 1), Ok(1));
        let failed: Section<i32> = section("ACTIONS", || panic!("no row {}", 3));
        assert_eq!(failed, Err("ACTIONS failed to draw: no row 3".into()));
        assert_eq!(counted(1, "commit"), "1 commit");
        assert_eq!(text_of(&[Span::plain("#296"), Span::dim(" open")]), "#296 open");
    }
}
