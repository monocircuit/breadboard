// Free agents (#289): an agent whose MR is merged stays for the next issue
// of its orchestration instead of going with its worktree. Before the
// clean-up its tab moves into the space `⇶ <name>` as `free <agent>`, it
// enters the orchestration's checkout (which outlives every worktree) and
// stops; the record keeps it, so a herdr restart resumes it there. The next
// issue goes to it before a fresh agent starts (workers.mjs, fit.mjs), a
// click dismisses it, and the end of the orchestration closes it with the space.
import fs from "node:fs";

import { herdr, herdrDid, isInHerdr, spaces, tabsOf } from "../lib/herdr.mjs";
import { quote } from "../lib/proc.mjs";
import { isConversationRunning, sessionFile } from "./conversations.mjs";
import { checkoutOf } from "./orchestrator.mjs";
import { orchestrationOf, recordOf, writeWorker } from "./origins.mjs";
import { ensureTab, featureOf } from "./tabs.mjs";

/** The tab of a free agent in the space `⇶ <name>`. */
export const freeTab = agent => `free ${agent}`;
/** Whether a tab's label is a free agent's. */
export const isFreeTab = label => String(label ?? "").startsWith("free ");
/**
 * The free agents of a record, by their conversation: `[{ id, name, files,
 * issues, free }]`, `free` the record's `{ at, from, mr }`: since when, the
 * worktree it came from and its merged MR.
 */
export const freeAgentsOf = record =>
  Object.entries(record?.workers ?? {})
    .filter(([, worker]) => worker?.free)
    .map(([id, worker]) => ({
      id,
      name: worker.name,
      files: worker.files ?? [],
      issues: worker.issues ?? [],
      was: worker.was ?? null,
      free: worker.free,
    }));
/**
 * The line a merged agent is freed with, typed into its prompt: it leaves the
 * worktree (wt:remove waits until it has) and stops, spending nothing.
 */
export const freeLine = (iid, checkout) =>
  `[overview] !${iid} merged, your worktree is removed: you are free. EnterWorktree(path: "${checkout}"), then stop and wait for the next issue, it comes as one line.`;
/**
 * The record's entry of an agent once it is free: what it knows (`files`,
 * those of the worktree it leaves added) and `free`, since when and from where.
 */
export const freedEntry = (known, { name, files, from, mr, at }) => ({
  name,
  files: [...new Set([...(known?.files ?? []), ...files])],
  free: { at, from, mr },
});

/** The space `⇶ <name>`, its id, or null. */
const spaceOfOrchestration = name =>
  spaces().find(w => orchestrationOf(w.label) === name)?.workspace_id ?? null;
/**
 * Frees the agent of a merged worktree (`dir`): its pane `pane` into the
 * space `⇶ <name>` as the tab `free <agent>`, the record learns it (and
 * `files` it knows), its binding to the worktree goes, and it is told
 * (freeLine). The worktree stays until the agent left it: wt:remove refuses
 * a worktree an unbound claude stands in (leftIn). `{ isFreed, isTold, why }`;
 * not freed outside herdr or without the space, then it goes with its clean-up.
 */
export const freeAgent = (name, { id, agent, pane, dir, iid, files = [] }) => {
  if (!isInHerdr()) return { isFreed: false, why: "outside herdr" };
  const workspace = spaceOfOrchestration(name);
  if (!workspace) return { isFreed: false, why: `no space of ${name}` };
  const moved = herdr([
    "pane",
    "move",
    pane,
    "--workspace",
    workspace,
    "--new-tab",
    "--no-focus",
  ]);
  const created = moved?.move_result?.created_tab;
  if (!created) return { isFreed: false, why: "herdr did not move its pane" };
  herdr(["tab", "rename", created.tab_id, freeTab(agent)]);
  const target = moved.move_result.pane?.pane_id ?? pane;
  writeWorker(
    name,
    id,
    freedEntry(recordOf(name)?.workers?.[id], {
      name: agent,
      files,
      from: featureOf(dir),
      mr: iid,
      at: new Date().toISOString(),
    }),
  );
  // Unbound: no restart resumes it in the worktree, and wt:remove waits until it left.
  fs.rmSync(sessionFile(dir), { force: true });
  const isTold = herdrDid([
    "agent",
    "prompt",
    target,
    freeLine(iid, checkoutOf(name)),
  ]);
  return { isFreed: true, isTold, why: null };
};
/**
 * The tabs of the free agents `free` (`{ id }`) that stand outside the space
 * `workspace` (moved by hand): the end of the orchestration closes them
 * too, those inside go with the space. `panes`: herdr's pane list.
 */
export const freeTabsOutside = (free, panes, workspace) => {
  const ids = new Set(free.map(({ id }) => id));
  return [
    ...new Set(
      panes
        .filter(
          pane =>
            ids.has(pane.agent_session?.value) &&
            pane.workspace_id !== workspace,
        )
        .map(pane => pane.tab_id),
    ),
  ];
};
/** The record forgets a free agent: dismissed, handed an issue (`free: null`). */
export const unfree = (name, id, fields = {}) =>
  writeWorker(name, id, { free: null, ...fields });
/**
 * A click dismisses a free agent: its tab closes (its conversation stays in
 * `/resume` of the main checkout), the record forgets it as free. `tab`:
 * herdr's tab of its pane, null when none runs it any more.
 */
export const dismissFree = (name, id, tab) => {
  if (tab) herdr(["tab", "close", tab]);
  unfree(name, id, { dismissed: new Date().toISOString() });
};
/**
 * After a herdr restart: every free agent of `name` runs again in its tab of
 * the space `workspace`, in the orchestration's checkout, unless its
 * conversation runs somewhere already. Returns the names resumed.
 */
export const resumeFree = (name, workspace) => {
  const checkout = checkoutOf(name);
  if (!fs.existsSync(checkout)) return [];
  const tabs = tabsOf(workspace);
  return freeAgentsOf(recordOf(name))
    .filter(({ id }) => !isConversationRunning(id))
    .filter(({ id, name: agent }) =>
      ensureTab(
        workspace,
        {
          label: freeTab(agent),
          cwd: checkout,
          command: `cd ${quote(checkout)} && claude --resume ${id}`,
        },
        tabs,
      ),
    )
    .map(({ name: agent }) => agent);
};
