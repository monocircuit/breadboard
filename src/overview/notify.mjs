// herdr notifications when a worktree needs you: its agent starts asking
// something, the pipeline of its MR fails, its MR is merged. On the change
// only: what the overview finds at its start notifies nothing. The overview
// of the main space (--herdr) is the one notifier of a session, except for
// the worktrees of an orchestration whose space is open: its overview
// notifies their actions (auto-actions.mjs).
import { glabApi } from "../lib/glab.mjs";
import { herdr, isInHerdr, spaces } from "../lib/herdr.mjs";
import { featureOf, mainDir, spaceLabel } from "../worktree.mjs";
import { orchestrationOf } from "../worktree/origins.mjs";

/**
 * The notifications for one worktree between two reads, as `{ body, sound }`.
 * A state: `isAsking` (an agent in its space is blocked), `mr` (the open MR's
 * `iid` and pipeline `status`, or null) and `hasRemote` (GitLab answered at
 * least once: before that, a missing MR says nothing). `wasMerged(iid)`
 * tells an MR that left the open ones merged from closed.
 */
export const eventsOf = async (before, now, wasMerged) => {
  if (!before) return [];
  const events = [];
  if (now.isAsking && !before.isAsking)
    events.push({ body: "its agent asks you something", sound: "request" });
  if (before.hasRemote && now.hasRemote) {
    const isSameMr = before.mr && now.mr?.iid === before.mr.iid;
    if (
      isSameMr &&
      now.mr.status === "failed" &&
      before.mr.status !== "failed"
    ) {
      events.push({
        body: `the pipeline of !${now.mr.iid} failed`,
        sound: "request",
      });
    }
    if (before.mr && !now.mr && (await wasMerged(before.mr.iid))) {
      events.push({ body: `!${before.mr.iid} is merged`, sound: "done" });
    }
  }
  return events;
};

/** worktree dir → its state at the last read. */
const last = new Map();

const wasMerged = async iid => {
  try {
    return (
      (await glabApi(`projects/:fullpath/merge_requests/${iid}`)).state ===
      "merged"
    );
  } catch {
    return false;
  }
};

/** Notifies what changed since the last call, from the local read and GitLab's (null until it answers). */
export const notifyChanges = async (local, remote) => {
  if (!isInHerdr()) return;
  const panes = herdr(["pane", "list"])?.panes ?? [];
  const workspaces = spaces();
  const spaceOf = new Map(
    workspaces.map(space => [space.label, space.workspace_id]),
  );
  const orchestrations = new Set(
    workspaces.map(space => orchestrationOf(space.label)).filter(Boolean),
  );
  const main = mainDir();
  for (const tree of local.trees) {
    if (tree.dir === main) continue;
    if (tree.origin && orchestrations.has(tree.origin.name)) continue;
    const workspace = spaceOf.get(spaceLabel(tree.dir));
    const mr =
      remote?.mrs.find(candidate => candidate.branch === tree.branch) ?? null;
    const now = {
      isAsking: panes.some(
        pane =>
          pane.workspace_id === workspace &&
          pane.agent &&
          pane.agent_status === "blocked",
      ),
      mr: mr ? { iid: mr.iid, status: mr.pipeline?.status ?? null } : null,
      hasRemote: Boolean(remote),
    };
    const events = await eventsOf(last.get(tree.dir), now, wasMerged);
    last.set(tree.dir, now);
    for (const { body, sound } of events) {
      herdr([
        "notification",
        "show",
        featureOf(tree.dir),
        "--body",
        body,
        "--sound",
        sound,
      ]);
    }
  }
  const current = new Set(local.trees.map(tree => tree.dir));
  for (const dir of last.keys()) if (!current.has(dir)) last.delete(dir);
};
