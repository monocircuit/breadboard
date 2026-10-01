//! What needs the user, split as the live view splits it (render-orchestration.mjs
//! `actionsOf`, order.mjs, auto-actions.mjs, action-block.mjs): ACTIONS, the
//! orchestrator's and the agents' own and the merges found by themselves,
//! by what blocks most; UP NEXT, the automatic follow-ups (a failed pipeline,
//! the end, a stale plan). Each drawn as a block: its kind as a chip, who,
//! how old, its text, then how it is answered.

use super::View;
use super::merge::{checks_of, is_change_open, merge_text};
use crate::cli::read::overview::paint::{bold, button, chip, dim, green, link, red, visible, wrap, yellow};
use breadboard_core::actions::{Action, answers_of, options_of};
use breadboard_core::json;
use breadboard_core::record::ORCHESTRATOR;
use breadboard_core::time::{ago, parse_iso, to_iso};
use serde_json::Value;

/// The kinds that follow by themselves: UP NEXT.
const FOLLOW_UPS: [&str; 6] = ["clean up", "pipeline", "end", "hand over", "start", "order"];
/// The kinds of ACTIONS by what blocks most.
const BLOCKS: [&str; 8] = [
    "waits",
    "decide",
    "plan",
    "push",
    "merge",
    "try",
    "browser-check",
    "look",
];
/// How long unpushed commits may sit before someone is needed (change.mjs).
const UNPUSHED_MS: i64 = 10 * 60_000;

/// An action as the view shows it.
#[derive(Debug, Clone)]
pub struct Item {
    pub action: Action,
    pub who: String,
    pub is_auto: bool,
    /// Its issue's place in the merge order, the orchestrator's own first.
    pub rank: i64,
    /// Its issue, an index into the view's items.
    pub item: Option<usize>,
    /// Posted by an agent or the orchestrator (`wt:action merge`): a draft is marked ready first.
    pub can_mark_ready: bool,
}

/// ACTIONS and UP NEXT.
#[derive(Debug, Default)]
pub struct Listed {
    pub actions: Vec<Item>,
    pub up_next: Vec<Item>,
}

/// Whether a posted merge shows: not once its MR is merged, nor while a try
/// or an acceptance box is open (#280).
fn is_merge_offered(view: &View, at: usize) -> bool {
    let item = &view.items[at];
    let boxes = view
        .found(item)
        .and_then(|found| found.issue.as_ref())
        .map(|issue| issue.boxes.as_slice());
    !view.mr(item).is_some_and(|mr| mr.is_merged()) && checks_of(boxes.unwrap_or_default(), view.tree(item)).is_empty()
}

/// An action made by the view itself.
fn auto(kind: &str, text: String, at: Option<i64>) -> Action {
    Action {
        kind: kind.into(),
        text,
        at: at.map(to_iso).unwrap_or_default(),
        ..Action::default()
    }
}

/// What a worktree remembers as sent to its agent (`.claude/overview-sent.json`).
fn sent_of(dir: &std::path::Path) -> Vec<String> {
    json::read(&dir.join(".claude").join("overview-sent.json"))
        .and_then(|value| match value {
            Value::Array(list) => Some(list.iter().filter_map(|key| key.as_str().map(String::from)).collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Every open action of the orchestration, split into ACTIONS and UP NEXT.
pub fn actions_of(view: &View, stale: &[String], has_next_merge: bool) -> Listed {
    let merge_order = view.open_order();
    let rank_of = |at: usize| {
        view.items[at]
            .number()
            .and_then(|number| merge_order.iter().position(|each| *each == number))
            .map_or(i64::MAX, |place| place as i64)
    };
    let mut items: Vec<Item> = Vec::new();
    // The orchestrator's own: a merge it posted for an MR (`--mr !N`) is that MR's.
    for action in &view.record.actions {
        let issue = (action.kind == "merge").then_some(action.mr).flatten().and_then(|iid| {
            view.items
                .iter()
                .position(|item| view.mr(item).is_some_and(|mr| mr.iid == iid))
        });
        if issue.is_some_and(|at| !is_merge_offered(view, at)) {
            continue;
        }
        items.push(Item {
            action: action.clone(),
            who: format!("{ORCHESTRATOR} orchestrator"),
            is_auto: false,
            rank: -1,
            item: issue,
            can_mark_ready: true,
        });
    }
    // The agents' own.
    for (at, entry) in view.items.iter().enumerate() {
        let Some(tree) = view.tree(entry) else { continue };
        for action in &tree.actions {
            if action.kind == "merge" && !is_merge_offered(view, at) {
                continue;
            }
            items.push(Item {
                action: action.clone(),
                who: entry.feature.clone(),
                is_auto: false,
                rank: rank_of(at),
                item: Some(at),
                can_mark_ready: true,
            });
        }
    }
    // Found by themselves (auto-actions.mjs `autoActionsOf`): a merge that is
    // ready, a failed pipeline, commits unpushed for long. Drafts nothing but
    // the draft holds up may be merged by the click too (#242).
    let ready_drafts: Vec<i64> = view
        .order
        .open
        .iter()
        .filter(|placed| placed.hold_up == ["draft"])
        .map(|placed| placed.number)
        .collect();
    for (at, entry) in view.items.iter().enumerate() {
        let Some(mr) = view.mr(entry) else { continue };
        let tree = view.tree(entry);
        let found = view.found(entry);
        let boxes = found
            .and_then(|found| found.issue.as_ref())
            .map(|issue| issue.boxes.as_slice());
        let status = mr.status();
        let is_ready_draft = mr.is_draft && entry.number().is_some_and(|number| ready_drafts.contains(&number));
        if mr.is_open()
            && (!mr.is_draft || is_ready_draft)
            && status == Some("success")
            && (mr.merge_status.as_deref() == Some("mergeable") || is_ready_draft)
            && !tree.is_some_and(|tree| tree.actions.iter().any(|action| action.kind == "merge"))
            && !is_change_open(tree, Some(mr))
            && checks_of(boxes.unwrap_or_default(), tree).is_empty()
        {
            let text = if is_ready_draft {
                "pipeline passed, every box and try done, still a draft"
            } else {
                "ready, pipeline passed"
            };
            items.push(Item {
                action: auto("merge", text.into(), None),
                who: entry.feature.clone(),
                is_auto: true,
                rank: rank_of(at),
                item: Some(at),
                can_mark_ready: is_ready_draft,
            });
        }
        let sent = tree.map(|tree| sent_of(&tree.dir)).unwrap_or_default();
        let failed_key = format!(
            "pipeline:!{}:{}",
            mr.iid,
            mr.pipeline.as_ref().map_or(0, |pipeline| pipeline.id)
        );
        if mr.is_open() && status == Some("failed") && !sent.contains(&failed_key) {
            let job = found.and_then(|found| found.jobs.iter().find(|job| job.status == "failed"));
            let what = job.map_or("failed".to_string(), |job| format!("job {} failed", job.name));
            items.push(Item {
                action: auto("pipeline", format!("!{} {what}: no agent to tell", mr.iid), None),
                who: entry.feature.clone(),
                is_auto: true,
                rank: rank_of(at),
                item: Some(at),
                can_mark_ready: false,
            });
        }
        if let Some(tree) = tree
            && tree.unpushed > 0
            && let Some(since) = tree.unpushed_at
            && view.now - since >= UNPUSHED_MS
        {
            let minutes = (view.now - since) / 60_000;
            let long = if minutes < 120 {
                format!("{minutes} min")
            } else {
                format!("{} h", (minutes + 30) / 60)
            };
            let fact = format!(
                "{} commit{} not pushed{}",
                tree.unpushed,
                if tree.unpushed == 1 { "" } else { "s" },
                if tree.has_remote_branch {
                    ""
                } else {
                    " (no remote branch)"
                }
            );
            items.push(Item {
                action: auto(
                    "push",
                    format!("{} for {long}: click to tell its agent", red(&fact)),
                    Some(since),
                ),
                who: entry.feature.clone(),
                is_auto: true,
                rank: rank_of(at),
                item: Some(at),
                can_mark_ready: false,
            });
        }
    }
    // The end, once all is merged and no worktree is left.
    if view.remote.is_some()
        && view.trees.is_empty()
        && !view.items.iter().any(|item| item.entry.is_queued)
        && !has_next_merge
    {
        items.push(Item {
            action: auto(
                "end",
                "all merged, no worktree left: end the orchestration".into(),
                None,
            ),
            who: format!("{ORCHESTRATOR} orchestrator"),
            is_auto: true,
            rank: i64::MAX,
            item: None,
            can_mark_ready: false,
        });
    }
    // A stale plan: its click would write the computed merge order (wt:plan).
    if !stale.is_empty() {
        let has_merge = view.record.plan.as_ref().is_some_and(|plan| !plan.merge.is_empty());
        let order: Vec<String> = view
            .order
            .open
            .iter()
            .map(|placed| format!("#{}", placed.number))
            .collect();
        let text = format!(
            "{}: {} · click to write it into the plan (wt:plan)",
            if has_merge {
                format!("the plan is stale: {}", stale.join("; "))
            } else {
                "no merge order in the plan yet".into()
            },
            order.join(" → ")
        );
        items.push(Item {
            action: auto("order", text, None),
            who: format!("{ORCHESTRATOR} orchestrator"),
            is_auto: true,
            rank: -1,
            item: None,
            can_mark_ready: false,
        });
    }
    order_actions(items)
}

/// ACTIONS by what blocks most, then rank, then age; UP NEXT by rank, then age.
fn order_actions(items: Vec<Item>) -> Listed {
    let block = |kind: &str| BLOCKS.iter().position(|each| *each == kind).unwrap_or(BLOCKS.len());
    let (mut up_next, mut actions): (Vec<Item>, Vec<Item>) = items
        .into_iter()
        .partition(|item| FOLLOW_UPS.contains(&item.action.kind.as_str()));
    actions.sort_by(|a, b| {
        block(&a.action.kind)
            .cmp(&block(&b.action.kind))
            .then(a.rank.cmp(&b.rank))
            .then(a.action.at.cmp(&b.action.at))
    });
    up_next.sort_by(|a, b| a.rank.cmp(&b.rank).then(a.action.at.cmp(&b.action.at)));
    Listed { actions, up_next }
}

/// The chip of a kind, an automatic one's marked: `merge · auto`.
pub fn kind_chip(kind: &str, is_auto: bool) -> String {
    let name = if kind == "browser-check" { "try" } else { kind };
    let tone = match name {
        "merge" => "green",
        "try" | "plan" | "order" | "hand over" | "start" | "clean up" => "cyan",
        "decide" | "waits" => "yellow",
        "end" | "pipeline" | "push" => "red",
        _ => "plain",
    };
    chip(&format!("{name}{}", if is_auto { " · auto" } else { "" }), tone)
}

/// A page as the row shows it: `:3002/atlas` for one on localhost.
fn short_page(page: &str) -> String {
    let page = page
        .strip_prefix("http://localhost")
        .or_else(|| page.strip_prefix("https://localhost"))
        .filter(|rest| rest.starts_with(':'))
        .unwrap_or(page);
    match page.strip_suffix('/') {
        Some(port) if port.starts_with(':') && port[1..].chars().all(|c| c.is_ascii_digit()) => port.to_string(),
        _ => page.to_string(),
    }
}

/// What a click on a try opens (try.mjs `trySpot`, without a click under way).
fn try_spot(view: &View, item: &Item) -> String {
    let action = &item.action;
    if action.on.as_deref() == Some("test") {
        return dim(&format!(
            "opens {} on the test instance",
            action.url.clone().unwrap_or_default()
        ));
    }
    if let Some(run) = &action.run {
        return dim(&format!("opens a terminal: {run}"));
    }
    let Some(url) = &action.url else { return String::new() };
    let Some(tree) = item.item.and_then(|at| view.tree(&view.items[at])) else {
        return dim(&format!("opens {}", short_page(url)));
    };
    let origin = format!("http://localhost:{}", 3000 + tree.offset);
    let page = if url.starts_with('/') {
        format!("{origin}{url}")
    } else {
        url.clone()
    };
    let is_on_app = page == origin || page.starts_with(&format!("{origin}/"));
    let will_start = tree.offset > 0 && tree.has_env && !tree.is_up && is_on_app;
    dim(&format!(
        "opens {}{}",
        short_page(&page),
        if will_start { " after starting the stack" } else { "" }
    ))
}

/// The text of an action: a merge's with its MR in front and what its two
/// clicks do or what blocks them, a try's with what its click opens.
pub fn action_text(view: &View, item: &Item) -> String {
    let action = &item.action;
    let entry = item.item.map(|at| &view.items[at]);
    let mr = entry.and_then(|entry| view.mr(entry));
    if action.kind == "merge"
        && let (Some(entry), Some(mr)) = (entry, mr)
    {
        let boxes = view
            .found(entry)
            .and_then(|found| found.issue.as_ref())
            .map(|issue| issue.boxes.as_slice());
        return format!(
            "{} {}",
            link(mr.url.as_deref(), &format!("!{}", mr.iid)),
            merge_text(
                mr,
                &action.text,
                boxes.unwrap_or_default(),
                item.can_mark_ready,
                view.tree(entry)
            )
        );
    }
    if action.is_try() {
        let spot = try_spot(view, item);
        return if visible(&spot) > 0 {
            format!("{}  {spot}", action.text)
        } else {
            action.text.clone()
        };
    }
    action.text.clone()
}

/// The cell of an action: its buttons, or the answer on its way (answers.mjs `answerCell`).
fn answer_cell(action: &Action) -> String {
    if let Some(answer) = &action.answer {
        return format!(
            "{} {}",
            yellow(&format!("→ {answer}")),
            dim(action.waits.as_deref().unwrap_or("at its next stop"))
        );
    }
    let answers = answers_of(action);
    let labels = if answers.is_empty() {
        vec!["✓".to_string()]
    } else {
        answers
    };
    labels
        .iter()
        .map(|label| button(label, if label == "✓" { "green" } else { "cyan" }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The lines of an action as a block `width` wide (action-block.mjs `actionBlock`).
pub fn block_of(view: &View, item: &Item, width: usize) -> Vec<String> {
    let action = &item.action;
    let head: Vec<String> = [
        kind_chip(&action.kind, item.is_auto),
        bold(&item.who),
        parse_iso(&action.at)
            .map(|at| dim(&ago(at, view.now)))
            .unwrap_or_default(),
    ]
    .into_iter()
    .filter(|part| visible(part) > 0)
    .collect();
    let mut lines = wrap(&head.join("  "), width);
    let text = action_text(view, item);
    if action.kind == "decide" {
        let (question, why) = match text.find('?') {
            Some(at) => (text[..=at].to_string(), text[at + 1..].trim().to_string()),
            None => (text.clone(), String::new()),
        };
        lines.extend(wrap(&bold(&question), width));
        if !why.is_empty() {
            lines.extend(wrap(&dim(&why), width));
        }
    } else {
        lines.extend(wrap(&text, width));
    }
    if item.is_auto {
        return lines;
    }
    if action.answer.is_none() && action.kind == "decide" && !action.options.is_empty() {
        for (at, option) in options_of(action).iter().enumerate() {
            let number = format!("{}. ", at + 1);
            let mark = if action.recommend.as_deref() == Some(option.answer.as_str()) {
                format!(" {}", green("(Recommended)"))
            } else {
                String::new()
            };
            let hang = number.len();
            let room = width.saturating_sub(hang).max(1);
            for (line_at, line) in wrap(&format!("{}{mark}", bold(&option.label)), room)
                .into_iter()
                .enumerate()
            {
                lines.push(format!(
                    "{}{line}",
                    if line_at == 0 { number.clone() } else { " ".repeat(hang) }
                ));
            }
            if !option.description.is_empty() {
                for line in wrap(&dim(&option.description), room) {
                    lines.push(format!("{}{line}", " ".repeat(hang)));
                }
            }
        }
        return lines;
    }
    lines.push(answer_cell(action));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_on_localhost_shows_short() {
        assert_eq!(short_page("http://localhost:3002/atlas"), ":3002/atlas");
        assert_eq!(short_page("http://localhost:3002/"), ":3002");
        assert_eq!(short_page("https://example.com/x"), "https://example.com/x");
    }

    #[test]
    fn actions_go_by_what_blocks_most_and_follow_ups_apart() {
        let item = |kind: &str, rank: i64| Item {
            action: Action {
                kind: kind.into(),
                ..Action::default()
            },
            who: String::new(),
            is_auto: false,
            rank,
            item: None,
            can_mark_ready: false,
        };
        let listed = order_actions(vec![
            item("look", 0),
            item("merge", 1),
            item("decide", 2),
            item("end", 9),
            item("order", -1),
        ]);
        let kinds: Vec<&str> = listed.actions.iter().map(|item| item.action.kind.as_str()).collect();
        assert_eq!(kinds, vec!["decide", "merge", "look"]);
        let kinds: Vec<&str> = listed.up_next.iter().map(|item| item.action.kind.as_str()).collect();
        assert_eq!(kinds, vec!["order", "end"]);
    }

    #[test]
    fn an_answer_on_its_way_or_the_buttons() {
        let mut action = Action {
            kind: "try".into(),
            ..Action::default()
        };
        assert_eq!(answer_cell(&action), "[ looks good ] [ broken ]");
        action.answer = Some("broken".into());
        assert_eq!(answer_cell(&action), "→ broken at its next stop");
        assert_eq!(answer_cell(&Action::default()), "[ ✓ ]");
    }
}
