// The order of the spaces in the group of the main space (#255): the
// worktrees of no orchestration first, then per orchestration its space
// `⇶ <title>` with its worktrees' spaces right below it, in the work order of
// its plan (`pnpm wt:plan`). A later orchestration comes below an earlier one.
//
//   Monocircuit/Platform
//   ├─ link-only-dot            no orchestration
//   ├─ ⇶ @monocircuit/breadboard
//   ├─ 249-…                    its worktrees, as `wt:plan` works on them
//   └─ 240-…
//
// herdr's CLI cannot move a space; its socket API can (`workspace.move_block`).
// The overview of the main space (`--herdr`) keeps the order on every read, so
// a new worktree, a handover, a changed plan or an orchestration that ended
// (its worktrees go back to the plain block) sorts itself within a tick.
import { herdrApi, spaces } from "../lib/herdr.mjs";
import { SPACE } from "./constants.mjs";
import { orchestrated, orchestrationOf, recordOf } from "./origins.mjs";
import { featureOf } from "./tabs.mjs";

/**
 * The features of an orchestration's worktrees in its work order: the
 * issues of `plan.work` first, then the rest of its record in the order they
 * were started.
 */
export const workOrderOf = record => {
  const issues = record?.issues ?? [];
  const planned = (record?.plan?.work ?? [])
    .map(number => issues.find(issue => issue.number === number))
    .filter(Boolean);
  return [...new Set([...planned, ...issues].map(issue => issue.feature))];
};

/**
 * The move that brings the group of the main space in `list` (herdr's
 * `workspace list`, in sidebar order) into its order, `{ workspace_ids,
 * before_workspace_id }` for `workspace.move_block`, or null when it stands
 * so already.
 * - `originOf`: the orchestration a worktree's space belongs to, by its label;
 * - `workOrderOf`: an orchestration's features in its work order, by its name;
 * - `orchestrationOf`: the orchestration a space `⇶ <title>` is of, by its label.
 *
 * The group is the main space's linked worktrees, the orchestrations' spaces
 * among them. Spaces outside it stay where they are, the block goes where
 * its first space stood.
 */
export const groupMove = (
  list,
  { originOf, workOrderOf: orderOf, orchestrationOf: ofLabel },
) => {
  const main = list.find(w => w.label === SPACE);
  const repo = main?.worktree?.repo_key;
  if (!repo) return null;
  const group = list.filter(
    w => w.worktree?.is_linked_worktree && w.worktree.repo_key === repo,
  );
  const orchestrations = group.filter(w => ofLabel(w.label));
  const open = new Set(orchestrations.map(w => ofLabel(w.label)));
  const worktrees = group.filter(w => !ofLabel(w.label));
  const plain = worktrees.filter(w => !open.has(originOf(w.label)));
  const blocks = orchestrations.flatMap(space => {
    const name = ofLabel(space.label);
    const order = orderOf(name);
    const rank = w =>
      order.includes(w.label) ? order.indexOf(w.label) : order.length;
    const own = worktrees
      .filter(w => originOf(w.label) === name)
      .toSorted((a, b) => rank(a) - rank(b));
    return [space, ...own];
  });
  const wanted = [...plain, ...blocks].map(w => w.workspace_id);
  if (wanted.join() === group.map(w => w.workspace_id).join()) return null;

  // Behind the main space and where the group starts, before the first space outside it.
  const ids = new Set(wanted);
  const from = Math.max(list.indexOf(main), list.indexOf(group[0]));
  const before = list
    .slice(from)
    .find(w => w !== main && !ids.has(w.workspace_id));
  return {
    workspace_ids: wanted,
    before_workspace_id: before?.workspace_id ?? null,
  };
};

/**
 * The group of the main space into its order, when it is out of it. Returns
 * a line for what moved, null when nothing had to (or outside herdr).
 */
export const sortSpaces = async () => {
  const list = spaces();
  const origins = new Map(
    orchestrated().map(({ dir, origin }) => [featureOf(dir), origin.name]),
  );
  // A label's orchestration reads every record: once per space.
  const orchestrations = new Map(
    list.map(w => [w.label, orchestrationOf(w.label)]),
  );
  const move = groupMove(list, {
    originOf: label => origins.get(label) ?? null,
    workOrderOf: name => workOrderOf(recordOf(name)),
    orchestrationOf: label => orchestrations.get(label),
  });
  if (!move) return null;
  const moved = await herdrApi("workspace.move_block", move);
  return moved ? `spaces of ${SPACE} sorted by orchestration` : null;
};
