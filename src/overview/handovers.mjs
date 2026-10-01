// The announced handovers of an orchestration (#227, worktree/handovers.mjs)
// as the overview reads them: whose worktree they are on, what they still
// wait for, whether they are due. workers.mjs does a due one by itself.
import { displayNamesOf } from "../worktree/display-names.mjs";
import { refusalOf } from "../worktree/workers.mjs";

/** The name an agent of the orchestration goes by: the record's (#217), else its worktree's. */
export const agentNameOf = (tree, known = {}) =>
  known[tree.claude?.id]?.name ?? tree.name;
/**
 * The names the overview shows its agents by (#292): `byId`, conversation
 * id → its display name (worktree/display-names.mjs), of the record's
 * workers and of the agents of `issues`' worktrees; `shown(who)`, the
 * display name of the agent the name `who` goes by (its record's, its
 * worktree's feature, its name before its last handover), else `who`.
 */
export const agentNamesOf = (issues, known = {}) => {
  const trees = issues.map(issue => issue.tree).filter(tree => tree?.claude);
  const byId = displayNamesOf(
    known,
    trees.map(tree => tree.claude.id),
  );
  const byName = new Map();
  for (const [id, entry] of Object.entries(known))
    for (const name of [entry?.was, entry?.name])
      if (name) byName.set(name, byId.get(id));
  for (const tree of trees) {
    byName.set(tree.name, byId.get(tree.claude.id));
    byName.set(agentNameOf(tree, known), byId.get(tree.claude.id));
  }
  return { byId, shown: who => byName.get(who) ?? who };
};

/** `!232`, `!232, !233`. */
const mrsText = iids => iids.map(iid => `!${iid}`).join(", ");

/**
 * Each announced handover (`{ number, to, after }` of the record) with
 * where it stands: `tree`, the worktree its agent is in (null when no
 * worktree of the orchestration has an agent of that name), `open`, the
 * awaited MRs not merged yet (one GitLab didn't answer for counts as open),
 * `isDue`, all merged and the agent idle (herdr's `idle` or `done`), else
 * `why` not, once the MRs are merged (#244): no agent of that name, it works.
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
    const agent = tree && agents.get(tree.claude.id);
    const why = tree
      ? refusalOf({
          pane: agent && { agent_status: agent.status },
          share: null,
        })
      : `no worktree of the orchestration has an agent ${handover.to}`;
    return {
      ...handover,
      tree,
      open,
      isDue: Boolean(awaited) && open.length === 0 && !why,
      why: awaited && open.length === 0 ? why : null,
    };
  });

/** What the node of the agent's worktree says about it: `→ next #226 after !232`, `→ next #226 now`. */
export const nextText = ({ number, open }) =>
  `→ next #${number} ${open.length ? `after ${mrsText(open)}` : "now"}`;
/** Why the clean-up of the agent's worktree waits: `its agent takes #226 after !232`. */
export const takesText = ({ number, open }) =>
  `its agent takes #${number}${open.length ? ` after ${mrsText(open)}` : " now"}`;
