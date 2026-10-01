// A merged worktree goes by itself (#227): the live overview of its
// orchestration runs wt:remove on its read once nothing is left in it (the
// wt:prune check) and its agent is idle; the agent goes free first (#289)
// and the removal waits until it left. What stops it is named on its node.
import { leftText } from "../worktree/conversations.mjs";

/** `1 commit`, `3 commits`. */
const counted = (count, word) => `${count} ${word}${count === 1 ? "" : "s"}`;

/**
 * Where the clean-up of a merged worktree stands: `{ key, why }`, `key`
 * - `kept`: changes or commits not in develop, wt:remove would refuse;
 * - `waits`: a handed-over or freed agent (#289) still stands in it
 *   (`tree.left`, #252),
 *   its agent is working or asks you, a try on the test instance
 *   is open (its actions go with the worktree), or `hold` (why a handover
 *   holds its agent, handovers.mjs / workers.mjs);
 * - `due`: nothing stops it.
 * Null for a worktree whose MR isn't merged. `issue.tree.ownCommits`: how
 * many commits develop doesn't have; `status`: herdr's state of its agent.
 */
export const cleanUpOf = (issue, { mr, status, hold } = {}) => {
  const { tree } = issue;
  if (!tree || mr?.state !== "merged") return null;
  const kept = [
    tree.changed ? `${tree.changed} changed` : "",
    tree.ownCommits
      ? `${counted(Number(tree.ownCommits), "commit")} not in develop`
      : "",
  ].filter(Boolean);
  if (kept.length) return { key: "kept", why: kept.join(", ") };
  if (tree.left?.length)
    return {
      key: "waits",
      why: `${leftText(tree.left)} still stands in it`,
    };
  if (status === "working")
    return { key: "waits", why: "its agent is working" };
  if (status === "blocked") return { key: "waits", why: "its agent asks you" };
  if (tree.actions?.some(action => action.on === "test" && !action.answer))
    return { key: "waits", why: "its try on the test instance is open" };
  if (hold) return { key: "waits", why: hold };
  return { key: "due", why: null };
};
