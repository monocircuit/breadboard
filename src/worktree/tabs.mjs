// The tabs of a herdr space: create, find idle ones, keep them in order, drop
// duplicates.
import path from "node:path";

import { herdr, panesOf, spaces, tabsOf } from "../lib/herdr.mjs";
import { quote } from "../lib/proc.mjs";
import { TOKEN_SOURCE } from "./origins.mjs";
import { mainDir, worktrees } from "./worktrees.mjs";

/** `platform-40-worktree-ports` → `40-worktree-ports`, the name of its space. */
export const featureOf = dir =>
  path.basename(dir).replace(`${path.basename(mainDir())}-`, "");
/**
 * The live overview a tab runs: of everything with `--herdr` (it keeps the
 * spaces in step), of one worktree with `--worktree`, of one orchestration with
 * `--orchestration`. From the scripts of the checkout that builds the space, so
 * a worktree branched before a change of overview.mjs gets the current one.
 */
export const overviewCommand = (dir, orchestration = null) =>
  `node ${quote(path.join(import.meta.dirname, "..", "overview.mjs"))} ${
    orchestration
      ? `--orchestration ${orchestration}`
      : dir
        ? `--worktree ${quote(dir)}`
        : "--herdr"
  }`;
/** What the tab `changelog` of the main space runs: the changelog of the repo, live (#261, changelog-view.mjs). */
export const overallChangelogTabCommand = () =>
  `node ${quote(path.join(import.meta.dirname, "..", "worktree.mjs"))} changelog --watch`;
/** A space by its label, created with the tab `first` (and its command) if missing. */
export const ensureSpace = (label, cwd, first) => {
  const found = spaces().find(w => w.label === label);
  if (found) return found.workspace_id;
  const created = herdr([
    "workspace",
    "create",
    "--label",
    label,
    "--cwd",
    cwd,
    "--no-focus",
  ]);
  if (!created?.workspace) return null;
  herdr(["tab", "rename", created.tab.tab_id, first.label]);
  if (first.command)
    herdr(["pane", "run", created.root_pane.pane_id, first.command]);
  return created.workspace.workspace_id;
};
/**
 * The space of a worktree, in the group of the main space `main`: herdr links
 * it only when `worktree open` opens it. A space of the worktree that is
 * already there but stands alone (opened before the groups, or by
 * `workspace create`) is taken into the group as it is, what runs in it keeps
 * running (`already_open`). An older herdr without `worktree open` gets a
 * space of its own, as before.
 */
export const ensureWorktreeSpace = (label, dir, main, first) => {
  const found = spaces().find(w => w.label === label);
  if (found?.worktree?.is_linked_worktree || !main)
    return found?.workspace_id ?? ensureSpace(label, dir, first);
  // herdr takes the main space for the worktree's while its first tab stands in the worktree
  // (herdrdev/herdr#4293), and `worktree open` would then turn the main space into the worktree's.
  const listed =
    herdr(["worktree", "list", "--workspace", main])?.worktrees ?? [];
  const isMainTaken = listed.some(
    w => w.path === dir && w.open_workspace_id === main,
  );
  const opened =
    !isMainTaken &&
    herdr([
      "worktree",
      "open",
      "--workspace",
      main,
      "--path",
      dir,
      "--label",
      label,
      "--no-focus",
    ]);
  if (!opened?.workspace)
    return found?.workspace_id ?? ensureSpace(label, dir, first);
  const workspace = opened.workspace.workspace_id;
  if (opened.already_open) return workspace;
  herdr(["tab", "rename", opened.tab.tab_id, first.label]);
  if (first.command)
    herdr(["pane", "run", opened.root_pane.pane_id, first.command]);
  if (found) {
    moveInto(found.workspace_id, workspace);
    dropDuplicates(workspace, first.label);
  }
  return workspace;
};
/**
 * Every tab of the space `from` into the space `to`, in order and with its
 * label, what runs in it keeps running; `from` closes once it is empty. For
 * a space herdr opened anew instead of taking the old one into the group (an
 * orchestration's, which stood in the main checkout). A tab split into panes
 * comes back as one tab, its panes side by side.
 */
export const moveInto = (from, to) => {
  const panes = panesOf(from);
  for (const tab of tabsOf(from)) {
    const [head, ...rest] = panes.filter(p => p.tab_id === tab.tab_id);
    const moved =
      head &&
      herdr([
        "pane",
        "move",
        head.pane_id,
        "--workspace",
        to,
        "--new-tab",
        "--no-focus",
      ]);
    const created = moved?.move_result?.created_tab;
    if (!created) continue;
    herdr(["tab", "rename", created.tab_id, tab.label]);
    for (const pane of rest)
      herdr([
        "pane",
        "move",
        pane.pane_id,
        "--tab",
        created.tab_id,
        "--split",
        "right",
        "--no-focus",
      ]);
  }
  if (spaces().some(w => w.workspace_id === from) && tabsOf(from).length === 0)
    herdr(["workspace", "close", from]);
};
/**
 * The branch of a worktree as the sidebar token `$wt_branch`: herdr's own
 * `branch` stays empty for a linked worktree (herdrdev/herdr#2952), so its
 * row under the main space would show no branch. Only on a change.
 */
export const BRANCH_TOKEN = "wt_branch";
export const markBranch = (workspace, dir) => {
  const branch = worktrees().find(w => w.dir === dir)?.branch;
  const found = spaces().find(w => w.workspace_id === workspace);
  if (!branch || !found || found.tokens?.[BRANCH_TOKEN] === branch) return;
  herdr([
    "workspace",
    "report-metadata",
    workspace,
    "--source",
    TOKEN_SOURCE,
    "--token",
    `${BRANCH_TOKEN}=${branch}`,
  ]);
};
export const isGrouped = workspace =>
  Boolean(
    spaces().find(w => w.workspace_id === workspace)?.worktree
      ?.is_linked_worktree,
  );
/** A pane whose shell has the terminal to itself: nothing runs in it, as after a restart of herdr. */
export const isIdle = paneId => {
  const info = herdr(["pane", "process-info", "--pane", paneId])?.process_info;
  return (
    Boolean(info?.shell_pid) &&
    info.foreground_process_group_id === info.shell_pid
  );
};
/**
 * The single pane of the tab `label` when nothing runs in it, else null. herdr
 * restores tabs after a restart, but not what ran in them.
 */
export const idlePaneOf = (workspace, label, tabs = tabsOf(workspace)) => {
  const tab = tabs.find(t => t.label === label);
  if (!tab || tab.pane_count !== 1) return null;
  const pane = panesOf(workspace).find(p => p.tab_id === tab.tab_id);
  return pane && isIdle(pane.pane_id) ? pane.pane_id : null;
};
/**
 * A tab by its label, created unfocused with `command` running in it if
 * missing, and `command` run again if the tab is there but its shell idles.
 * True when either happened.
 */
export const ensureTab = (
  workspace,
  { label, cwd, command },
  tabs = tabsOf(workspace),
) => {
  if (tabs.some(tab => tab.label === label)) {
    const idle = command ? idlePaneOf(workspace, label, tabs) : null;
    return Boolean(idle && herdr(["pane", "run", idle, command]));
  }
  const created = herdr([
    "tab",
    "create",
    "--workspace",
    workspace,
    "--cwd",
    cwd,
    "--label",
    label,
    "--no-focus",
  ]);
  if (created && command)
    herdr(["pane", "run", created.root_pane.pane_id, command]);
  return Boolean(created);
};
/**
 * The tabs `order` names at the left end of their space, in that order —
 * `root`, `overview`, the conversation's tab, `nvim` — the others behind them.
 * herdr appends every new tab, so this acts on an older space, a tab that was
 * closed and came back, or the conversation's tab an agent moved in. herdr
 * can't reorder tabs: a tab out of place moves its pane into a new tab at the
 * end, what runs there keeps running. When the named tabs after the first one
 * out of place are in order, only the others in between move; else all of
 * them. A move focuses the tab: focus goes back to where it was. It stops at
 * a tab split into several panes, a move would undo that.
 */
export const keepOrder = (workspace, order) => {
  const tabs = tabsOf(workspace);
  const wanted = order.filter(label => tabs.some(tab => tab.label === label));
  let fixed = 0;
  while (fixed < wanted.length && tabs[fixed]?.label === wanted[fixed]) fixed++;
  if (fixed === wanted.length) return;
  const rest = tabs.slice(fixed);
  const named = rest.filter(tab => wanted.includes(tab.label));
  const others = rest.filter(tab => !wanted.includes(tab.label));
  const isInOrder = named.every((tab, i) => tab.label === wanted[fixed + i]);
  const moving = isInOrder
    ? others.filter(tab => rest.indexOf(tab) < rest.indexOf(named.at(-1)))
    : [
        ...wanted
          .slice(fixed)
          .map(label => named.find(tab => tab.label === label)),
        ...others,
      ];

  const before = spaces();
  const panes = panesOf(workspace);
  const renamed = new Map();
  for (const tab of moving) {
    if (tab.pane_count !== 1) break;
    const pane = panes.find(p => p.tab_id === tab.tab_id);
    const moved =
      pane &&
      herdr([
        "pane",
        "move",
        pane.pane_id,
        "--workspace",
        workspace,
        "--new-tab",
      ]);
    const created = moved?.move_result?.created_tab;
    if (!created) break;
    herdr(["tab", "rename", created.tab_id, tab.label]);
    renamed.set(tab.tab_id, created.tab_id);
  }
  if (renamed.size === 0) return;
  const now = tabId => renamed.get(tabId) ?? tabId;
  const own = before.find(w => w.workspace_id === workspace)?.active_tab_id;
  const focused = before.find(w => w.focused)?.active_tab_id;
  if (own) herdr(["tab", "focus", now(own)]);
  if (focused) herdr(["tab", "focus", now(focused)]);
};
/**
 * One tab `label` in the space. Two come about when the overview builds the
 * space of a new worktree, with an empty tab for its conversation, right
 * before the agent that made it moves its own tab in. The spare ones close:
 * a single pane, no agent, its shell idle. The one claude runs in stays; when
 * all are spare (after a restart) the first stays, the conversation resumes there.
 */
export const dropDuplicates = (workspace, label) => {
  const tabs = tabsOf(workspace).filter(tab => tab.label === label);
  if (tabs.length < 2) return;
  const panes = panesOf(workspace);
  const spare = tabs.filter(tab => {
    const own = panes.filter(p => p.tab_id === tab.tab_id);
    return own.length === 1 && !own[0].agent && isIdle(own[0].pane_id);
  });
  for (const tab of spare.length === tabs.length ? spare.slice(1) : spare)
    herdr(["tab", "close", tab.tab_id]);
};
