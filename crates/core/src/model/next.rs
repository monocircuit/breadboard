//! NEXT, the band at the top of an orchestration's overview (#278, next.mjs):
//! only what the user can do now, with one click there; else one line that
//! nothing is for them. And when the plan of `wt:plan` went stale, and the
//! plan the live overview writes then by itself (#376), so that its `why`
//! never stands under NEXT naming issues long done.

use super::order::Item;
use crate::record::Plan;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// The MR next in the merge order: the first nothing holds up, else the first
/// and what holds it up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NextMerge {
    pub is_ready: bool,
    pub iid: i64,
    pub url: String,
    pub feature: String,
    pub branch: String,
    pub hold_up: Vec<String>,
    /// How many more are ready after it.
    pub more_ready: usize,
}

/// What NEXT shows: an action of ACTIONS, a row of UP NEXT (their indexes), or the ready merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Action(usize),
    UpNext(usize),
    Merge(NextMerge),
}

/// nextOf: the first action not answered yet and, a try, ready; else the
/// first row of UP NEXT not under way; else the merge that is ready.
pub fn next_of(actions: &[Item], up_next: &[Item], merge: Option<&NextMerge>) -> Option<Next> {
    if let Some(at) = actions
        .iter()
        .position(|item| item.answer().is_none() && !item.is_waiting())
    {
        return Some(Next::Action(at));
    }
    if let Some(at) = up_next.iter().position(|item| !item.is_running) {
        return Some(Next::UpNext(at));
    }
    merge
        .filter(|merge| merge.is_ready)
        .map(|merge| Next::Merge(merge.clone()))
}

/// What an MR next in the merge order waits for, after `once`: the first of
/// its hold-ups the user can't do anything about.
pub fn once_text(hold_up: &[String]) -> String {
    static ONCE: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        [
            (r"^conflicts", "its conflicts are resolved"),
            (r"^pipeline failed", "its pipeline is fixed"),
            (r"^pipeline", "its pipeline passes"),
            (r"^after (!\d+)", "$1 is merged"),
            (r"work box", "its work boxes are ticked"),
            (r"acceptance box", "its acceptance boxes are ticked"),
            (r"^try #\d+ broken", "its fix is tried again"),
            (r"^try #", "its tries look good"),
            (r"^draft", "its agent posts the merge"),
        ]
        .into_iter()
        .map(|(pattern, text)| (Regex::new(pattern).unwrap(), text))
        .collect()
    });
    for (pattern, text) in ONCE.iter() {
        if let Some(hold) = hold_up.iter().find(|hold| pattern.is_match(hold)) {
            if !text.contains('$') {
                return text.to_string();
            }
            let whole = Regex::new(&format!("{}.*", pattern.as_str())).unwrap();
            return whole.replace(hold, *text).into_owned();
        }
    }
    hold_up.first().cloned().unwrap_or_default()
}

/// The one line NEXT shows when nothing is for the user: `nothing for you · 2
/// agents at work · next merge: !286 once its pipeline passes`.
pub fn nothing_text(at_work: usize, merge: Option<&NextMerge>) -> String {
    let mut parts = vec!["nothing for you".to_string()];
    if at_work > 0 {
        parts.push(format!(
            "{at_work} agent{} at work",
            if at_work == 1 { "" } else { "s" }
        ));
    }
    if let Some(merge) = merge {
        let once = if merge.hold_up.is_empty() {
            String::new()
        } else {
            format!(" once {}", once_text(&merge.hold_up))
        };
        parts.push(format!("next merge: !{}{once}", merge.iid));
    }
    parts.join(" · ")
}

/// The issue numbers a text names (`#256`), each once.
pub fn named_in(text: &str) -> Vec<i64> {
    static NAMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#(\d+)\b").unwrap());
    let mut numbers = Vec::new();
    for found in NAMED.captures_iter(text) {
        if let Ok(number) = found[1].parse::<i64>()
            && !numbers.contains(&number)
        {
            numbers.push(number);
        }
    }
    numbers
}

/// An issue's state for the plan: merged, closed (without a merge), open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    Open,
    Merged,
    Closed,
}

/// Why the plan is stale, one short line each; none while it holds.
/// `states`: of the orchestration's issues; one it doesn't know was removed.
pub fn stale_of(plan: Option<&Plan>, states: &HashMap<i64, IssueState>, is_off_plan: bool) -> Vec<String> {
    let (work, merge, why) = plan
        .map(|plan| (&plan.work[..], &plan.merge[..], plan.why.as_deref()))
        .unwrap_or((&[], &[], None));
    let mut listed: Vec<i64> = Vec::new();
    for number in work.iter().chain(merge) {
        if !listed.contains(number) {
            listed.push(*number);
        }
    }
    let gone: Vec<i64> = listed
        .iter()
        .copied()
        .filter(|number| !states.contains_key(number))
        .collect();
    let closed: Vec<i64> = listed
        .iter()
        .copied()
        .filter(|number| states.get(number) == Some(&IssueState::Closed))
        .collect();
    let merged: Vec<i64> = listed
        .iter()
        .copied()
        .filter(|number| states.get(number) == Some(&IssueState::Merged))
        .collect();
    let done: Vec<i64> = named_in(&own_why(why.unwrap_or("")))
        .into_iter()
        .filter(|number| !listed.contains(number) || states.get(number) != Some(&IssueState::Open))
        .collect();
    let issues = |numbers: &[i64]| {
        numbers
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut stale = Vec::new();
    if !merged.is_empty() {
        stale.push(format!("{} merged", issues(&merged)));
    }
    if !closed.is_empty() {
        stale.push(format!("{} closed", issues(&closed)));
    }
    if !gone.is_empty() {
        stale.push(format!("{} no longer in the orchestration", issues(&gone)));
    }
    if !done.is_empty() {
        stale.push(format!("its why names {}, done or gone", issues(&done)));
    }
    if is_off_plan {
        stale.push("the merge order left the plan's".into());
    }
    stale
}

/// Whether NEXT shows a merge: an action of kind merge, or the ready merge.
fn is_merge(next: Option<&Next>, actions: &[Item]) -> bool {
    match next {
        Some(Next::Merge(_)) => true,
        Some(Next::Action(at)) => actions.get(*at).is_some_and(|item| item.kind == "merge"),
        _ => false,
    }
}

/// The plan's `why` when it belongs to what NEXT shows: a merge, and only while the plan isn't stale.
pub fn why_of(plan: Option<&Plan>, stale: &[String], next: Option<&Next>, actions: &[Item]) -> Option<String> {
    let plan = plan?;
    let why = own_why(plan.why.as_deref().unwrap_or(""));
    if why.is_empty() || plan.merge.is_empty() || !stale.is_empty() || !is_merge(next, actions) {
        return None;
    }
    Some(why)
}

/// The line the live overview adds to the plan's why when it writes the merge order (#376).
pub const UPDATED: &str = "updated from the merge order";

/// A why without the overview's `updated from the merge order …` line: what a person wrote.
fn own_why(why: &str) -> String {
    why.lines()
        .filter(|line| !line.starts_with(UPDATED))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The why of a plan the overview wrote at `at` (`2026-10-13 14:01 UTC`): the
/// one there, its old `updated …` line replaced by the new.
pub fn stamped(why: Option<&str>, at: &str) -> String {
    let own = own_why(why.unwrap_or(""));
    let line = format!("{UPDATED} {at}");
    if own.is_empty() { line } else { format!("{own}\n{line}") }
}

/// The plan the live overview writes over a stale one (#376): the computed
/// merge order and the work order, each without the issues done or gone; its
/// why (without the time it was last updated) only while every issue it names
/// is open, else it went stale with them.
pub fn updated_plan(plan: Option<&Plan>, states: &HashMap<i64, IssueState>, planned: &[i64]) -> Plan {
    let keeps = |number: &i64| states.get(number) == Some(&IssueState::Open);
    let why = own_why(plan.and_then(|plan| plan.why.as_deref()).unwrap_or(""));
    Plan {
        work: plan
            .map(|plan| plan.work.iter().copied().filter(keeps).collect())
            .unwrap_or_default(),
        merge: planned.iter().copied().filter(keeps).collect(),
        why: Some(
            if named_in(&why)
                .iter()
                .all(|number| states.get(number) == Some(&IssueState::Open))
            {
                why
            } else {
                String::new()
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Action;
    use crate::model::try_state::{TryState, TryTone};

    fn row(kind: &str) -> Item {
        Item {
            kind: kind.into(),
            text: kind.into(),
            who: "a".into(),
            ..Default::default()
        }
    }
    fn ready() -> NextMerge {
        NextMerge {
            is_ready: true,
            iid: 286,
            ..Default::default()
        }
    }
    fn held() -> NextMerge {
        NextMerge {
            is_ready: false,
            iid: 286,
            hold_up: vec!["draft".into(), "pipeline running".into()],
            ..Default::default()
        }
    }

    #[test]
    fn names_the_first_action_not_answered_and_not_a_waiting_try() {
        let answered = Item {
            action: Some(Action {
                answer: Some("Go".into()),
                ..Default::default()
            }),
            ..row("decide")
        };
        let waiting = Item {
            try_state: Some(TryState {
                is_ready: false,
                label: String::new(),
                tone: TryTone::Waits,
                url: None,
            }),
            ..row("try")
        };
        assert_eq!(
            next_of(&[answered, waiting, row("look")], &[row("clean up")], Some(&ready())),
            Some(Next::Action(2))
        );
    }

    #[test]
    fn names_the_first_up_next_row_not_under_way_then_a_ready_merge() {
        let busy = Item {
            is_running: true,
            ..row("hand over")
        };
        assert_eq!(
            next_of(&[], &[busy.clone(), row("clean up")], Some(&ready())),
            Some(Next::UpNext(1))
        );
        assert_eq!(next_of(&[], &[busy], Some(&ready())), Some(Next::Merge(ready())));
        assert_eq!(next_of(&[], &[], Some(&held())), None);
        assert_eq!(next_of(&[], &[], None), None);
    }

    #[test]
    fn says_what_the_next_merge_waits_for() {
        let once = |hold: &[&str]| once_text(&hold.iter().map(|h| h.to_string()).collect::<Vec<_>>());
        assert_eq!(once(&["try #1 open"]), "its tries look good");
        assert_eq!(once(&["try #2 broken", "try #3 open"]), "its fix is tried again");
        assert_eq!(once(&["1 acceptance box open: a"]), "its acceptance boxes are ticked");
        assert_eq!(once(&["after !223 (shares view.mjs)"]), "!223 is merged");
        assert_eq!(
            once(&["draft", "pipeline failed", "conflicts with develop (3 behind)"]),
            "its conflicts are resolved"
        );
        assert_eq!(once(&["draft"]), "its agent posts the merge");
        assert_eq!(once(&["something new"]), "something new");
        assert_eq!(
            nothing_text(2, Some(&held())),
            "nothing for you · 2 agents at work · next merge: !286 once its pipeline passes"
        );
        assert_eq!(nothing_text(1, None), "nothing for you · 1 agent at work");
        assert_eq!(nothing_text(0, None), "nothing for you");
    }

    fn states() -> HashMap<i64, IssueState> {
        [
            (256, IssueState::Merged),
            (262, IssueState::Closed),
            (273, IssueState::Open),
            (278, IssueState::Open),
        ]
        .into()
    }
    fn plan() -> Plan {
        Plan {
            work: vec![273, 278],
            merge: vec![273, 278],
            why: Some("#273 first: #278 builds on its NEXT".into()),
        }
    }

    #[test]
    fn finds_when_the_plan_is_stale() {
        assert_eq!(named_in("#256 then #262, #256 again"), [256, 262]);
        assert!(named_in("").is_empty());
        assert!(stale_of(Some(&plan()), &states(), false).is_empty());
        let stale = Plan {
            work: vec![262, 299],
            merge: vec![256, 273],
            why: Some("#256 then the overall changelog; #262 touches workers".into()),
        };
        assert_eq!(
            stale_of(Some(&stale), &states(), true),
            [
                "#256 merged",
                "#262 closed",
                "#299 no longer in the orchestration",
                "its why names #256, #262, done or gone",
                "the merge order left the plan's",
            ]
        );
    }

    #[test]
    fn shows_the_why_only_under_a_merge_while_the_plan_holds() {
        let merge = Next::Merge(ready());
        let actions = [row("merge"), row("look")];
        let why = plan().why;
        assert_eq!(why_of(Some(&plan()), &[], Some(&merge), &actions), why);
        assert_eq!(why_of(Some(&plan()), &[], Some(&Next::Action(0)), &actions), why);
        assert_eq!(why_of(Some(&plan()), &[], Some(&Next::Action(1)), &actions), None);
        assert_eq!(why_of(Some(&plan()), &[], None, &actions), None);
        assert_eq!(
            why_of(Some(&plan()), &["#262 closed".into()], Some(&merge), &actions),
            None
        );
    }

    #[test]
    fn updates_a_stale_plan_to_the_computed_order() {
        let old = Plan {
            work: vec![262, 273, 299],
            merge: vec![262],
            why: Some("#262 first".into()),
        };
        assert_eq!(
            updated_plan(Some(&old), &states(), &[256, 278, 273, 262]),
            Plan {
                work: vec![273],
                merge: vec![278, 273],
                why: Some(String::new())
            }
        );
        assert_eq!(updated_plan(Some(&plan()), &states(), &[]).why, plan().why);
    }

    #[test]
    fn stamps_the_why_with_the_time_it_followed_the_merge_order() {
        let at = "2026-10-13 14:01 UTC";
        assert_eq!(stamped(None, at), "updated from the merge order 2026-10-13 14:01 UTC");
        let once = stamped(Some("#273 first"), "2026-10-13 13:00 UTC");
        assert_eq!(once, "#273 first\nupdated from the merge order 2026-10-13 13:00 UTC");
        // Its old line replaced, never piled up; NEXT and the staleness read the person's lines only.
        assert_eq!(
            stamped(Some(&once), at),
            "#273 first\nupdated from the merge order 2026-10-13 14:01 UTC"
        );
        let plan = Plan {
            why: Some(once.clone()),
            ..plan()
        };
        let actions = [row("merge")];
        assert_eq!(
            why_of(Some(&plan), &[], Some(&Next::Merge(ready())), &actions).as_deref(),
            Some("#273 first")
        );
        assert!(stale_of(Some(&plan), &states(), false).is_empty());
        assert_eq!(
            updated_plan(Some(&plan), &states(), &[273]).why.as_deref(),
            Some("#273 first")
        );
    }
}
