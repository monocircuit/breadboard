// NEXT, the band at the top of an orchestration's overview (#278): only what
// the user can do now, with one click there; else one dim line that nothing
// is for them. And when the plan of `wt:plan` went stale, so that its `why`
// never stands under NEXT naming issues long done.

/**
 * Whether a row of UP NEXT does something on a click: not one already under
 * way (`handing over …`), which shows only where it stands.
 */
export const isClickable = item =>
  typeof item.go === "function" && !item.action.isRunning;

/**
 * What NEXT shows, in this order: the first action not answered yet (an
 * answered one waits for its agent) and, a try, ready (`isWaiting` says
 * which isn't); else the first clickable row of UP NEXT; else the merge
 * that is ready (`merge.isReady`: pipeline passed, work boxes ticked, its
 * turn). `{ kind: "action" | "up next", item }`, `{ kind: "merge", merge }`,
 * or null: nothing for the user.
 */
export const nextOf = ({ actions, upNext, merge, isWaiting = () => false }) => {
  const action = actions.find(item => !item.action.answer && !isWaiting(item));
  if (action) return { kind: "action", item: action };
  const follow = upNext.find(isClickable);
  if (follow) return { kind: "up next", item: follow };
  if (merge?.isReady) return { kind: "merge", merge };
  return null;
};

/**
 * What an MR next in the merge order waits for, after `once`: the first of
 * its hold-ups (merge-order.mjs `holdUpOf`) the user can't do anything about.
 */
const ONCE = [
  [/^conflicts/, "its conflicts are resolved"],
  [/^pipeline failed/, "its pipeline is fixed"],
  [/^pipeline/, "its pipeline passes"],
  [/^after (!\d+)/, (_, iid) => `${iid} is merged`],
  [/work box/, "its work boxes are ticked"],
  [/^draft/, "its agent posts the merge"],
];
export const onceText = holdUp => {
  for (const [pattern, text] of ONCE) {
    const hold = holdUp.find(candidate => pattern.test(candidate));
    if (hold)
      return typeof text === "string"
        ? text
        : hold.replace(new RegExp(`${pattern.source}.*`), text);
  }
  return holdUp[0] ?? "";
};

/**
 * The one line NEXT shows when nothing is for the user: how many agents are
 * at work and the MR next in the merge order with what it waits for
 * (`{ iid, holdUp }`), e.g. `nothing for you · 2 agents at work · next
 * merge: !286 once its pipeline passes`.
 */
export const nothingText = ({ atWork = 0, merge = null } = {}) =>
  [
    "nothing for you",
    atWork ? `${atWork} agent${atWork === 1 ? "" : "s"} at work` : "",
    merge
      ? `next merge: !${merge.iid}${merge.holdUp?.length ? ` once ${onceText(merge.holdUp)}` : ""}`
      : "",
  ]
    .filter(Boolean)
    .join(" · ");

/** The issue numbers a text names (`#256`), each once. */
export const namedIn = text => [
  ...new Set(
    [...String(text ?? "").matchAll(/#(\d+)\b/g)].map(([, n]) => Number(n)),
  ),
];

/**
 * Why the plan (`{ work, merge, why }`) is stale, one short line each; none
 * while it holds. `states`: issue number → `open`, `merged` or `closed`
 * (closed without a merge), for the issues of the orchestration; one it
 * doesn't know was removed from it. `isOffPlan`: the computed merge order
 * left the plan's (merge-order.mjs).
 * - an issue of the plan closed without a merge, or no longer in the
 *   orchestration (a merged one stays: the plan keeps the merged first);
 * - its `why` names an issue that isn't open or isn't in the plan;
 * - the merge order left the plan's.
 */
export const staleOf = ({ plan, states, isOffPlan = false }) => {
  if (!plan) return [];
  const listed = [...new Set([...(plan.work ?? []), ...(plan.merge ?? [])])];
  const gone = listed.filter(number => !states.has(number));
  const closed = listed.filter(number => states.get(number) === "closed");
  const inPlan = new Set(listed);
  const done = namedIn(plan.why).filter(
    number => !inPlan.has(number) || states.get(number) !== "open",
  );
  const issuesText = numbers => numbers.map(number => `#${number}`).join(", ");
  return [
    closed.length ? `${issuesText(closed)} closed` : "",
    gone.length ? `${issuesText(gone)} no longer in the orchestration` : "",
    done.length ? `its why names ${issuesText(done)}, done or gone` : "",
    isOffPlan ? "the merge order left the plan's" : "",
  ].filter(Boolean);
};

/**
 * The plan's `why` when it belongs to what NEXT shows (`next` as nextOf
 * returns it): a merge, the order of which it explains, and only while the
 * plan isn't stale (`stale` as staleOf returns it); else null.
 */
export const whyOf = (plan, stale, next) => {
  if (!plan?.why || !plan.merge?.length || stale.length) return null;
  const isMerge =
    next?.kind === "merge" ||
    (next?.kind === "action" && next.item.action.kind === "merge");
  return isMerge ? plan.why : null;
};

/**
 * The plan to write when a click updates a stale one: the computed merge
 * order (`planned`, merge-order.mjs) and the work order without the issues
 * closed or gone; its why only while every issue it names is open.
 */
export const updatedPlan = ({ plan, states, planned }) => {
  const keeps = number => {
    const state = states.get(number);
    return state === "open" || state === "merged";
  };
  return {
    work: (plan?.work ?? []).filter(keeps),
    merge: planned.filter(keeps),
    why: namedIn(plan?.why).every(number => states.get(number) === "open")
      ? (plan?.why ?? "")
      : "",
  };
};
