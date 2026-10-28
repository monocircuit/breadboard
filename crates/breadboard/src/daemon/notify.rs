//! The desktop notifications of a repo (#449, core's `notify`), run by the
//! daemon beside the CI queue while any overview of that repo is open:
//! every EVERY it reads GitLab's newest issues and MRs and the repo's open
//! `decide` actions, and each event since the last read is one
//! `notify-send`. One daemon per checkout sends (its pid file), what it saw
//! is kept, so a restart repeats nothing. Switched off, nothing is read and
//! what was seen is forgotten: switched on again, it starts from what is
//! there. Only for a repo GitLab serves: a gauntlet world opts in with its
//! push URL, as for the marks.

use super::ci::every;
use breadboard_core::ci;
use breadboard_core::gitlab::client::Gitlab;
use breadboard_core::gitlab::read::{NOTIFY_QUERY, notify_of};
use breadboard_core::notify;
use serde_json::json;
use std::path::{Path, PathBuf};

/// The notifications of `main`, as long as the daemon runs.
pub async fn notify(main: PathBuf) {
    let gitlab = Gitlab::new(&main);
    let mut tick = tokio::time::interval(every());
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if !main.is_dir() {
            return;
        }
        let events = {
            let main = main.clone();
            tokio::task::spawn_blocking(move || {
                let events = breadboard_core::settings::of_product(&main, None).notify;
                if events.is_empty() {
                    // Off: switched on again, it learns first instead of sending all that came meanwhile.
                    let _ = std::fs::remove_file(notify::seen_file(&main));
                }
                let is_on = !events.is_empty() && ci::is_served(&main) && ci::holds(&main, &notify::marker_file(&main));
                is_on.then_some(events)
            })
            .await
            .ok()
            .flatten()
        };
        let Some(events) = events else { continue };
        let Ok(project) = gitlab.graphql(NOTIFY_QUERY, json!({})).await else {
            continue;
        };
        let main = main.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut now = notify_of(&project);
            now.decides = notify::decides_of(&main);
            round(&main, &now, &events, notify::program().as_deref());
        })
        .await;
    }
}

/// One read's notifications sent, what it saw kept: those of a switched-off
/// event, and all without `notify-send`, are passed over, never kept for later.
fn round(main: &Path, now: &notify::Now, events: &[&str], program: Option<&Path>) -> usize {
    let seen = notify::read_seen(main);
    let (notes, next) = notify::events_of(seen.as_ref(), now);
    let mut sent = 0;
    if let Some(program) = program {
        for note in notes.iter().filter(|note| events.contains(&note.event)) {
            if notify::send(program, note) {
                sent += 1;
            }
        }
    }
    if seen.as_ref() != Some(&next) {
        let _ = ci::write_whole(&notify::seen_file(main), &next);
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::notify::{MR_MERGED, MR_OPENED, MergedMr, Now, OpenMr};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn sends_each_switched_on_event_once_and_keeps_what_it_saw() {
        let scratch = std::env::temp_dir().join(format!("notify-round-test-{}", std::process::id()));
        let main = scratch.join("platform");
        std::fs::create_dir_all(&main).unwrap();
        let log = scratch.join("sent.log");
        let program = scratch.join("notify-send");
        std::fs::write(
            &program,
            format!("#!/bin/sh\nprintf '%s|' \"$@\" >> '{}'\necho >> '{0}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mr = |iid: i64| OpenMr {
            iid,
            title: "feat: x".into(),
            target: "develop".into(),
            ..Default::default()
        };
        let all = [MR_OPENED, MR_MERGED];
        // The first read learns, sends nothing.
        let before = Now {
            open: vec![mr(1)],
            ..Default::default()
        };
        assert_eq!(round(&main, &before, &all, Some(&program)), 0);
        assert!(notify::read_seen(&main).is_some());
        let now = Now {
            open: vec![mr(2)],
            merged: vec![MergedMr {
                iid: 1,
                title: "feat: x".into(),
                target: "develop".into(),
            }],
            ..Default::default()
        };
        // Only the merge is switched on: the opened one is passed over, and stays so.
        assert_eq!(round(&main, &now, &[MR_MERGED], Some(&program)), 1);
        assert_eq!(round(&main, &now, &all, Some(&program)), 0, "nothing again");
        let sent = std::fs::read_to_string(&log).unwrap();
        assert_eq!(sent, "--app-name=breadboard|!1 feat: x|merged into develop|\n");
        // No notify-send: none sent, still kept.
        let later = Now {
            open: vec![mr(2), mr(3)],
            ..now
        };
        assert_eq!(round(&main, &later, &all, None), 0);
        assert_eq!(notify::read_seen(&main).unwrap().mr, 3);
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
