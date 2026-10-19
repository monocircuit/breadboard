//! The run-now clicks the queue keeps up front (#357) and forgets once done.

use breadboard_core::ci::{self, RunNow};
use std::collections::HashMap;
use std::path::Path;

/// How long a click keeps an MR up front when its pipeline never came.
const RUN_NOW_HOLDS_MS: i64 = 60 * 60_000;

/// The clicks that are done: their MR closed, the pipeline asked for since
/// ended (while it runs the click keeps it up front, never cancelled for
/// another), or an hour gone. `open`: iid → whether its newest commit's pipeline runs.
pub fn forget(
    main: &Path,
    run_now: &RunNow,
    started: &HashMap<i64, (String, i64)>,
    open: &HashMap<i64, bool>,
    now: i64,
) {
    let is_done = |iid: &i64, at: &i64| match open.get(iid) {
        None => true,
        Some(is_live) => {
            now - at > RUN_NOW_HOLDS_MS || (!is_live && started.get(iid).is_some_and(|(_, asked)| asked >= at))
        }
    };
    if !run_now.at.iter().any(|(iid, at)| is_done(iid, at)) {
        return;
    }
    // Read again: a click may have come since.
    let file = ci::run_now_file(main);
    let mut fresh: RunNow = ci::read_or_default(&file);
    fresh.at.retain(|iid, at| !is_done(iid, at));
    let _ = ci::write_whole(&file, &fresh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forgets_a_click_once_its_pipeline_was_asked_for() {
        let scratch = std::env::temp_dir().join(format!("ci-click-test-{}", std::process::id()));
        let mut run_now = RunNow::default();
        run_now.at.insert(1, 1000);
        run_now.at.insert(2, 1000);
        run_now.at.insert(3, 1000);
        ci::write_whole(&ci::run_now_file(&scratch), &run_now).unwrap();
        let started = HashMap::from([(1, ("a".to_string(), 2000))]);
        // 1 asked for since and ended, 2 waits still, 3 closed.
        let open = HashMap::from([(1, false), (2, false)]);
        forget(&scratch, &run_now, &started, &open, 3000);
        let left: RunNow = ci::read_or_default(&ci::run_now_file(&scratch));
        assert_eq!(left.at.keys().copied().collect::<Vec<_>>(), vec![2]);
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
