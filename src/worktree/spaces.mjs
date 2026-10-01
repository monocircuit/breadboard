// The herdr spaces: the main one, one per worktree, and where a checkout's
// space is.
import { spawnSync } from "node:child_process";

import { canGroupTabs, openInGroup } from "../browser.mjs";
import { herdr, isInHerdr, spaces, tabsOf } from "../lib/herdr.mjs";
import { step } from "../lib/proc.mjs";
import { CHANGELOG, EDITOR, OVERVIEW, ROOT, SPACE } from "./constants.mjs";
import { resumeSession } from "./conversations.mjs";
import {
  dropDuplicates,
  ensureSpace,
  ensureTab,
  ensureWorktreeSpace,
  featureOf,
  keepOrder,
  markBranch,
} from "./tabs.mjs";
import { editorCommand, openOnSocket } from "./nvim.mjs";
import { mainDir } from "./worktrees.mjs";
import { overallChangelogTabCommand, overviewCommand } from "./tabs.mjs";

/**
 * The main space: `root`, `overview` and `changelog` (what got into develop,
 * by product and package, #261). `root` is the tab a request for the
 * next piece of work is typed into, and an agent takes it along into its
 * worktree — so it comes back whenever it is missing. The tab was called
 * `develop` before.
 */
export const mainSpace = () => {
  const main = mainDir();
  const workspace = ensureSpace(SPACE, main, { label: ROOT });
  if (!workspace) return null;
  let tabs = tabsOf(workspace);
  const old = tabs.find(tab => tab.label === "develop");
  if (old && !tabs.some(tab => tab.label === ROOT)) {
    herdr(["tab", "rename", old.tab_id, ROOT]);
    tabs = tabsOf(workspace);
  }
  ensureTab(
    workspace,
    { label: OVERVIEW, cwd: main, command: overviewCommand() },
    tabs,
  );
  ensureTab(workspace, { label: ROOT, cwd: main }, tabs);
  ensureTab(
    workspace,
    { label: CHANGELOG, cwd: main, command: overallChangelogTabCommand() },
    tabs,
  );
  keepOrder(workspace, [ROOT, OVERVIEW, CHANGELOG]);
  return workspace;
};
let hasNvim = null;
/** Whether nvim is on the PATH: without it no nvim tab. */
export const canEdit = () =>
  (hasNvim ??=
    spawnSync("nvim", ["--version"], { stdio: "ignore" }).status === 0);
/**
 * The tab `nvim` with nvim open in the worktree, on the files it changes
 * against develop (packages/breadboard/nvim/changes.lua: `:Changes`, `:ChangesDiff`; from
 * the checkout that builds the space, like the overview), listening on
 * `.claude/nvim.sock` (nvim.mjs). Quit, its shell idles and the next layout
 * (a herdr restart, a worktree coming or going) opens it again. Nothing
 * without nvim on the PATH.
 */
export const ensureEditor = (workspace, dir, tabs) => {
  if (canEdit())
    ensureTab(
      workspace,
      { label: EDITOR, cwd: dir, command: editorCommand(dir) },
      tabs,
    );
};
/**
 * `file` (relative to `dir`) at `line` in the nvim tab `label` of
 * `workspace`, listening on the socket of `name` (nvim.mjs), and that tab in
 * front. A tab that isn't there, or whose shell idles, starts nvim on the
 * file. Defaults: the tab `nvim` of the worktree `dir`. The orchestration's
 * tab passes its `name`, `label`, space and the `commands` it starts with. An answer for the overview:
 * `{ isOpened, why }`.
 */
export const openInEditor = (
  dir,
  file,
  line,
  { name = null, label = EDITOR, workspace = spaceOf(dir), commands } = {},
) => {
  if (!canEdit()) return { isOpened: false, why: "no nvim on the PATH" };
  const isOpened =
    openOnSocket(dir, file, line, name) ||
    (Boolean(workspace) &&
      ensureTab(workspace, {
        label,
        cwd: dir,
        command: editorCommand(dir, { name, commands, file, line }),
      }));
  if (!isOpened)
    return {
      isOpened,
      why: workspace
        ? `the tab ${label} runs an nvim without its socket: quit it, the next one listens`
        : "no herdr space to open nvim in",
    };
  const tab = tabsOf(workspace).find(t => t.label === label);
  herdr(["workspace", "focus", workspace]);
  if (tab) herdr(["tab", "focus", tab.tab_id]);
  return { isOpened, why: null };
};
/**
 * The space of a worktree: `root`, its `overview`, the tab of its claude
 * conversation (unless an agent brings its own) and `nvim`, in that order.
 * `shouldResume`: the conversation bound to the space comes back in its tab
 * (after a restart, not on every layout).
 */
export const worktreeSpace = (
  dir,
  { hasWorkTab = true, shouldResume = false } = {},
) => {
  const feature = featureOf(dir);
  const workspace = ensureWorktreeSpace(feature, dir, spaceOf(mainDir()), {
    label: ROOT,
  });
  if (!workspace) return null;
  markBranch(workspace, dir);
  const tabs = tabsOf(workspace);
  ensureTab(
    workspace,
    { label: OVERVIEW, cwd: dir, command: overviewCommand(dir) },
    tabs,
  );
  if (hasWorkTab) ensureTab(workspace, { label: feature, cwd: dir }, tabs);
  ensureTab(workspace, { label: ROOT, cwd: dir }, tabs);
  // Behind the conversation's tab: an agent that brings its own opens it after the move (openSpace).
  if (hasWorkTab) ensureEditor(workspace, dir, tabs);
  dropDuplicates(workspace, feature);
  if (shouldResume) resumeSession(dir, workspace);
  keepOrder(workspace, [ROOT, OVERVIEW, feature, EDITOR]);
  return workspace;
};
/** The label of a checkout's space, also the name of its tab group in the browser: the main space for the main checkout. */
export const spaceLabel = dir => (dir === mainDir() ? SPACE : featureOf(dir));
/** The space a checkout's stack tabs belong in. */
export const spaceOf = dir =>
  spaces().find(w => w.label === spaceLabel(dir))?.workspace_id;
/** Bring a checkout's space to the front; false outside herdr or without one. */
export const focusSpace = dir => {
  const workspace = isInHerdr() && spaceOf(dir);
  return Boolean(workspace && herdr(["workspace", "focus", workspace]));
};
/**
 * A new own stack: its app in the browser, in the tab group of its space
 * (browser.mjs). Inside herdr only, where the tab `dev` starts the app,
 * and only with the extension: its page waits until the app answers, a plain
 * browser would show a connection error.
 */
export const openApp = (dir, offset) => {
  if (!offset || !isInHerdr() || !canGroupTabs()) return;
  const url = `http://localhost:${3000 + offset}`;
  if (openInGroup(url, spaceLabel(dir)))
    step(`browser: ${url} in the tab group ${spaceLabel(dir)}`);
};
