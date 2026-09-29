// The merge order of an orchestration, computed on each read from what the
// overview knows anyway (#213): no model and no orchestrator turn keeps it.
// `wt:plan` is the record and the tie-breaker; the facts go before it:
// - dependencies: an issue that names another as a prerequisite comes after it;
// - file overlap: of two MRs touching the same files, the one further along
//   goes first, so the other merges develop in once;
// - readiness: a ready, green MR moves up past the ones ahead that are all
//   blocked (draft, failed pipeline, conflicts) and share no files with it.
import { blockersOf } from "./merge.mjs";

/** How an issue names a prerequisite: `after #180`, `needs #180 and #181`, `builds on #180`. */
const PREREQUISITE =
  /\b(?:after|needs|builds on|depends on|requires|blocked by)\s+(#\d+(?:(?:\s*,\s*|\s+and\s+|\s*&\s*|\s*\/\s*)#\d+)*)/gi;

/** The issues `text` (an issue's description) names as prerequisites, by number. */
export const needsOf = text => [
  ...new Set(
    [...String(text ?? "").matchAll(PREREQUISITE)].flatMap(([, list]) =>
      [...list.matchAll(/#(\d+)/g)].map(([, n]) => Number(n)),
    ),
  ),
];

/**
 * How far along an open MR is, lower is further: 0 ready and green, 1 ready
 * with its pipeline still to come, 2 blocked by a failed pipeline or
 * conflicts with develop, 3 a draft.
 */
export const progressOf = mr => {
  if (mr.isDraft) return 3;
  const status = mr.pipeline?.status ?? mr.pipeline;
  if (status === "failed" || mr.hasConflicts) return 2;
  return status === "success" ? 0 : 1;
};
const isReady = entry => progressOf(entry.mr) === 0;
const isBlocked = entry => progressOf(entry.mr) >= 2;

/**
 * What holds an open MR up, each in a few words; none when it is ready to
 * merge: what keeps the merge action's two clicks from merging it
 * (merge.mjs), named with the draft, how far behind develop a conflicting
 * branch is, and the MRs it comes after with why (`after !223 (shares
 * view.mjs)`). `entry` as mergeOrderOf takes it, `after` its direct
 * predecessors `{ iid, why }`.
 */
export const holdUpOf = (entry, after = []) => {
  const { mr, boxes = [], behind = 0 } = entry;
  const blockers = blockersOf(mr, boxes, { canMarkReady: true }).map(
    blocker => {
      if (blocker === "conflicts with develop" && behind)
        return `${blocker} (${behind} behind)`;
      // Of the open work boxes the first by name, the rest counted: one line.
      const [head, ...more] = blocker.split(", ");
      return /^\d+ work box/.test(blocker) && more.length
        ? `${head} +${more.length}`
        : blocker;
    },
  );
  return [
    ...(mr.isDraft ? ["draft"] : []),
    ...blockers,
    ...(after.length
      ? [`after ${after.map(pred => `!${pred.iid} (${pred.why})`).join(", ")}`]
      : []),
  ];
};

/** How many shared files a reason names before `+n`. */
const NAMED_FILES = 1;
/** `render-orchestration.mjs +2`: the base names of the first few files. */
const filesText = files =>
  `${files
    .slice(0, NAMED_FILES)
    .map(file => file.split("/").at(-1))
    .join(
      ", ",
    )}${files.length > NAMED_FILES ? ` +${files.length - NAMED_FILES}` : ""}`;

/**
 * The merge order. `entries`: the issues of the orchestration with an MR,
 * `{ number, mr, needs, files, boxes, behind }`, `mr` as the overview reads
 * it (`iid`, `state`, `isDraft`, `pipeline`, `hasConflicts`, `mergedAt`),
 * `needs` the issue numbers it names as prerequisites, `files` what its work
 * touches, `boxes` its issue's (boxesOf), `behind` how many commits of
 * develop its branch lacks.
 * `plan`: the merge order of `wt:plan`, issue numbers.
 * Returns
 * - `open`: the open MRs in the computed order, `{ number, reason, after,
 *   holdUp }`, the reason one short line, `after` the MRs it comes directly
 *   after (`{ number, iid, why }`, `why` the edge's phrase), `holdUp` what
 *   keeps it from merging (holdUpOf);
 * - `merged`: the merged ones in the order they were merged, then the closed;
 * - `numbers`: every issue in the order the view ranks them by: the open,
 *   what else the plan lists, the merged;
 * - `planned`: the order to write into the plan (merged, open, the rest);
 * - `isOffPlan`: whether the open ones leave the plan's order.
 */
export const mergeOrderOf = (entries, plan = []) => {
  const placeIn = number => {
    const at = plan?.indexOf(number) ?? -1;
    return at === -1 ? Infinity : at;
  };
  const byPlan = (a, b) =>
    placeIn(a.number) - placeIn(b.number) || a.number - b.number;
  const base = entries
    .filter(entry => entry.mr?.state === "opened")
    .toSorted(byPlan);
  const merged = entries
    .filter(entry => entry.mr?.state === "merged")
    .toSorted(
      (a, b) =>
        String(a.mr.mergedAt ?? "").localeCompare(
          String(b.mr.mergedAt ?? ""),
        ) || a.number - b.number,
    );
  const closed = entries
    .filter(entry => entry.mr && !["opened", "merged"].includes(entry.mr.state))
    .toSorted(byPlan);

  // number → its predecessors: [{ number, reason, why }].
  const before = new Map(base.map(entry => [entry.number, []]));
  const reaches = (from, to, seen = new Set()) => {
    if (from === to) return true;
    if (seen.has(from)) return false;
    seen.add(from);
    return [...before]
      .filter(([, preds]) => preds.some(pred => pred.number === from))
      .some(([next]) => reaches(next, to, seen));
  };
  const add = (first, second, reason, why) => {
    if (!reaches(second, first))
      before.get(second).push({ number: first, reason, why });
  };
  for (const entry of base)
    for (const need of entry.needs ?? [])
      if (before.has(need) && need !== entry.number)
        before.get(entry.number).push({
          number: need,
          reason: `after #${need} (needs it)`,
          why: `needs #${need}`,
        });
  for (const [i, a] of base.entries())
    for (const b of base.slice(i + 1)) {
      const theirs = new Set(b.files ?? []);
      const shared = [...(a.files ?? [])]
        .filter(file => theirs.has(file))
        .sort();
      if (!shared.length) continue;
      // Equally far along, the plan decides.
      const [first, second] =
        progressOf(b.mr) < progressOf(a.mr) ? [b, a] : [a, b];
      add(
        first.number,
        second.number,
        `after !${first.mr.iid}: shares ${filesText(shared)}`,
        `shares ${filesText(shared)}`,
      );
    }

  const open = [];
  const placed = new Set();
  let remaining = base;
  while (remaining.length) {
    const available = remaining.filter(entry =>
      before.get(entry.number).every(pred => placed.has(pred.number)),
    );
    let pick;
    let reason;
    if (!available.length) {
      pick = remaining[0];
      reason = "as planned: its prerequisites go in a circle";
    } else {
      pick = available[0];
      const readyAt = available.findIndex(isReady);
      if (readyAt > 0 && available.slice(0, readyAt).every(isBlocked)) {
        pick = available[readyAt];
        reason = `ready while ${available
          .slice(0, readyAt)
          .map(entry => `!${entry.mr.iid}`)
          .join(", ")} ${readyAt > 1 ? "are" : "is"} blocked`;
      }
    }
    if (!reason) {
      // The predecessor placed last is the one it waited for.
      const preds = before.get(pick.number);
      const last = open.findLast(item =>
        preds.some(pred => pred.number === item.number),
      );
      reason = last
        ? preds.find(pred => pred.number === last.number).reason
        : placeIn(pick.number) === Infinity
          ? "not in the plan: by number"
          : "as planned";
    }
    // The ones it comes directly after: not those another of them comes after.
    const preds = before.get(pick.number);
    const after = preds
      .filter(
        (pred, i) =>
          preds.findIndex(other => other.number === pred.number) === i &&
          !preds.some(
            other =>
              other.number !== pred.number &&
              reaches(pred.number, other.number),
          ),
      )
      .map(pred => ({
        number: pred.number,
        iid: base.find(entry => entry.number === pred.number).mr.iid,
        why: pred.why,
      }));
    open.push({
      number: pick.number,
      reason,
      after,
      holdUp: holdUpOf(pick, after),
    });
    placed.add(pick.number);
    remaining = remaining.filter(entry => entry !== pick);
  }

  const openNumbers = open.map(item => item.number);
  const listed = new Set(entries.map(entry => entry.number));
  const planRest = (plan ?? []).filter(number => !listed.has(number));
  const done = [...merged, ...closed].map(entry => entry.number);
  const planOpen = (plan ?? []).filter(number => before.has(number));
  return {
    open,
    merged: done,
    numbers: [...openNumbers, ...planRest, ...done],
    planned: [
      ...merged.map(entry => entry.number),
      ...openNumbers,
      ...planRest,
    ],
    isOffPlan: openNumbers.length > 0 && openNumbers.join() !== planOpen.join(),
  };
};

/**
 * The open MRs as a graph like `git log --graph`, top to bottom in the merge
 * order: MRs that don't depend on each other stand in lanes side by side,
 * one that comes after others below them, their lanes joined into its own.
 * `open` as mergeOrderOf returns it. Returns rows `{ number, graph, below }`
 * for an MR (its `●` in its lane; `below` the lanes on its further lines,
 * where only the lanes that go on show) and `{ graph }` for a line that
 * joins or forks lanes; a lane is two characters wide.
 */
export const graphOf = open => {
  const successors = new Map(open.map(item => [item.number, []]));
  for (const item of open)
    for (const pred of item.after ?? [])
      successors.get(pred.number)?.push(item.number);
  // Each lane: the MR it leads down to, null when it's free.
  const lanes = [];
  const rows = [];
  const trim = () => {
    while (lanes.length && lanes.at(-1) === null) lanes.pop();
  };
  const bar = lane => (lane === null ? "  " : "│ ");
  /** A line that ties the lanes `cols` to the first of them: `├─┘`, `├─┐`. */
  const tie = (cols, corner, middle) => {
    const [first, last] = [cols[0], cols.at(-1)];
    let graph = "";
    for (let col = 0; col < lanes.length; col++) {
      const isInside = col > first && col < last;
      if (col === first) graph += "├─";
      else if (col === last) graph += `${corner} `;
      else if (cols.includes(col)) graph += `${middle}─`;
      else if (isInside) graph += lanes[col] === null ? "──" : "┼─";
      else graph += bar(lanes[col]);
    }
    return { graph: graph.trimEnd() };
  };
  for (const item of open) {
    let cols = lanes.flatMap((lane, col) =>
      lane === item.number ? [col] : [],
    );
    if (!cols.length) {
      const free = lanes.indexOf(null);
      const col = free === -1 ? lanes.length : free;
      lanes[col] = item.number;
      cols = [col];
    }
    // Lanes that lead to it join into the leftmost.
    if (cols.length > 1) {
      rows.push(tie(cols, "┘", "┴"));
      for (const col of cols.slice(1)) lanes[col] = null;
    }
    const [col] = cols;
    const graph = lanes
      .map((lane, at) => (at === col ? "● " : bar(lane)))
      .join("")
      .trimEnd();
    const next = successors.get(item.number);
    lanes[col] = next[0] ?? null;
    rows.push({
      number: item.number,
      graph,
      below: lanes.map(bar).join("").trimEnd(),
    });
    // More MRs come after it: a lane forks off for each of the others.
    if (next.length > 1) {
      const forks = [col];
      for (const number of next.slice(1)) {
        const free = lanes.findIndex((lane, at) => lane === null && at > col);
        const at = free === -1 ? lanes.length : free;
        lanes[at] = number;
        forks.push(at);
      }
      forks.sort((a, b) => a - b);
      rows.push(tie(forks, "┐", "┬"));
    }
    trim();
  }
  return rows;
};
