//! A red batch in the overview (#413): the MR into develop of a batch the
//! product shipped has a failed pipeline. BATCH names it apart from the
//! current batch (#491), `batch 7 (!468, shipped) failed: <job>`, and lists
//! the tasks merged into it, those whose files a failed job covers first. Its fix goes into the current batch, whose ship replaces it (#464).

use super::orchestration::OrchestrationLocal;
use super::{Paint, Span};
use crate::batch_red::{Suspect, covers, job_of, suspects};
use crate::gitlab::Remote;

/// At most this many suspects under BATCH; the others in a count.
const SHOWN: usize = 6;

/// The red batch of an orchestration's product, the oldest one red.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Red {
    /// The red batch's branch: `batch/breadboard-2`.
    pub branch: String,
    /// Its MR into develop.
    pub mr: i64,
    pub mr_url: String,
    /// Its pipeline's page: the link of a red batch without a failed job.
    pub pipeline_url: Option<String>,
    /// Its failed jobs, each once, with the page of the first of its copies.
    pub jobs: Vec<(String, Option<String>)>,
    /// The tasks merged into it, the likeliest first.
    pub suspects: Vec<Suspect>,
    /// The files each job covers at its head.
    pub globs: Vec<Option<Vec<String>>>,
    /// Its fix issue, once `wt:batch --fix` filed it.
    pub fix: Option<i64>,
}

impl Red {
    /// `: check:vita`, `: check:vita +1`: its failed jobs; none before a job ran.
    fn failed(&self) -> String {
        match &self.jobs[..] {
            [] => String::new(),
            [(first, _)] => format!(": {first}"),
            [(first, _), rest @ ..] => format!(": {first} +{}", rest.len()),
        }
    }

    /// Whether its fix issue is merged into `tasks`, the current batch's (#464).
    pub fn is_fixed_in(&self, tasks: &[crate::gitlab::BatchTask]) -> bool {
        self.fix.is_some_and(|number| {
            tasks
                .iter()
                .any(|task| crate::git::issue_of_branch(&task.branch) == Some(number as u64))
        })
    }

    /// BATCH's line of it, apart from the current batch's own state (#491),
    /// linked to the failed job: `batch 7 (!468, shipped) failed:
    /// scan:deps:mr · ship of batch 8 held until it is green`.
    pub fn line(&self, current: &str, is_fixed: bool) -> Vec<Span> {
        let current = name_of(current);
        let then = if is_fixed {
            format!("its fix is in {current}, whose ship replaces it")
        } else {
            format!("ship of {current} held until it is green")
        };
        vec![
            Span {
                text: format!(
                    "{} (!{}, shipped) failed{}",
                    name_of(&self.branch),
                    self.mr,
                    self.failed()
                ),
                paint: Paint::Red,
                url: self
                    .jobs
                    .first()
                    .and_then(|(_, url)| url.clone())
                    .or_else(|| self.pipeline_url.clone()),
            },
            Span::dim(format!(" · {then}")),
        ]
    }

    /// `batch/breadboard-2 is red (check:vita)`.
    pub fn said(&self) -> String {
        let first = self.jobs.first().map_or("its pipeline", |(job, _)| job.as_str());
        format!("{} is red ({first})", self.branch)
    }

    /// What is done about it: `#460 fixes it`, or the click that files the fix.
    pub fn fixing(&self) -> String {
        match self.fix {
            Some(number) => format!("#{number} fixes it"),
            None => "a click files its fix".into(),
        }
    }

    /// One line per suspect, the likeliest first: `!431 #411 feat: … · its files run in check:vita`.
    pub fn lines(&self) -> Vec<Vec<Span>> {
        let mut lines: Vec<Vec<Span>> = self
            .suspects
            .iter()
            .take(SHOWN)
            .map(|suspect| {
                let task = &suspect.merged;
                let mut line = match task.mr {
                    Some(iid) => vec![Span::link(format!("!{iid}"), Some(&mr_url_of(&self.mr_url, iid)))],
                    None => vec![Span::dim(task.sha.chars().take(8).collect::<String>())],
                };
                if let Some(issue) = task.issue {
                    line.push(Span::dim(format!(" #{issue}")));
                }
                line.push(if suspect.is_covered {
                    Span::plain(format!(" {}", task.title))
                } else {
                    Span::dim(format!(" {}", task.title))
                });
                if let Some(job) = self.covering(suspect) {
                    line.push(Span::new(format!(" · its files run in {job}"), Paint::Yellow));
                }
                line
            })
            .collect();
        if self.suspects.len() > SHOWN {
            lines.push(vec![Span::dim(format!("… {} more", self.suspects.len() - SHOWN))]);
        }
        lines
    }

    /// The first failed job whose files the suspect changed; none for a job of any change.
    fn covering(&self, suspect: &Suspect) -> Option<&str> {
        self.jobs.iter().zip(&self.globs).find_map(|((job, _), globs)| {
            let globs = globs.as_ref()?;
            suspect
                .merged
                .files
                .iter()
                .any(|file| covers(globs, file))
                .then_some(job.as_str())
        })
    }
}

/// `batch/breadboard-7` → `batch 7`; another branch as it is.
pub fn name_of(branch: &str) -> String {
    crate::batch::parse(branch).map_or_else(|| branch.to_string(), |(_, number)| format!("batch {number}"))
}

/// The page of MR `iid` beside the page `url` of another MR of the project.
pub fn mr_url_of(url: &str, iid: i64) -> String {
    match url.rsplit_once("/merge_requests/") {
        Some((project, _)) => format!("{project}/merge_requests/{iid}"),
        None => String::new(),
    }
}

/// The oldest batch of the orchestration's product whose MR into develop
/// failed its pipeline: the batches shipped before its own, then its own.
/// None while every one passes, runs, or GitLab hasn't answered.
pub fn red_of(local: &OrchestrationLocal, remote: Option<&Remote>) -> Option<Red> {
    let epic = local
        .epic
        .as_ref()
        .filter(|epic| crate::batch::is_batch(&epic.branch))?;
    let found = remote?.epic.as_ref()?;
    // A pipeline that failed before any job ran (its CI config) is red all the same (#491).
    let mr = found
        .shipping
        .iter()
        .chain(&found.mr)
        .find(|mr| mr.pipeline_status() == Some("failed"))?;
    let failed = found.failed_jobs.get(&mr.iid).map(Vec::as_slice).unwrap_or_default();
    let mut jobs: Vec<(String, Option<String>)> = Vec::new();
    for job in failed {
        let name = job_of(&job.name);
        if !jobs.iter().any(|(known, _)| known == name) {
            jobs.push((name.to_string(), job.url.clone()));
        }
    }
    let shipped = epic.shipped.iter().find(|shipped| shipped.branch == mr.branch);
    let globs: Vec<Option<Vec<String>>> = jobs
        .iter()
        .map(|(job, _)| shipped.and_then(|shipped| shipped.jobs.get(job).cloned()))
        .collect();
    let suspects = shipped.map_or_else(Vec::new, |shipped| {
        let given: Vec<&[String]> = globs.iter().flatten().map(Vec::as_slice).collect();
        suspects(&shipped.tasks, &given)
    });
    let fix = epic
        .fixes
        .iter()
        .find(|(_, branch)| *branch == mr.branch)
        .map(|(number, _)| *number);
    Some(Red {
        fix,
        branch: mr.branch.clone(),
        mr: mr.iid,
        mr_url: mr.url.clone(),
        pipeline_url: mr.pipeline.as_ref().and_then(|pipeline| pipeline.web_url.clone()),
        jobs,
        suspects,
        globs,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::batch_red::{Merged, Shipped};
    use crate::gitlab::{EpicRemote, Job, Mr, Pipeline};
    use crate::model::epic_row::EpicLocal;
    use crate::model::text_of;

    fn task(iid: i64, issue: u64, title: &str, files: &[&str]) -> Merged {
        Merged {
            sha: format!("{iid:08}"),
            subject: String::new(),
            title: title.into(),
            mr: Some(iid),
            issue: Some(issue),
            files: files.iter().map(|file| file.to_string()).collect(),
        }
    }

    pub(crate) fn red_world() -> (OrchestrationLocal, Remote) {
        let shipped = Shipped {
            branch: "batch/b-2".into(),
            // Newest first.
            tasks: vec![
                task(440, 413, "feat: a red batch", &["packages/breadboard/x.rs"]),
                task(431, 411, "feat: vita's page", &["products/vita/page.tsx"]),
                task(429, 409, "docs: the readme", &["README.md"]),
            ],
            jobs: [("check:vita".to_string(), vec!["products/vita/**/*".to_string()])].into(),
        };
        let local = OrchestrationLocal {
            name: "b".into(),
            epic: Some(EpicLocal {
                branch: "batch/b-3".into(),
                behind: Some(0),
                shipped: vec![shipped],
                fixes: Vec::new(),
            }),
            ..OrchestrationLocal::default()
        };
        let failed = |name: &str, id: i64| Job {
            id,
            name: name.into(),
            status: "failed".into(),
            url: Some(format!("https://g/p/-/jobs/{id}")),
            ..Job::default()
        };
        let remote = Remote {
            epic: Some(Box::new(EpicRemote {
                branch: "batch/b-3".into(),
                shipping: vec![Mr {
                    iid: 500,
                    branch: "batch/b-2".into(),
                    url: "https://g/p/-/merge_requests/500".into(),
                    pipeline: Some(Pipeline {
                        status: "failed".into(),
                        ..Pipeline::default()
                    }),
                    ..Mr::default()
                }],
                failed_jobs: [(
                    500,
                    vec![
                        failed("check:vita", 7),
                        failed("build:image 1/2", 8),
                        failed("build:image 2/2", 9),
                    ],
                )]
                .into(),
                ..EpicRemote::default()
            })),
            ..Remote::default()
        };
        (local, remote)
    }

    #[test]
    fn names_the_red_batch_its_jobs_and_the_likeliest_tasks_first() {
        let (local, remote) = red_world();
        let red = red_of(&local, Some(&remote)).unwrap();
        assert_eq!((red.branch.as_str(), red.mr), ("batch/b-2", 500));
        assert_eq!(
            text_of(&red.line("batch/b-3", false)),
            "batch 2 (!500, shipped) failed: check:vita +1 · ship of batch 3 held until it is green"
        );
        assert_eq!(
            text_of(&red.line("batch/b-3", true)),
            "batch 2 (!500, shipped) failed: check:vita +1 · its fix is in batch 3, whose ship replaces it"
        );
        assert_eq!(red.said(), "batch/b-2 is red (check:vita)");
        assert_eq!(
            red.line("batch/b-3", false)[0].url.as_deref(),
            Some("https://g/p/-/jobs/7")
        );
        let lines: Vec<String> = red.lines().iter().map(|line| text_of(line)).collect();
        assert_eq!(
            lines,
            [
                "!431 #411 feat: vita's page · its files run in check:vita",
                // build:image runs on any change: it ranks no task.
                "!440 #413 feat: a red batch",
                "!429 #409 docs: the readme",
            ]
        );
        assert_eq!(
            red.lines()[0][0].url.as_deref(),
            Some("https://g/p/-/merge_requests/431")
        );
    }

    #[test]
    fn no_red_batch_while_its_pipeline_passes_runs_or_gitlab_has_not_answered() {
        let (local, mut remote) = red_world();
        assert_eq!(red_of(&local, None), None);
        let found = remote.epic.as_mut().unwrap();
        found.shipping[0].pipeline.as_mut().unwrap().status = "running".into();
        assert_eq!(red_of(&local, Some(&remote)), None, "a fix's pipeline runs");
        let (mut local, remote) = red_world();
        local.epic.as_mut().unwrap().branch = "epic/15-b".into();
        assert_eq!(red_of(&local, Some(&remote)), None, "an epic of before");
        // git hasn't read the batch yet: red all the same, no suspects.
        let (mut local, remote) = red_world();
        local.epic.as_mut().unwrap().shipped.clear();
        let red = red_of(&local, Some(&remote)).unwrap();
        assert!(red.suspects.is_empty());
        assert!(text_of(&red.line("batch/b-3", false)).starts_with("batch 2 (!500, shipped) failed: check:vita +1"));
        // Failed before any job ran (its CI config): red, its pipeline linked (#491).
        let (local, mut remote) = red_world();
        remote.epic.as_mut().unwrap().failed_jobs.clear();
        let red = red_of(&local, Some(&remote)).unwrap();
        let line = red.line("batch/b-3", false);
        assert_eq!(
            text_of(&line),
            "batch 2 (!500, shipped) failed · ship of batch 3 held until it is green"
        );
        assert_eq!(line[0].url, red.pipeline_url);
        assert_eq!(red.suspects.len(), 3, "its tasks, unranked");
    }
}
