//! What an agent waits for (#288, agent-state.mjs): one state per agent, the
//! same words in WORKTREES of an orchestration and herdr's sidebar, never a
//! bare `idle`.

use super::change::is_change_open;
use super::handovers::next_text;
use super::merge_turn::{MergeTurn, waits_for_text};
use super::{Tone, Tree, counted};
use crate::conversations::question_of;
use crate::gitlab::Mr;
use crate::wait::waits_text;
use std::collections::HashSet;

/// GitLab's statuses of a pipeline that hasn't finished yet.
pub const RUNNING: [&str; 6] = [
    "created",
    "waiting_for_resource",
    "preparing",
    "pending",
    "running",
    "scheduled",
];

/// What an agent waits for: `key` the case, `text` its words, `tone` whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentState {
    pub key: &'static str,
    pub text: String,
    pub tone: Tone,
}

fn state(key: &'static str, text: impl Into<String>, tone: Tone) -> AgentState {
    AgentState {
        key,
        text: text.into(),
        tone,
    }
}

/// The handover announced for an agent: the issue it takes, the MRs it still waits for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NextIssue {
    pub number: i64,
    pub open: Vec<i64>,
}

/// What agentStateOf knows besides the worktree: its MR, herdr's state of the
/// agent, the iids known merged (a wait names only the others), the announced
/// handover, what the human's checks still lack (merge.rs `checks_of`), its
/// MR's merge turn (merge_turn.rs; None where the merge order isn't known).
#[derive(Debug, Clone, Default)]
pub struct Facts<'a> {
    pub mr: Option<&'a Mr>,
    pub status: Option<&'a str>,
    pub merged: Option<&'a HashSet<i64>>,
    pub handover: Option<&'a NextIssue>,
    pub checks: &'a [String],
    pub turn: Option<&'a MergeTurn>,
}

/// A done MR whose merge order isn't known here (herdr's sidebar without the
/// orchestration's verdict): no merge is claimed for the user, as none may
/// be offered (#344).
pub const MERGE_TURN: &str = "done: waits for its merge turn";
/// The key of a headless agent whose lines wait for a run (#358).
pub const RUN_QUEUED: &str = "run-queued";
/// The key of a headless agent whose run stopped at the usage limit (#393).
pub const RUN_LIMIT: &str = "run-limit";
/// A green MR whose turn says it lacks something no check names (work
/// boxes, conflicts, a change): its MR row says what.
pub const NOT_READY: &str = "not ready to merge: its MR row says why";

/// What an agent waits for, the first that holds: at work; queued for a run; the usage limit; asks you; waits
/// for the pipeline; pipeline failed; waits for you (its open actions, the
/// merge while its turn offers it, #344); waits for the MRs its turn comes
/// after; checks open; done without a known turn; waits for other MRs; the
/// next issue announced; change requested; not pushed; free.
pub fn agent_state_of(tree: &Tree, facts: &Facts) -> AgentState {
    let Facts {
        mr,
        status,
        merged,
        handover,
        checks,
        turn,
    } = facts;
    if *status == Some("working") {
        return state("working", "at work", Tone::Work);
    }
    // A headless agent's lines wait for a run (#358): memory, the merge order, its open tab.
    // Its key is no issue's `queued` (#361): the tree folds those into the start queue.
    if *status == Some("queued") {
        return state(RUN_QUEUED, "queued for a run", Tone::Work);
    }
    // Its run stopped at the Claude usage limit, it goes on after the reset (#393; the band says when).
    if *status == Some("limit") {
        return state(RUN_LIMIT, "waits for the usage limit", Tone::Work);
    }
    let is_asking = tree
        .claude
        .as_ref()
        .and_then(|claude| claude.answer.as_deref())
        .and_then(question_of)
        .is_some();
    if *status == Some("blocked") || is_asking {
        return state("asks", "asks you", Tone::You);
    }
    let is_open = mr.is_some_and(|mr| mr.is_open());
    let pipeline = mr.and_then(|mr| mr.pipeline_status());
    let sha = mr
        .and_then(|mr| mr.pipeline.as_ref())
        .and_then(|pipeline| pipeline.sha.as_deref());
    // Pushed, but GitLab's pipeline is still the one of a commit before.
    let is_behind = sha.is_some() && tree.pushed.is_some() && sha != tree.pushed.as_deref();
    if is_open && (pipeline.is_some_and(|status| RUNNING.contains(&status)) || is_behind) {
        return state("pipeline", "waits for the pipeline", Tone::Gitlab);
    }
    if is_open && pipeline == Some("failed") {
        return state("failed", "pipeline failed", Tone::You);
    }
    let mut kinds: Vec<String> = Vec::new();
    // A posted merge is the user's only while it is offered (ACTIONS shows it then).
    let is_withheld = |action: &&crate::actions::Action| {
        action.kind == "merge" && turn.is_some_and(|turn| *turn != MergeTurn::Offered)
    };
    for action in tree
        .actions
        .iter()
        .filter(|action| action.answer.is_none())
        .filter(|action| !is_withheld(action))
    {
        let kind = if action.is_try() {
            format!("try #{}", action.id)
        } else {
            action.kind.clone()
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    let is_green = is_open && pipeline == Some("success") && !is_change_open(Some(tree), *mr);
    // The merge from its turn alone, never from a green pipeline: it may come after another MR.
    let is_offered = turn.is_some_and(|turn| *turn == MergeTurn::Offered);
    if is_green && is_offered && !kinds.iter().any(|kind| kind == "merge") {
        kinds.push("merge".into());
    }
    if !kinds.is_empty() {
        return state("waits", format!("waits for you: {}", kinds.join(", ")), Tone::You);
    }
    if is_green && let Some(MergeTurn::Waits(iids)) = turn {
        return state("waitsForMr", waits_for_text(iids), Tone::Gitlab);
    }
    if is_green && checks.is_empty() {
        let text = if turn.is_none() { MERGE_TURN } else { NOT_READY };
        return state("turn", text, Tone::Gitlab);
    }
    if is_green {
        return state("checks", format!("checks open: {}", checks.join(", ")), Tone::You);
    }
    if let Some(wait) = &tree.wait {
        let none = HashSet::new();
        return state("waitsForMr", waits_text(wait, merged.unwrap_or(&none)), Tone::Gitlab);
    }
    if let Some(handover) = handover {
        return state("next", next_text(handover.number, &handover.open), Tone::Gitlab);
    }
    if is_change_open(Some(tree), *mr) {
        return state("change", "change requested", Tone::You);
    }
    if tree.unpushed > 0 {
        return state(
            "unpushed",
            format!("not pushed: {}", counted(tree.unpushed, "commit")),
            Tone::You,
        );
    }
    state("free", "free", Tone::Free)
}

/// The line of an agent in herdr's sidebar: its state, its MR behind it.
pub fn sidebar_line_of(tree: &Tree, facts: &Facts) -> String {
    let reference = facts.mr.map(|mr| format!(" · !{}", mr.iid)).unwrap_or_default();
    format!("{}{reference}", agent_state_of(tree, facts).text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Action, Change};
    use crate::conversations::Claude;
    use crate::gitlab::Pipeline;
    use crate::wait::Wait;
    use std::path::PathBuf;

    fn claude(answer: &str) -> Claude {
        Claude {
            id: "c1".into(),
            prompt: None,
            prompt_at: None,
            answer: Some(answer.into()),
            answer_at: None,
            transcript: PathBuf::new(),
            cwd: PathBuf::new(),
            model: None,
            at: None,
            used: 0,
            window: 1,
        }
    }
    fn tree() -> Tree {
        Tree {
            claude: Some(claude("Pushed, the pipeline runs.")),
            pushed: Some("a".into()),
            ..Default::default()
        }
    }
    fn mr(status: &str) -> Mr {
        Mr {
            iid: 192,
            state: "opened".into(),
            is_draft: true,
            pipeline: Some(Pipeline {
                status: status.into(),
                sha: Some("a".into()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    /// Its merge offered: what the overview of its orchestration finds for a done MR first in the order.
    fn at(tree: &Tree, mr: Option<&Mr>, status: &str) -> AgentState {
        agent_state_of(
            tree,
            &Facts {
                mr,
                status: Some(status),
                turn: Some(&MergeTurn::Offered),
                ..Default::default()
            },
        )
    }
    fn action(id: i64, kind: &str, answer: Option<&str>) -> Action {
        Action {
            id,
            kind: kind.into(),
            answer: answer.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn says_the_agent_is_at_work() {
        assert_eq!(
            at(&tree(), Some(&mr("success")), "working"),
            state("working", "at work", Tone::Work)
        );
    }

    #[test]
    fn lines_waiting_for_a_run_are_no_issue_in_the_start_queue() {
        let queued = at(&tree(), Some(&mr("success")), "queued");
        assert_eq!(queued, state(RUN_QUEUED, "queued for a run", Tone::Work));
        // The tree folds every node keyed `queued` into the queue of issues to start.
        assert_ne!(queued.key, "queued");
    }

    #[test]
    fn a_run_stopped_at_the_usage_limit_waits_for_it() {
        assert_eq!(
            at(&tree(), Some(&mr("failed")), "limit"),
            state(RUN_LIMIT, "waits for the usage limit", Tone::Work)
        );
    }

    #[test]
    fn asks_you_when_blocked_or_stopped_on_a_question() {
        assert_eq!(at(&tree(), None, "blocked").text, "asks you");
        let asking = Tree {
            claude: Some(claude("Done.\n\nShall I merge?")),
            ..tree()
        };
        assert_eq!(at(&asking, None, "idle").key, "asks");
        assert_eq!(at(&asking, None, "idle").tone, Tone::You);
    }

    #[test]
    fn waits_for_the_pipeline_while_it_runs_or_gitlab_has_none_for_the_push() {
        for status in ["running", "pending", "created"] {
            assert_eq!(at(&tree(), Some(&mr(status)), "idle").text, "waits for the pipeline");
        }
        let behind = at(
            &Tree {
                pushed: Some("b".into()),
                ..tree()
            },
            Some(&mr("success")),
            "idle",
        );
        assert_eq!(
            (behind.text.as_str(), behind.tone),
            ("waits for the pipeline", Tone::Gitlab)
        );
    }

    #[test]
    fn says_a_failed_pipeline() {
        assert_eq!(
            at(&tree(), Some(&mr("failed")), "idle"),
            state("failed", "pipeline failed", Tone::You)
        );
    }

    #[test]
    fn waits_for_your_merge_once_the_pipeline_passed() {
        assert_eq!(at(&tree(), Some(&mr("success")), "idle").text, "waits for you: merge");
        assert_eq!(
            at(
                &tree(),
                Some(&Mr {
                    is_draft: false,
                    ..mr("success")
                }),
                "done"
            )
            .text,
            "waits for you: merge"
        );
    }

    #[test]
    fn a_done_mr_behind_another_waits_for_it_never_for_you() {
        // #344: !324 done, after !333 in the merge order: ACTIONS offers nothing.
        let waits = MergeTurn::Waits(vec![333]);
        let posted = Tree {
            actions: vec![action(1, "merge", None)],
            ..tree()
        };
        for tree in [tree(), posted] {
            let found = agent_state_of(
                &tree,
                &Facts {
                    mr: Some(&mr("success")),
                    status: Some("idle"),
                    turn: Some(&waits),
                    ..Default::default()
                },
            );
            assert_eq!(found, state("waitsForMr", "waits for !333", Tone::Gitlab));
        }
    }

    #[test]
    fn claims_no_merge_without_a_turn() {
        // herdr's sidebar doesn't know the merge order: a green pipeline alone offers nothing.
        let facts = Facts {
            mr: Some(&mr("success")),
            status: Some("idle"),
            ..Default::default()
        };
        assert_eq!(agent_state_of(&tree(), &facts), state("turn", MERGE_TURN, Tone::Gitlab));
        let lacks = Facts {
            turn: Some(&MergeTurn::Lacks),
            ..facts.clone()
        };
        assert_eq!(agent_state_of(&tree(), &lacks).text, NOT_READY);
    }

    #[test]
    fn offers_no_merge_while_a_try_or_box_is_open() {
        let checks = vec![
            "try #2 broken".to_string(),
            "1 acceptance box open: free shows".to_string(),
        ];
        let facts = Facts {
            mr: Some(&mr("success")),
            status: Some("idle"),
            checks: &checks,
            ..Default::default()
        };
        assert_eq!(
            agent_state_of(&tree(), &facts),
            state(
                "checks",
                "checks open: try #2 broken, 1 acceptance box open: free shows",
                Tone::You
            )
        );
        let checks = vec!["try #3 open".to_string()];
        let with_try = Tree {
            actions: vec![action(3, "try", None)],
            ..tree()
        };
        let facts = Facts {
            mr: Some(&mr("success")),
            status: Some("idle"),
            checks: &checks,
            ..Default::default()
        };
        assert_eq!(agent_state_of(&with_try, &facts).text, "waits for you: try #3");
    }

    #[test]
    fn names_the_open_actions_a_try_by_its_number() {
        let actions = vec![
            action(3, "decide", None),
            action(4, "try", None),
            action(5, "browser-check", None),
            action(6, "look", Some("done")),
            action(7, "merge", None),
        ];
        assert_eq!(
            at(&Tree { actions, ..tree() }, Some(&mr("success")), "idle").text,
            "waits for you: decide, try #4, try #5, merge"
        );
    }

    #[test]
    fn waits_for_other_mrs_then_for_an_announced_handover() {
        let wait = Wait {
            mrs: vec![271, 272],
            ..Default::default()
        };
        let merged: HashSet<i64> = [272].into();
        let waiting = Tree {
            wait: Some(wait.clone()),
            ..tree()
        };
        let facts = Facts {
            status: Some("idle"),
            merged: Some(&merged),
            ..Default::default()
        };
        assert_eq!(
            agent_state_of(&waiting, &facts),
            state("waitsForMr", "waits for !271 · 1 merged", Tone::Gitlab)
        );
        let handover = NextIssue {
            number: 226,
            open: vec![232],
        };
        let facts = Facts {
            status: Some("idle"),
            handover: Some(&handover),
            ..Default::default()
        };
        assert_eq!(agent_state_of(&tree(), &facts).text, "→ next #226 after !232");
        assert_eq!(agent_state_of(&waiting, &facts).text, "waits for !271, !272");
    }

    #[test]
    fn says_a_requested_change_then_commits_not_pushed() {
        let change = Change {
            text: "less padding".into(),
            head: Some("a".into()),
            ..Default::default()
        };
        assert_eq!(
            at(
                &Tree {
                    change: Some(change),
                    ..tree()
                },
                Some(&mr("success")),
                "idle"
            )
            .text,
            "change requested"
        );
        assert_eq!(
            at(&Tree { unpushed: 2, ..tree() }, None, "idle").text,
            "not pushed: 2 commits"
        );
        assert_eq!(
            at(&Tree { unpushed: 1, ..tree() }, None, "idle").text,
            "not pushed: 1 commit"
        );
    }

    #[test]
    fn is_free_only_with_nothing_open() {
        assert_eq!(at(&tree(), None, "idle"), state("free", "free", Tone::Free));
        let merged = Mr {
            state: "merged".into(),
            ..mr("success")
        };
        assert_eq!(at(&tree(), Some(&merged), "idle").text, "free");
        let handover = NextIssue {
            number: 9,
            open: vec![],
        };
        let facts = Facts {
            mr: Some(&merged),
            status: Some("idle"),
            handover: Some(&handover),
            ..Default::default()
        };
        assert_eq!(agent_state_of(&tree(), &facts).text, "→ next #9 now");
    }

    #[test]
    fn takes_the_first_that_holds_in_this_order() {
        let mut extra = Tree {
            claude: Some(claude("Shall I?")),
            actions: vec![action(1, "decide", None)],
            wait: Some(Wait {
                mrs: vec![271],
                ..Default::default()
            }),
            change: Some(Change {
                text: "x".into(),
                head: Some("a".into()),
                ..Default::default()
            }),
            unpushed: 1,
            pushed: Some("a".into()),
            ..Default::default()
        };
        let handover = NextIssue {
            number: 9,
            open: vec![271],
        };
        let mut status = "working";
        let mut current = Some(mr("running"));
        let mut has_handover = true;
        let mut order = Vec::new();
        let key = |extra: &Tree, current: &Option<Mr>, status: &str, has_handover: bool| {
            let facts = Facts {
                mr: current.as_ref(),
                status: Some(status),
                handover: has_handover.then_some(&handover),
                ..Default::default()
            };
            agent_state_of(extra, &facts).key
        };
        order.push(key(&extra, &current, status, has_handover));
        for step in 0..9 {
            match step {
                0 => status = "idle",
                1 => extra.claude = Some(claude("Done.")),
                2 => current = Some(mr("failed")),
                3 => current = None,
                4 => extra.actions.clear(),
                5 => extra.wait = None,
                6 => has_handover = false,
                7 => extra.change = None,
                _ => extra.unpushed = 0,
            }
            order.push(key(&extra, &current, status, has_handover));
        }
        assert_eq!(
            order,
            [
                "working",
                "asks",
                "pipeline",
                "failed",
                "waits",
                "waitsForMr",
                "next",
                "change",
                "unpushed",
                "free"
            ]
        );
    }

    #[test]
    fn the_sidebar_line_is_the_state_with_the_mr() {
        let facts = Facts {
            mr: Some(&mr("success")),
            status: Some("idle"),
            turn: Some(&MergeTurn::Offered),
            ..Default::default()
        };
        assert_eq!(sidebar_line_of(&tree(), &facts), "waits for you: merge · !192");
        assert_eq!(
            sidebar_line_of(
                &tree(),
                &Facts {
                    status: Some("working"),
                    ..Default::default()
                }
            ),
            "at work"
        );
    }
}
