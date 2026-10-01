#!/usr/bin/env node
// A live overview of the parallel work: worktrees, their stacks, the merge
// requests with their pipelines and the open issues with who claimed them.
//
//   pnpm overview                   full screen, refreshes itself; q quits, r fetches GitLab now,
//                                   the button or row under the pointer is lit (lib/mouse.mjs),
//                                   a click on a row opens it (a worktree: its herdr space; a url:
//                                   the browser, in the tab group of its space, browser.mjs),
//                                   Ctrl-click on !n, #n or a pipeline opens just that link,
//                                   ▶ (a click or u) starts a stack that is down; a view taller
//                                   than the pane scrolls (wheel, j k ↑ ↓, PgUp PgDn space, g G)
//                                   under its title bar (overview/scroll.mjs)
//   pnpm overview --once            print it once (no terminal, an agent, a pipe)
//   pnpm overview --worktree <dir>  only that worktree: its ports, stack, MR and pipeline,
//                                   DIFF: how far it is from develop and where it collides
//                                   (overview/diff.mjs); AGENT, ISSUE (its boxes),
//                                   PIPELINE (its jobs), DEVELOP (what came in, would a merge
//                                   conflict) and the migrations the stack lacks
//                                   (overview/develop.mjs); a click on a file there (DIFF, a
//                                   conflict, a file:line of a failed job) opens it in the
//                                   worktree's nvim tab at the change (overview/files.mjs);
//                                   ✕ (two clicks) removes it once nothing would be lost
//   pnpm overview --orchestration <name>
//                                   only that orchestration (pnpm wt:orchestrate): its issues, the
//                                   worktree and agent of each, their merge requests and pipelines
//                                   (the tab `overview` of the space ⇶ <name> runs this); a merge
//                                   action merges on two clicks (overview/merge.mjs)
//   pnpm overview --herdr           and keep herdr in step: a space per worktree, stack
//                                   tabs while a stack runs (the tab `overview` runs this)
//
// The local part (git, ports, docker) is read every LOCAL_MS, GitLab every
// REMOTE_MS: glab costs a request, the ports cost nothing.
import {
  FOCUS_OUT,
  MOUSE_OFF,
  MOUSE_ON,
  keyReader,
  mouseOf,
} from "./lib/mouse.mjs";
import { startStack } from "./overview/actions.mjs";
import { reportStates } from "./overview/agent-state.mjs";
import {
  LOCAL_MS,
  REMOTE_MS,
  isOnce,
  isSyncing,
  only,
} from "./overview/args.mjs";
import { guarded } from "./overview/guard.mjs";
import { notifyChanges } from "./overview/notify.mjs";
import { screen } from "./overview/screen.mjs";
import { WRAP_OFF, WRAP_ON, painter, watchSize } from "./overview/frame.mjs";
import { pageOf, scrollBy } from "./overview/scroll.mjs";
import { reader } from "./overview/reader.mjs";
import {
  readRemoteSafely,
  readView,
  retryDeliveries,
  view,
} from "./overview/view.mjs";
import {
  layout,
  pruneSpaces,
  rememberSessions,
  sortSpaces,
  syncStackTabs,
} from "./worktree.mjs";

// --once: one print, no screen of its own.
if (isOnce) {
  const [local, { remote, error }] = await Promise.all([
    readView(),
    readRemoteSafely(),
  ]);
  console.log(view(local, remote, error));
  process.exit(0);
}

// The local read runs in a worker (overview/reader.mjs): what it blocks
// doesn't hold up the wheel. What it sets off stays here, with the clicks.
const readInWorker = reader();
const read = async () => {
  const next = await readInWorker();
  retryDeliveries(next);
  return next;
};
let local = await read();
let remote = null;
let error = null;
let isFetching = false;
const rows = () => process.stdout.rows || 40;
// A view that throws shows its error in the first line of the last one and is tried
// again at the next redraw, instead of ending the tab (overview/guard.mjs);
// the clicks stay those of the view on screen.
let good = null;
const render = guarded(() => {
  const text = view(local, remote, error);
  good = {
    clicks: screen.clicks,
    sticky: screen.sticky,
    upTarget: screen.upTarget,
  };
  return text;
});
// The view is kept until its data changes; scrolling cuts a window out of
// it, once per frame, and only the rows that changed are written
// (overview/frame.mjs).
const frame = painter({
  render: () => {
    const text = render();
    Object.assign(screen, good);
    return text;
  },
  rows,
  size: () => `${process.stdout.columns}x${process.stdout.rows}`,
  write: text => process.stdout.write(text),
});
const draw = frame.changed;
screen.redraw = async ({ isReread = false } = {}) => {
  if (isReread) local = await read();
  draw();
};
const fetchRemote = async () => {
  if (isFetching) return;
  isFetching = true;
  const next = await readRemoteSafely();
  // A failed fetch keeps the last good state on screen, with the error above it.
  if (next.remote) remote = next.remote;
  error = next.error;
  isFetching = false;
  draw();
};
// GitLab again at once, after a merge from the overview (overview/merge.mjs).
screen.refresh = fetchRemote;
/** How far a key scrolls, given the rows of a page. */
const SCROLL = {
  j: () => 1,
  k: () => -1,
  "\x1b[B": () => 1,
  "\x1b[A": () => -1,
  "\x1b[6~": page => page,
  "\x1b[5~": page => -page,
  " ": page => page,
  g: () => -Infinity,
  G: () => Infinity,
};
/** Lines per notch of the wheel. */
const WHEEL = 3;
const press = key => {
  if (key === "q" || key === "\u0003") leave();
  if (key === "r") fetchRemote();
  if (key === "u" && screen.upTarget) startStack(screen.upTarget);
  if (SCROLL[key]) {
    scrollBy(SCROLL[key](pageOf(rows())));
    frame.scrolled();
  }
  // The pane lost the focus: the pointer is elsewhere, nothing stays lit.
  if (key === FOCUS_OUT) frame.hoverAt(null);
  const mouse = mouseOf(key);
  if (mouse?.kind === "move") frame.hoverAt(mouse.row - 1, mouse.column);
  if (mouse?.kind === "wheel") {
    scrollBy(mouse.by * WHEEL);
    frame.scrolled();
  }
  if (mouse?.kind === "click") frame.clickAt(mouse.row - 1, mouse.column);
};
const leave = () => {
  process.stdout.write(`${MOUSE_OFF}${WRAP_ON}\x1b[?25h\x1b[?1049l`);
  process.exit(0);
};
process.stdout.write(`\x1b[?1049h\x1b[?25l${WRAP_OFF}${MOUSE_ON}`);
process.on("SIGINT", leave);
process.on("SIGTERM", leave);
process.stdout.on("resize", frame.repaint);
// A resize the pane didn't signal (a hidden tab) still comes as one (#253).
watchSize(process.stdout);
process.stdin.setRawMode?.(true);
process.stdin.resume();
const keysOf = keyReader();
process.stdin.on("data", chunk => {
  for (const key of keysOf(chunk.toString())) press(key);
});
draw();
fetchRemote();
setInterval(fetchRemote, REMOTE_MS);
/**
 * `--herdr`: the layout follows what the local read sees. A worktree that
 * appears or goes rebuilds it, a stack that comes up or goes down changes its
 * tabs. Only on a change: the read runs every LOCAL_MS, herdr need not. The
 * claude conversation of each worktree's space is written down on every read,
 * for a restart of herdr to resume, the space of a worktree that is gone
 * closes, however it went, and each worktree agent's line in the sidebar says
 * what it waits for (agent-state.mjs); what needs you notifies
 * (notify.mjs).
 */
let known = null;
let isBusy = false;
const keepInStep = async () => {
  if (!isSyncing || isBusy) return;
  const now = new Map(
    local.trees.map(tree => [
      tree.dir,
      Boolean(tree.stack?.includes("running")),
    ]),
  );
  const before = known;
  known = now;
  isBusy = true;
  try {
    rememberSessions();
    pruneSpaces();
    // Cheap when the order stands: a plan or a handover may have changed it (#255).
    await sortSpaces();
    reportStates(local, remote);
    await notifyChanges(local, remote);
    if (!before || [...now.keys()].join() !== [...before.keys()].join()) {
      await layout();
      return;
    }
    for (const [dir, isUp] of now)
      if (before.get(dir) !== isUp) await syncStackTabs(dir);
  } finally {
    isBusy = false;
  }
};
keepInStep();
setInterval(async () => {
  // A read that fails keeps the last one: the next tick reads again.
  local = await read().catch(() => local);
  // Its worktree is gone (the ✕, or wt:remove elsewhere): nothing left to show.
  if (only && local.trees.length === 0) leave();
  draw();
  keepInStep();
}, LOCAL_MS);
