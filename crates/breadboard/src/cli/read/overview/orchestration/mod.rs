//! The view of one orchestration printed (render-orchestration.mjs, once
//! mode): the title band, NEXT, ACTIONS and UP NEXT, WORKTREES, AGENTS, the
//! MERGE REQUESTS in the merge order with what each still lacks and who acts,
//! the end button and the footer. The same frame the live view draws, but
//! for its clicks: buttons show as `[ label ]`, nothing is delivered, merged
//! or written.

mod actions;
mod local;
mod merge;
mod remote;
mod tree;
mod try_state;
mod workers;

use super::agents::{Agents, agents_of, context_share_of};
use super::clock::{band_time, minute};
use super::frame::Frame;
use super::paint::{bold, button, dim, green, link, pad, red, table_rows, visible, wrap};
use super::theme::{Theme, paint_tone, pink, theme};
use actions::{Item as ActionItem, action_text, actions_of, block_of, kind_chip};
use breadboard_core::git;
use breadboard_core::record::{ORCHESTRATOR, Record, is_name, record_of};
use breadboard_core::time::{ago, now_ms, parse_iso};
pub use local::{Check, check_of};
use local::{Item, Tree, items_of, trees_of};
use merge::{Entry, Order, blockers_text, conflict_text, graph_of, merge_order_of, row_blockers_of};
use remote::{Found, Mr, Remote};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::thread;
pub use tree::check_fact;

/// What the view draws from: the record, its worktrees and issues, GitLab's part.
pub struct View<'a> {
    pub name: String,
    pub record: Record,
    pub trees: Vec<Tree>,
    pub items: Vec<Item>,
    pub remote: Option<Remote>,
    /// Why GitLab didn't answer, and when.
    pub error: Option<(String, i64)>,
    pub now: i64,
    pub theme: &'a Theme,
    pub order: Order,
    /// The issues with an MR, by number: the last of a number wins (#221).
    pub with_mr: HashMap<i64, usize>,
    /// What herdr says each agent does, by its conversation's id.
    pub agents: Agents,
    /// The starts running now (#272).
    pub setups: Vec<workers::Setup>,
    /// How much of its context each free agent (#289) used, by its id.
    pub free_shares: HashMap<String, f64>,
    /// The test instance a try `--on test` opens on.
    pub test_origin: Option<String>,
}

impl View<'_> {
    pub fn found(&self, item: &Item) -> Option<&Found> {
        self.remote.as_ref()?.of(item.branch())
    }

    pub fn mr(&self, item: &Item) -> Option<&Mr> {
        self.found(item)?.mr.as_ref()
    }

    pub fn tree(&self, item: &Item) -> Option<&Tree> {
        item.tree.map(|at| &self.trees[at])
    }

    /// The open MRs' issues in the computed merge order.
    pub fn open_order(&self) -> Vec<i64> {
        self.order.open.iter().map(|placed| placed.number).collect()
    }

    /// The MRs known over (merged or closed): the orchestration's and the awaited ones.
    fn over_iids(&self, is_merged_only: bool) -> HashSet<i64> {
        let own = self
            .items
            .iter()
            .filter_map(|item| self.mr(item))
            .filter(|mr| !mr.is_open() && (!is_merged_only || mr.is_merged()))
            .map(|mr| mr.iid);
        let awaited = self
            .remote
            .iter()
            .flat_map(|remote| remote.awaited.iter())
            .filter(|(_, state)| *state != "opened" && (!is_merged_only || *state == "merged"))
            .map(|(iid, _)| *iid);
        own.chain(awaited).collect()
    }

    pub fn merged_iids(&self) -> HashSet<i64> {
        self.over_iids(true)
    }

    fn done_iids(&self) -> HashSet<i64> {
        self.over_iids(false)
    }
}

/// `overview --orchestration <name>` printed into `frame`.
pub fn orchestration(frame: &mut Frame, main: &Path, name: &str) {
    let now = now_ms();
    let record = is_name(name).then(|| record_of(main, name)).flatten();
    let title = record
        .as_ref()
        .and_then(|record| record.title.clone())
        .unwrap_or_else(|| name.to_string());
    let label = format!("{ORCHESTRATOR} {title}");
    frame.band(&label, &band_time(now));
    let Some(record) = record.filter(|record| !record.issues.is_empty()) else {
        frame.blank();
        frame.text_at(
            0,
            &dim(&format!(
                "  no issues recorded for {label}: pnpm wt:orchestrate {name} <issue>…"
            )),
        );
        return;
    };

    let worktrees = git::worktrees(main);
    let (trees, remote, agents) = thread::scope(|scope| {
        let trees = scope.spawn(|| trees_of(main, &worktrees, name));
        // herdr's state of every agent: one call, read only.
        let agents = scope.spawn(agents_of);
        // The branches: the record's, and those of worktrees started for it by hand.
        let mut branches: Vec<String> = record.issues.iter().filter_map(|issue| issue.branch.clone()).collect();
        let awaited = awaited_of(&record, &worktrees, name);
        for tree in worktrees.iter().skip(1) {
            let is_ours = breadboard_core::record::origin_of(&tree.dir).is_some_and(|origin| origin.name == name);
            if let Some(branch) = tree
                .branch
                .clone()
                .filter(|branch| is_ours && !branches.contains(branch))
            {
                branches.push(branch);
            }
        }
        let remote = remote::read(main, &branches, &awaited);
        (
            trees.join().unwrap_or_default(),
            remote,
            agents.join().unwrap_or_default(),
        )
    });
    let (mut remote, error) = match remote {
        Ok(remote) => (Some(remote), None),
        Err(reason) => (None, Some((reason, now_ms()))),
    };
    let items = items_of(&record.issues, &trees);
    let test_origin = try_state::test_origin(main);
    if let Some(remote) = remote.as_mut() {
        read_deploys(main, remote, &items, &trees, now);
    }
    let free_shares = record
        .free_agents()
        .into_iter()
        .map(|agent| (agent.id.clone(), context_share_of(&agent.id).unwrap_or(0.0)))
        .collect();
    let mut view = View {
        name: name.to_string(),
        record,
        trees,
        items,
        remote,
        error,
        now,
        theme: theme(main),
        order: Order::default(),
        with_mr: HashMap::new(),
        agents,
        setups: workers::setups_of(main, name),
        free_shares,
        test_origin,
    };
    view.with_mr = view
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| view.mr(item).is_some())
        .filter_map(|(at, item)| Some((item.number()?, at)))
        .collect();
    let plan_merge = view
        .record
        .plan
        .as_ref()
        .map(|plan| plan.merge.clone())
        .unwrap_or_default();
    let entries: Vec<Entry> = view
        .with_mr
        .iter()
        .filter_map(|(number, at)| {
            let item = &view.items[*at];
            let found = view.found(item)?;
            let tree = view.tree(item);
            Some(Entry {
                number: *number,
                mr: found.mr.as_ref()?,
                needs: found
                    .issue
                    .as_ref()
                    .map(|issue| issue.needs.clone())
                    .unwrap_or_default(),
                files: tree.map(|tree| tree.files.clone()).unwrap_or_default(),
                boxes: found
                    .issue
                    .as_ref()
                    .map(|issue| issue.boxes.as_slice())
                    .unwrap_or_default(),
                behind: tree.map_or(0, |tree| tree.behind),
                tree,
            })
        })
        .collect();
    let order = merge_order_of(&entries, &plan_merge);
    view.order = order;
    draw(frame, &view);
}

/// Where the deployment of a merged MR stands, for a try of its worktree on
/// the test instance that is still open: only those, the print asks nothing more.
fn read_deploys(main: &Path, remote: &mut Remote, items: &[Item], trees: &[Tree], now: i64) {
    for item in items {
        let Some(tree) = item.tree.map(|at| &trees[at]) else {
            continue;
        };
        let is_open_test_try = tree
            .actions
            .iter()
            .any(|action| action.is_try() && action.on.as_deref() == Some("test") && action.answer.is_none());
        if !is_open_test_try {
            continue;
        }
        let Some(mr) = item
            .branch()
            .and_then(|branch| remote.of.get_mut(branch))
            .and_then(|found| found.mr.as_mut())
            .filter(|mr| mr.is_merged())
        else {
            continue;
        };
        if let Some(sha) = mr.merge_sha.clone() {
            mr.deploy = try_state::deploy_of(main, &sha, now);
        }
    }
}

/// The MRs the handovers, the queued issues and the waits of its worktrees name.
fn awaited_of(record: &Record, worktrees: &[git::Worktree], name: &str) -> Vec<i64> {
    let mut iids: Vec<i64> = record
        .handovers
        .iter()
        .flat_map(|handover| handover.after.clone())
        .collect();
    iids.extend(
        record
            .issues
            .iter()
            .filter(|issue| issue.is_queued)
            .flat_map(|issue| issue.after.clone()),
    );
    for tree in worktrees.iter().skip(1) {
        if breadboard_core::record::origin_of(&tree.dir).is_some_and(|origin| origin.name == name) {
            iids.extend(local::wait_of(&tree.dir).unwrap_or_default());
        }
    }
    let mut seen = HashSet::new();
    iids.retain(|iid| seen.insert(*iid));
    iids
}

/// The MR next in the merge order: the first nothing holds up, else the
/// first and what holds it up; its line when it is ready.
struct NextMerge {
    is_ready: bool,
    iid: i64,
    hold_up: Vec<String>,
    line: String,
}

fn next_merge(view: &View) -> Option<NextMerge> {
    let open: Vec<_> = view
        .order
        .open
        .iter()
        .filter(|placed| view.with_mr.contains_key(&placed.number))
        .collect();
    let ready: Vec<_> = open.iter().filter(|placed| placed.hold_up.is_empty()).collect();
    let first = ready.first().copied().or(open.first())?;
    let item = &view.items[view.with_mr[&first.number]];
    let mr = view.mr(item)?;
    let more = if ready.len() > 1 {
        dim(&format!(" · {} more ready", ready.len() - 1))
    } else {
        String::new()
    };
    Some(NextMerge {
        is_ready: !ready.is_empty(),
        iid: mr.iid,
        hold_up: first.hold_up.clone(),
        line: format!(
            "{}  {} {}{more}",
            green("ready to merge"),
            link(mr.url.as_deref(), &format!("!{}", mr.iid)),
            bold(&item.feature)
        ),
    })
}

/// What an MR next in the merge order waits for, after `once` (next.mjs `onceText`).
fn once_text(hold_up: &[String]) -> String {
    type Pattern<'a> = (&'a dyn Fn(&str) -> bool, &'a str);
    let patterns: [Pattern; 9] = [
        (&|hold| hold.starts_with("conflicts"), "its conflicts are resolved"),
        (&|hold| hold.starts_with("pipeline failed"), "its pipeline is fixed"),
        (&|hold| hold.starts_with("pipeline"), "its pipeline passes"),
        (&|hold| hold.starts_with("after !"), ""),
        (&|hold| hold.contains("work box"), "its work boxes are ticked"),
        (
            &|hold| hold.contains("acceptance box"),
            "its acceptance boxes are ticked",
        ),
        (
            &|hold| hold.starts_with("try #") && hold.contains(" broken"),
            "its fix is tried again",
        ),
        (&|hold| hold.starts_with("try #"), "its tries look good"),
        (&|hold| hold.starts_with("draft"), "its agent posts the merge"),
    ];
    for (matches, text) in patterns {
        if let Some(hold) = hold_up.iter().find(|hold| matches(hold)) {
            if text.is_empty() {
                let iid: String = hold["after ".len()..]
                    .chars()
                    .take_while(|c| *c == '!' || c.is_ascii_digit())
                    .collect();
                return format!("{iid} is merged");
            }
            return text.to_string();
        }
    }
    hold_up.first().cloned().unwrap_or_default()
}

/// Each issue's state for the plan's staleness: merged, closed (without a merge) or open.
fn states_of(view: &View) -> HashMap<i64, &'static str> {
    view.items
        .iter()
        .filter_map(|item| {
            let number = item.number()?;
            let state = if view.mr(item).is_some_and(Mr::is_merged) {
                "merged"
            } else if view
                .found(item)
                .and_then(|found| found.issue.as_ref())
                .is_some_and(|issue| issue.state == "closed")
            {
                "closed"
            } else {
                "open"
            };
            Some((number, state))
        })
        .collect()
}

/// Why the plan is stale, one short line each (next.mjs `staleOf`).
fn stale_of(view: &View) -> Vec<String> {
    let Some(plan) = &view.record.plan else {
        return if view.order.is_off_plan {
            vec!["the merge order left the plan's".into()]
        } else {
            Vec::new()
        };
    };
    let states = states_of(view);
    let mut listed: Vec<i64> = Vec::new();
    for number in plan.work.iter().chain(&plan.merge) {
        if !listed.contains(number) {
            listed.push(*number);
        }
    }
    let issues = |numbers: Vec<i64>| {
        numbers
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let gone: Vec<i64> = listed
        .iter()
        .copied()
        .filter(|number| !states.contains_key(number))
        .collect();
    let closed: Vec<i64> = listed
        .iter()
        .copied()
        .filter(|number| states.get(number) == Some(&"closed"))
        .collect();
    let mut named: Vec<i64> = Vec::new();
    let why = plan.why.clone().unwrap_or_default();
    for part in why.split('#').skip(1) {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(number) = digits.parse::<i64>()
            && !named.contains(&number)
            && !part[digits.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
        {
            named.push(number);
        }
    }
    let done: Vec<i64> = named
        .into_iter()
        .filter(|number| !listed.contains(number) || states.get(number) != Some(&"open"))
        .collect();
    [
        (!closed.is_empty()).then(|| format!("{} closed", issues(closed))),
        (!gone.is_empty()).then(|| format!("{} no longer in the orchestration", issues(gone))),
        (!done.is_empty()).then(|| format!("its why names {}, done or gone", issues(done))),
        view.order
            .is_off_plan
            .then(|| "the merge order left the plan's".to_string()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// `GitLab didn't answer 22:29 (reason)` (gitlab-status.mjs `errorText`, without
/// its `tried again`: a print asks once).
fn error_text(reason: &str, at: i64) -> String {
    // The reason alone: the read says it is GitLab's, the call isn't the reader's business.
    let reason = reason.lines().next().unwrap_or_default();
    let reason = reason.strip_prefix("GitLab: ").unwrap_or(reason);
    let reason = reason.rfind(" (projects/").map_or(reason, |at| &reason[..at]);
    format!("GitLab didn't answer {} ({reason})", minute(at))
}

fn draw(frame: &mut Frame, view: &View) {
    let width = frame.width();
    if let Some((reason, at)) = &view.error {
        frame.blank();
        frame.text_at(0, &red(&error_text(reason, *at)));
    }
    let stale = if view.remote.is_some() {
        stale_of(view)
    } else {
        Vec::new()
    };
    let merge = view.remote.as_ref().and(next_merge(view));
    let workers = workers::workers_of(view, tree::free_memory().unwrap_or(0.0));
    let listed = actions_of(view, &workers, &stale, merge.is_some());
    frame.blank();
    draw_next(frame, view, &listed, merge.as_ref(), &stale);
    frame.blank();

    let inner = frame.inner();
    let count = listed.actions.len();
    let has_auto = listed.actions.iter().any(|item| item.is_auto);
    let summary = if count > 0 {
        format!(
            "{count} for you · a button or an option answers, the rest of a block does it{}",
            if has_auto { " · auto ones go by themselves" } else { "" }
        )
    } else {
        "nothing waits for you".into()
    };
    frame.boxed("ACTIONS", &format!("{} {}", bold("ACTIONS"), dim(&summary)), |width| {
        if listed.actions.is_empty() {
            return vec![dim("none: an agent asks with pnpm wt:action <kind> <text>")];
        }
        let mut lines = Vec::new();
        for (at, item) in listed.actions.iter().enumerate() {
            if at > 0 {
                lines.push(dim(&"─".repeat(width)));
            }
            lines.extend(block_of(view, item, width));
        }
        lines
    });
    if !listed.up_next.is_empty() {
        let title = format!(
            "{} {}",
            bold("UP NEXT"),
            dim(&format!(
                "{} automatic · a click does it, each goes by itself",
                listed.up_next.len()
            ))
        );
        frame.boxed("UP NEXT", &title, |width| up_next_lines(view, &listed.up_next, width));
    }

    let (title, lines) = match super::frame::guarded(|| tree::worktrees(view, &workers, inner)) {
        Ok(drawn) => drawn,
        Err(message) => (
            bold("WORKTREES"),
            vec![red(&format!("WORKTREES could not be shown: {message}"))],
        ),
    };
    frame.boxed("WORKTREES", &title, |_| lines);

    let free = view.record.free_agents();
    if !free.is_empty() {
        let title = format!(
            "{} {}",
            bold("AGENTS"),
            dim(&format!(
                "{} free · wait in their tab for the next issue; two clicks dismiss one",
                free.len()
            ))
        );
        frame.boxed("AGENTS", &title, |width| {
            let rows: Vec<Vec<String>> = free
                .iter()
                .map(|agent| {
                    let name = agent.name.clone().unwrap_or_else(|| agent.id.clone());
                    let from = agent.free.as_ref().and_then(|free| {
                        let from = free.from.clone()?;
                        Some(match free.mr {
                            Some(mr) => format!("from {from} (!{mr})"),
                            None => format!("from {from}"),
                        })
                    });
                    let issues = (!agent.issues.is_empty()).then(|| {
                        format!(
                            "{} issue{}",
                            agent.issues.len(),
                            if agent.issues.len() == 1 { "" } else { "s" }
                        )
                    });
                    let share = view.free_shares.get(&agent.id).copied().unwrap_or(0.0);
                    let percent = format!("{} %", (share * 100.0).round() as i64);
                    let text: Vec<String> = [Some(percent), from, issues].into_iter().flatten().collect();
                    // Its state as for an agent without a worktree: at work while it takes its line, asks you, else free.
                    let (state, tone) = match view.agents.get(&agent.id).map(|agent| agent.status.as_str()) {
                        Some("working") => ("at work", "work"),
                        Some("blocked") => ("asks you", "you"),
                        _ => ("free", "free"),
                    };
                    vec![
                        format!(
                            "{} {}",
                            paint_tone(view.theme, Some(tone), state),
                            pink(view.theme, &name)
                        ),
                        format!("{} {}", dim(&text.join(" · ")), button("✕ dismiss", "red")),
                    ]
                })
                .collect();
            table_rows(&rows, width).into_iter().flatten().collect()
        });
    }

    draw_mrs(frame, view);

    let mut left: Vec<String> = view.trees.iter().map(|tree| tree.name.clone()).collect();
    left.extend(
        view.items
            .iter()
            .filter(|item| item.entry.is_queued)
            .filter_map(|item| item.number().map(|number| format!("#{number} (queued)"))),
    );
    let end = if left.is_empty() {
        format!(
            "{} {}",
            button("✕ end orchestration", "red"),
            dim(&format!("pnpm wt:orchestrate {} --end", view.name))
        )
    } else {
        format!(
            "{} {}",
            button("✕ end orchestration", "gray"),
            dim(&format!(
                "not yet, worktree{} left: {}",
                if left.len() == 1 { "" } else { "s" },
                left.join(", ")
            ))
        )
    };
    let room = frame.inner().saturating_sub(2).max(16);
    for line in wrap(&end, room) {
        frame.push(format!("  {line}"));
    }
    frame.blank();
    frame.push(dim(
        " SendMessage(to: \"<feature>\") reaches an agent · pnpm overview (everything)",
    ));
    let _ = width;
}

/// NEXT (render-orchestration.mjs `pushNext`): the first action not answered
/// yet, else the first of UP NEXT, else the merge that is ready, each as in
/// its section; else one dim line that nothing is for you.
fn draw_next(frame: &mut Frame, view: &View, listed: &actions::Listed, merge: Option<&NextMerge>, stale: &[String]) {
    let width = frame.width();
    let label = format!(" {}  ", bold("NEXT"));
    let indent = " ".repeat(visible(&label));
    let action = listed
        .actions
        .iter()
        .find(|item| item.action.answer.is_none() && !item.is_waiting());
    let lines: Vec<String> = if let Some(item) = action {
        block_of(view, item, width - indent.len())
    } else if let Some(item) = listed.up_next.first() {
        wrap(
            &format!(
                "{} {}  {}",
                kind_chip(&item.action.kind, false),
                bold(&item.who),
                action_text(view, item)
            ),
            width - indent.len(),
        )
    } else if let Some(merge) = merge.filter(|merge| merge.is_ready) {
        wrap(&merge.line, width - indent.len())
    } else {
        let text = if view.remote.is_some() {
            let merge = merge
                .map(|merge| {
                    let once = if merge.hold_up.is_empty() {
                        String::new()
                    } else {
                        format!(" once {}", once_text(&merge.hold_up))
                    };
                    format!(" · next merge: !{}{once}", merge.iid)
                })
                .unwrap_or_default();
            let at_work = view
                .trees
                .iter()
                .filter_map(|tree| tree.claude.as_ref())
                .filter(|claude| {
                    view.agents
                        .get(&claude.id)
                        .is_some_and(|agent| agent.status == "working")
                })
                .count();
            let at_work = match at_work {
                0 => String::new(),
                1 => " · 1 agent at work".into(),
                count => format!(" · {count} agents at work"),
            };
            format!("nothing for you{at_work}{merge}")
        } else {
            "what is for you can't be told: GitLab didn't answer".into()
        };
        for line in wrap(&dim(&text), width - 1) {
            frame.push(format!(" {line}"));
        }
        return;
    };
    for (at, line) in lines.iter().enumerate() {
        frame.push(format!("{}{line}", if at == 0 { &label } else { &indent }));
    }
    // The plan's why under a merge, never once the plan is stale.
    let is_merge = action.is_some_and(|item| item.action.kind == "merge")
        || (action.is_none() && listed.up_next.is_empty() && merge.is_some_and(|merge| merge.is_ready));
    let why = view
        .record
        .plan
        .as_ref()
        .filter(|plan| !plan.merge.is_empty() && stale.is_empty() && is_merge)
        .and_then(|plan| plan.why.clone())
        .filter(|why| !why.is_empty());
    if let Some(why) = why {
        for line in wrap(&dim(&why), width - indent.len()) {
            frame.push(format!("{indent}{line}"));
        }
    }
}

/// UP NEXT's rows: a head of chip, who and age, the text under it.
fn up_next_lines(view: &View, items: &[ActionItem], width: usize) -> Vec<String> {
    let heads: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            vec![
                kind_chip(&item.action.kind, false),
                bold(&item.who),
                parse_iso(&item.action.at)
                    .map(|at| dim(&ago(at, view.now)))
                    .unwrap_or_default(),
            ]
        })
        .collect();
    let heads = table_rows(&heads, usize::MAX);
    items
        .iter()
        .zip(heads)
        .flat_map(|(item, head)| {
            let mut lines = head;
            lines.extend(wrap(&action_text(view, item), width));
            lines
        })
        .collect()
}

/// A row of MERGE REQUESTS.
struct MrRow {
    graph: String,
    below: String,
    cells: Vec<String>,
    title: String,
    status: String,
    text: String,
}

/// MERGE REQUESTS: the open ones as a graph in the merge order with what
/// each lacks, the merged below, GitLab's minute at the end.
fn draw_mrs(frame: &mut Frame, view: &View) {
    let done: Vec<usize> = view
        .order
        .merged
        .iter()
        .filter_map(|number| view.with_mr.get(number).copied())
        .collect();
    let open_count = view.order.open.len();
    let merged_count = done
        .iter()
        .filter(|at| view.mr(&view.items[**at]).is_some_and(Mr::is_merged))
        .count();
    let waiting = view
        .items
        .iter()
        .filter(|item| item.tree.is_some() && view.remote.is_some() && view.mr(item).is_none())
        .count();
    let counts: Vec<String> = [
        Some(format!("{open_count} open")),
        (merged_count > 0).then(|| format!("{merged_count} merged")),
        (waiting > 0).then(|| format!("{waiting} without one yet")),
        (open_count > 0).then(|| "in merge order, side by side where independent".to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let title = format!("{} {}", bold("MERGE REQUESTS"), dim(&counts.join(" · ")));
    frame.boxed("MERGE REQUESTS", &title, |width| {
        let Some(remote) = &view.remote else {
            return vec![dim("GitLab does not answer yet")];
        };
        if open_count + done.len() == 0 {
            return vec![dim("none yet"), dim(&format!("GitLab {}", minute(remote.at)))];
        }
        let done_iids = view.done_iids();
        let after_of: HashMap<i64, &[merge::Pred]> = view
            .order
            .open
            .iter()
            .map(|placed| (placed.number, placed.after.as_slice()))
            .collect();
        let mut rows: Vec<MrRow> = Vec::new();
        let mut is_merged: Vec<bool> = Vec::new();
        for graph in graph_of(&view.order.open) {
            let item = graph
                .number
                .and_then(|number| view.with_mr.get(&number))
                .map(|at| &view.items[*at]);
            let found = item.and_then(|item| view.found(item));
            let row = match (item, found.and_then(|found| found.mr.as_ref())) {
                (Some(item), Some(mr)) => {
                    let tree = view.tree(item);
                    let boxes = found
                        .and_then(|found| found.issue.as_ref())
                        .map(|issue| issue.boxes.as_slice());
                    let behind = tree.map_or(0, |tree| tree.behind);
                    let reasons = row_blockers_of(
                        mr,
                        boxes.unwrap_or_default(),
                        behind,
                        item.number()
                            .and_then(|number| after_of.get(&number).copied())
                            .unwrap_or_default(),
                        &found.map(|found| found.jobs.clone()).unwrap_or_default(),
                        tree,
                        &done_iids,
                    );
                    let text: Vec<String> = [conflict_text(mr, tree, behind, &view.agents), blockers_text(&reasons)]
                        .into_iter()
                        .filter(|part| visible(part) > 0)
                        .collect();
                    MrRow {
                        graph: graph.graph,
                        below: graph.below,
                        cells: vec![
                            link(mr.url.as_deref(), &format!("!{}", mr.iid)),
                            tree::pipeline_mark(mr),
                        ],
                        title: mr.title.clone(),
                        status: if reasons.is_empty() {
                            green("ready to merge")
                        } else {
                            String::new()
                        },
                        text: text.join(&dim(" · ")),
                    }
                }
                _ => MrRow {
                    graph: graph.graph,
                    below: graph.below,
                    cells: Vec::new(),
                    title: String::new(),
                    status: String::new(),
                    text: String::new(),
                },
            };
            rows.push(row);
            is_merged.push(false);
        }
        for at in &done {
            let Some(mr) = view.mr(&view.items[*at]) else { continue };
            let when = mr
                .merged_at
                .as_deref()
                .and_then(parse_iso)
                .map(|at| ago(at, view.now))
                .unwrap_or_default();
            rows.push(MrRow {
                graph: String::new(),
                below: String::new(),
                cells: vec![
                    link(mr.url.as_deref(), &format!("!{}", mr.iid)),
                    tree::pipeline_mark(mr),
                ],
                title: mr.title.clone(),
                status: dim(&format!("merged {when}")),
                text: String::new(),
            });
            is_merged.push(true);
        }
        let mut lines = Vec::new();
        for (at, row_lines) in mr_row_lines(&rows, width).into_iter().enumerate() {
            if is_merged[at] && (at == 0 || !is_merged[at - 1]) {
                lines.push(dim(&format!("▾ {} merged", done.len())));
            }
            lines.extend(row_lines);
        }
        lines.push(dim(&format!("GitLab {}", minute(remote.at))));
        lines
    });
}

/// The fewest columns an MR's title gets, however narrow the view.
const MIN_TITLE: usize = 24;
/// Below this many columns the reasons start further left.
const MIN_TEXT: usize = 40;

/// The rows of MERGE REQUESTS as the lines of each (mr-blockers.mjs
/// `mrRowLines`): the graph, `!N` and the pipeline, the title with the room
/// left and the status at its end; below it what the MR still lacks, under
/// the title, the lanes that go on down to its left. A title too long for
/// its column wraps under itself (#304: never cut).
fn mr_row_lines(rows: &[MrRow], width: usize) -> Vec<Vec<String>> {
    let leads: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            let mut lead = vec![dim(&row.graph)];
            lead.extend(row.cells.iter().cloned());
            lead
        })
        .collect();
    let mut widths: Vec<usize> = Vec::new();
    for lead in &leads {
        for (column, cell) in lead.iter().enumerate() {
            if widths.len() <= column {
                widths.push(0);
            }
            widths[column] = widths[column].max(visible(cell));
        }
    }
    let start: usize = widths.iter().take(3).map(|width| width + 2).sum();
    let status = rows.iter().map(|row| visible(&row.status)).max().unwrap_or(0);
    let room = width
        .saturating_sub(start + if status > 0 { status + 2 } else { 0 })
        .max(MIN_TITLE);
    let column = room.min(rows.iter().map(|row| visible(&row.title)).max().unwrap_or(0));
    let indent = if width.saturating_sub(start) >= MIN_TEXT {
        start
    } else {
        widths.first().map_or(0, |width| width + 2)
    };
    let below_room = width.saturating_sub(indent).max(16);
    rows.iter()
        .zip(&leads)
        .map(|(row, lead)| {
            let head: String = lead
                .iter()
                .enumerate()
                .map(|(column, cell)| format!("{}  ", pad(cell, widths[column])))
                .collect();
            let head = pad(&head, start);
            if row.cells.is_empty() {
                return vec![lead[0].clone()];
            }
            let title_room = if row.status.is_empty() {
                column + if status > 0 { status + 2 } else { 0 }
            } else {
                column
            };
            let titles = wrap(&row.title, title_room.max(1));
            let mut lines: Vec<String> = Vec::new();
            for (at, title) in titles.iter().enumerate() {
                let lead = if at == 0 { head.clone() } else { " ".repeat(start) };
                if at == 0 && !row.status.is_empty() {
                    lines.push(format!("{lead}{}  {}", pad(title, column), row.status));
                } else {
                    lines.push(format!("{lead}{title}"));
                }
            }
            if !row.text.is_empty() {
                lines.extend(
                    wrap(&row.text, below_room)
                        .into_iter()
                        .map(|line| format!("{}{line}", " ".repeat(indent))),
                );
            }
            // The lanes that go on below it, down its further lines.
            let lanes = visible(&row.graph);
            lines
                .into_iter()
                .enumerate()
                .map(|(at, line)| {
                    if at > 0 && !row.below.is_empty() && line.starts_with(&" ".repeat(lanes)) {
                        format!("{}{}", dim(&format!("{:<lanes$}", row.below)), &line[lanes..])
                    } else {
                        line
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_read_says_why_in_a_line() {
        let text = error_text(
            "GitLab: glab: 502 Bad Gateway (projects/:fullpath/merge_requests?state=opened)",
            0,
        );
        assert!(text.starts_with("GitLab didn't answer "), "{text}");
        assert!(text.ends_with(" (glab: 502 Bad Gateway)"), "{text}");
    }

    #[test]
    fn says_what_the_next_merge_waits_for() {
        let hold = |list: &[&str]| list.iter().map(|hold| hold.to_string()).collect::<Vec<_>>();
        assert_eq!(once_text(&hold(&["draft", "pipeline pending"])), "its pipeline passes");
        assert_eq!(
            once_text(&hold(&["draft", "after !303 (needs #292)"])),
            "!303 is merged"
        );
        assert_eq!(once_text(&hold(&["draft"])), "its agent posts the merge");
        assert_eq!(once_text(&hold(&["GitLab is checking"])), "GitLab is checking");
    }

    #[test]
    fn an_mr_row_has_its_reasons_under_its_title_and_never_cuts() {
        let rows = vec![
            MrRow {
                graph: "●".into(),
                below: "│".into(),
                cells: vec!["!311".into(), "… pending".into()],
                title: "feat(breadboard): a title".into(),
                status: String::new(),
                text: "checking for conflicts… · pipeline pending\u{a0}→\u{a0}GitLab".into(),
            },
            MrRow {
                graph: String::new(),
                below: String::new(),
                cells: vec!["!249".into(), "✓ passed".into()],
                title: "feat(breadboard): an orchestration shows its own title".into(),
                status: "merged 29 h ago".into(),
                text: String::new(),
            },
        ];
        let lines = mr_row_lines(&rows, 61);
        assert_eq!(lines[0][0], "●  !311  … pending  feat(breadboard): a title");
        assert_eq!(lines[0][1], "│                   checking for conflicts… · pipeline");
        assert!(
            lines[1][0].starts_with("   !249  ✓ passed   feat(breadboard): an"),
            "{lines:?}"
        );
        assert!(lines[1][0].ends_with("merged 29 h ago"), "{lines:?}");
        assert!(lines.iter().flatten().all(|line| visible(line) <= 61), "{lines:?}");
        // The whole title, over its lines: nothing cut.
        assert_eq!(
            lines[1]
                .join(" ")
                .split_whitespace()
                .filter(|word| *word == "title")
                .count(),
            1
        );
    }
}
