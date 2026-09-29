// Every worktree of an orchestration carries its number in WORKTREES (#227),
// whether the orchestrator listed it in `wt:plan --work` or not.

/**
 * When an issue's MR was merged: the record's time (wt:remove) or GitLab's,
 * its removal's when neither knows; null for one not merged.
 */
const mergedAtOf = (issue, mr) =>
  issue.mergedAt ??
  (mr?.state === "merged" ? (mr.mergedAt ?? "") : null) ??
  issue.removedAt ??
  null;

/**
 * The number of each issue of `issues`, 1 up: first the work order of
 * `wt:plan` (`work`), then the open MRs in the computed merge order
 * (`merge`, merge-order.mjs), then the ones without an MR by issue number,
 * then the merged and removed ones by their merge time. `of(issue)`: its
 * part of GitLab. A map issue → its number: two entries may share an issue
 * number (#221), each gets its own.
 */
export const numbersOf = (
  issues,
  { work = [], merge = [], of = () => ({}) },
) => {
  const merged = issue => mergedAtOf(issue, of(issue).mr) !== null;
  const byNumber = (a, b) => (a.number ?? Infinity) - (b.number ?? Infinity);
  const rank = (order, issue) => {
    const at = order.indexOf(issue.number);
    return at === -1 ? Infinity : at;
  };
  const inWork = issues
    .filter(issue => rank(work, issue) !== Infinity)
    .toSorted((a, b) => rank(work, a) - rank(work, b));
  const rest = issues.filter(issue => !inWork.includes(issue));
  const done = rest.filter(merged);
  const live = rest.filter(issue => !merged(issue));
  const ordered = [
    ...inWork,
    ...live
      .filter(issue => rank(merge, issue) !== Infinity)
      .toSorted((a, b) => rank(merge, a) - rank(merge, b)),
    ...live.filter(issue => rank(merge, issue) === Infinity).toSorted(byNumber),
    ...done.toSorted(
      (a, b) =>
        String(mergedAtOf(a, of(a).mr)).localeCompare(
          String(mergedAtOf(b, of(b).mr)),
        ) || byNumber(a, b),
    ),
  ];
  return new Map(ordered.map((issue, i) => [issue, i + 1]));
};
