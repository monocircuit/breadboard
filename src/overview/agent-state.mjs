// The tokens of the agents in herdr's sidebar: what a worktree's agent waits
// for ($wt_state), and the repo it belongs to ($wt_repo; agent-repo.mjs). The overview of the main space (--herdr)
// reports them on every read; the rows that show them are written by pnpm
// herdr (worktree/sidebar.mjs).
import { herdr, isInHerdr, spaces } from "../lib/herdr.mjs";
import { REPO_TOKEN, STATE_TOKEN, mainDir, spaceLabel } from "../worktree.mjs";
import { TOKEN_SOURCE } from "../worktree/origins.mjs";
import { repoOfAgents } from "./agent-repo.mjs";

/** Tokens of an earlier layout (#167, the group head and tree marks): cleared, nothing shows them any more. */
const GONE = { wt_head: null, wt_tree: null };

/**
 * The line for an agent in a worktree, or null for none: what wants you
 * first. `agentStatus` is herdr's (`blocked` asks you something), `mr` the
 * open merge request of the worktree's branch, if any.
 */
export const stateOf = ({ agentStatus, mr }) => {
  const ref = mr ? ` · !${mr.iid}` : "";
  if (agentStatus === "blocked") return `needs you${ref}`;
  if (mr?.pipeline?.status === "failed") return `pipeline failed${ref}`;
  if (agentStatus === "working") return `working${ref}`;
  if (mr) return `${mr.isDraft ? "draft" : "review"}${ref}`;
  return null;
};

/** pane → token → the value it shows, as reported last: a token is reported again only when it changes. */
const reported = new Map();
/**
 * The panes this overview has reported to. One it hasn't gets every token,
 * clears too: an overview started again doesn't know what the one before it
 * left there.
 */
const known = new Set();

/** The tokens of one pane, `{ token: value or null }`: set what changed, clear what went. */
const report = (pane, tokens) => {
  const before = reported.get(pane) ?? {};
  const isNew = !known.has(pane);
  known.add(pane);
  const args = [];
  for (const [token, value] of Object.entries(tokens)) {
    if (!isNew && (before[token] ?? null) === value) continue;
    args.push(
      ...(value ? ["--token", `${token}=${value}`] : ["--clear-token", token]),
    );
  }
  // An answer of null is fine too (report-metadata answers with an empty body).
  if (args.length)
    herdr(["pane", "report-metadata", pane, "--source", TOKEN_SOURCE, ...args]);
  if (Object.values(tokens).some(Boolean)) reported.set(pane, tokens);
  else reported.delete(pane);
};

/**
 * Reports the tokens of every agent: the line of an agent in a worktree's
 * space, from the local read (`local.trees`) and GitLab's (`remote.mrs`, null
 * before the first answer: then the merge request part waits for it), and
 * its repo. The main checkout's agents get neither, their space names the
 * repo and they work on no branch of their own; agents outside the repo get
 * nothing.
 */
export const reportStates = (local, remote) => {
  if (!isInHerdr()) return;
  const workspaces = spaces();
  const agents = (herdr(["pane", "list"])?.panes ?? []).filter(
    pane => pane.agent,
  );
  const spaceOf = new Map(
    workspaces.map(space => [space.label, space.workspace_id]),
  );
  const main = mainDir();
  const lines = new Map();
  for (const tree of local.trees) {
    if (tree.dir === main) continue;
    const workspace = spaceOf.get(spaceLabel(tree.dir));
    if (!workspace) continue;
    const mr =
      remote?.mrs.find(candidate => candidate.branch === tree.branch) ?? null;
    for (const pane of agents) {
      if (pane.workspace_id === workspace) {
        lines.set(
          pane.pane_id,
          stateOf({ agentStatus: pane.agent_status, mr }),
        );
      }
    }
  }
  const repos = repoOfAgents(workspaces, agents);
  const panes = new Set([
    ...agents.map(pane => pane.pane_id),
    ...reported.keys(),
  ]);
  for (const pane of panes) {
    report(pane, {
      [STATE_TOKEN]: lines.get(pane) ?? null,
      [REPO_TOKEN]: repos.get(pane) ?? null,
      ...GONE,
    });
  }
  // A pane that is gone is forgotten: herdr may give its id to a new one.
  for (const pane of known) if (!panes.has(pane)) known.delete(pane);
};
