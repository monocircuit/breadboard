// Which of an orchestration's actions need the user and in which order: the
// overview splits them into ACTIONS and UP NEXT (render-orchestration.mjs).

/** Where `number` stands in `order` (from 0), after every listed one when it isn't. */
export const placeIn = (order, number) => {
  const at = order?.indexOf(number) ?? -1;
  return at === -1 ? Infinity : at;
};

/**
 * The automatic follow-ups: they need a click, not a thought, so they wait
 * under UP NEXT instead of between the user's decisions. `order` writes the
 * computed merge order into the plan (merge-order.mjs).
 */
export const FOLLOW_UPS = new Set([
  "clean up",
  "pipeline",
  "end",
  "hand over",
  "start",
  "order",
]);
/**
 * The kinds of ACTIONS by what blocks most: an agent that stands still
 * (`waits`, `decide`, `plan`), work only a worktree has (`push`), then the
 * merges, then what only looks.
 */
const BLOCKS = [
  "waits",
  "decide",
  "plan",
  "push",
  "merge",
  "try",
  "browser-check",
  "look",
];
const blockOf = kind => {
  const at = BLOCKS.indexOf(kind);
  return at === -1 ? BLOCKS.length : at;
};

/**
 * `items` (`{ action: { kind, at }, rank }`, `rank` the place of its issue
 * in the merge order, the orchestrator's own before all) split into
 * `{ actions, upNext }`: ACTIONS sorted by what blocks most, then by rank,
 * then by age; UP NEXT, the follow-ups, by rank, then by age.
 */
export const orderActions = items => {
  const byRank = (a, b) =>
    a.rank - b.rank || a.action.at.localeCompare(b.action.at);
  return {
    actions: items
      .filter(item => !FOLLOW_UPS.has(item.action.kind))
      .toSorted(
        (a, b) =>
          blockOf(a.action.kind) - blockOf(b.action.kind) || byRank(a, b),
      ),
    upNext: items
      .filter(item => FOLLOW_UPS.has(item.action.kind))
      .toSorted(byRank),
  };
};
