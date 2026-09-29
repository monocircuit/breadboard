// Which repo an agent belongs to, for its card in herdr's agents panel
// ($wt_repo): a worktree's agent shows `Monocircuit/Platform` where its space
// name would only repeat the tab's. The row that shows it: worktree/sidebar.mjs.
import { SPACE } from "../worktree.mjs";

/**
 * pane → the repo of its agent: the label of the space of the repo's own
 * checkout, `Monocircuit/Platform`. Only for agents in a linked worktree's
 * space of such a repo: one in the repo's own space shows it as its space
 * already, one outside the repo gets nothing. `workspaces` as herdr lists
 * them (`label`, `worktree.repo_root`, `worktree.is_linked_worktree`),
 * `agents` its agents (`pane_id`, `workspace_id`).
 */
export const repoOfAgents = (workspaces, agents) => {
  const byId = new Map(workspaces.map(space => [space.workspace_id, space]));
  const repos = new Map(
    workspaces
      .filter(space => space.worktree && !space.worktree.is_linked_worktree)
      .map(space => [space.worktree.repo_root, space.label]),
  );
  const found = new Map();
  for (const agent of agents) {
    const worktree = byId.get(agent.workspace_id)?.worktree;
    if (!worktree?.is_linked_worktree) continue;
    const repo = repos.get(worktree.repo_root);
    if (repo?.startsWith(SPACE)) found.set(agent.pane_id, repo);
  }
  return found;
};
