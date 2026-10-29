//! A node's snapshot (#497): everything this machine runs, as data a server
//! keeps, versioned (`PROTOCOL`). Built from the same local facts as the
//! views (`OrchestrationLocal`, `EverythingLocal`, the run engine's table),
//! without what only draws them (`Paint`, labels, merge offers). GitLab's
//! side (MRs, pipelines) is not in it: the server reads GitLab once for
//! everybody.
//!
//! Its JSON keys are camelCase, its times ms since the epoch; a field that
//! is unknown is `null`, a list without entries `[]`: every key is always
//! there, so its shape stays one (`tests` pins it).
//!
//! An orchestration's daemon-side actor (#498) is in it too (#509): what it
//! has under way and its log, from `<name>.acting`, so a reader sees what
//! the node does by itself.
//!
//! And what the node's processes take (#513, protocol 2): the machine, the
//! top processes, the sums per worktree and agent run (`resources`), in the
//! snapshot and, sampled every few seconds, as the stream's `Event::Resources`.

use crate::check::CheckState;
use crate::conversations::question_of;
use crate::model::acting::MergePhase;
use crate::model::agent_state::{Facts, agent_state_of};
use crate::model::everything::EverythingLocal;
use crate::model::orchestration::OrchestrationLocal;
use crate::model::{Entry, Tree, issues_of};
use crate::spend::Spend;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The version of the snapshot's shape: a change a reader of the last one
/// misreads raises it.
pub const PROTOCOL: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub protocol: u32,
    /// When it was taken.
    pub at: i64,
    pub node: Node,
    pub repos: Vec<Repo>,
    /// What its processes take, None where the OS couldn't be read.
    pub resources: Option<crate::resources::Resources>,
}

/// What a node sends between two snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    /// A sample of what its processes take.
    #[serde(rename_all = "camelCase")]
    Resources {
        at: i64,
        resources: crate::resources::Resources,
    },
}

/// The machine and who runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub host: String,
    /// The OS user.
    pub user: String,
    /// The binary's version and the key of its sources.
    pub version: String,
}

/// A repo the node works in: its main checkout's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub main: PathBuf,
    /// The main checkout's folder name.
    pub name: String,
    pub develop: String,
    pub orchestrations: Vec<Orchestration>,
    /// Every worktree, the main checkout first.
    pub worktrees: Vec<Worktree>,
    pub agents: Vec<Agent>,
    pub runs: Runs,
    /// What the repo's claudes spent today.
    pub today: Tokens,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Orchestration {
    pub name: String,
    pub title: String,
    /// The product of `[products.*]` it belongs to.
    pub product: Option<String>,
    /// A checker answers its tries (#384).
    pub is_auto: bool,
    pub issues: Vec<Issue>,
    /// The orchestrator's order (`wt:plan`): issues to work on, to merge.
    pub work: Vec<i64>,
    pub merge: Vec<i64>,
    /// The orchestrator's own actions.
    pub actions: Vec<Action>,
    /// Its epic branch (#350), None for one on develop or a batch.
    pub epic: Option<String>,
    pub batch: Option<Batch>,
    /// What its agents and its orchestrator spent, all time.
    pub spent: Tokens,
    /// What its actor keeps, None while no process acts for it.
    pub acting: Option<Acting>,
}

/// What the actor of an orchestration has under way and did (#498's `Acting`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Acting {
    /// The process that acts.
    pub pid: i64,
    /// The automatic wt:removes, by worktree.
    pub removals: Vec<Removal>,
    /// The starts and handovers, by issue.
    pub starts: Vec<Start>,
    /// The failed pipelines' lines that didn't get in.
    pub pipeline_tries: Vec<PipelineTry>,
    /// The worktrees whose ended wait didn't get in yet.
    pub undelivered_waits: Vec<PathBuf>,
    /// The merges, by MR.
    pub merges: Vec<Merge>,
    /// What it did, newest first.
    pub log: Vec<Act>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removal {
    pub worktree: PathBuf,
    pub is_armed: bool,
    pub is_running: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Start {
    pub issue: i64,
    pub is_running: bool,
    pub error: Option<String>,
    /// When it ended, None while it never did.
    pub ended: Option<i64>,
    /// The worktree of the agent it hands the issue to.
    pub worktree: Option<PathBuf>,
    /// For a fresh agent.
    pub is_fresh: bool,
    /// The agent it goes to.
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineTry {
    pub key: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Merge {
    pub mr: i64,
    pub phase: Phase,
    pub error: Option<String>,
}

/// Where a merge stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Armed,
    Running,
    Merged,
    Failed,
}

/// An entry of the actor's log, without its paint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Act {
    /// When it was done first, ms.
    pub at: i64,
    /// When it was done the last time, ms.
    pub last: i64,
    /// How often it was done.
    pub times: u32,
    /// One act's entry: its end replaces its start.
    pub key: Option<String>,
    pub text: String,
    /// The command run or the line typed, word for word.
    pub detail: Option<String>,
}

/// An issue of an orchestration, as its record keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: Option<i64>,
    /// Every issue of its task (#321), `number` first.
    pub numbers: Vec<i64>,
    pub title: Option<String>,
    pub feature: String,
    pub branch: String,
    /// Its worktree while it has one.
    pub worktree: Option<PathBuf>,
    /// Waits for an agent (`--queue`).
    pub is_queued: bool,
    /// The MRs it starts after.
    pub after: Vec<i64>,
    /// Why it got no worktree or is back in the queue.
    pub why: Option<String>,
    /// ISO times, as the record has them.
    pub merged_at: Option<String>,
    pub removed_at: Option<String>,
}

/// A product's batch an orchestration's tasks merge into (#411).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub branch: String,
    pub number: i64,
    /// Commits of develop it lacks, None while git can't tell.
    pub behind: Option<u64>,
    /// The batches shipped before it, their MRs into develop still open.
    pub shipped: Vec<String>,
    /// The fix issues filed for red shipped batches: (issue, batch).
    pub fixes: Vec<Fix>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub issue: i64,
    pub batch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub dir: PathBuf,
    /// Its feature, the folder name of the main checkout.
    pub name: String,
    pub branch: String,
    /// The branch it is based on: develop, an epic, a batch.
    pub base: String,
    pub is_main: bool,
    /// The orchestration that started it.
    pub orchestration: Option<String>,
    /// Every issue of its task.
    pub issues: Vec<i64>,
    /// The session of its agent.
    pub agent: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub changed: usize,
    pub unpushed: u64,
    /// The commit of `origin/<branch>`.
    pub pushed: Option<String>,
    pub check: Check,
    /// docker's status of its stack, None while it is down or unknown.
    pub stack: Option<String>,
    /// Its open actions.
    pub actions: Vec<Action>,
}

/// Where its HEAD stands with wt:check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Check {
    #[serde(rename_all = "camelCase")]
    Checking {
        since: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Tested {
        sha: String,
        packages: Vec<String>,
        no_test: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        sha: String,
        /// `<package> <task>`, each failing test after it.
        failures: Vec<CheckFailure>,
        refused: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Untested {
        commits: u64,
    },
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckFailure {
    pub package: String,
    pub task: String,
    pub tests: Vec<String>,
}

/// What an agent asks of its user: decide, try, permission, merge, look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: i64,
    pub kind: String,
    pub text: String,
    pub url: Option<String>,
    /// A try's command.
    pub run: Option<String>,
    /// `test` for a try on the test instance.
    pub on: Option<String>,
    /// A decide's options, what each does.
    pub options: Vec<Choice>,
    pub recommend: Option<String>,
    /// The MR of a merge.
    pub mr: Option<i64>,
    /// A permission's rule.
    pub rule: Option<String>,
    /// ISO times, as the actions file has them.
    pub at: String,
    pub answer: Option<String>,
    pub answered_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub label: String,
    pub description: Option<String>,
}

/// An agent: a claude conversation working in a worktree, or free (#289).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub session: String,
    /// The person's name it is shown by.
    pub name: Option<String>,
    pub orchestration: Option<String>,
    /// Its worktree, None for a free one.
    pub worktree: Option<PathBuf>,
    /// What it waits for (`agent_state_of`'s key): `working`, `asks`,
    /// `pipeline`, `free`, … GitLab's side unknown here.
    pub state: String,
    /// Its question, else what it is on: its note or its last answer's first line.
    pub line: Option<String>,
    pub line_at: Option<i64>,
    pub is_asking: bool,
    /// The model it last answered with.
    pub model: Option<String>,
    /// How full its context is, 0 to 1.
    pub context: Option<f64>,
    /// What it spent today.
    pub today: Tokens,
}

/// The run engine's side (#358): each session's run, and what holds them all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runs {
    pub runs: Vec<Run>,
    /// Since when the user paused every run (#397).
    pub paused_since: Option<i64>,
    /// Why no run starts: the daily limit (#372).
    pub held: Option<String>,
    /// The Claude usage limit holding them (#393).
    pub usage: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub session: String,
    pub dir: PathBuf,
    pub name: String,
    pub state: crate::runs::State,
    pub since: i64,
    /// The lines it was given.
    pub lines: usize,
    /// The lines waiting for its next run.
    pub waiting: usize,
    /// The tools its last run was denied.
    pub denied: Vec<String>,
    /// How its last run ended, None while it runs or when unknown.
    pub code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub tokens: u64,
    pub dollars: f64,
}

impl From<Spend> for Tokens {
    fn from(spend: Spend) -> Tokens {
        Tokens {
            tokens: spend.tokens,
            dollars: spend.dollars,
        }
    }
}

/// What a repo's snapshot is built from: the local reads of the views and
/// the run engine's files.
#[derive(Debug, Clone, Default)]
pub struct RepoFacts {
    pub main: PathBuf,
    pub develop: String,
    pub everything: EverythingLocal,
    pub orchestrations: Vec<OrchestrationLocal>,
    pub table: crate::runs::Table,
    pub queue: crate::runs::Queue,
    pub paused: Option<crate::runs::Paused>,
    pub held: Option<String>,
    pub usage: Option<String>,
    pub today: crate::spend::Today,
    /// What the actors of its orchestrations keep, by name, of a process that lives.
    pub acting: HashMap<String, crate::model::acting::Acting>,
}

impl RepoFacts {
    /// The facts of the repo `here` stands in, read now; None outside a repo.
    pub fn read(here: &Path, now: i64) -> Option<RepoFacts> {
        let mut reader = crate::reader::LocalReader::new(here, crate::reader::View::Everything)?;
        reader.should_wait_for_docker = true;
        let main = reader.main.clone();
        let everything = reader.read_everything(now);
        let names = crate::record::recorded(&main);
        let orchestrations = names.iter().map(|name| reader.read_orchestration(name, now)).collect();
        let acting = names
            .iter()
            .filter_map(|name| Some((name.clone(), crate::model::acting::read(&main, name)?)))
            .collect();
        Some(RepoFacts {
            develop: crate::git::default_branch().to_string(),
            everything,
            orchestrations,
            table: crate::runs::read_or_default(&crate::runs::status_file(&main)),
            queue: crate::runs::read_or_default(&crate::runs::queue_file(&main)),
            paused: crate::runs::paused_of(&main),
            held: crate::runs::held_now(&main, now),
            usage: crate::runs::usage_now(&main, now),
            today: crate::spend::today_of(&main, now),
            acting,
            main,
        })
    }
}

/// The snapshot of `node` with its repos' facts and what its processes take.
pub fn snapshot_of(
    node: Node,
    repos: &[RepoFacts],
    resources: Option<crate::resources::Resources>,
    now: i64,
) -> Snapshot {
    Snapshot {
        protocol: PROTOCOL,
        at: now,
        node,
        repos: repos.iter().map(repo_of).collect(),
        resources,
    }
}

fn repo_of(facts: &RepoFacts) -> Repo {
    // An orchestration's read has its worktrees' agent facts; the rest are from the read of everything.
    let mut worktrees: Vec<Worktree> = Vec::new();
    let mut agents: Vec<Agent> = Vec::new();
    for local in &facts.orchestrations {
        for tree in &local.trees {
            worktrees.push(worktree_of(tree));
            agents.extend(agent_of(tree, local, &facts.today));
        }
        agents.extend(local.free.iter().map(|(worker, _)| Agent {
            session: worker.id.clone(),
            name: worker.display_name.clone().or_else(|| worker.name.clone()),
            orchestration: Some(local.name.clone()),
            worktree: None,
            state: "free".into(),
            line: None,
            line_at: None,
            is_asking: false,
            model: None,
            context: None,
            today: today_of(&facts.today, &worker.id),
        }));
    }
    for tree in &facts.everything.trees {
        if !worktrees.iter().any(|known| known.dir == tree.dir) {
            worktrees.push(worktree_of(tree));
        }
    }
    let order = |dir: &Path| {
        facts
            .everything
            .trees
            .iter()
            .position(|tree| tree.dir == dir)
            .unwrap_or(usize::MAX)
    };
    worktrees.sort_by_key(|worktree| order(&worktree.dir));
    Repo {
        name: facts
            .main
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        main: facts.main.clone(),
        develop: facts.develop.clone(),
        orchestrations: facts
            .orchestrations
            .iter()
            .map(|local| orchestration_of(local, facts.acting.get(&local.name)))
            .collect(),
        worktrees,
        agents,
        runs: runs_of(facts),
        today: facts.today.total.into(),
    }
}

fn today_of(today: &crate::spend::Today, session: &str) -> Tokens {
    today.by_session.get(session).copied().unwrap_or_default().into()
}

fn orchestration_of(local: &OrchestrationLocal, acting: Option<&crate::model::acting::Acting>) -> Orchestration {
    let record = local.record.as_ref();
    let entries: Vec<Entry> = record
        .map(|record| issues_of(&record.issues, &local.trees))
        .unwrap_or_default();
    let plan = record.and_then(|record| record.plan.as_ref());
    let branch = local.epic.as_ref().map(|epic| epic.branch.as_str());
    let batch = local.epic.as_ref().and_then(|epic| {
        let (_, number) = crate::batch::parse(&epic.branch)?;
        Some(Batch {
            branch: epic.branch.clone(),
            number,
            behind: epic.behind,
            shipped: epic.shipped.iter().map(|shipped| shipped.branch.clone()).collect(),
            fixes: epic
                .fixes
                .iter()
                .map(|(issue, batch)| Fix {
                    issue: *issue,
                    batch: batch.clone(),
                })
                .collect(),
        })
    });
    Orchestration {
        name: local.name.clone(),
        title: local.title.clone(),
        product: record.and_then(|record| record.product.clone()),
        is_auto: record.is_some_and(|record| record.is_auto),
        issues: entries.iter().map(issue_of).collect(),
        work: plan.map(|plan| plan.work.clone()).unwrap_or_default(),
        merge: plan.map(|plan| plan.merge.clone()).unwrap_or_default(),
        actions: record
            .map(|record| record.actions.iter().map(action_of).collect())
            .unwrap_or_default(),
        epic: branch
            .filter(|branch| !crate::batch::is_batch(branch))
            .map(String::from),
        batch,
        spent: local.spent.total.into(),
        acting: acting.map(acting_of),
    }
}

fn acting_of(acting: &crate::model::acting::Acting) -> Acting {
    Acting {
        pid: acting.pid,
        removals: acting
            .removals
            .iter()
            .map(|(dir, removal)| Removal {
                worktree: dir.clone(),
                is_armed: removal.is_armed,
                is_running: removal.is_running,
                error: removal.error.clone(),
            })
            .collect(),
        starts: acting
            .starts
            .iter()
            .map(|(issue, run)| Start {
                issue: *issue,
                is_running: run.is_running,
                error: run.error.clone(),
                ended: (run.ended > 0).then_some(run.ended),
                worktree: run.dir.clone(),
                is_fresh: run.is_fresh,
                to: run.to.clone(),
            })
            .collect(),
        pipeline_tries: acting
            .pipeline_tries
            .iter()
            .map(|(key, why)| PipelineTry {
                key: key.clone(),
                why: why.clone(),
            })
            .collect(),
        undelivered_waits: acting.undelivered_waits.iter().cloned().collect(),
        merges: acting
            .merges
            .iter()
            .map(|(mr, merging)| Merge {
                mr: *mr,
                phase: match merging.phase {
                    MergePhase::Armed => Phase::Armed,
                    MergePhase::Running => Phase::Running,
                    MergePhase::Merged => Phase::Merged,
                    MergePhase::Failed => Phase::Failed,
                },
                error: merging.error.clone(),
            })
            .collect(),
        log: acting
            .did
            .iter()
            .map(|did| Act {
                at: did.at,
                last: did.last,
                times: did.times,
                key: did.key.clone(),
                text: did.text.clone(),
                detail: did.detail.clone(),
            })
            .collect(),
    }
}

fn issue_of(entry: &Entry) -> Issue {
    Issue {
        number: entry.number,
        numbers: entry.numbers.clone(),
        title: entry.title.clone(),
        feature: entry.feature.clone(),
        branch: entry.branch.clone(),
        worktree: entry.tree.as_ref().map(|tree| tree.dir.clone()),
        is_queued: entry.is_queued,
        after: entry.after.clone(),
        why: entry.why.clone().or_else(|| entry.requeued.clone()),
        merged_at: entry.merged_at.clone(),
        removed_at: entry.removed_at.clone(),
    }
}

fn worktree_of(tree: &Tree) -> Worktree {
    Worktree {
        dir: tree.dir.clone(),
        name: tree.name.clone(),
        branch: tree.branch.clone(),
        base: tree.base.local.clone(),
        is_main: tree.is_main,
        orchestration: tree.origin.as_ref().map(|origin| origin.name.clone()),
        issues: tree.issues.clone(),
        agent: tree.claude.as_ref().map(|claude| claude.id.clone()),
        ahead: tree.ahead,
        behind: tree.behind,
        changed: tree.changed,
        unpushed: tree.unpushed,
        pushed: tree.pushed.clone(),
        check: check_of(&tree.checked),
        stack: tree.stack.clone(),
        actions: tree.actions.iter().map(action_of).collect(),
    }
}

fn check_of(checked: &CheckState) -> Check {
    match checked {
        CheckState::Checking { since } => Check::Checking { since: since.clone() },
        CheckState::Tested { sha, packages, no_test } => Check::Tested {
            sha: sha.clone(),
            packages: packages.clone(),
            no_test: no_test.clone(),
        },
        CheckState::Failed { sha, failures, refused } => Check::Failed {
            sha: sha.clone(),
            failures: failures
                .iter()
                .map(|failure| CheckFailure {
                    package: failure.package.clone(),
                    task: failure.task.clone(),
                    tests: failure.tests.clone(),
                })
                .collect(),
            refused: refused.clone(),
        },
        CheckState::Untested { count } => Check::Untested { commits: *count },
        CheckState::None => Check::None,
    }
}

fn action_of(action: &crate::actions::Action) -> Action {
    Action {
        id: action.id,
        kind: action.kind.clone(),
        text: action.text.clone(),
        url: action.url.clone(),
        run: action.run.clone(),
        on: action.on.clone(),
        options: action
            .options
            .iter()
            .enumerate()
            .map(|(at, label)| Choice {
                label: label.clone(),
                description: action.descriptions.get(at).filter(|text| !text.is_empty()).cloned(),
            })
            .collect(),
        recommend: action.recommend.clone(),
        mr: action.mr,
        rule: action.rule.clone(),
        at: action.at.clone(),
        answer: action.answer.clone(),
        answered_at: action.answered_at.clone(),
    }
}

/// The agent of a worktree, None while no conversation is bound to it.
fn agent_of(tree: &Tree, local: &OrchestrationLocal, today: &crate::spend::Today) -> Option<Agent> {
    let claude = tree.claude.as_ref()?;
    let worker = local
        .record
        .as_ref()
        .and_then(|record| record.workers.iter().find(|worker| worker.id == claude.id));
    let status = local.agents.get(&claude.id).and_then(|agent| agent.status.as_deref());
    let state = agent_state_of(
        tree,
        &Facts {
            status,
            ..Facts::default()
        },
    );
    let question = claude.answer.as_deref().and_then(question_of);
    let now = crate::said::now_of(tree.note.as_ref(), claude.answer.as_deref(), claude.answer_at);
    let (line, line_at) = match (&question, now) {
        (Some(question), _) => (Some(question.clone()), claude.answer_at),
        (None, Some(now)) => (Some(now.text), now.at),
        (None, None) => (None, None),
    };
    Some(Agent {
        session: claude.id.clone(),
        name: worker.and_then(|worker| worker.display_name.clone().or_else(|| worker.name.clone())),
        orchestration: Some(local.name.clone()),
        worktree: Some(tree.dir.clone()),
        state: state.key.to_string(),
        line,
        line_at,
        is_asking: question.is_some(),
        model: claude.model.clone(),
        context: (claude.window > 0).then(|| claude.share()),
        today: today_of(today, &claude.id),
    })
}

fn runs_of(facts: &RepoFacts) -> Runs {
    let mut waiting: HashMap<&str, usize> = HashMap::new();
    for event in &facts.queue.events {
        *waiting.entry(event.session.as_str()).or_default() += 1;
    }
    let mut runs: Vec<Run> = crate::runs::states_of(&facts.table, &facts.queue)
        .into_iter()
        .map(|(session, run)| Run {
            waiting: waiting.get(session.as_str()).copied().unwrap_or(0),
            session,
            dir: run.dir,
            name: run.name,
            state: run.state,
            since: run.since,
            lines: run.lines,
            denied: run.denials.into_iter().map(|denial| denial.tool).collect(),
            code: run.code,
        })
        .collect();
    runs.sort_by(|a, b| a.session.cmp(&b.session));
    Runs {
        runs,
        paused_since: facts.paused.as_ref().map(|paused| paused.since),
        held: facts.held.clone(),
        usage: facts.usage.clone(),
    }
}

/// The machine's name, empty when the OS doesn't say.
pub fn host() -> String {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is writable for its length; gethostname writes a NUL-ended name into it.
    let status = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if status != 0 {
        return String::new();
    }
    let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::Claude;
    use crate::record::{IssueEntry, Origin, Plan, Record, Worker};
    use serde_json::json;

    fn facts() -> RepoFacts {
        let dir = PathBuf::from("/r/platform-7-x");
        let tree = Tree {
            dir: dir.clone(),
            name: "7-x".into(),
            branch: "feature/7-x".into(),
            origin: Some(Origin {
                name: "lab".into(),
                slot: 1,
            }),
            issues: vec![7],
            base: crate::git::Base::of("batch/lab-2"),
            ahead: 3,
            behind: 1,
            changed: 2,
            unpushed: 1,
            pushed: Some("ab12".into()),
            checked: CheckState::Failed {
                sha: "cd34".into(),
                failures: vec![crate::check::Failure {
                    package: "breadboard".into(),
                    task: "test:rs".into(),
                    tests: vec!["node::tests::x".into()],
                }],
                refused: None,
            },
            claude: Some(Claude {
                id: "s-1".into(),
                prompt: None,
                prompt_at: None,
                answer: Some("Which name stays?".into()),
                answer_at: Some(900),
                transcript: PathBuf::new(),
                cwd: dir.clone(),
                model: Some("claude-opus-5-5".into()),
                at: Some(900),
                used: 50_000,
                window: 200_000,
            }),
            is_bound: true,
            actions: vec![crate::actions::Action {
                id: 4,
                kind: "decide".into(),
                text: "Which name stays?".into(),
                options: vec!["Old".into(), "New".into()],
                descriptions: vec!["keeps it".into(), String::new()],
                recommend: Some("New".into()),
                at: "2026-10-29T10:00:00.000Z".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let main = Tree {
            dir: PathBuf::from("/r/platform"),
            name: "platform".into(),
            branch: "develop".into(),
            is_main: true,
            base: crate::git::Base::of("develop"),
            ..Default::default()
        };
        let record = Record {
            title: Some("Lab".into()),
            product: Some("lab".into()),
            issues: vec![
                IssueEntry {
                    number: Some(7),
                    numbers: vec![7],
                    title: Some("x".into()),
                    feature: Some("7-x".into()),
                    branch: Some("feature/7-x".into()),
                    ..Default::default()
                },
                IssueEntry {
                    number: Some(8),
                    numbers: vec![8],
                    is_queued: true,
                    after: vec![12],
                    ..Default::default()
                },
            ],
            plan: Some(Plan {
                work: vec![7, 8],
                merge: vec![7],
                ..Default::default()
            }),
            workers: vec![Worker {
                id: "s-1".into(),
                display_name: Some("Mira Albrecht".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let orchestration = OrchestrationLocal {
            name: "lab".into(),
            title: "Lab".into(),
            trees: vec![tree.clone()],
            record: Some(record),
            free: vec![(
                Worker {
                    id: "s-2".into(),
                    name: Some("Kai".into()),
                    ..Default::default()
                },
                crate::fresh::Context::default(),
            )],
            agents: [(
                "s-1".to_string(),
                crate::herdr::Agent {
                    pane: String::new(),
                    tab: String::new(),
                    status: Some("blocked".into()),
                },
            )]
            .into(),
            epic: Some(crate::model::epic_row::EpicLocal {
                branch: "batch/lab-2".into(),
                behind: Some(4),
                fixes: vec![(30, "batch/lab-1".into())],
                ..Default::default()
            }),
            spent: crate::spend::Spent {
                total: Spend {
                    tokens: 2_000,
                    dollars: 1.5,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let run = crate::runs::Run {
            dir: dir.clone(),
            name: "7-x".into(),
            state: crate::runs::State::Asks,
            since: 800,
            pid: None,
            lines: 2,
            denials: vec![crate::runs::Denial {
                tool: "Bash".into(),
                input: json!({}),
            }],
            code: Some(0),
        };
        RepoFacts {
            main: PathBuf::from("/r/platform"),
            develop: "develop".into(),
            everything: EverythingLocal {
                trees: vec![main, tree],
                ..Default::default()
            },
            orchestrations: vec![orchestration],
            table: crate::runs::Table {
                runs: [("s-1".to_string(), run)].into(),
            },
            queue: crate::runs::Queue {
                events: vec![crate::runs::Event {
                    session: "s-1".into(),
                    dir,
                    name: "7-x".into(),
                    line: "[overview] #4 decide → New".into(),
                    at: 950,
                    allowed: Vec::new(),
                    is_notice: false,
                }],
            },
            paused: Some(crate::runs::Paused { since: 700, ci: false }),
            held: None,
            usage: None,
            today: crate::spend::Today {
                day: "2026-10-29".into(),
                total: Spend {
                    tokens: 5_000,
                    dollars: 2.0,
                },
                by_session: [(
                    "s-1".to_string(),
                    Spend {
                        tokens: 1_000,
                        dollars: 0.5,
                    },
                )]
                .into(),
                ..Default::default()
            },
            acting: [("lab".to_string(), acting())].into(),
        }
    }

    fn acting() -> crate::model::acting::Acting {
        use crate::model::acting::{Acting, Did, MergePhase, Merging};
        use crate::model::auto_actions::Removal;
        use crate::model::workers::StartRun;
        Acting {
            pid: 42,
            did: vec![Did {
                at: 960,
                paint: crate::model::Paint::Green,
                text: "removed 6-y".into(),
                detail: Some("pnpm wt:remove 6-y".into()),
                key: Some("remove:/r/platform-6-y".into()),
                times: 2,
                last: 990,
            }],
            removals: [(
                PathBuf::from("/r/platform-6-y"),
                Removal {
                    is_armed: false,
                    is_running: true,
                    error: None,
                },
            )]
            .into(),
            starts: [(
                9,
                StartRun {
                    is_running: false,
                    error: Some("no free agent".into()),
                    ended: 970,
                    dir: Some("/r/platform-7-x".into()),
                    is_fresh: false,
                    to: Some("s-1".into()),
                },
            )]
            .into(),
            pipeline_tries: [("failed:12:300".to_string(), "the agent runs".to_string())].into(),
            undelivered_waits: [PathBuf::from("/r/platform-7-x")].into(),
            merges: [(
                12,
                Merging {
                    phase: MergePhase::Running,
                    error: None,
                },
            )]
            .into(),
        }
    }

    fn resources() -> crate::resources::Resources {
        use crate::resources::{Machine, Process, Resources, RunShare, Share, Sum, Via};
        let sum = Sum {
            processes: 2,
            cpu: 150.5,
            rss: 3_000,
        };
        Resources {
            machine: Machine {
                cores: 16,
                cpu: 40.5,
                memory: 32_000,
                used: 29_000,
                available: 3_000,
                swap: 8_000,
                swap_used: 1_000,
                load: [12.5, 8.0, 4.25],
            },
            top: vec![Process {
                pid: 812,
                ppid: 800,
                command: "node vitest".into(),
                cpu: 98.5,
                rss: 2_000,
                worktree: Some("/r/platform-7-x".into()),
                run: Some("s-1".into()),
                stack: None,
                via: Via::Cwd,
            }],
            worktrees: vec![Share {
                worktree: "/r/platform-7-x".into(),
                sum,
            }],
            runs: vec![RunShare {
                session: "s-1".into(),
                name: "7-x".into(),
                dir: "/r/platform-7-x".into(),
                sum,
            }],
            other: Sum {
                processes: 300,
                cpu: 2.0,
                rss: 9_000,
            },
        }
    }

    fn resources_json() -> serde_json::Value {
        json!({
            "machine": {
                "cores": 16, "cpu": 40.5, "memory": 32_000, "used": 29_000, "available": 3_000,
                "swap": 8_000, "swapUsed": 1_000, "load": [12.5, 8.0, 4.25],
            },
            "top": [{
                "pid": 812, "ppid": 800, "command": "node vitest", "cpu": 98.5, "rss": 2_000,
                "worktree": "/r/platform-7-x", "run": "s-1", "stack": null, "via": "cwd",
            }],
            "worktrees": [{ "worktree": "/r/platform-7-x", "processes": 2, "cpu": 150.5, "rss": 3_000 }],
            "runs": [{ "session": "s-1", "name": "7-x", "dir": "/r/platform-7-x", "processes": 2, "cpu": 150.5, "rss": 3_000 }],
            "other": { "processes": 300, "cpu": 2.0, "rss": 9_000 },
        })
    }

    #[test]
    fn pins_the_shape_of_a_resources_event() {
        let event = Event::Resources {
            at: 2_000,
            resources: resources(),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value,
            json!({ "kind": "resources", "at": 2_000, "resources": resources_json() })
        );
        assert_eq!(serde_json::from_value::<Event>(value).unwrap(), event);
    }

    /// The shape lab's server reads: a change here is a change of `PROTOCOL`.
    #[test]
    fn pins_the_shape_of_a_snapshot() {
        let node = Node {
            host: "machine".into(),
            user: "mira".into(),
            version: "0.1.0".into(),
        };
        let snapshot = snapshot_of(node, &[facts()], Some(resources()), 1_000);
        let value = serde_json::to_value(&snapshot).unwrap();
        let worktree = json!({
            "dir": "/r/platform-7-x", "name": "7-x", "branch": "feature/7-x", "base": "batch/lab-2",
            "isMain": false, "orchestration": "lab", "issues": [7], "agent": "s-1",
            "ahead": 3, "behind": 1, "changed": 2, "unpushed": 1, "pushed": "ab12",
            "check": {
                "state": "failed", "sha": "cd34", "refused": null,
                "failures": [{ "package": "breadboard", "task": "test:rs", "tests": ["node::tests::x"] }],
            },
            "stack": null,
            "actions": [{
                "id": 4, "kind": "decide", "text": "Which name stays?", "url": null, "run": null, "on": null,
                "options": [{ "label": "Old", "description": "keeps it" }, { "label": "New", "description": null }],
                "recommend": "New", "mr": null, "rule": null, "at": "2026-10-29T10:00:00.000Z",
                "answer": null, "answeredAt": null,
            }],
        });
        let expected = json!({
            "protocol": 2,
            "at": 1_000,
            "node": { "host": "machine", "user": "mira", "version": "0.1.0" },
            "repos": [{
                "main": "/r/platform",
                "name": "platform",
                "develop": "develop",
                "orchestrations": [{
                    "name": "lab", "title": "Lab", "product": "lab", "isAuto": false,
                    "issues": [
                        {
                            "number": 7, "numbers": [7], "title": "x", "feature": "7-x", "branch": "feature/7-x",
                            "worktree": "/r/platform-7-x", "isQueued": false, "after": [], "why": null,
                            "mergedAt": null, "removedAt": null,
                        },
                        {
                            "number": 8, "numbers": [8], "title": null, "feature": "", "branch": "",
                            "worktree": null, "isQueued": true, "after": [12], "why": null,
                            "mergedAt": null, "removedAt": null,
                        },
                    ],
                    "work": [7, 8], "merge": [7], "actions": [], "epic": null,
                    "batch": {
                        "branch": "batch/lab-2", "number": 2, "behind": 4, "shipped": [],
                        "fixes": [{ "issue": 30, "batch": "batch/lab-1" }],
                    },
                    "spent": { "tokens": 2_000, "dollars": 1.5 },
                    "acting": {
                        "pid": 42,
                        "removals": [{ "worktree": "/r/platform-6-y", "isArmed": false, "isRunning": true, "error": null }],
                        "starts": [{
                            "issue": 9, "isRunning": false, "error": "no free agent", "ended": 970,
                            "worktree": "/r/platform-7-x", "isFresh": false, "to": "s-1",
                        }],
                        "pipelineTries": [{ "key": "failed:12:300", "why": "the agent runs" }],
                        "undeliveredWaits": ["/r/platform-7-x"],
                        "merges": [{ "mr": 12, "phase": "running", "error": null }],
                        "log": [{
                            "at": 960, "last": 990, "times": 2, "key": "remove:/r/platform-6-y",
                            "text": "removed 6-y", "detail": "pnpm wt:remove 6-y",
                        }],
                    },
                }],
                "worktrees": [
                    {
                        "dir": "/r/platform", "name": "platform", "branch": "develop", "base": "develop",
                        "isMain": true, "orchestration": null, "issues": [], "agent": null,
                        "ahead": 0, "behind": 0, "changed": 0, "unpushed": 0, "pushed": null,
                        "check": { "state": "none" }, "stack": null, "actions": [],
                    },
                    worktree,
                ],
                "agents": [
                    {
                        "session": "s-1", "name": "Mira Albrecht", "orchestration": "lab",
                        "worktree": "/r/platform-7-x", "state": "asks", "line": "Which name stays?",
                        "lineAt": 900, "isAsking": true, "model": "claude-opus-5-5", "context": 0.25,
                        "today": { "tokens": 1_000, "dollars": 0.5 },
                    },
                    {
                        "session": "s-2", "name": "Kai", "orchestration": "lab", "worktree": null,
                        "state": "free", "line": null, "lineAt": null, "isAsking": false, "model": null,
                        "context": null, "today": { "tokens": 0, "dollars": 0.0 },
                    },
                ],
                "runs": {
                    "runs": [{
                        "session": "s-1", "dir": "/r/platform-7-x", "name": "7-x", "state": "asks",
                        "since": 800, "lines": 2, "waiting": 1, "denied": ["Bash"], "code": 0,
                    }],
                    "pausedSince": 700, "held": null, "usage": null,
                },
                "today": { "tokens": 5_000, "dollars": 2.0 },
            }],
            "resources": resources_json(),
        });
        assert_eq!(value, expected);
        // What a server reads back is what the node sent.
        assert_eq!(serde_json::from_value::<Snapshot>(value).unwrap(), snapshot);
    }

    #[test]
    fn says_every_check_state_by_its_name() {
        let names: Vec<serde_json::Value> = [
            CheckState::Checking { since: None },
            CheckState::Tested {
                sha: "a".into(),
                packages: vec![],
                no_test: Some("docs".into()),
            },
            CheckState::Untested { count: 2 },
        ]
        .iter()
        .map(|state| serde_json::to_value(check_of(state)).unwrap())
        .collect();
        assert_eq!(
            names,
            [
                json!({ "state": "checking", "since": null }),
                json!({ "state": "tested", "sha": "a", "packages": [], "noTest": "docs" }),
                json!({ "state": "untested", "commits": 2 }),
            ]
        );
    }
}
