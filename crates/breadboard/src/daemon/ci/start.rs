//! A pipeline started for the MR the queue decided for (#357, #369, #383).

use super::first_line;
use crate::daemon::glab::{glab_get, glab_post};
use breadboard_core::ci::{self, Asked};
use std::path::Path;

/// What `run` did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ran {
    /// Created, its id when GitLab said it.
    Created(Option<i64>),
    /// GitLab has one of that commit already: none created (#369).
    Exists(i64),
    /// The MR has a newer commit than the one decided for: nothing done.
    Superseded,
}

/// Starts `start` and keeps what was asked for in `asked`: what the queue says it did.
pub fn step(main: &Path, start: &ci::Start, asked: &mut Asked, now: i64) -> String {
    let said = run(main, start);
    // Its superseded pipeline, cancelled by the queue: no person's cancel (#383).
    if let (Some(id), false) = (start.cancel, matches!(said, Ok(Ran::Superseded | Ran::Exists(_)))) {
        asked.add_cancelled(id);
    }
    // Asked for, whether GitLab took it or not: a refusal (no job for
    // these changes, the activity limit) isn't asked again for the
    // same commit; a commit superseded meanwhile is decided anew.
    if !matches!(said, Ok(Ran::Superseded)) {
        asked.preempted.remove(&start.iid);
        asked.started.insert(start.iid, (start.sha.clone(), now));
        if let Some(blocking) = start.after {
            asked.rescued.insert(start.iid, blocking);
        }
    }
    match said {
        Ok(Ran::Created(id)) => {
            if let Some(id) = id {
                asked.add_ours(id);
            }
            format!("started !{}", start.iid)
        }
        Ok(Ran::Exists(id)) => format!("!{} has pipeline {id} of its commit", start.iid),
        Ok(Ran::Superseded) => format!("!{} pushed meanwhile", start.iid),
        Err(error) => format!("!{} not started: {}", start.iid, first_line(&error)),
    }
}

/// GitLab read back first (#369: its list may lag or be read wrong): the MR
/// still at the commit decided for, and no pipeline of that commit but a
/// cancelled or skipped one (a click's: none still to finish; one replacing
/// a blocking pipeline: none newer than that one). Then its
/// superseded pipeline cancelled, and the pipeline of the MR's newest commit created.
fn run(main: &Path, start: &ci::Start) -> Result<Ran, String> {
    run_with(main, start, "glab")
}

fn run_with(main: &Path, start: &ci::Start, glab: &str) -> Result<Ran, String> {
    let mr = glab_get(main, glab, &format!("projects/:fullpath/merge_requests/{}", start.iid))?;
    if mr.get("sha").and_then(serde_json::Value::as_str) != Some(start.sha.as_str()) {
        return Ok(Ran::Superseded);
    }
    let pipelines = glab_get(
        main,
        glab,
        &format!("projects/:fullpath/merge_requests/{}/pipelines?per_page=100", start.iid),
    )?;
    let existing = pipelines.as_array().into_iter().flatten().find(|pipeline| {
        let status = pipeline
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let id = pipeline.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0);
        pipeline.get("sha").and_then(serde_json::Value::as_str) == Some(start.sha.as_str())
            && start.after.is_none_or(|blocking| id > blocking)
            && if start.is_again {
                ci::is_live(status)
            } else {
                !matches!(status, "canceled" | "canceling" | "skipped")
            }
    });
    if let Some(pipeline) = existing {
        return Ok(Ran::Exists(
            pipeline.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0),
        ));
    }
    if let Some(id) = start.cancel {
        // A cancel that fails (finished meanwhile) doesn't stop the new one.
        let _ = glab_post(main, glab, &format!("projects/:fullpath/pipelines/{id}/cancel"));
    }
    let created = glab_post(
        main,
        glab,
        &format!("projects/:fullpath/merge_requests/{}/pipelines", start.iid),
    )?;
    Ok(Ran::Created(created.get("id").and_then(serde_json::Value::as_i64)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::glab::fake_glab;

    const MR: &str = "api projects/:fullpath/merge_requests/339";
    const PIPELINES: &str = "api projects/:fullpath/merge_requests/339/pipelines?per_page=100";

    #[test]
    fn creates_the_pipeline_after_reading_gitlab_and_cancelling_the_superseded_one() {
        let scratch = std::env::temp_dir().join(format!("ci-run-test-{}", std::process::id()));
        let glab = fake_glab(
            &scratch,
            &[
                (MR, r#"{"sha":"a"}"#),
                (
                    PIPELINES,
                    r#"[{"id":70,"sha":"old","status":"running"},{"id":60,"sha":"a","status":"canceled"}]"#,
                ),
                (
                    "api --method POST projects/:fullpath/merge_requests/339/pipelines",
                    r#"{"id":71}"#,
                ),
            ],
        );
        let start = ci::Start {
            iid: 339,
            sha: "a".into(),
            cancel: Some(70),
            is_again: false,
            after: None,
        };
        assert_eq!(run_with(&scratch, &start, &glab), Ok(Ran::Created(Some(71))));
        assert_eq!(
            std::fs::read_to_string(scratch.join("glab.log")).unwrap(),
            format!(
                "{MR}\n{PIPELINES}\napi --method POST projects/:fullpath/pipelines/70/cancel\n\
                 api --method POST projects/:fullpath/merge_requests/339/pipelines\n"
            )
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn creates_none_when_gitlab_has_one_of_the_commit_or_a_newer_commit() {
        // #369: !337 got seven pipelines of e426e44f, one failed at its creation.
        let scratch = std::env::temp_dir().join(format!("ci-exists-test-{}", std::process::id()));
        let start = ci::Start {
            iid: 339,
            sha: "a".into(),
            cancel: Some(70),
            is_again: false,
            after: None,
        };
        for (status, ran) in [("pending", Ran::Exists(80)), ("failed", Ran::Exists(80))] {
            let pipelines = format!(r#"[{{"id":80,"sha":"a","status":"{status}"}}]"#);
            let glab = fake_glab(&scratch, &[(MR, r#"{"sha":"a"}"#), (PIPELINES, &pipelines)]);
            assert_eq!(run_with(&scratch, &start, &glab), Ok(ran));
        }
        // A click asks again for one that ended, never while one runs.
        let again = ci::Start {
            is_again: true,
            ..start.clone()
        };
        let glab = fake_glab(
            &scratch,
            &[
                (MR, r#"{"sha":"a"}"#),
                (PIPELINES, r#"[{"id":80,"sha":"a","status":"failed"}]"#),
            ],
        );
        assert_eq!(run_with(&scratch, &again, &glab), Ok(Ran::Created(None)));
        // Over a blocking cancelled one: the older pass doesn't hold it, a newer one does.
        let rescue = ci::Start {
            after: Some(80),
            ..start.clone()
        };
        let pipelines = r#"[{"id":80,"sha":"a","status":"canceled"},{"id":70,"sha":"a","status":"success"}]"#;
        let glab = fake_glab(&scratch, &[(MR, r#"{"sha":"a"}"#), (PIPELINES, pipelines)]);
        assert_eq!(run_with(&scratch, &rescue, &glab), Ok(Ran::Created(None)));
        let pipelines = r#"[{"id":81,"sha":"a","status":"pending"},{"id":80,"sha":"a","status":"canceled"}]"#;
        let glab = fake_glab(&scratch, &[(MR, r#"{"sha":"a"}"#), (PIPELINES, pipelines)]);
        assert_eq!(run_with(&scratch, &rescue, &glab), Ok(Ran::Exists(81)));
        // Pushed meanwhile: the old commit gets nothing.
        let glab = fake_glab(&scratch, &[(MR, r#"{"sha":"b"}"#)]);
        assert_eq!(run_with(&scratch, &start, &glab), Ok(Ran::Superseded));
        let log = std::fs::read_to_string(scratch.join("glab.log")).unwrap();
        assert_eq!(
            log.matches("POST projects/:fullpath/merge_requests").count(),
            2,
            "{log}"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
