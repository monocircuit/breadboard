//! A red batch in the overview (#413): the MR into develop of a batch the
//! product shipped has a failed pipeline. BATCH says `batch red: <job>` and
//! lists the tasks merged into it, those whose files a failed job covers
//! first; `ship batch` is held while it is red, and says why.

use super::orchestration::OrchestrationLocal;
use super::{Paint, Span};
use crate::batch_red::{Suspect, covers, headline, job_of, suspects};
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
    /// `batch red: check:vita`, `batch red: check:vita +1`.
    pub fn headline(&self) -> String {
        let first = self.jobs.first().map_or("its pipeline", |(job, _)| job.as_str());
        match self.jobs.len() {
            0 | 1 => headline(first),
            count => format!("{} +{}", headline(first), count - 1),
        }
    }

    /// Why `ship batch` waits: `ship held: batch/breadboard-2 is red (check:vita)`.
    pub fn held(&self) -> String {
        let first = self.jobs.first().map_or("its pipeline", |(job, _)| job.as_str());
        format!("ship held: {} is red ({first})", self.branch)
    }

    /// What is done about it: `#460 fixes it`, or the click that files the fix.
    pub fn fixing(&self) -> String {
        match self.fix {
            Some(number) => format!("#{number} fixes it"),
            None => "a click files its fix".into(),
        }
    }

    /// The part of the BATCH row, linked to the failed job.
    pub fn part(&self) -> Vec<Span> {
        vec![Span {
            text: self.headline(),
            paint: Paint::Red,
            url: self.jobs.first().and_then(|(_, url)| url.clone()),
        }]
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

/// The page of MR `iid` beside the page `url` of another MR of the project.
fn mr_url_of(url: &str, iid: i64) -> String {
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
    let (mr, failed) = found.shipping.iter().chain(&found.mr).find_map(|mr| {
        let failed = found.failed_jobs.get(&mr.iid).filter(|jobs| !jobs.is_empty())?;
        (mr.pipeline_status() == Some("failed")).then_some((mr, failed))
    })?;
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
        assert_eq!(red.headline(), "batch red: check:vita +1");
        assert_eq!(red.held(), "ship held: batch/b-2 is red (check:vita)");
        assert_eq!(red.part()[0].url.as_deref(), Some("https://g/p/-/jobs/7"));
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
        assert_eq!(red.headline(), "batch red: check:vita +1");
    }
}
