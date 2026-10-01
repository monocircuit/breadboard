// A `try` action (`pnpm wt:action try`): something only the human can check,
// and the click that puts them at the spot. `--run <command>`: a terminal, the
// tab `try` of the worktree's space running the command in the worktree.
// `--url <page>`: the page on the worktree's app, its stack and `pnpm dev`
// started first when the app is down (as `▶ start the stack` does). The
// command runs only on the click, and the row shows it in full before.
// `--on test --url <path>`: the path on the test instance, whose origin comes
// from VITA_TEST_ORIGIN (never a hostname in the repo); try-state.mjs says
// when it is deployed, and before that a click does nothing.

import { herdr, isInHerdr, tabsOf } from "../lib/herdr.mjs";
import { dim, red, yellow } from "../lib/tty.mjs";
import { envFileOf, isPortFree, mainDir, readEnv } from "../worktree.mjs";
import { spaceOf } from "../worktree/spaces.mjs";
import { ensureTab } from "../worktree/tabs.mjs";
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
 * What a click on the try `action` of the worktree `tree` does, for its row:
 * the command in full, or the page and whether the stack starts first; while
 * a click is at it, how far it got.
 */
export const trySpot = (action, tree) => {
  if (action.on === "test")
    return dim(`opens ${action.url} on the test instance`);
  if (action.run)
    return tree && unopened.has(tree.dir)
      ? red(
          `no space of its worktree in herdr: run ${action.run} there yourself`,
        )
      : dim(`opens a terminal: ${action.run}`);
  if (!action.url) return "";
  if (!tree) return dim(`opens ${shortPage(action.url)}`);
  const page = pageOf(action.url, tree.offset);
  const state = trying.get(tree.dir);
  const error =
    state?.error ??
    (state?.stage === "stack" && stackProgress(tree.dir)?.error);
  if (error) return red(error);
  if (state?.stage === "stack")
    return `${yellow("◐ starting the stack …")} ${dim(stackProgress(tree.dir)?.line ?? "")}`;
  if (state?.stage === "app")
    return `${yellow("◐ waiting for pnpm dev …")} ${dim(shortPage(page))}`;
  const willStart =
    tree.hasOwnPort && tree.hasEnv && !tree.isUp && isOnApp(page, tree.offset);
  return dim(
    `opens ${shortPage(page)}${willStart ? " after starting the stack" : ""}`,
  );
};

/**
 * The tab `try` of the space of `dir` running `command` in `dir`, focused: a
 * second click focuses the tab that is there (its shell idle, the command
 * runs again). False outside herdr or without a space.
 */
const openTerminal = (dir, command) => {
  const workspace = isInHerdr() && spaceOf(dir);
  if (!workspace) return false;
  ensureTab(workspace, { label: TRY_TAB, cwd: dir, command });
  const tab = tabsOf(workspace).find(t => t.label === TRY_TAB);
  if (!tab) return false;
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
 * The page of `url` in the tab group of the space of `tree`. On its own app
 * while that is down: its stack started first (`stack:up`, which starts `pnpm
 * dev` in the tab `dev`), then the page once the app answers. A worktree on
 * the shared stack has no app of its own to start: the page opens as it is.
 */
const openPage = async (tree, url) => {
  const { dir, offset } = tree;
  const page = pageOf(url, offset);
  const state = trying.get(dir);
  // A click while one is at it waits for that one; after a failure it tries again.
  if (state && !state.error && !stackProgress(dir)?.error) return;
  const isOwn = tree.hasOwnPort && tree.hasEnv && isOnApp(page, offset);
  // A start of the stack already under way (▶ start the stack): the page waits for the app in the browser.
  if (!isOwn || isStarting(dir) || !(await isPortFree(3000 + offset))) {
    trying.delete(dir);
    open(page, dir);
    return;
  }
  const next = { stage: "stack", error: null };
  trying.set(dir, next);
  startStack(dir, async () => {
    next.stage = "app";
    screen.redraw();
    if (await waitForApp(3000 + offset)) {
      trying.delete(dir);
      open(page, dir);
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
    if (openTerminal(tree.dir, action.run)) unopened.delete(tree.dir);
    else unopened.add(tree.dir);
    return screen.redraw();
  }
  if (action.url && tree) return openPage(tree, action.url);
  if (action.url) return open(action.url);
  return fallback();
};
