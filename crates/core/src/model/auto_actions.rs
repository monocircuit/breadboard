//! The automatic actions of an orchestration (auto-actions.mjs): what the
//! overview finds waiting without anyone posting it: a merge that is ready, a
//! failed pipeline, commits sitting unpushed, the clean-up after a merge, an
//! agent that stopped on a question. And the lines it sends by itself about
//! conflicts and failed pipelines, as data: the writing side sends them.

use super::change::{is_change_open, unpushed_action_of};
use super::half_merged::half_merged_action_of;
use super::merge::checks_of;
use super::order::{Go, Item, Target};
use super::resolve::is_conflict_settled;
use super::{Entry, Tree};
use crate::conversations::{first_line, question_of};
use crate::gitlab::{DevelopPipeline, Job, Mr, Remote};
use crate::herdr::Agent;
use crate::time::to_iso;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The automatic wt:remove of a worktree, as the writing side keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Removal {
    pub is_armed: bool,
    pub is_running: bool,
    pub error: Option<String>,
}

/// Where a worktree remembers the automatic lines that got in to its agent:
/// `{ sent: [key, …] }` in its ignored `.claude/`.
pub fn sent_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("overview-sent.json")
}

/// The keys a worktree remembers as sent.
pub fn sent_of(dir: &Path) -> HashSet<String> {
    crate::json::read(&sent_file(dir))
        .map(|value| crate::json::strs_of(&value, "sent").into_iter().collect())
        .unwrap_or_default()
}

/// The key a failed pipeline of `mr` is sent under: once per pipeline.
pub fn failed_key_of(mr: &Mr) -> String {
    let pipeline = mr.pipeline.as_ref();
    let id = pipeline
        .filter(|pipeline| pipeline.id != 0)
        .map(|pipeline| pipeline.id.to_string())
        .or_else(|| pipeline.and_then(|pipeline| pipeline.finished_at.clone().or(pipeline.updated_at.clone())))
        .unwrap_or_else(|| "undefined".into());
    format!("pipeline:!{}:{id}", mr.iid)
}

/// How long a log line may be in the line to the agent.
const LOG_LINE_CHARS: usize = 200;

/// The line that tells an agent its MR's pipeline failed: the failed jobs with
/// their urls and the end of the first one's log. A job develop's last
/// pipeline fails too is not the MR's doing: the line says so.
pub fn pipeline_line_of(mr: &Mr, jobs: &[Job], develop: Option<&DevelopPipeline>) -> String {
    let failed: Vec<&Job> = jobs.iter().filter(|job| job.status == "failed").collect();
    let on_develop: HashSet<&str> = develop
        .map(|develop| {
            develop
                .jobs
                .iter()
                .filter(|job| job.status == "failed")
                .map(|job| job.name.as_str())
                .collect()
        })
        .unwrap_or_default();
    let own: Vec<&&Job> = failed
        .iter()
        .filter(|job| !on_develop.contains(job.name.as_str()))
        .collect();
    let theirs: Vec<&&Job> = failed
        .iter()
        .filter(|job| on_develop.contains(job.name.as_str()))
        .collect();
    let what = if failed.is_empty() {
        mr.pipeline
            .as_ref()
            .and_then(|pipeline| pipeline.web_url.clone())
            .unwrap_or_else(|| mr.url.clone())
    } else {
        failed
            .iter()
            .map(|job| format!("job {}, {}", job.name, job.url.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let names = theirs
        .iter()
        .map(|job| job.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let verdict = if !failed.is_empty() && own.is_empty() {
        crate::config::in_repo(&format!(
            "develop fails the same job{}: not yours, waits for the fix (say so in the MR)",
            if theirs.len() == 1 { "" } else { "s" }
        ))
    } else if theirs.is_empty() {
        "look into it".into()
    } else {
        crate::config::in_repo(&format!("look into it (develop fails {names} too: not yours)"))
    };
    let log = own
        .first()
        .map(|job| (**job).clone())
        .or_else(|| failed.first().map(|job| (*job).clone()))
        .and_then(|job| job.log)
        .unwrap_or_default();
    let end = if !own.is_empty() && !log.is_empty() {
        let lines: Vec<String> = log
            .iter()
            .map(|line| line.chars().take(LOG_LINE_CHARS).collect())
            .collect();
        format!("; its log ends: {}", lines.join(" ⏎ "))
    } else {
        String::new()
    };
    format!("[overview] the pipeline of !{} failed: {what}: {verdict}{end}", mr.iid)
}

/// A line the overview sends by itself, or a key to forget once its cause went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tell {
    Send {
        dir: PathBuf,
        key: String,
        line: String,
        iid: i64,
        id: Option<String>,
    },
    Forget {
        dir: PathBuf,
        key: String,
    },
}

/// What to do about conflicts with develop: of each open MR with a worktree,
/// the line to send once GitLab settled conflicts its agent wasn't told of,
/// the key to forget once they are gone. Nothing while GitLab checks or about
/// commits not pushed (#290).
pub fn conflicts_of(issues: &[Entry], remote: &Remote) -> Vec<Tell> {
    issues
        .iter()
        .filter_map(|issue| {
            let tree = issue.tree.as_ref()?;
            let mr = remote.of(&issue.branch).mr.as_ref().filter(|mr| mr.is_open())?;
            let key = format!("conflicts:{}", mr.iid);
            let is_sent = tree.sent.contains(&key);
            if !is_conflict_settled(mr, Some(tree)) {
                return None;
            }
            if mr.has_conflicts && !is_sent && tree.unpushed == 0 {
                return Some(Tell::Send {
                    dir: tree.dir.clone(),
                    line: format!(
                        "[overview] !{} has conflicts with {target}: merge {target} in with {run} wt:merge-base --resolve, resolve the conflicts, git add, git commit --no-edit, {run} wt:check and push in this run",
                        mr.iid,
                        target = mr.target(),
                        run = crate::config::run_word()
                    ),
                    key,
                    iid: mr.iid,
                    id: None,
                });
            }
            (!mr.has_conflicts && is_sent).then(|| Tell::Forget { dir: tree.dir.clone(), key })
        })
        .collect()
}

/// The failed pipelines of open MRs whose agent wasn't told yet, once per pipeline.
pub fn failed_pipelines_of(issues: &[Entry], remote: &Remote, develop: Option<&DevelopPipeline>) -> Vec<Tell> {
    issues
        .iter()
        .filter_map(|issue| {
            let tree = issue.tree.as_ref()?;
            let claude = tree.claude.as_ref()?;
            let part = remote.of(&issue.branch);
            let mr = part
                .mr
                .as_ref()
                .filter(|mr| mr.is_open() && mr.pipeline_status() == Some("failed"))?;
            let key = failed_key_of(mr);
            (!tree.sent.contains(&key)).then(|| Tell::Send {
                dir: tree.dir.clone(),
                line: pipeline_line_of(mr, &part.jobs, develop),
                key,
                iid: mr.iid,
                id: Some(claude.id.clone()),
            })
        })
        .collect()
}

/// What autoActionsOf knows besides the issues.
#[derive(Debug, Clone, Default)]
pub struct AutoFacts<'a> {
    /// Drafts nothing but the draft holds up, next in the merge order (#242).
    pub ready_drafts: Option<&'a HashSet<i64>>,
    pub develop: Option<&'a DevelopPipeline>,
    /// The automatic wt:removes, by worktree.
    pub removals: Option<&'a HashMap<PathBuf, Removal>>,
    /// Why the line of a failed pipeline didn't get in yet, by its key.
    pub pipeline_tries: Option<&'a HashMap<String, String>>,
    pub now: i64,
}

/// The automatic actions of the issues, `key` the same for as long as the cause lasts:
/// - `merge`: ready and green, every check done, GitLab says mergeable; a ready draft too;
/// - `pipeline`: the pipeline of the open MR failed and its line didn't get in yet;
/// - `push`: commits sit unpushed for long;
/// - `half-merged`: a merge open for long, no run of its agent in flight (#396);
/// - `clean up`: merged: a removal that failed, the issue's open boxes for its agent;
/// - `waits`: the agent asks something or stopped on a question;
/// - `held`: a claude open in its tab holds its lines past ten minutes (#386).
pub fn auto_actions_of(
    issues: &[Entry],
    remote: Option<&Remote>,
    agents: &HashMap<String, Agent>,
    facts: &AutoFacts,
) -> Vec<Item> {
    let none = Remote::default();
    let remote = remote.unwrap_or(&none);
    let mut actions = Vec::new();
    for issue in issues {
        let part = remote.of(&issue.branch);
        let mr = part.mr.as_ref();
        let tree = issue.tree.as_ref();
        let agent = tree
            .and_then(|tree| tree.claude.as_ref())
            .and_then(|claude| agents.get(&claude.id));
        let empty = Tree::default();
        let sent = &tree.unwrap_or(&empty).sent;
        let base = |kind: &str, key: String, text: String, at: Option<&str>, go: Go| Item {
            key,
            kind: kind.into(),
            text,
            at: at.map(String::from).unwrap_or_else(|| to_iso(facts.now)),
            who: issue.feature.clone(),
            is_auto: true,
            number: issue.number,
            branch: Some(issue.branch.clone()),
            to: tree.map(|tree| Target::Dir(tree.dir.clone())),
            go,
            ..Default::default()
        };
        let status = mr.and_then(Mr::pipeline_status);
        let pipeline_at = mr
            .and_then(|mr| mr.pipeline.as_ref())
            .and_then(|pipeline| pipeline.finished_at.clone().or(pipeline.updated_at.clone()));
        let pipeline_at = pipeline_at.as_deref().and_then(crate::time::parse_iso).map(to_iso);
        if let Some(mr) = mr.filter(|mr| mr.is_open()) {
            let is_ready_draft = mr.is_draft
                && issue
                    .number
                    .is_some_and(|number| facts.ready_drafts.is_some_and(|ready| ready.contains(&number)));
            let boxes = part.issue.as_ref().map(|found| &found.boxes[..]).unwrap_or(&[]);
            let (open, answered) = tree
                .map(|tree| (&tree.actions[..], &tree.answered[..]))
                .unwrap_or((&[], &[]));
            if (!mr.is_draft || is_ready_draft)
                && status == Some("success")
                // GitLab's word, not ours: conflicts, a check still running, … block.
                && (mr.merge_status.as_deref() == Some("mergeable") || is_ready_draft)
                && !open.iter().any(|action| action.kind == "merge")
                && !is_change_open(tree, Some(mr))
                && checks_of(boxes, open, answered).is_empty()
            {
                let text = if is_ready_draft {
                    "pipeline passed, every box and try done, still a draft"
                } else {
                    "ready, pipeline passed"
                };
                let mut item = base(
                    "merge",
                    format!("merge:{}", issue.branch),
                    text.into(),
                    pipeline_at.as_deref(),
                    Go::Merge {
                        branch: issue.branch.clone(),
                        action: None,
                    },
                );
                item.merge = Some(super::order::MergeOffer {
                    iid: mr.iid,
                    url: mr.url.clone(),
                    is_draft: mr.is_draft,
                    can_mark_ready: is_ready_draft,
                    ..Default::default()
                });
                actions.push(item);
            }
            let failed = failed_key_of(mr);
            if status == Some("failed") && !sent.contains(&failed) {
                let job = part.jobs.iter().find(|job| job.status == "failed");
                let what = job
                    .map(|job| format!("job {} failed", job.name))
                    .unwrap_or_else(|| "failed".into());
                // A worktree whose conversation isn't written down may still have its
                // agent in herdr (WORKTREES names it): a click tries it, the job opens if
                // none takes the line. Without a worktree there is no one.
                let why = match tree {
                    None => "no agent to tell".to_string(),
                    Some(_) => facts
                        .pipeline_tries
                        .and_then(|tries| tries.get(&failed).cloned())
                        .unwrap_or_else(|| "click to tell its agent".into()),
                };
                let show = Go::Show {
                    url: job.and_then(|job| job.url.clone()).unwrap_or_else(|| mr.url.clone()),
                    dir: tree.map(|tree| tree.dir.clone()),
                };
                let go = match tree {
                    Some(tree) => Go::Send {
                        dir: tree.dir.clone(),
                        key: failed.clone(),
                        line: pipeline_line_of(mr, &part.jobs, facts.develop),
                        fallback: Box::new(show),
                    },
                    None => show,
                };
                actions.push(base(
                    "pipeline",
                    format!("pipeline:{}", issue.branch),
                    format!("!{} {what}: {why}", mr.iid),
                    pipeline_at.as_deref(),
                    go,
                ));
            }
        }
        if let Some(tree) = tree
            && let Some(unpushed) = unpushed_action_of(&issue.branch, tree, mr, facts.now)
        {
            let focus = Go::FocusAgent(tree.dir.clone());
            // Its row says a click tells its agent: it tries herdr's agent of its tab,
            // known conversation or not, and brings the tab to the front when none takes it.
            let go = if unpushed.is_sent {
                focus
            } else {
                Go::Send {
                    dir: tree.dir.clone(),
                    key: unpushed.key.clone(),
                    line: unpushed.line.clone(),
                    fallback: Box::new(focus),
                }
            };
            actions.push(base(
                "push",
                format!("push:{}", issue.branch),
                unpushed.text,
                Some(&unpushed.at),
                go,
            ));
        }
        let is_running = agent.is_some_and(|agent| agent.status.as_deref() == Some("working"));
        if let Some(tree) = tree
            && let Some(half) = half_merged_action_of(&issue.branch, tree, is_running, facts.now)
        {
            let focus = Go::FocusAgent(tree.dir.clone());
            let go = if half.is_sent {
                focus
            } else {
                Go::Send {
                    dir: tree.dir.clone(),
                    key: half.key.clone(),
                    line: half.line.clone(),
                    fallback: Box::new(focus),
                }
            };
            actions.push(base(
                "half-merged",
                format!("half-merged:{}", issue.branch),
                half.text,
                Some(&half.at),
                go,
            ));
        }
        if let Some(mr) = mr.filter(|mr| mr.is_merged()) {
            let merged_at = mr.merged_at.as_deref().and_then(crate::time::parse_iso).map(to_iso);
            // The automatic one does the rest: a row only for one that failed.
            if let Some(tree) = tree
                && let Some(error) = facts
                    .removals
                    .and_then(|removals| removals.get(&tree.dir))
                    .and_then(|removal| removal.error.as_ref())
            {
                actions.push(base(
                    "clean up",
                    format!("clean up:{}:remove", issue.branch),
                    format!("remove the worktree {}  {error}", issue.feature),
                    merged_at.as_deref(),
                    Go::Remove(tree.dir.clone()),
                ));
            }
            let open = part
                .issue
                .as_ref()
                .map(|found| found.boxes.iter().filter(|found| !found.is_ticked).count())
                .unwrap_or(0);
            let boxes = format!("boxes:!{}", mr.iid);
            if let (Some(tree), Some(found)) = (tree, part.issue.as_ref())
                && open > 0
                && agent.is_some()
                && !sent.contains(&boxes)
            {
                actions.push(base(
                    "clean up",
                    format!("clean up:{}:boxes", issue.branch),
                    format!("#{} has {open} open boxes: click to have its agent tick the done ones", found.iid),
                    merged_at.as_deref(),
                    Go::Send {
                        dir: tree.dir.clone(),
                        key: boxes,
                        line: format!(
                            "MR !{} is merged: tick the boxes of #{} that are done ({run} wt:tick), say what stays open.",
                            mr.iid, found.iid, run = crate::config::run_word()),
                        fallback: Box::new(Go::FocusAgent(tree.dir.clone())),
                    },
                ));
            }
        }
        // A claude open in its tab holds its lines past ten minutes (#386), one
        // herdr may have resumed by itself after a restart: a click ends it once it idles.
        if let (Some(agent), Some(tree)) = (agent, tree)
            && agent.status.as_deref() == Some(crate::runs::HELD)
        {
            actions.push(base(
                "held",
                format!("held:{}", issue.branch),
                "an open claude in its tab holds its lines: a click ends it once it is idle, they run headless again"
                    .into(),
                None,
                Go::EndHeld(tree.dir.clone()),
            ));
        }
        // A headless agent at a would-be question has its `permission` action
        // (#360): that is what it waits for, no `waits` beside it.
        let is_asking = tree.is_some_and(|tree| tree.actions.iter().any(|action| action.kind == "permission"));
        if let (Some(agent), Some(tree)) = (agent, tree)
            && let Some(claude) = &tree.claude
            && !is_asking
        {
            let answer = claude.answer.as_deref().unwrap_or("");
            let question = match agent.status.as_deref() {
                Some("blocked") => Some(tree.pending.clone().unwrap_or_else(|| {
                    let line = first_line(answer);
                    if line.is_empty() { "needs you".into() } else { line }
                })),
                Some("working") => None,
                _ => question_of(answer),
            };
            if let Some(question) = question {
                let at = claude.at.map(to_iso);
                actions.push(base(
                    "waits",
                    format!("waits:{}", issue.branch),
                    question,
                    at.as_deref(),
                    Go::FocusAgent(tree.dir.clone()),
                ));
            }
        }
    }
    actions
}

/// The actions in `actions` that weren't there at the last read; the first
/// read only takes note: what the overview finds at its start is not new.
pub fn new_actions_of<'a>(before: Option<&HashSet<String>>, actions: &'a [Item]) -> Vec<&'a Item> {
    match before {
        Some(before) => actions.iter().filter(|action| !before.contains(&action.key)).collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Action, Change};
    use crate::gitlab::{BranchRemote, Issue, IssueBox, Pipeline};
    use crate::model::handovers::tests::claude_of;
    use crate::time::parse_iso;

    const NOW: i64 = 1_790_000_000_000;

    fn issue() -> Entry {
        let mut claude = claude_of("c7");
        claude.answer = Some("Working on it.".into());
        Entry {
            number: Some(7),
            feature: "7-thing".into(),
            branch: "feature/7-thing".into(),
            tree: Some(Tree {
                dir: "/w/7".into(),
                claude: Some(claude),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn mr() -> Mr {
        Mr {
            iid: 70,
            state: "opened".into(),
            url: "u".into(),
            pipeline: Some(Pipeline {
                status: "success".into(),
                web_url: Some("p".into()),
                ..Default::default()
            }),
            merge_status: Some("mergeable".into()),
            ..Default::default()
        }
    }
    fn remote(mr: Option<Mr>, jobs: Vec<Job>, found: Option<Issue>) -> Remote {
        Remote {
            of: [("feature/7-thing".to_string(), BranchRemote { mr, jobs, issue: found })].into(),
            ..Default::default()
        }
    }
    fn agents(status: &str) -> HashMap<String, Agent> {
        [(
            "c7".to_string(),
            Agent {
                pane: "p1".into(),
                tab: "t".into(),
                status: Some(status.into()),
            },
        )]
        .into()
    }
    fn kinds_with(
        issues: &[Entry],
        remote: &Remote,
        agents: &HashMap<String, Agent>,
        facts: &AutoFacts,
    ) -> Vec<(String, String)> {
        auto_actions_of(issues, Some(remote), agents, facts)
            .into_iter()
            .map(|item| (item.kind, item.text))
            .collect()
    }
    fn kinds(issues: &[Entry], remote: &Remote) -> Vec<(String, String)> {
        kinds_with(
            issues,
            remote,
            &HashMap::new(),
            &AutoFacts {
                now: NOW,
                ..Default::default()
            },
        )
    }
    fn pair(kind: &str, text: &str) -> (String, String) {
        (kind.into(), text.into())
    }

    #[test]
    fn asks_for_the_merge_of_a_ready_green_mergeable_mr() {
        assert_eq!(
            kinds(&[issue()], &remote(Some(mr()), vec![], None)),
            [pair("merge", "ready, pipeline passed")]
        );
        for status in ["conflict", "checking", "ci_must_pass"] {
            let found = Mr {
                merge_status: Some(status.into()),
                ..mr()
            };
            assert!(kinds(&[issue()], &remote(Some(found), vec![], None)).is_empty());
        }
        assert!(kinds(&[issue()], &remote(Some(Mr { is_draft: true, ..mr() }), vec![], None)).is_empty());
        let running = Mr {
            pipeline: Some(Pipeline {
                status: "running".into(),
                ..Default::default()
            }),
            ..mr()
        };
        assert!(kinds(&[issue()], &remote(Some(running), vec![], None)).is_empty());
        let mut posted = issue();
        posted.tree.as_mut().unwrap().actions = vec![Action {
            kind: "merge".into(),
            ..Default::default()
        }];
        assert!(kinds(&[posted], &remote(Some(mr()), vec![], None)).is_empty());
    }

    #[test]
    fn asks_for_the_merge_of_a_ready_draft() {
        let draft = remote(
            Some(Mr {
                is_draft: true,
                merge_status: Some("draft_status".into()),
                ..mr()
            }),
            vec![],
            None,
        );
        let ready: HashSet<i64> = [7].into();
        let found = auto_actions_of(
            &[issue()],
            Some(&draft),
            &HashMap::new(),
            &AutoFacts {
                ready_drafts: Some(&ready),
                now: NOW,
                ..Default::default()
            },
        );
        assert_eq!(found[0].text, "pipeline passed, every box and try done, still a draft");
        assert!(found[0].merge.as_ref().unwrap().can_mark_ready);
        assert!(kinds(&[issue()], &draft).is_empty());
    }

    #[test]
    fn leaves_the_merge_out_while_a_check_or_a_change_is_open() {
        let mut tried = issue();
        tried.tree.as_mut().unwrap().actions = vec![Action {
            id: 1,
            kind: "try".into(),
            text: "the row".into(),
            ..Default::default()
        }];
        assert!(kinds(&[tried.clone()], &remote(Some(mr()), vec![], None)).is_empty());
        tried.tree.as_mut().unwrap().actions[0].on = Some("test".into());
        assert_eq!(
            kinds(&[tried], &remote(Some(mr()), vec![], None)),
            [pair("merge", "ready, pipeline passed")]
        );
        let boxes = vec![IssueBox {
            heading: Some("Acceptance criteria".into()),
            text: "Checked".into(),
            ..Default::default()
        }];
        assert!(
            kinds(
                &[issue()],
                &remote(
                    Some(mr()),
                    vec![],
                    Some(Issue {
                        boxes,
                        ..Default::default()
                    })
                )
            )
            .is_empty()
        );
        let mut changed = issue();
        let tree = changed.tree.as_mut().unwrap();
        tree.change = Some(Change {
            text: "padding".into(),
            head: Some("a".into()),
            ..Default::default()
        });
        tree.pushed = Some("a".into());
        let green = |sha: &str| Mr {
            pipeline: Some(Pipeline {
                status: "success".into(),
                sha: Some(sha.into()),
                ..Default::default()
            }),
            ..mr()
        };
        assert!(kinds(&[changed.clone()], &remote(Some(green("a")), vec![], None)).is_empty());
        changed.tree.as_mut().unwrap().pushed = Some("b".into());
        assert_eq!(
            kinds(&[changed], &remote(Some(green("b")), vec![], None)),
            [pair("merge", "ready, pipeline passed")]
        );
    }

    #[test]
    fn asks_for_a_push_once_commits_sit_unpushed_for_long() {
        let mut late = issue();
        let tree = late.tree.as_mut().unwrap();
        tree.unpushed = 2;
        tree.unpushed_at = Some(NOW - 11 * 60_000);
        tree.pushed = Some("a".into());
        tree.has_remote_branch = true;
        assert_eq!(
            kinds(&[late.clone()], &Remote::default()),
            [pair("push", "2 commits not pushed for 11 min: click to tell its agent")]
        );
        late.tree.as_mut().unwrap().unpushed_at = Some(NOW);
        assert!(kinds(&[late], &Remote::default()).is_empty());
    }

    #[test]
    fn flags_a_worktree_half_merged_for_long_unless_its_agent_runs() {
        let mut half = issue();
        half.tree.as_mut().unwrap().merge_head_at = Some(NOW - 11 * 60_000);
        let facts = AutoFacts {
            now: NOW,
            ..Default::default()
        };
        let found = kinds_with(&[half.clone()], &Remote::default(), &agents("idle"), &facts);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "half-merged");
        assert!(found[0].1.starts_with("half-merged since "));
        assert!(kinds_with(&[half.clone()], &Remote::default(), &agents("working"), &facts).is_empty());
        half.tree.as_mut().unwrap().merge_head_at = Some(NOW - 60_000);
        assert!(kinds_with(&[half], &Remote::default(), &agents("idle"), &facts).is_empty());
    }

    #[test]
    fn names_the_failed_job_and_why_its_line_didnt_get_in() {
        let failed = Mr {
            pipeline: Some(Pipeline {
                status: "failed".into(),
                ..Default::default()
            }),
            ..mr()
        };
        let jobs = vec![
            Job {
                name: "lint".into(),
                status: "success".into(),
                ..Default::default()
            },
            Job {
                name: "check:vita".into(),
                status: "failed".into(),
                url: Some("j".into()),
                ..Default::default()
            },
        ];
        let gitlab = remote(Some(failed), jobs, None);
        assert_eq!(
            kinds(&[Entry { tree: None, ..issue() }], &gitlab),
            [pair("pipeline", "!70 job check:vita failed: no agent to tell")]
        );
        assert_eq!(
            kinds(&[issue()], &gitlab),
            [pair("pipeline", "!70 job check:vita failed: click to tell its agent")]
        );
        // No conversation written down: herdr may still have its agent, a click tries it.
        let mut unbound = issue();
        if let Some(tree) = unbound.tree.as_mut() {
            tree.claude = None;
        }
        assert_eq!(
            kinds(&[unbound], &gitlab),
            [pair("pipeline", "!70 job check:vita failed: click to tell its agent")]
        );
    }

    #[test]
    fn cleans_up_the_open_boxes_through_the_agent() {
        let boxes = vec![
            IssueBox {
                is_ticked: true,
                ..Default::default()
            },
            IssueBox {
                is_ticked: false,
                ..Default::default()
            },
        ];
        let gitlab = remote(
            Some(Mr {
                state: "merged".into(),
                ..mr()
            }),
            vec![],
            Some(Issue {
                iid: 7,
                boxes,
                ..Default::default()
            }),
        );
        let found = auto_actions_of(
            &[issue()],
            Some(&gitlab),
            &agents("working"),
            &AutoFacts {
                now: NOW,
                ..Default::default()
            },
        );
        assert_eq!(
            found.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(),
            ["clean up:feature/7-thing:boxes"]
        );
        assert!(found[0].text.contains("1 open boxes"));
        assert!(kinds(&[Entry { tree: None, ..issue() }], &gitlab).is_empty());
    }

    #[test]
    fn hides_a_row_whose_line_got_in_until_a_new_pipeline_fails() {
        let mut sent = issue();
        sent.tree.as_mut().unwrap().sent = ["pipeline:!70:41".to_string(), "boxes:!70".to_string()].into();
        let failed = |id: i64| {
            remote(
                Some(Mr {
                    pipeline: Some(Pipeline {
                        id,
                        status: "failed".into(),
                        ..Default::default()
                    }),
                    ..mr()
                }),
                vec![],
                None,
            )
        };
        let facts = AutoFacts {
            now: NOW,
            ..Default::default()
        };
        assert!(kinds_with(std::slice::from_ref(&sent), &failed(41), &agents("working"), &facts).is_empty());
        assert_eq!(
            kinds_with(std::slice::from_ref(&sent), &failed(42), &agents("working"), &facts),
            [pair("pipeline", "!70 failed: click to tell its agent")]
        );
        let merged = remote(
            Some(Mr {
                state: "merged".into(),
                ..mr()
            }),
            vec![],
            Some(Issue {
                iid: 7,
                boxes: vec![IssueBox::default()],
                ..Default::default()
            }),
        );
        assert!(kinds_with(&[sent], &merged, &agents("working"), &facts).is_empty());
    }

    #[test]
    fn shows_an_agent_that_waits_and_none_that_works() {
        let mut asking = issue();
        asking.tree.as_mut().unwrap().claude.as_mut().unwrap().answer =
            Some("Done with A.\n\nShall I do B too?".into());
        let facts = AutoFacts {
            now: NOW,
            ..Default::default()
        };
        let none = Remote::default();
        assert_eq!(
            kinds_with(std::slice::from_ref(&asking), &none, &agents("idle"), &facts),
            [pair("waits", "Shall I do B too?")]
        );
        assert!(kinds_with(std::slice::from_ref(&asking), &none, &agents("working"), &facts).is_empty());
        assert_eq!(
            kinds_with(std::slice::from_ref(&asking), &none, &agents("blocked"), &facts),
            [pair("waits", "Done with A.")]
        );
        asking.tree.as_mut().unwrap().pending = Some("may it run Bash: git push?".into());
        assert_eq!(
            kinds_with(std::slice::from_ref(&asking), &none, &agents("blocked"), &facts),
            [pair("waits", "may it run Bash: git push?")]
        );
        // Headless (#360): its `permission` action asks, no `waits` beside it.
        asking.tree.as_mut().unwrap().actions = vec![Action {
            id: 1,
            kind: "permission".into(),
            text: "may 12-x run Bash: docker system prune?".into(),
            ..Default::default()
        }];
        assert!(kinds_with(&[asking], &none, &agents("blocked"), &facts).is_empty());
    }

    #[test]
    fn a_claude_holding_its_lines_has_a_row_that_ends_it() {
        let facts = AutoFacts {
            now: NOW,
            ..Default::default()
        };
        let items = auto_actions_of(&[issue()], Some(&Remote::default()), &agents(crate::runs::HELD), &facts);
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].kind, "held");
        assert_eq!(items[0].go, Go::EndHeld("/w/7".into()));
        assert!(items[0].text.contains("ends it once it is idle"));
    }

    #[test]
    fn finds_what_is_new_since_the_last_read() {
        let items = [
            Item {
                key: "a".into(),
                ..Default::default()
            },
            Item {
                key: "b".into(),
                ..Default::default()
            },
        ];
        assert!(new_actions_of(None, &items).is_empty());
        let before: HashSet<String> = ["a".to_string()].into();
        assert_eq!(new_actions_of(Some(&before), &items)[0].key, "b");
    }

    fn conflicted(has_conflicts: bool) -> (Entry, Remote) {
        let entry = Entry {
            branch: "feature/1-a".into(),
            tree: Some(Tree {
                dir: "/w/a".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let found = Mr {
            iid: 221,
            state: "opened".into(),
            has_conflicts,
            ..Default::default()
        };
        let gitlab = Remote {
            of: [(
                "feature/1-a".to_string(),
                BranchRemote {
                    mr: Some(found),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        (entry, gitlab)
    }

    #[test]
    fn tells_the_agent_of_an_mr_with_conflicts_once_and_forgets_it_after() {
        let (entry, gitlab) = conflicted(true);
        assert_eq!(
            conflicts_of(std::slice::from_ref(&entry), &gitlab),
            [Tell::Send {
                dir: "/w/a".into(),
                key: "conflicts:221".into(),
                line: format!(
                    "[overview] !221 has conflicts with develop: merge develop in with {run} wt:merge-base --resolve, resolve the conflicts, git add, git commit --no-edit, {run} wt:check and push in this run",
                    run = crate::config::run_word()
                ),
                iid: 221,
                id: None,
            }]
        );
        let mut told = entry.clone();
        told.tree.as_mut().unwrap().sent = ["conflicts:221".to_string()].into();
        assert!(conflicts_of(std::slice::from_ref(&told), &gitlab).is_empty());
        let (_, clean) = conflicted(false);
        assert_eq!(
            conflicts_of(std::slice::from_ref(&told), &clean),
            [Tell::Forget {
                dir: "/w/a".into(),
                key: "conflicts:221".into()
            }]
        );
        assert!(conflicts_of(std::slice::from_ref(&entry), &clean).is_empty());
        let mut checking = gitlab.clone();
        checking
            .of
            .get_mut("feature/1-a")
            .unwrap()
            .mr
            .as_mut()
            .unwrap()
            .merge_check = Some("checking".into());
        assert!(conflicts_of(std::slice::from_ref(&entry), &checking).is_empty());
        assert!(conflicts_of(&[told], &checking).is_empty());
        let mut unpushed = entry.clone();
        unpushed.tree.as_mut().unwrap().unpushed = 3;
        assert!(conflicts_of(&[unpushed], &gitlab).is_empty());
        assert!(conflicts_of(&[Entry { tree: None, ..entry }], &gitlab).is_empty());
    }

    #[test]
    fn writes_the_line_of_a_failed_pipeline() {
        let failed = Mr {
            iid: 70,
            url: "u".into(),
            pipeline: Some(Pipeline {
                id: 41,
                status: "failed".into(),
                web_url: Some("p".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let jobs = vec![
            Job {
                name: "lint".into(),
                status: "success".into(),
                url: Some("j1".into()),
                ..Default::default()
            },
            Job {
                name: "check:vita".into(),
                status: "failed".into(),
                url: Some("j2".into()),
                log: Some(vec!["FAIL a.test.mjs".into(), "1 failing".into()]),
                ..Default::default()
            },
        ];
        assert_eq!(
            pipeline_line_of(&failed, &jobs, None),
            "[overview] the pipeline of !70 failed: job check:vita, j2: look into it; its log ends: FAIL a.test.mjs ⏎ 1 failing"
        );
        let develop = |status: &str, name: &str| DevelopPipeline {
            jobs: vec![Job {
                name: name.into(),
                status: status.into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            pipeline_line_of(&failed, &jobs, Some(&develop("failed", "check:vita"))),
            "[overview] the pipeline of !70 failed: job check:vita, j2: develop fails the same job: not yours, waits for the fix (say so in the MR)"
        );
        assert!(pipeline_line_of(&failed, &jobs, Some(&develop("success", "check:vita"))).contains(": look into it;"));
        let mut both = jobs.clone();
        both.push(Job {
            name: "scan:deps".into(),
            status: "failed".into(),
            url: Some("j3".into()),
            log: Some(vec![]),
            ..Default::default()
        });
        assert!(
            pipeline_line_of(&failed, &both, Some(&develop("failed", "scan:deps")))
                .contains("look into it (develop fails scan:deps too: not yours); its log ends: FAIL a.test.mjs")
        );
        assert_eq!(
            pipeline_line_of(&failed, &[], None),
            "[overview] the pipeline of !70 failed: p: look into it"
        );
    }

    #[test]
    fn finds_the_failed_pipelines_whose_agent_wasnt_told() {
        let entry = Entry {
            branch: "feature/1-a".into(),
            tree: Some(Tree {
                dir: "/w/a".into(),
                claude: Some(claude_of("c1")),
                ..Default::default()
            }),
            ..Default::default()
        };
        let of = |status: &str| {
            let found = Mr {
                iid: 221,
                state: "opened".into(),
                pipeline: Some(Pipeline {
                    id: 9,
                    status: status.into(),
                    web_url: Some("p".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            Remote {
                of: [(
                    "feature/1-a".to_string(),
                    BranchRemote {
                        mr: Some(found),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            }
        };
        assert_eq!(
            failed_pipelines_of(std::slice::from_ref(&entry), &of("failed"), None),
            [Tell::Send {
                dir: "/w/a".into(),
                key: "pipeline:!221:9".into(),
                line: "[overview] the pipeline of !221 failed: p: look into it".into(),
                iid: 221,
                id: Some("c1".into()),
            }]
        );
        assert!(failed_pipelines_of(std::slice::from_ref(&entry), &of("success"), None).is_empty());
        let mut told = entry.clone();
        told.tree.as_mut().unwrap().sent = ["pipeline:!221:9".to_string()].into();
        assert!(failed_pipelines_of(&[told], &of("failed"), None).is_empty());
        let mut alone = entry;
        alone.tree.as_mut().unwrap().claude = None;
        assert!(failed_pipelines_of(&[alone], &of("failed"), None).is_empty());
        let _ = parse_iso("");
    }
}
