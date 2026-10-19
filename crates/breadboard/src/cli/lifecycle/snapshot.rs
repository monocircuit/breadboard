//! `wt:snapshot [<feature>] | take <feature> | restore <feature> [<time>]`
//! (#395): the snapshots breadboard keeps of an agent's uncommitted work
//! before it ends or restarts its claude (core's `snapshot`), one taken by
//! hand, or one applied to its worktree again.

use super::args::{Args, Takes};
use super::proc::{Done, done, fail, step};
use super::repo::Repo;
use breadboard_core::snapshot::{self, Snapshot};
use breadboard_core::time::{ago, now_ms};

const USAGE: &str = "usage: pnpm wt:snapshot [<feature>] | take <feature> | restore <feature> [<time>]";
const TAKES: Takes = Takes {
    switches: &[],
    valued: &[],
    lists: &[],
    words: 3,
    about: "The snapshots of agents' uncommitted work (refs/breadboard/snapshots/<feature>/<time>, never pushed), kept before breadboard ends or restarts a claude: every one, or <feature>'s. take <feature> keeps one now; restore <feature> [<time>] applies its newest (or that one) to its worktree, untracked files included. Older than 14 days, or of a removed branch, they are pruned.",
    usage: USAGE,
};

pub fn command(args: &Args) -> Done {
    if args.check(&TAKES)? {
        return Ok(());
    }
    let repo = Repo::here()?;
    let now = now_ms();
    let words: Vec<&str> = args.words().iter().map(String::as_str).collect();
    match words.as_slice() {
        ["take", feature] => take(&repo, feature, now),
        ["restore", feature] => restore(&repo, feature, None),
        ["restore", feature, time] => restore(&repo, feature, Some(time)),
        [] => {
            print_list(&repo, None, now);
            Ok(())
        }
        [feature] if *feature != "take" && *feature != "restore" => {
            print_list(&repo, Some(feature), now);
            Ok(())
        }
        _ => fail(USAGE),
    }
}

fn print_list(repo: &Repo, feature: Option<&str>, now: i64) {
    snapshot::prune(&repo.main, now);
    let snapshots = snapshot::list(&repo.main, feature);
    if snapshots.is_empty() {
        println!(
            "no snapshots{}",
            feature.map(|f| format!(" of {f}")).unwrap_or_default()
        );
    }
    for line in lines_of(&snapshots, now) {
        println!("{line}");
    }
}

/// A line per snapshot: its feature, its time, how long ago.
pub fn lines_of(snapshots: &[Snapshot], now: i64) -> Vec<String> {
    let width = snapshots.iter().map(|s| s.feature.len()).max().unwrap_or(0);
    snapshots
        .iter()
        .map(|s| format!("{:<width$}  {}  {}", s.feature, s.time, ago(s.at, now)))
        .collect()
}

fn take(repo: &Repo, feature: &str, now: i64) -> Done {
    let dir = repo.dir_of(feature);
    if !dir.is_dir() {
        return fail(format!(
            "{feature} has no worktree ({}): pnpm wt:list names them",
            dir.display()
        ));
    }
    snapshot::prune(&repo.main, now);
    match snapshot::take(&dir, feature, now) {
        Ok(Some(taken)) => {
            done(&format!("kept as {}", taken.reference()));
            Ok(())
        }
        Ok(None) => {
            done(&format!(
                "nothing to keep: {feature} has no uncommitted work, or none new since its newest snapshot"
            ));
            Ok(())
        }
        Err(why) => fail(format!("not kept: {why}")),
    }
}

fn restore(repo: &Repo, feature: &str, time: Option<&str>) -> Done {
    let snapshots = snapshot::list(&repo.main, Some(feature));
    let chosen = match time {
        Some(time) => snapshots.iter().find(|s| s.time == time),
        None => snapshots.first(),
    };
    let Some(chosen) = chosen else {
        if snapshots.is_empty() {
            return fail(format!("{feature} has no snapshot"));
        }
        let known: Vec<&str> = snapshots.iter().map(|s| s.time.as_str()).collect();
        return fail(format!(
            "{feature} has no snapshot {}; it has {}",
            time.unwrap_or_default(),
            known.join(", ")
        ));
    };
    let dir = repo.dir_of(feature);
    if !dir.is_dir() {
        return fail(format!(
            "{feature} has no worktree ({}): make it again, or apply it elsewhere with git stash apply {}",
            dir.display(),
            chosen.reference()
        ));
    }
    step(&format!("git stash apply {}", chosen.reference()));
    snapshot::restore(&dir, chosen).map_err(|why| {
        super::proc::Fail(format!(
            "not restored: {why}\ncommit or move the files it names, then again"
        ))
    })?;
    done(&format!(
        "{feature}: its work of {} is back in {}",
        chosen.time,
        dir.display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_up_features_and_says_how_long_ago() {
        let at = 1_792_160_553_000;
        let of = |feature: &str| Snapshot {
            feature: feature.into(),
            time: "20261016T142233Z".into(),
            at,
            sha: "abc".into(),
        };
        assert_eq!(
            lines_of(&[of("7-x"), of("395-long")], at + 3 * 3_600_000),
            vec![
                "7-x       20261016T142233Z  3 h ago".to_string(),
                "395-long  20261016T142233Z  3 h ago".to_string()
            ]
        );
    }
}
