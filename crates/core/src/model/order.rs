//! Which of an orchestration's actions need the user and in which order
//! (order.mjs): ACTIONS, what needs a thought, by what blocks most; UP NEXT,
//! the automatic follow-ups that need a click, by merge order. And the item
//! itself: an action as a row, with what a click on it does (`Go`), which the
//! writing side carries out.

use super::try_state::TryState;
use crate::actions::Action;
use std::path::PathBuf;

/// Where `number` stands in `order` (from 0), after every listed one when it isn't.
pub fn place_in(order: &[i64], number: Option<i64>) -> i64 {
    number
        .and_then(|number| order.iter().position(|known| *known == number))
        .map(|at| at as i64)
        .unwrap_or(i64::MAX)
}

/// The automatic follow-ups: a click, not a thought, so they wait under UP NEXT.
pub const FOLLOW_UPS: [&str; 5] = ["clean up", "pipeline", "end", "hand over", "start"];
/// The kinds of ACTIONS by what blocks most: an agent that stands still, work
/// only a worktree has, then the merges, then what only looks.
const BLOCKS: [&str; 10] = [
    "permission",
    "waits",
    "decide",
    "plan",
    "half-merged",
    "push",
    "merge",
    "try",
    "browser-check",
    "look",
];

fn block_of(kind: &str) -> usize {
    BLOCKS.iter().position(|known| *known == kind).unwrap_or(BLOCKS.len())
}

/// Where a line for an agent goes: a worktree's agent, or the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Dir(PathBuf),
    Orchestration(String),
}

/// What a merge row offers: its MR and what its two clicks need.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeOffer {
    pub iid: i64,
    pub url: String,
    pub is_draft: bool,
    /// A posted merge (or a ready draft's) marks the draft ready first.
    pub can_mark_ready: bool,
    /// What keeps two clicks from merging it; none: they merge.
    pub blockers: Vec<String>,
    /// How many checks wait for the deployment.
    pub to_check: usize,
    /// The open MRs after it in the merge order, by issue number: told after the merge.
    pub others: Vec<i64>,
}

/// What a click on an item does; the writing side carries it out.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Go {
    #[default]
    Nothing,
    /// Two clicks merge (merge.mjs `clickMerge`); `action` the posted merge it ticks off.
    Merge {
        branch: String,
        action: Option<(PathBuf, i64)>,
    },
    /// Opens the spot of a try (try.mjs `clickTry`), or the page of its failed deployment.
    Try { dir: Option<PathBuf>, id: i64 },
    /// The worktree's space in front.
    FocusSpace(PathBuf),
    /// The agent's tab in front.
    FocusAgent(PathBuf),
    /// A page in the browser, in the tab group of a worktree when there is one.
    Show { url: String, dir: Option<PathBuf> },
    /// One line to the agent of `dir` once (auto-actions.mjs `sendOnce`), else `fallback`.
    Send {
        dir: PathBuf,
        key: String,
        line: String,
        fallback: Box<Go>,
    },
    /// Two clicks run wt:remove of the worktree.
    Remove(PathBuf),
    /// `wt:orchestrate <name> <number> --to <to>`.
    HandOver {
        number: i64,
        to: String,
        dir: Option<PathBuf>,
    },
    /// `wt:orchestrate <name> <number> --now`: a fresh agent.
    StartFresh(i64),
    /// Two clicks end the orchestration.
    End,
}

/// An action as a row of ACTIONS, UP NEXT or NEXT (render-orchestration.mjs
/// `actionsOf`): posted (`action` holds it) or automatic.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    /// The same for as long as its cause lasts: a new key notifies.
    pub key: String,
    pub kind: String,
    pub text: String,
    /// ISO time, "" for none.
    pub at: String,
    /// The stored action of a posted one: its id, options, answer.
    pub action: Option<Action>,
    /// Whose it is: a feature, `⇶ orchestrator`, an agent's name.
    pub who: String,
    pub is_auto: bool,
    /// The place of its issue in the merge order, -1 the orchestrator's own.
    pub rank: i64,
    /// The issue it is about.
    pub number: Option<i64>,
    pub branch: Option<String>,
    /// Where its answer is stored, and the agent it goes to.
    pub file: Option<PathBuf>,
    pub to: Option<Target>,
    /// A try's state: one that isn't ready is grey and takes no answer.
    pub try_state: Option<TryState>,
    /// Under way: its row says where it stands, a click does nothing.
    pub is_running: bool,
    pub merge: Option<MergeOffer>,
    pub go: Go,
}

impl Item {
    /// Its answer, on its way to the agent.
    pub fn answer(&self) -> Option<&str> {
        self.action.as_ref().and_then(|action| action.answer.as_deref())
    }
    /// A try that can't be tried yet: grey, no answer, NEXT skips it.
    pub fn is_waiting(&self) -> bool {
        self.try_state.as_ref().is_some_and(|state| !state.is_ready)
    }
    /// Whether a click on its body can do nothing (#290): a try that isn't
    /// ready and has no page to show why, a start already under way.
    pub fn is_inert(&self) -> bool {
        self.try_state
            .as_ref()
            .is_some_and(|state| !state.is_ready && state.url.is_none())
            || self.is_running
    }
}

/// `items` split into ACTIONS, by what blocks most, then rank, then age; and
/// UP NEXT, the follow-ups, by rank, then age.
pub fn order_actions(items: Vec<Item>) -> (Vec<Item>, Vec<Item>) {
    let by_rank = |a: &Item, b: &Item| a.rank.cmp(&b.rank).then_with(|| a.at.cmp(&b.at));
    let (mut up_next, mut actions): (Vec<Item>, Vec<Item>) = items
        .into_iter()
        .partition(|item| FOLLOW_UPS.contains(&item.kind.as_str()));
    actions.sort_by(|a, b| block_of(&a.kind).cmp(&block_of(&b.kind)).then_with(|| by_rank(a, b)));
    up_next.sort_by(by_rank);
    (actions, up_next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, rank: i64, at: &str) -> Item {
        Item {
            kind: kind.into(),
            rank,
            at: at.into(),
            ..Default::default()
        }
    }
    fn kinds(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| {
                format!(
                    "{}@{}",
                    item.kind,
                    if item.rank == i64::MAX {
                        "Infinity".into()
                    } else {
                        item.rank.to_string()
                    }
                )
            })
            .collect()
    }
    const AT: &str = "2026-09-29T10:00:00Z";

    #[test]
    fn places_from_zero_and_an_unlisted_one_last() {
        assert_eq!(place_in(&[5, 3], Some(3)), 1);
        assert_eq!(place_in(&[5, 3], Some(7)), i64::MAX);
        assert_eq!(place_in(&[], Some(7)), i64::MAX);
    }

    #[test]
    fn keeps_the_follow_ups_under_up_next() {
        let (actions, up_next) = order_actions(vec![
            item("clean up", 0, AT),
            item("waits", 2, AT),
            item("merge", 1, AT),
            item("pipeline", 1, AT),
            item("end", i64::MAX, ""),
            item("clean up", 3, AT),
        ]);
        assert_eq!(kinds(&actions), ["waits@2", "merge@1"]);
        assert_eq!(
            kinds(&up_next),
            ["clean up@0", "pipeline@1", "clean up@3", "end@Infinity"]
        );
    }

    #[test]
    fn sorts_actions_by_what_blocks_most_then_merge_order_then_age() {
        let (actions, _) = order_actions(vec![
            item("look", 0, AT),
            item("browser-check", 0, AT),
            item("merge", 2, AT),
            item("merge", 1, AT),
            item("plan", 0, AT),
            item("decide", 3, "2026-09-29T11:00:00Z"),
            item("decide", 3, "2026-09-29T09:00:00Z"),
            item("decide", -1, AT),
            item("waits", 4, AT),
        ]);
        assert_eq!(
            kinds(&actions),
            [
                "waits@4",
                "decide@-1",
                "decide@3",
                "decide@3",
                "plan@0",
                "merge@1",
                "merge@2",
                "browser-check@0",
                "look@0"
            ]
        );
        assert_eq!(actions[2].at, "2026-09-29T09:00:00Z");
    }
}
