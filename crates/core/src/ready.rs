//! An MR marked ready by its bar (#399, the user's rule: ready is purely
//! whether every box is ticked). Its agent ended its run without
//! `glab mr update --ready` (!383): the MR sat as a draft with a full bar.
//! So `breadboard serve` decides it from what the overviews publish for the
//! CI queue (`ci::Published`, each MR's `Bar`): a draft whose boxes are all
//! ticked, no try or change holding it, is marked ready; one it marked ready
//! whose bar is no longer full goes back to draft. Once per head: a person
//! who sets it back to draft is respected until the next push. What it did
//! is kept in `<main>/.claude/ci/ready.json`, the rows say it from there.

use crate::ci::{Bar, OpenMr, Published, dir_of, read_or_default};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn ready_file(main: &Path) -> PathBuf {
    dir_of(main).join("ready.json")
}

/// The pid file of the daemon that marks `main`'s MRs: one per checkout.
pub fn marker_file(main: &Path) -> PathBuf {
    dir_of(main).join("ready.pid")
}

/// What breadboard did to an MR last, at which head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub sha: String,
    /// Marked ready (true), or put back to draft (false).
    pub is_ready: bool,
}

/// iid → its mark, of the open MRs.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Marks {
    pub mrs: BTreeMap<i64, Mark>,
}

impl Marks {
    pub fn read(main: &Path) -> Marks {
        read_or_default(&ready_file(main))
    }

    /// A person set it back to draft at this head after breadboard marked it
    /// ready: it stays a draft until the next push.
    pub fn is_drafted_by_hand(&self, iid: i64, is_draft: bool, sha: Option<&str>) -> bool {
        is_draft
            && self
                .mrs
                .get(&iid)
                .is_some_and(|mark| mark.is_ready && Some(mark.sha.as_str()) == sha)
    }
}

/// What to write to one MR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub iid: i64,
    /// The head it was decided at: a push since and nothing is written.
    pub sha: String,
    /// Ready (true) or back to draft (false).
    pub is_ready: bool,
    /// The orchestration whose order has it, for its journal.
    pub orchestration: String,
}

/// Whether a bar marks its MR ready: every box ticked, nothing holding it.
pub fn is_ready_bar(bar: &Bar) -> bool {
    bar.is_boxes_full() && bar.holds.is_none()
}

/// The steps for the open `mrs`, by the orchestrations' `published` bars
/// (name, published) and what was done before: an MR of no orchestration is
/// never touched, nor one without a head.
pub fn steps_of(mrs: &[OpenMr], published: &[(String, Published)], marks: &Marks) -> Vec<Step> {
    let mut steps = Vec::new();
    for mr in mrs {
        let Some(sha) = mr.sha.as_deref().filter(|sha| !sha.is_empty()) else {
            continue;
        };
        let Some((name, bar)) = published
            .iter()
            .find_map(|(name, order)| order.bars.get(&mr.iid).map(|bar| (name, bar)))
        else {
            continue;
        };
        let mark = marks.mrs.get(&mr.iid);
        let is_ready = if is_ready_bar(bar) {
            // Marked ready at this head and a draft again: a person's.
            let is_respected = mark.is_some_and(|mark| mark.is_ready && mark.sha == sha);
            if !mr.is_draft || is_respected {
                continue;
            }
            true
        } else {
            // Only one it marked ready itself goes back.
            if mr.is_draft || !mark.is_some_and(|mark| mark.is_ready) {
                continue;
            }
            false
        };
        steps.push(Step {
            iid: mr.iid,
            sha: sha.to_string(),
            is_ready,
            orchestration: name.clone(),
        });
    }
    steps
}

/// The journal's line for a step done.
pub fn journal_line(step: &Step) -> String {
    if step.is_ready {
        format!("!{} marked ready automatically (bar full)", step.iid)
    } else {
        format!("!{} back to draft automatically: its bar is no longer full", step.iid)
    }
}

/// The marks of MRs no longer open go.
pub fn forget_closed(marks: &mut Marks, mrs: &[OpenMr]) {
    marks.mrs.retain(|iid, _| mrs.iter().any(|mr| mr.iid == *iid));
}

/// Every orchestration's published order in `main`, with its name.
pub fn published_by_name(main: &Path) -> Vec<(String, Published)> {
    let Ok(entries) = std::fs::read_dir(dir_of(main)) else {
        return Vec::new();
    };
    let mut named: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.path();
            let name = file
                .file_name()?
                .to_str()?
                .strip_prefix("order-")?
                .strip_suffix(".json")?
                .to_string();
            Some((name, file))
        })
        .collect();
    named.sort();
    named
        .into_iter()
        .map(|(name, file)| (name, read_or_default(&file)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mr(iid: i64, sha: &str, is_draft: bool) -> OpenMr {
        OpenMr {
            iid,
            sha: Some(sha.into()),
            is_draft,
            ..Default::default()
        }
    }

    fn bar(ticked: usize, total: usize) -> Bar {
        Bar {
            ticked,
            total,
            ..Default::default()
        }
    }

    fn order(bars: &[(i64, Bar)]) -> Vec<(String, Published)> {
        vec![(
            "breadboard".into(),
            Published {
                bars: bars.iter().cloned().collect(),
                ..Default::default()
            },
        )]
    }

    fn marked(iid: i64, sha: &str, is_ready: bool) -> Marks {
        Marks {
            mrs: [(
                iid,
                Mark {
                    sha: sha.into(),
                    is_ready,
                },
            )]
            .into(),
        }
    }

    fn step(iid: i64, sha: &str, is_ready: bool) -> Step {
        Step {
            iid,
            sha: sha.into(),
            is_ready,
            orchestration: "breadboard".into(),
        }
    }

    #[test]
    fn a_draft_with_a_full_bar_is_marked_ready_whatever_its_check() {
        let published = order(&[(1, bar(3, 3)), (2, bar(2, 3)), (3, bar(0, 0))]);
        let mrs = [mr(1, "a", true), mr(2, "b", true), mr(3, "c", true), mr(4, "d", true)];
        // 2: a box open; 3: no boxes at all; 4: of no orchestration.
        assert_eq!(steps_of(&mrs, &published, &Marks::default()), [step(1, "a", true)]);
        assert_eq!(steps_of(&[mr(1, "a", false)], &published, &Marks::default()), []);
    }

    #[test]
    fn an_open_try_or_a_change_holds_the_draft() {
        let held = Bar {
            holds: Some("try #7 open".into()),
            ..bar(3, 3)
        };
        assert_eq!(
            steps_of(&[mr(1, "a", true)], &order(&[(1, held)]), &Marks::default()),
            []
        );
    }

    #[test]
    fn a_person_s_draft_is_respected_until_the_next_push() {
        let published = order(&[(1, bar(3, 3))]);
        let marks = marked(1, "a", true);
        assert_eq!(steps_of(&[mr(1, "a", true)], &published, &marks), []);
        assert!(marks.is_drafted_by_hand(1, true, Some("a")));
        assert_eq!(steps_of(&[mr(1, "b", true)], &published, &marks), [step(1, "b", true)]);
        assert!(!marks.is_drafted_by_hand(1, true, Some("b")));
    }

    #[test]
    fn a_box_unticked_puts_its_own_back_to_draft_and_a_tick_again_ready() {
        let open = order(&[(1, bar(2, 3))]);
        assert_eq!(
            steps_of(&[mr(1, "a", false)], &open, &marked(1, "a", true)),
            [step(1, "a", false)]
        );
        // Ready at an earlier head, pushed since: still its own.
        assert_eq!(
            steps_of(&[mr(1, "b", false)], &open, &marked(1, "a", true)),
            [step(1, "b", false)]
        );
        // Ready by a person: not breadboard's to undo.
        assert_eq!(steps_of(&[mr(1, "a", false)], &open, &Marks::default()), []);
        let full = order(&[(1, bar(3, 3))]);
        assert_eq!(
            steps_of(&[mr(1, "a", true)], &full, &marked(1, "a", false)),
            [step(1, "a", true)]
        );
    }

    #[test]
    fn reads_every_orchestration_s_order_with_its_name() {
        let main = std::env::temp_dir().join(format!("ready-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&main);
        let published = Published {
            bars: [(5, bar(1, 1))].into(),
            ..Default::default()
        };
        crate::ci::write_whole(&crate::ci::order_file(&main, "epic-a"), &published).unwrap();
        crate::ci::write_whole(&ready_file(&main), &marked(5, "x", true)).unwrap();
        assert_eq!(published_by_name(&main), [("epic-a".to_string(), published)]);
        let mut marks = Marks::read(&main);
        assert_eq!(marks, marked(5, "x", true));
        forget_closed(&mut marks, &[mr(6, "y", true)]);
        assert!(marks.mrs.is_empty());
        let _ = std::fs::remove_dir_all(&main);
    }
}
