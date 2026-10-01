// A `try` action (`pnpm wt:action try`): something only the human can check,
// and the click that puts them at the spot. `--run <command>`: a terminal, the
// tab `try` of the worktree's space running the command in the worktree.
// `--url <page>`: the page on the worktree's app, its stack and `pnpm dev`
// started first when the app is down (as `▶ start the stack` does). The
// command runs only on the click, and the row shows it in full before.
// `--on test --url <path>`: the path on the test instance, whose origin comes
// from VITA_TEST_ORIGIN (never a hostname in the repo); try-state.mjs says
// when it is deployed, and before that a click does nothing. A click on a try
// runs that try: an older one in the tab `try` is closed first, a page waiting
// for the app becomes the new one (#295).

import fs from "node:fs";
import path from "node:path";

import { herdr, isInHerdr, tabsOf } from "../lib/herdr.mjs";
import { dim, green, red, yellow } from "../lib/tty.mjs";
import { envFileOf, isPortFree, mainDir, readEnv } from "../worktree.mjs";
import { spaceOf } from "../worktree/spaces.mjs";
import { ensureTab, idlePaneOf } from "../worktree/tabs.mjs";
import { isStarting, stackProgress, startStack } from "./actions.mjs";
import { open } from "./io.mjs";
import { screen } from "./screen.mjs";

/** The tab of a worktree's space a terminal check runs in; it goes with the space. */
export const TRY_TAB = "try";
/** How long a click waits for `pnpm dev` to answer after the stack is up. */
const APP_WAIT_MS = 120_000;
const APP_POLL_MS = 1000;

/** The app of a checkout whose stack has `offset`: `http://localhost:3002`. */
export const appOrigin = offset => `http://localhost:${3000 + offset}`;
/** The page an action names: a path (`/atlas/…`) on the worktree's app, anything else as it is. */
export const pageOf = (url, offset) =>
  url.startsWith("/") ? `${appOrigin(offset)}${url}` : url;
/** Whether `page` is on the worktree's app, the one a click starts when it is down. */
export const isOnApp = (page, offset) =>
  page === appOrigin(offset) || page.startsWith(`${appOrigin(offset)}/`);
/** A page as the row shows it: `:3002/atlas` for one on localhost. */
export const shortPage = page =>
  page.replace(/^https?:\/\/localhost(?=:)/, "").replace(/^(:\d+)\/$/, "$1");

/**
 * The origin of the test instance, `https://…` without a slash at the end:
 * `$VITA_TEST_ORIGIN`, else `VITA_TEST_ORIGIN` in the ignored `.env.local`
 * of vita in the main checkout; null without either.
 */
export const testOrigin = () => {
  const origin =
    process.env.VITA_TEST_ORIGIN ||
    readEnv(envFileOf(mainDir()))?.VITA_TEST_ORIGIN;
  return origin?.trim().replace(/\/+$/, "") || null;
};

/** Clicks on their way to a page: dir → `{ stage: "stack" | "app", error }`. */
const trying = new Map();
/** The worktrees whose try terminal a click couldn't open (no herdr, no space): their row says so. */
const unopened = new Set();
/**
 * The try a click started in the tab `try` of the worktree `dir`, `{ tab, id,
 * run }`: in the worktree's ignored `.claude`, so every overview knows it.
 */
export const tryTabFile = dir => path.join(dir, ".claude", "try-tab.json");
const runningIn = dir => {
  try {
    return JSON.parse(fs.readFileSync(tryTabFile(dir), "utf-8"));
  } catch {
    return null;
  }
};
const markRunning = (dir, running) => {
  try {
    fs.mkdirSync(path.dirname(tryTabFile(dir)), { recursive: true });
    fs.writeFileSync(tryTabFile(dir), `${JSON.stringify(running)}\n`);
  } catch {
    // Without it the next click replaces the tab, as for any unknown one.
  }
};
/** What the last click on a try of a worktree did, for its row (#290): dir → `{ id, text }`. */
const clicked = new Map();
/** What a page of a try opened in place of an older one says on its row. */
const replacedText = id =>
  id ? `opened, replaced the try of #${id}` : "opened";
/** What the click on `action` of `dir` did, when it was the last one there. */
const clickedText = (action, dir) => {
  const last = clicked.get(dir);
  return last && last.id === action.id ? last.text : null;
};

/**
 * What a click on the try `action` of the worktree `tree` does, for its row:
 * the command in full, or the page and whether the stack starts first; while
 * a click is at it, how far it got.
 */
export const trySpot = (action, tree) => {
  if (action.on === "test")
    return dim(`opens ${action.url} on the test instance`);
  if (action.run) {
    if (tree && unopened.has(tree.dir))
      return red(
        `no space of its worktree in herdr: run ${action.run} there yourself`,
      );
    const done = tree && clickedText(action, tree.dir);
    return done
      ? `${green(done)} ${dim(action.run)}`
      : dim(`opens a terminal: ${action.run}`);
  }
  if (!action.url) return "";
  if (!tree) return dim(`opens ${shortPage(action.url)}`);
  const page = pageOf(action.url, tree.offset);
  // A click under way shows on the row of the try whose page will open.
  const state =
    trying.get(tree.dir)?.id === action.id ? trying.get(tree.dir) : null;
  const error =
    state?.error ??
    (state?.stage === "stack" && stackProgress(tree.dir)?.error);
  if (error) return red(error);
  const replacing = state?.replaced
    ? `, replaced the try of #${state.replaced}`
    : "";
  if (state?.stage === "stack")
    return `${yellow(`◐ starting the stack …${replacing}`)} ${dim(stackProgress(tree.dir)?.line ?? "")}`;
  if (state?.stage === "app")
    return `${yellow(`◐ waiting for pnpm dev …${replacing}`)} ${dim(shortPage(page))}`;
  const done = clickedText(action, tree.dir);
  if (done) return `${green(done)} ${dim(shortPage(page))}`;
  const willStart =
    tree.hasOwnPort && tree.hasEnv && !tree.isUp && isOnApp(page, tree.offset);
  return dim(
    `opens ${shortPage(page)}${willStart ? " after starting the stack" : ""}`,
  );
};

/**
 * The tab `try` of the space of `dir` running the try `action`, focused. The
 * same try clicked again keeps its tab, its command runs again only when it
 * has ended; anything else in the tab (an older try, what the overview didn't
 * start) is closed first. What it did → the row, false outside herdr or
 * without a space.
 */
const openTerminal = (dir, action) => {
  const workspace = isInHerdr() && spaceOf(dir);
  if (!workspace) return false;
  const tabs = tabsOf(workspace);
  const old = tabs.filter(t => t.label === TRY_TAB);
  const known = runningIn(dir);
  const spec = { label: TRY_TAB, cwd: dir, command: action.run };
  let text;
  let closed = [];
  if (
    old.length === 1 &&
    known?.tab === old[0].tab_id &&
    known.id === action.id &&
    known.run === action.run
  ) {
    const isIdle = Boolean(idlePaneOf(workspace, TRY_TAB, tabs));
    ensureTab(workspace, spec, tabs);
    text = isIdle
      ? "runs again in the tab try"
      : "focused, runs in the tab try";
  } else {
    closed = old.map(t => t.tab_id);
    for (const tab of closed) herdr(["tab", "close", tab]);
    ensureTab(workspace, spec, []);
    text = !old.length
      ? "runs in the tab try"
      : known && old.some(t => t.tab_id === known.tab)
        ? `replaced the try of #${known.id}, runs in the tab try`
        : "replaced what ran in the tab try";
  }
  const tab = tabsOf(workspace).find(
    t => t.label === TRY_TAB && !closed.includes(t.tab_id),
  );
  if (!tab) return false;
  markRunning(dir, { tab: tab.tab_id, id: action.id, run: action.run });
  clicked.set(dir, { id: action.id, text });
  herdr(["workspace", "focus", workspace]);
  return Boolean(herdr(["tab", "focus", tab.tab_id]));
};

const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
/** Whether the app on `port` answers within APP_WAIT_MS. */
const waitForApp = async port => {
  for (let waited = 0; waited < APP_WAIT_MS; waited += APP_POLL_MS) {
    if (!(await isPortFree(port))) return true;
    await pause(APP_POLL_MS);
  }
  return false;
};
/**
 * The page of the try `action` in the tab group of the space of `tree`. On its
 * own app while that is down: its stack started first (`stack:up`, which
 * starts `pnpm dev` in the tab `dev`), then the page once the app answers. A
 * worktree on the shared stack has no app of its own to start: the page opens
 * as it is. A click while an older try waits for the app takes its place: the
 * page of the last click opens.
 */
const openPage = async (tree, action) => {
  const { dir, offset } = tree;
  const page = pageOf(action.url, offset);
  const state = trying.get(dir);
  // A click while one is at it: its page instead, the start goes on; after a failure it tries again.
  if (state && !state.error && !stackProgress(dir)?.error) {
    if (state.id !== action.id)
      Object.assign(state, { page, id: action.id, replaced: state.id });
    return screen.redraw();
  }
  const isOwn = tree.hasOwnPort && tree.hasEnv && isOnApp(page, offset);
  // A start of the stack already under way (▶ start the stack): the page waits for the app in the browser.
  if (!isOwn || isStarting(dir) || !(await isPortFree(3000 + offset))) {
    trying.delete(dir);
    clicked.set(dir, { id: action.id, text: "opened" });
    open(page, dir);
    return screen.redraw();
  }
  const next = {
    stage: "stack",
    error: null,
    page,
    id: action.id,
    replaced: null,
  };
  trying.set(dir, next);
  startStack(dir, async () => {
    next.stage = "app";
    screen.redraw();
    if (await waitForApp(3000 + offset)) {
      trying.delete(dir);
      clicked.set(dir, { id: next.id, text: replacedText(next.replaced) });
      open(next.page, dir);
    } else
      next.error = `the app on ${shortPage(appOrigin(offset))} didn't answer: see the tab dev`;
    screen.redraw();
  });
};

/**
 * A click on the try `action` of the worktree `tree` (past its buttons): the
 * terminal or the page it names. Without a spot, the worktree's space. One
 * that isn't ready (`state`, try-state.mjs) opens nothing, a failed
 * deployment its job.
 */
export const clickTry = (action, tree, fallback, state = null) => {
  if (state && !state.isReady) {
    if (state.url) open(state.url);
    return;
  }
  if (action.on === "test") {
    const origin = testOrigin();
    if (origin) open(`${origin}${action.url}`, tree?.dir);
    return;
  }
  if (action.run && tree) {
    // Without a terminal the row says why instead of a click doing nothing.
    if (openTerminal(tree.dir, action)) unopened.delete(tree.dir);
    else unopened.add(tree.dir);
    return screen.redraw();
  }
  if (action.url && tree) return openPage(tree, action);
  if (action.url) return open(action.url);
  return fallback();
};
