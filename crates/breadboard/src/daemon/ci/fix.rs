//! A failed MR pipeline (#371): its jobs retried, a refused one asked for
//! again, a job that failed twice left to its agent (core's `ci_fix`).

use super::{first_line, start};
use crate::daemon::glab::{glab_get, glab_post};
use breadboard_core::ci::{self, Asked, OpenMr};
use breadboard_core::ci_fix::{self, Act, Verdict};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The failed pipelines of the members' newest commits (#371), each looked
/// at once per failure: its jobs' tries read, the failed ones retried (the
/// runner's failure, a first try's), a refused one asked for again; a job
/// that failed twice is left to its agent, whom the overview tells. Where
/// each stands goes into `asked.fixes`; a pipeline that passed ends them.
/// What it did, for the band.
#[allow(clippy::too_many_arguments)]
pub fn step(
    main: &Path,
    glab: &str,
    mrs: &[OpenMr],
    members: &HashSet<i64>,
    asked: &mut Asked,
    looked: &mut HashMap<i64, String>,
    max: u32,
    now: i64,
) -> Option<String> {
    let mut said = None;
    for mr in mrs.iter().filter(|mr| members.contains(&mr.iid)) {
        let (Some(pipeline), Some(sha)) = (&mr.pipeline, &mr.sha) else {
            continue;
        };
        if pipeline.sha.as_ref() != Some(sha) {
            continue;
        }
        match pipeline.status.as_str() {
            "success" => {
                asked.fixes.remove(&mr.iid);
                continue;
            }
            "failed" => {}
            _ => {
                looked.remove(&pipeline.id);
                continue;
            }
        }
        // A retried job may fail again between two looks: a new failure moves its `updatedAt`.
        let updated = pipeline.updated_at.clone().unwrap_or_default();
        let is_looked = looked.get(&pipeline.id) == Some(&updated);
        // A newer pipeline refused over a passed one: the queue's rescue asks once (#369).
        if mr.blocked_by.is_some_and(|blocking| !asked.ours.contains(&blocking)) || is_looked {
            continue;
        }
        let verdict = if mr.is_refused {
            Verdict::Refused
        } else {
            match glab_get(
                main,
                glab,
                &format!(
                    "projects/:fullpath/pipelines/{}/jobs?include_retried=true&per_page=100",
                    pipeline.id
                ),
            ) {
                Ok(listed) => ci_fix::verdict_of(&ci_fix::tries_of(&listed)),
                Err(_) => continue,
            }
        };
        let is_live = verdict == Verdict::Live;
        let fixes = asked.fixes.entry(mr.iid).or_default();
        let act = ci_fix::step(fixes, pipeline.id, sha, verdict, max, now);
        // A refusal waits its time and is looked at again; a retry under way when it ends.
        let is_waiting = fixes
            .now
            .as_ref()
            .is_some_and(|fix| fix.stage == ci_fix::Stage::Refused);
        if !is_live && !is_waiting {
            looked.insert(pipeline.id, updated);
        }
        match act {
            Some(Act::Retry(jobs)) => {
                let retried: Vec<String> = jobs
                    .iter()
                    .filter(|id| glab_post(main, glab, &format!("projects/:fullpath/jobs/{id}/retry")).is_ok())
                    .map(|id| id.to_string())
                    .collect();
                said = Some(format!("!{} retried job {}", mr.iid, retried.join(", ")));
            }
            Some(Act::AskAgain) => {
                let start = ci::Start {
                    iid: mr.iid,
                    sha: sha.clone(),
                    cancel: None,
                    is_again: true,
                    after: None,
                };
                said = Some(match start::run_with(main, &start, glab) {
                    Ok(start::Ran::Created(id)) => {
                        if let Some(id) = id {
                            asked.add_ours(id);
                        }
                        asked.started.insert(mr.iid, (sha.clone(), now));
                        format!("!{} refused, asked again", mr.iid)
                    }
                    Ok(_) => format!("!{} has a newer pipeline", mr.iid),
                    Err(error) => format!("!{} not asked again: {}", mr.iid, first_line(&error)),
                });
            }
            None => {}
        }
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::glab::fake_glab;

    #[test]
    fn retries_a_first_failure_once_and_leaves_a_second_to_the_agent() {
        // #371: a flake passes its retry; a job that failed twice is the MR's doing.
        let scratch = std::env::temp_dir().join(format!("ci-fix-test-{}", std::process::id()));
        const JOBS: &str = "api projects/:fullpath/pipelines/90/jobs?include_retried=true&per_page=100";
        const RETRY: &str = "api --method POST projects/:fullpath/jobs/5/retry";
        let mr = OpenMr {
            iid: 339,
            sha: Some("a".into()),
            pipeline: Some(breadboard_core::gitlab::Pipeline {
                id: 90,
                status: "failed".into(),
                sha: Some("a".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let members = HashSet::from([339]);
        let (mut asked, mut looked) = (Asked::default(), HashMap::new());
        let once = r#"[{"id":5,"name":"check:vita","status":"failed","failure_reason":"script_failure"}]"#;
        let glab = fake_glab(&scratch, &[(JOBS, once)]);
        let said = step(
            &scratch,
            &glab,
            std::slice::from_ref(&mr),
            &members,
            &mut asked,
            &mut looked,
            3,
            0,
        );
        assert_eq!(said.as_deref(), Some("!339 retried job 5"));
        // Looked at once: the next tick asks nothing while it stays failed.
        step(
            &scratch,
            &glab,
            std::slice::from_ref(&mr),
            &members,
            &mut asked,
            &mut looked,
            3,
            0,
        );
        let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
        assert_eq!(log, format!("{JOBS}\n{RETRY}\n"));
        // The retry runs, then fails as well: its agent's, the first of three runs.
        let running = OpenMr {
            pipeline: mr.pipeline.clone().map(|pipeline| breadboard_core::gitlab::Pipeline {
                status: "running".into(),
                ..pipeline
            }),
            ..mr.clone()
        };
        step(&scratch, &glab, &[running], &members, &mut asked, &mut looked, 3, 0);
        let twice = r#"[{"id":5,"name":"check:vita","status":"failed","failure_reason":"script_failure"},{"id":6,"name":"check:vita","status":"failed","failure_reason":"script_failure"}]"#;
        let glab = fake_glab(&scratch, &[(JOBS, twice)]);
        step(
            &scratch,
            &glab,
            std::slice::from_ref(&mr),
            &members,
            &mut asked,
            &mut looked,
            3,
            0,
        );
        let fix = asked.fixes[&339].now.clone().unwrap();
        assert_eq!(
            (fix.stage, fix.mark_text()),
            (ci_fix::Stage::Fix, "CI fix 1/3".to_string())
        );
        let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
        assert_eq!(log.matches("/retry").count(), 1, "{log}");
        // Failed again between two looks, never seen running: its new `updatedAt` tells.
        let (mut quick, mut quick_looked) = (Asked::default(), HashMap::new());
        let at = |updated: &str| OpenMr {
            pipeline: mr.pipeline.clone().map(|pipeline| breadboard_core::gitlab::Pipeline {
                updated_at: Some(updated.into()),
                ..pipeline
            }),
            ..mr.clone()
        };
        let glab = fake_glab(&scratch, &[(JOBS, once)]);
        step(
            &scratch,
            &glab,
            &[at("1")],
            &members,
            &mut quick,
            &mut quick_looked,
            3,
            0,
        );
        let glab = fake_glab(&scratch, &[(JOBS, twice)]);
        step(
            &scratch,
            &glab,
            &[at("2")],
            &members,
            &mut quick,
            &mut quick_looked,
            3,
            0,
        );
        assert_eq!(quick.fixes[&339].now.as_ref().unwrap().stage, ci_fix::Stage::Fix);
        // A pipeline that passed ends its fixes; an MR of no orchestration is never looked at.
        let passed = OpenMr {
            pipeline: mr.pipeline.clone().map(|pipeline| breadboard_core::gitlab::Pipeline {
                status: "success".into(),
                ..pipeline
            }),
            ..mr.clone()
        };
        step(&scratch, &glab, &[passed], &members, &mut asked, &mut looked, 3, 0);
        assert!(asked.fixes.is_empty());
        let (mut asked, mut looked) = (Asked::default(), HashMap::new());
        step(&scratch, &glab, &[mr], &HashSet::new(), &mut asked, &mut looked, 3, 0);
        assert!(asked.fixes.is_empty());
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
