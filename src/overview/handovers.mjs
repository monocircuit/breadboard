// The announced handovers of an orchestration (#227, worktree/handovers.mjs)
// as the overview reads them: whose worktree they are on, what they still
// wait for, whether they are due. workers.mjs does a due one by itself.

/** The name an agent of the orchestration goes by: the record's (#217), else its worktree's. */
export const agentNameOf = (tree, known = {}) =>
  known[tree.claude?.id]?.name ?? tree.name;

/** `!232`, `!232, !233`. */
const mrsText = iids => iids.map(iid => `!${iid}`).join(", ");

/**
 * Each announced handover (`{ number, to, after }` of the record) with
 * where it stands: `tree`, the worktree its agent is in (null when no
 * worktree of the orchestration has an agent of that name), `open`, the
 * awaited MRs not merged yet (one GitLab didn't answer for counts as open),
 * `isDue`, all merged and the agent idle (herdr's `idle` or `done`).
 * `issues`: the orchestration's; `known`: the record's workers; `agents`:
 * agentsOf(); `awaited`: iid → `{ state }` from GitLab, null before it answered.
 */
export const announcedOf = (
  handovers,
  issues,
  { known = {}, agents = new Map(), awaited = null } = {},
) =>
  (handovers ?? []).map(handover => {
    const tree =
      issues
        .map(issue => issue.tree)
        .find(
          candidate =>
            candidate?.claude &&
            // Announced under its name before a handover renamed it (#251): the same agent.
            [
              agentNameOf(candidate, known),
              known[candidate.claude.id]?.was,
            ].includes(handover.to),
        ) ?? null;
    const open = handover.after.filter(
      iid => awaited?.get(iid)?.state !== "merged",
    );
    const status = tree ? agents.get(tree.claude.id)?.status : null;
    return {
      ...handover,
      tree,
      open,
      isDue:
        Boolean(awaited) &&
        open.length === 0 &&
        ["idle", "done"].includes(status),
    };
  });

/** What the node of the agent's worktree says about it: `→ next #226 after !232`, `→ next #226 now`. */
export const nextText = ({ number, open }) =>
  `→ next #${number} ${open.length ? `after ${mrsText(open)}` : "now"}`;
/** Why the clean-up of the agent's worktree waits: `its agent takes #226 after !232`. */
export const takesText = ({ number, open }) =>
  `its agent takes #${number}${open.length ? ` after ${mrsText(open)}` : " now"}`;
