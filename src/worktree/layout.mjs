import fs from "node:fs";
import path from "node:path";

import {
  currentPane,
  herdr,
  isAgent,
  isInHerdr,
  panesOf,
  spaces,
  tabsOf,
} from "../lib/herdr.mjs";
import {
  CHANGELOG,
  EDITOR,
  OVERVIEW,
  ROOT,
  SPACE,
  STORYBOOK_PORT,
  STUDIO_PORT,
  VITA,
} from "./constants.mjs";
import { busyReason, isBusy, shareConversations } from "./conversations.mjs";
import { orchestratorSpace } from "./orchestrator.mjs";
import {
  markOrigins,
  markSpace,
  orchestrationOf,
  orchestratorLabel,
  originMark,
} from "./origins.mjs";
import { exclusive } from "./lock.mjs";
import { sortSpaces } from "./order.mjs";
import { pruneSpaces } from "./prune.mjs";
import { ensureEditor, mainSpace, spaceOf, worktreeSpace } from "./spaces.mjs";
import { compose, envFileOf, isPortFree, offsetOf } from "./stack.mjs";
import {
  dropDuplicates,
  ensureTab,
  featureOf,
  isGrouped,
  keepOrder,
} from "./tabs.mjs";
import { mainDir, worktrees } from "./worktrees.mjs";

// ---------------------------------------------------------------------------
// herdr: a space per purpose.
//
//   Monocircuit/Platform   root (the main checkout) · overview (pnpm overview, live)
//                          · changelog (pnpm changelog, live) · stack tabs
//   └─ <feature>           root (a shell in the worktree) · overview (this worktree only)
//                          · <feature> (the agent's tab) · stack tabs
//
// A worktree's space hangs under the main space in the sidebar, a group that
// folds: herdr links it when it is opened with `worktree open` on the main
// space, and takes a space that is already there into the group the same way.
//
// The stack tabs are a tab per container following its logs, `dev` (pnpm dev)
// and `studio` (drizzle studio), there while the space's own stack runs.
// `root` stays the leftmost tab. The overview of the main space keeps all of
// it in step (`--herdr`): a new worktree gets its space, a stack that comes up
// gets its tabs, one that goes down loses them.
//
// After a restart herdr brings the tabs back, but as bare shells: the layout
// runs their commands again, and the `<feature>` tab resumes the one claude
// conversation bound to the space (.claude/herdr-session of the worktree).
//
// Outside herdr none of this runs, and a failing call never fails wt:new: the
// worktree is the point, the space is comfort. herdr tells a pane which pane
// it is, so a tab moves itself without anyone guessing an id.
//
// The spaces themselves: spaces.mjs; their tabs: tabs.mjs.
// ---------------------------------------------------------------------------

const DEV = "dev";
const STUDIO = "studio";
const STORYBOOK = "storybook";
/** A lock left behind by a killed process counts as gone after this long; a whole layout takes seconds. */
/**
 * While the stack of `dir` runs, its space shows it: a tab per container
 * following its logs, `dev` running the app and `studio` running drizzle
 * studio — the last two only when their port is free, so a `pnpm dev` started
 * by hand is not started twice. When the stack is down, those tabs close,
 * unless an agent sits in one. A worktree on the shared stack has none: the
 * stack is the main checkout's. Returns what changed, `+postgres` / `-dev`.
 */
export const syncStackTabs = exclusive(
  async (dir, workspace = spaceOf(dir)) => {
    if (!workspace || !fs.existsSync(envFileOf(dir))) return [];
    const offset = dir === mainDir() ? 0 : offsetOf(dir);
    if (dir !== mainDir() && !offset) return [];

    const vita = path.join(dir, VITA);
    const services = compose(dir, ["config", "--services"], {
      quiet: true,
      allowFail: true,
    })
      .out.split("\n")
      .filter(Boolean);
    const isUp = Boolean(
      compose(dir, ["ps", "--status", "running", "-q"], {
        quiet: true,
        allowFail: true,
      }).out,
    );
    const tabs = tabsOf(workspace);
    const changed = [];

    if (isUp) {
      const logs =
        "docker compose --env-file .env.local -f docker-compose.local.yml logs -f --tail 200";
      const wanted = services.map(service => ({
        label: service,
        cwd: vita,
        command: `${logs} ${service}`,
      }));
      if (await isPortFree(3000 + offset))
        wanted.push({ label: DEV, cwd: vita, command: "pnpm dev" });
      if (await isPortFree(STUDIO_PORT + offset)) {
        wanted.push({
          label: STUDIO,
          cwd: vita,
          command: `pnpm pg:studio --port ${STUDIO_PORT + offset}`,
        });
      }
      for (const tab of wanted)
        if (ensureTab(workspace, tab, tabs)) changed.push(`+${tab.label}`);
      return changed;
    }

    const ours = new Set([...services, DEV, STUDIO]);
    const panes = panesOf(workspace);
    const current = currentPane();
    for (const tab of tabs) {
      if (!ours.has(tab.label) || isBusy(tab, panes, current)) continue;
      if (herdr(["tab", "close", tab.tab_id])) changed.push(`-${tab.label}`);
    }
    return changed;
  },
);
/**
 * monolithium's Storybook for `dir`, in the tab `storybook` of its space, on
 * STORYBOOK_PORT + its offset (a worktree on the shared stack has none, like
 * its studio). On demand only, it needs no stack; `--ci` keeps it from opening
 * a browser of its own. True when the tab was created or its idle shell got
 * the command, false outside herdr or without a space.
 */
export const startStorybook = exclusive(async dir => {
  const workspace = isInHerdr() && spaceOf(dir);
  if (!workspace) return false;
  const port = STORYBOOK_PORT + (dir === mainDir() ? 0 : offsetOf(dir));
  const cwd = path.join(dir, "packages", "monolithium");
  return ensureTab(workspace, {
    label: STORYBOOK,
    cwd,
    command: `pnpm exec storybook dev -p ${port} --ci`,
  });
});
/**
 * The whole layout from `git worktree list`: the main space, a space per
 * worktree, the stack tabs of every stack that runs. Idempotent, so it is
 * also the way back after a herdr restart or tabs closed by hand.
 *
 * It also moves the layout before this one over: a tab of a worktree in the
 * main space (`40-worktree-ports :3001`) moves into the worktree's space with
 * its panes — an agent in it keeps running — and a tab whose directory is
 * gone and that nothing runs in closes. Returns a line per change.
 */
export const layout = exclusive(async ({ shouldResume = false } = {}) => {
  const main = mainSpace();
  if (!main) return null;
  const report = [];
  const trees = worktrees().slice(1);
  report.push(...pruneSpaces());
  const merged = shareConversations();
  if (merged.length)
    report.push(
      `conversations of ${merged.join(", ")} moved to the project of ${path.basename(mainDir())}`,
    );

  const tabs = tabsOf(main);
  const panes = panesOf(main);
  const current = currentPane();
  for (const tab of tabs) {
    if ([OVERVIEW, ROOT, CHANGELOG].includes(tab.label)) continue;
    const tree = trees.find(
      ({ dir }) =>
        tab.label === featureOf(dir) ||
        tab.label.startsWith(`${featureOf(dir)} :`),
    );
    if (tree) {
      const workspace = worktreeSpace(tree.dir, { hasWorkTab: false });
      for (const pane of panes.filter(p => p.tab_id === tab.tab_id)) {
        const moved = herdr([
          "pane",
          "move",
          pane.pane_id,
          "--workspace",
          workspace,
          "--new-tab",
        ]);
        const created = moved?.move_result?.created_tab;
        if (created)
          herdr(["tab", "rename", created.tab_id, featureOf(tree.dir)]);
      }
      report.push(`${tab.label} moved into the space ${featureOf(tree.dir)}`);
      continue;
    }
    const isGone = panes
      .filter(p => p.tab_id === tab.tab_id)
      .every(p => p.cwd?.endsWith(" (deleted)"));
    if (
      isGone &&
      !isBusy(tab, panes, current) &&
      herdr(["tab", "close", tab.tab_id])
    ) {
      report.push(`${tab.label} closed, its directory is gone`);
    }
  }

  // The spaces of the orchestrations, in the group of the main space above their worktrees'.
  const orchestrations = new Set(
    spaces()
      .map(w => orchestrationOf(w.label))
      .filter(Boolean),
  );
  for (const name of orchestrations) orchestratorSpace(name, { shouldResume });

  const known = new Map(spaces().map(w => [w.label, w]));
  for (const { dir } of trees) {
    const before = known.get(featureOf(dir));
    const workspace = worktreeSpace(dir, { shouldResume });
    if (!before && workspace) report.push(`space ${featureOf(dir)} opened`);
    if (
      before &&
      !before.worktree?.is_linked_worktree &&
      isGrouped(workspace)
    ) {
      report.push(`space ${featureOf(dir)} moved into its group`);
    }
    const changed = await syncStackTabs(dir, workspace);
    if (changed.length) report.push(`${featureOf(dir)}: ${changed.join(" ")}`);
  }
  const changed = await syncStackTabs(mainDir(), main);
  if (changed.length) report.push(`${SPACE}: ${changed.join(" ")}`);
  // Last, when every space is there: herdr forgets the marks on a restart.
  markOrigins();
  const sorted = await sortSpaces();
  if (sorted) report.push(sorted);
  return { main, report };
});
/**
 * The orchestration whose space `⇶ <title>` holds the pane running this, or
 * null. An orchestrator that runs wt:new itself stays there (#245).
 */
export const orchestrationHere = () => {
  if (!isInHerdr() || !process.env.HERDR_PANE_ID) return null;
  const pane = currentPane();
  return orchestrationOf(
    pane && spaces().find(w => w.workspace_id === pane.workspace_id)?.label,
  );
};
/**
 * Whether wt:new takes the tab running it into the new space: an agent's
 * goes along, a human's stays, and so does the orchestrator's, whether
 * wt:orchestrate started the worktree (`origin`) or it ran wt:new itself from
 * its space (`orchestration`, #245).
 */
export const movesTab = ({ isAgent, origin = null, orchestration = null }) =>
  isAgent && !origin && !orchestration;
/**
 * After wt:new: the worktree gets its space.
 *
 * A human gets it with a tab to start `claude` in, in front. An agent takes
 * its own tab into it (it follows with EnterWorktree); when that was `root`,
 * the main space gets a new one. The ids come from the pane, not from the
 * environment: a move gives the pane a new one.
 *
 * `origin`: the worktree is an orchestration's. Its space is built as for a
 * human, unfocused and marked, and the tab `<feature>` waits for its agent
 * (startAgent); the orchestrator's own tab stays where it is.
 *
 * `stays`: the orchestration whose space runs this (#245), the orchestrator
 * started the worktree itself. Its space is built as for a human, unfocused
 * and unmarked, and the orchestrator's tab stays where it is.
 */
export const openSpace = exclusive(
  async (feature, dir, origin = null, stays = null) => {
    if (!isInHerdr() || !mainSpace()) return null;
    if (!movesTab({ isAgent: isAgent(), origin, orchestration: stays })) {
      const workspace = worktreeSpace(dir);
      if (!workspace) return null;
      await syncStackTabs(dir, workspace);
      if (origin) {
        const found = spaces().find(w => w.workspace_id === workspace);
        if (found) markSpace(found, origin);
        return `opened the space ${feature}, marked ${originMark(origin.name)}`;
      }
      if (stays)
        return `opened the space ${feature}, this tab stays in ${orchestratorLabel(stays)}`;
      const tab = tabsOf(workspace).find(t => t.label === feature);
      if (tab) herdr(["tab", "focus", tab.tab_id]);
      return `opened the space ${feature}`;
    }

    const workspace = worktreeSpace(dir, { hasWorkTab: false });
    const pane = currentPane();
    if (!workspace || !pane) return null;
    const moved = herdr([
      "pane",
      "move",
      pane.pane_id,
      "--workspace",
      workspace,
      "--new-tab",
    ]);
    const created = moved?.move_result?.created_tab;
    if (!created) return null;
    herdr(["tab", "rename", created.tab_id, feature]);
    // Before the stack tabs, so they open behind it and nothing has to move but the conversation's tab.
    dropDuplicates(workspace, feature);
    ensureEditor(workspace, dir, tabsOf(workspace));
    keepOrder(workspace, [ROOT, OVERVIEW, feature, EDITOR]);
    await syncStackTabs(dir, workspace);
    const hadRoot = tabsOf(spaceOf(mainDir()) ?? "").some(
      tab => tab.label === ROOT,
    );
    mainSpace();
    return `this tab moved into the space ${feature}${hadRoot ? "" : `, a new tab ${ROOT} waits in ${SPACE}`}`;
  },
);
/**
 * After wt:remove: the space of the worktree goes, its directory is gone.
 *
 * Not blindly, though. A tab with an agent at work, more than one pane, or the
 * very command that is removing the worktree stays: closing it would cut off
 * a turn or kill a running server. An idle agent goes; `/resume` in the main
 * checkout brings its conversation back. Tabs of the layout before this one, in
 * the main space, go the same way.
 *
 * `isWhole` (the ✕ of the overview, `--close-space`): someone chose to delete
 * it all, so everything goes but the command's own tab — an agent's
 * conversation too, it stays resumable (`/resume` in the main checkout).
 */
export const releaseSpace = (feature, { isWhole = false } = {}) => {
  if (!isInHerdr()) return null;
  // Without a pane of its own (detached, from the ✕ of the overview) herdr
  // answers with the focused pane, and that tab would stay as the one running this.
  const current = process.env.HERDR_PANE_ID ? currentPane() : null;
  let closed = 0;
  /** Why each kept tab stays, one entry per tab. */
  const kept = [];

  for (const workspace of spaces()) {
    const isOwn = workspace.label === feature;
    const tabs = tabsOf(workspace.workspace_id).filter(
      tab =>
        isOwn || tab.label === feature || tab.label.startsWith(`${feature} :`),
    );
    if (tabs.length === 0) continue;
    const panes = panesOf(workspace.workspace_id);
    const busy = tabs.filter(tab => {
      const why = isWhole
        ? tab.tab_id === current?.tab_id
          ? "it runs this command"
          : null
        : busyReason(tab, panes, current);
      if (why) kept.push(`${tab.label}: ${why}`);
      return Boolean(why);
    });
    // Nothing to keep in its own space: the space goes as a whole.
    if (
      isOwn &&
      busy.length === 0 &&
      herdr(["workspace", "close", workspace.workspace_id])
    ) {
      closed += tabs.length;
      continue;
    }
    for (const tab of tabs) {
      if (!busy.includes(tab) && herdr(["tab", "close", tab.tab_id])) closed++;
    }
  }

  if (closed === 0 && kept.length === 0) return null;
  const parts = [];
  if (closed) {
    parts.push(
      kept.length
        ? `closed ${closed} tab${closed === 1 ? "" : "s"} of ${feature}`
        : `closed the space ${feature}`,
    );
  }
  if (kept.length)
    parts.push(
      `kept ${kept.length === 1 ? "a tab" : `${kept.length} tabs`} of ${feature} (${kept.join("; ")})`,
    );
  return parts.join(", ");
};
