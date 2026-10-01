// What an agent waits for (#288): one state per agent, the same words in
// WORKTREES of an orchestration (tree.mjs) and herdr's sidebar ($wt_state),
// never a bare `idle`. And the sidebar's tokens: the state, the repo an agent
// belongs to ($wt_repo; agent-repo.mjs). The overview of the main space
// (--herdr) reports them on every read; the rows that show them are written
// by pnpm herdr (worktree/sidebar.mjs).
import { herdr, isInHerdr, spaces } from "../lib/herdr.mjs";
import {
  REPO_TOKEN,
  STATE_TOKEN,
  actionsFile,
  actionsOf,
  changeOf,
  handoversOf,
  isChangeOpen,
  mainDir,
  originOf,
  pushStateOf,
  recordOf,
  spaceLabel,
  waitOf,
  waitsText,
} from "../worktree.mjs";
import { TOKEN_SOURCE } from "../worktree/origins.mjs";
import { repoOfAgents } from "./agent-repo.mjs";
import { questionOf } from "./auto-actions.mjs";
import { readClaude } from "./claude.mjs";
import { nextText } from "./handovers.mjs";
import { checksOf } from "./merge.mjs";

/** Tokens of an earlier layout (#167, the group head and tree marks): cleared, nothing shows them any more. */
const GONE = { wt_head: null, wt_tree: null };

/** GitLab's statuses of a pipeline that hasn't finished yet. */
const RUNNING = new Set([
  "created",
  "waiting_for_resource",
  "preparing",
  "pending",
  "running",
  "scheduled",
]);
/** The kinds of action that wait for the user under another name: `browser-check` is a `try`. */
const KIND_NAMES = { "browser-check": "try" };

/**
 * What an agent waits for, first match wins: `{ key, text, tone }`. `tone`
 * is whom it waits for, each in a colour of its own (herdr-tones.mjs
 * `paintOf`, worktree/sidebar.mjs): `work`, `gitlab` (the pipeline, another
 * MR), `you`, `free`.
 * - `working` `at work`; `asks` `asks you`: blocked (herdr), or it stopped
 *   on a question
 * - `pipeline` `waits for the pipeline`: its MR's pipeline runs, or GitLab
 *   has none yet for the commit it pushed; `failed` `pipeline failed`
 * - `waits` `waits for you: merge, try #4, decide`: its open actions, or a
 *   merge when its MR's pipeline passed, no requested change is open and
 *   no check (#280); `checks` `checks open: try #2 broken, 1 acceptance
 *   box open: …` when only those hold the merge
 * - `waitsForMr` `waits for !271` (wt:wait); `next` `→ next #226 after
 *   !232` (a handover announced for it, handovers.mjs `announcedOf`)
 * - `change` `change requested`; `unpushed` `not pushed: 2 commits`
 * - `free`: none of these, the one case with nothing to do
 * `tree` as the overview reads a worktree (`claude`, `actions`, `wait`,
 * `change` and pushStateOf's fields), `mr` its MR (`state`, `pipeline` with
 * `status` and `sha`), `status` herdr's state of its agent, `merged` the iids
 * known merged (a wait names only the others), `handover` the announced one,
 * `checks` what the human's checks still lack (merge.mjs `checksOf`).
 */
export const agentStateOf = (
  tree,
  {
    mr = null,
    status = null,
    merged = new Set(),
    handover = null,
    checks = [],
  } = {},
) => {
  const state = (key, text, tone) => ({ key, text, tone });
  if (status === "working") return state("working", "at work", "work");
  if (status === "blocked" || (tree.claude && questionOf(tree.claude.answer)))
    return state("asks", "asks you", "you");
  const isOpen = mr?.state === "opened";
  const pipeline = mr?.pipeline?.status ?? mr?.pipeline ?? null;
  const sha = mr?.pipeline?.sha;
  // Pushed, but GitLab's pipeline is still the one of a commit before.
  const isBehind = Boolean(sha && tree.pushed) && sha !== tree.pushed;
  if (isOpen && (RUNNING.has(pipeline) || isBehind))
    return state("pipeline", "waits for the pipeline", "gitlab");
  if (isOpen && pipeline === "failed")
    return state("failed", "pipeline failed", "you");
  const kinds = (tree.actions ?? [])
    .filter(action => !action.answer)
    .map(action => {
      const kind = KIND_NAMES[action.kind] ?? action.kind;
      return kind === "try" ? `try #${action.id}` : kind;
    });
  const isGreen = isOpen && pipeline === "success" && !isChangeOpen(tree, mr);
  if (isGreen && !checks.length) kinds.push("merge");
  if (kinds.length)
    return state(
      "waits",
      `waits for you: ${[...new Set(kinds)].join(", ")}`,
      "you",
    );
  if (isGreen)
    return state("checks", `checks open: ${checks.join(", ")}`, "you");
  if (tree.wait)
    return state("waitsForMr", waitsText(tree.wait, merged), "gitlab");
  if (handover) return state("next", nextText(handover), "gitlab");
  if (isChangeOpen(tree, mr)) return state("change", "change requested", "you");
  if (tree.unpushed > 0)
    return state(
      "unpushed",
      `not pushed: ${tree.unpushed} commit${tree.unpushed === 1 ? "" : "s"}`,
      "you",
    );
  return state("free", "free", "free");
};

/**
 * The line of an agent in the sidebar: its state (agentStateOf), its MR
 * behind it (`waits for you: merge · !292`).
 */
export const sidebarLineOf = (tree, facts) => {
  const ref = facts.mr ? ` · !${facts.mr.iid}` : "";
  return `${agentStateOf(tree, facts).text}${ref}`;
};

/**
 * The handover announced for the agent of a worktree, from the record of
 * the orchestration that started it (`{ number, after, open }`), null
 * without one. `openMrs`: the iids GitLab has open, an awaited one not among
 * them is over; null before GitLab answered, every awaited one open then.
 */
const handoverOf = (tree, openMrs) => {
  const origin = originOf(tree.dir);
  const record = origin && recordOf(origin.name);
  if (!record) return null;
  const names = [
    tree.name,
    record.workers?.[tree.claude?.id]?.name,
    record.workers?.[tree.claude?.id]?.was,
  ].filter(Boolean);
  const handover = handoversOf(record).find(({ to }) => names.includes(to));
  return handover
    ? {
        ...handover,
        open: handover.after.filter(iid => !openMrs || openMrs.has(iid)),
      }
    : null;
};

/**
 * A worktree as the main space's read has it, with what agentStateOf needs
 * besides: its agent's last answer, its actions, wait, requested change and
 * what isn't pushed.
 */
const withAgentFacts = tree => ({
  ...tree,
  claude: readClaude(tree.dir),
  actions: actionsOf(actionsFile(tree.dir)),
  wait: waitOf(tree.dir),
  change: changeOf(actionsFile(tree.dir)),
  ...pushStateOf(tree.dir, tree.branch),
});

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
  // GitLab's list holds the open MRs only: one not in it is merged or closed.
  const openMrs = new Set(remote?.mrs.map(mr => mr.iid) ?? []);
  for (const listed of local.trees) {
    if (listed.dir === main) continue;
    const workspace = spaceOf.get(spaceLabel(listed.dir));
    const panes = agents.filter(pane => pane.workspace_id === workspace);
    if (!workspace || !panes.length) continue;
    const tree = withAgentFacts(listed);
    const found = remote?.mrs.find(mr => mr.branch === tree.branch);
    const mr = found ? { ...found, state: "opened" } : null;
    // Before GitLab's first answer an awaited MR counts as open.
    const merged = new Set(
      remote ? (tree.wait?.mrs ?? []).filter(iid => !openMrs.has(iid)) : [],
    );
    const handover = handoverOf(tree, remote ? openMrs : null);
    for (const pane of panes)
      lines.set(
        pane.pane_id,
        sidebarLineOf(tree, {
          mr,
          status: pane.agent_status,
          merged,
          handover,
          // The issue's boxes aren't read here: its tries only.
          checks: checksOf([], tree),
        }),
      );
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
