//! Every worktree of an orchestration carries its number in WORKTREES (#227,
//! numbering.mjs), whether the orchestrator listed it in `wt:plan --work` or not.

use super::Entry;
use crate::gitlab::Mr;

/// When an issue's MR was merged: the record's time or GitLab's, its removal's
/// when neither knows; None for one not merged. `Some("")` sorts first.
fn merged_at_of(issue: &Entry, mr: Option<&Mr>) -> Option<String> {
    if let Some(at) = &issue.merged_at {
        return Some(at.clone());
    }
    if mr.is_some_and(Mr::is_merged) {
        return Some(mr.and_then(|mr| mr.merged_at.clone()).unwrap_or_default());
    }
    issue.removed_at.clone()
}

/// The number of each issue, 1 up, by its index in `issues`: first the work
/// order of `wt:plan`, then the open MRs in the computed merge order, then the
/// ones without an MR by issue number, then the merged and removed ones by
/// their merge time. Two entries may share an issue number (#221): each gets
/// its own. `mr_of(i)`: the MR of the issue at `i`.
pub fn numbers_of<'a>(
    issues: &[Entry],
    work: &[i64],
    merge: &[i64],
    mr_of: impl Fn(&Entry) -> Option<&'a Mr>,
) -> Vec<usize> {
    let rank = |order: &[i64], issue: &Entry| -> usize {
        issue
            .number
            .and_then(|number| order.iter().position(|known| *known == number))
            .unwrap_or(usize::MAX)
    };
    let by_number = |a: &usize, b: &usize| {
        issues[*a]
            .number
            .unwrap_or(i64::MAX)
            .cmp(&issues[*b].number.unwrap_or(i64::MAX))
    };
    let mut in_work: Vec<usize> = (0..issues.len())
        .filter(|i| rank(work, &issues[*i]) != usize::MAX)
        .collect();
    in_work.sort_by_key(|i| rank(work, &issues[*i]));
    let rest: Vec<usize> = (0..issues.len()).filter(|i| !in_work.contains(i)).collect();
    let is_merged = |i: &usize| merged_at_of(&issues[*i], mr_of(&issues[*i])).is_some();
    let mut done: Vec<usize> = rest.iter().copied().filter(is_merged).collect();
    let live: Vec<usize> = rest.iter().copied().filter(|i| !is_merged(i)).collect();
    let mut planned: Vec<usize> = live
        .iter()
        .copied()
        .filter(|i| rank(merge, &issues[*i]) != usize::MAX)
        .collect();
    planned.sort_by_key(|i| rank(merge, &issues[*i]));
    let mut unplanned: Vec<usize> = live
        .iter()
        .copied()
        .filter(|i| rank(merge, &issues[*i]) == usize::MAX)
        .collect();
    unplanned.sort_by(by_number);
    done.sort_by(|a, b| {
        let at = |i: &usize| merged_at_of(&issues[*i], mr_of(&issues[*i])).unwrap_or_default();
        at(a).cmp(&at(b)).then_with(|| by_number(a, b))
    });
    let mut numbers = vec![0; issues.len()];
    for (place, i) in in_work
        .into_iter()
        .chain(planned)
        .chain(unplanned)
        .chain(done)
        .enumerate()
    {
        numbers[i] = place + 1;
    }
    numbers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: i64) -> Entry {
        Entry {
            number: Some(number),
            ..Default::default()
        }
    }

    #[test]
    fn numbers_the_work_order_then_the_merge_order_then_by_number_then_the_merged() {
        let merged = Mr {
            state: "merged".into(),
            merged_at: Some("2026-09-30T10:00:00Z".into()),
            ..Default::default()
        };
        let issues = vec![
            issue(5),
            Entry {
                merged_at: Some("2026-09-29T10:00:00Z".into()),
                ..issue(1)
            },
            issue(9),
            issue(3),
            issue(7),
            issue(2),
            Entry {
                number: None,
                ..issue(0)
            },
        ];
        let numbers = numbers_of(&issues, &[7], &[9, 5], |entry| {
            (entry.number == Some(2)).then_some(&merged)
        });
        // 7 works first; 9, 5 by the merge order; 3 and the one without a number by number; then 1, 2 by merge time.
        assert_eq!(numbers, [3, 6, 2, 4, 1, 7, 5]);
    }

    #[test]
    fn gives_two_entries_of_one_issue_each_its_own() {
        assert_eq!(numbers_of(&[issue(4), issue(4)], &[], &[], |_| None), [1, 2]);
    }
}
