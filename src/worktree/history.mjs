// What each agent of an orchestration worked on (#264), for the statistics
// of reuse: does handing an idle agent the next issue (#217, workers.mjs)
// pay off, how often does it fail. In the record's `workers`, keyed by the
// agent's conversation id (stable across renames, #251):
//
//   history  [{ number, how, at, context }]   its issues in order, `how` it
//            got each (`fresh`, `handover`, `announced`), `context` the
//            share of its context used when it started on it (0 to 1)
//   failed   [{ number, reason, at, times }]  handovers to it that failed
//
// The MR and its merge come from the issue's entry and GitLab. An agent of
// before has only `issues`: its history is read from that (historyOf).
import { displayNamesOf } from "./display-names.mjs";
import { recordOf, writeWorker } from "./origins.mjs";

/** How an agent came to an issue: started for it, handed it, handed it as announced (#227). */
export const HOW = ["fresh", "handover", "announced"];

/**
 * The issues of a worker entry in order, `[{ number, how, at, context }]`:
 * its `history`, else, for one recorded before it, its `issues`, the first
 * started fresh and the others handed over, without times and contexts.
 */
export const historyOf = entry => {
  if (Array.isArray(entry?.history)) return entry.history;
  return (entry?.issues ?? []).map((number, i) => ({
    number,
    how: i === 0 ? "fresh" : "handover",
    at: null,
    context: null,
  }));
};
/** `entry` with the issue `start` (`{ number, how, at, context }`) as its latest, once. */
export const withStart = (entry, start) => {
  const history = historyOf(entry).filter(
    known => known.number !== start.number,
  );
  return {
    ...entry,
    history: [...history, start],
    issues: [...history.map(known => known.number), start.number],
  };
};
/**
 * `entry` with a failed handover of the issue `number`: the same failure
 * again (a retry of the overview, #244) counts its `times`, not a new one.
 */
export const withFailure = (entry, { number, reason, at }) => {
  const failed = Array.isArray(entry?.failed) ? entry.failed : [];
  const same = failed.find(
    known => known.number === number && known.reason === reason,
  );
  return {
    ...entry,
    failed: same
      ? failed.map(known =>
          known === same
            ? { ...known, at, times: (known.times ?? 1) + 1 }
            : known,
        )
      : [...failed, { number, reason, at, times: 1 }],
  };
};

/**
 * The record of `name` learns that the agent `id` started on the issue
 * `number`: `how` (HOW), `context` its share used then (null unknown).
 * `fields` go with it: the name a fresh one starts with. Its display name
 * (#292) is picked on its first start and kept on every one after; the
 * record's other workers without one keep the one they are shown by now,
 * so the new one takes none of theirs.
 */
export const recordStart = (
  name,
  id,
  { number, how, context },
  fields = {},
) => {
  const known = recordOf(name)?.workers ?? {};
  const entry = known[id];
  const names = displayNamesOf(known, [id]);
  for (const [other, worker] of Object.entries(known))
    if (other !== id && !worker?.displayName)
      writeWorker(name, other, { displayName: names.get(other) });
  writeWorker(name, id, {
    displayName: names.get(id),
    ...fields,
    ...withStart(entry, {
      number,
      how,
      at: new Date().toISOString(),
      context,
    }),
  });
};
/** The record of `name` learns that a handover of `number` to the agent `id` failed, and why. */
export const recordFailure = (name, id, number, reason) => {
  if (!recordOf(name)) return;
  const entry = recordOf(name).workers?.[id];
  const { failed } = withFailure(entry, {
    number,
    reason,
    at: new Date().toISOString(),
  });
  writeWorker(name, id, { failed });
};

/**
 * The agent that worked on the issue `number` among the record's workers
 * (`known`, by id): `{ id, history, index }`, `index` the place of the
 * issue in its history; null when no history has it.
 */
export const agentOfIssue = (known, number) => {
  for (const [id, entry] of Object.entries(known ?? {})) {
    const history = historyOf(entry);
    const index = history.findIndex(start => start.number === number);
    if (index !== -1) return { id, history, index };
  }
  return null;
};

/** `31%`: how much of its context an agent had used. */
const percent = share => `${Math.round(share * 100)}%`;

/**
 * What an agent's name line counts (#292): `issues`, how many it worked on,
 * the issue `number` its worktree has included, and `merged`, how many of
 * their MRs are merged (`isMerged(number)`). `entry`: its record's worker.
 */
export const countsOf = (entry, number, isMerged) => {
  const numbers = [
    ...new Set([...historyOf(entry).map(start => start.number), number]),
  ].filter(Boolean);
  return {
    issues: numbers.length,
    merged: numbers.filter(isMerged).length,
  };
};

/**
 * How reuse works in an orchestration: `agents`, how many worked on its
 * issues, `issues`, how many went to an agent, `reused`, how many of them
 * went to one that had worked on another before, `context`, the share of
 * its context such an agent started on, on average (null unknown), and
 * `failed`, the failed handovers. `issues` as issuesOf gives them (queued
 * and skipped ones went to no agent); an issue no history has went to a
 * fresh agent of its own (one from before the history, or started by hand).
 */
export const reuseOf = (issues, known = {}) => {
  const worked = issues.filter(issue => !issue.queued && !issue.why);
  const histories = Object.values(known)
    .map(historyOf)
    .filter(history => history.length);
  const covered = new Set(histories.flat().map(start => start.number));
  const later = histories.flatMap(history => history.slice(1));
  const contexts = later
    .map(start => start.context)
    .filter(context => typeof context === "number");
  return {
    agents:
      histories.length +
      worked.filter(issue => !covered.has(issue.number)).length,
    issues: worked.length,
    reused: later.length,
    context: contexts.length
      ? contexts.reduce((sum, context) => sum + context, 0) / contexts.length
      : null,
    failed: Object.values(known).reduce(
      (sum, entry) =>
        sum + (Array.isArray(entry?.failed) ? entry.failed.length : 0),
      0,
    ),
  };
};
/** The line of reuseOf: `agents 14 · issues 22 · reused 5, started at 34% · handovers failed 2`. */
export const reuseText = ({ agents, issues, reused, context, failed }) =>
  [
    `agents ${agents}`,
    `issues ${issues}`,
    `reused ${reused}${context === null ? "" : `, started at ${percent(context)}`}`,
    `handovers failed ${failed}`,
  ].join(" · ");
