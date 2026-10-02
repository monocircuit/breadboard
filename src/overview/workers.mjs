// Agents are workers (#217), in the overview of an orchestration: under UP
// NEXT, for each queued issue the free agent that knows its code best, else
// the one with the most room, else a fresh one (#265). The live overview
// does it by itself (worktree/workers.mjs). A merged agent goes free with
// the clean-up of its worktree (#289) and is suggested from there.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { red } from "../lib/tty.mjs";
import { CONTEXT_LIMIT, ENTRY, filesOf, mainDir } from "../worktree.mjs";
import { suggestionsOf } from "../worktree/fit.mjs";
import { freeAgent } from "../worktree/free.mjs";
import { THRESHOLD, freeMemory, gb } from "../worktree/memory.mjs";
import { setupsOf } from "../worktree/setups.mjs";
import { agentNameOf } from "./handovers.mjs";
import { screen } from "./screen.mjs";

export { pathsOf, sharedOf, suggestionsOf } from "../worktree/fit.mjs";

/**
 * Before the clean-up of a merged worktree (#289): its agent goes free
 * (worktree/free.mjs) instead of closing with the space, unless its context
 * is above the limit: then it goes, a next issue would run out in the middle.
 * `local`: `{ agents, workers }` of the read. True when it was freed.
 */
export const freeOnCleanUp = (
  name,
  issue,
  mr,
  { agents, workers = {} },
  free = freeAgent,
) => {
  const { tree } = issue;
  const claude = tree?.claude;
  const agent = claude && agents.get(claude.id);
  if (!agent || mr?.state !== "merged") return false;
  const share = claude.window ? claude.used / claude.window : 0;
  if (share > CONTEXT_LIMIT) return false;
  return free(name, {
    id: claude.id,
    agent: agentNameOf(tree, workers),
    pane: agent.pane,
    dir: tree.dir,
    iid: mr.iid,
    files: filesOf(tree.dir),
  }).isFreed;
};

/**
 * An agent whose work waits for no one but the human: its MR merged, or
 * ready with its pipeline passed. Only such an agent is suggested, and only
 * a merged one gets a queued issue by itself.
 */
export const isDoneMr = mr =>
  mr?.state === "merged" ||
  (mr?.state === "opened" && !mr.isDraft && mr.pipeline?.status === "success");
/**
 * The free workers of an orchestration: an agent of one of its worktrees
 * that herdr sees idle (or done), whose MR is done and whose context is
 * below the limit. `{ name, share, files, dir, isMerged, issue }`,
 * `files` those of its worktree and, from the record, of its earlier ones.
 * Then the free agents of `free` (#289, `{ id, name, files, share }`, its
 * worktree gone) herdr sees idle and below the limit: `isFree`, `dir` null.
 */
export const freeWorkersOf = (issues, of, agents, known = {}, free = []) => [
  ...issues.flatMap(issue => {
    const { tree } = issue;
    const claude = tree?.claude;
    const agent = claude && agents.get(claude.id);
    const { mr } = of(issue);
    if (!agent || !["idle", "done"].includes(agent.status) || !isDoneMr(mr))
      return [];
    const share = claude.window ? claude.used / claude.window : 0;
    if (share > CONTEXT_LIMIT) return [];
    const record = known[claude.id];
    return [
      {
        name: record?.name ?? tree.name,
        share,
        files: [...new Set([...(record?.files ?? []), ...filesOf(tree.dir)])],
        dir: tree.dir,
        isMerged: mr.state === "merged",
        issue,
      },
    ];
  }),
  // The free agents (#289), their worktree gone: idle, below the limit.
  ...free.flatMap(agent => {
    const status = agents.get(agent.id)?.status;
    if (!["idle", "done"].includes(status) || agent.share > CONTEXT_LIMIT)
      return [];
    return [
      {
        name: agent.name,
        share: agent.share ?? 0,
        files: agent.files,
        dir: null,
        isMerged: true,
        isFree: true,
        issue: null,
      },
    ];
  }),
];

/**
 * Which queued issues (in the order they wait) go to their suggested agent
 * now (`suggested`, suggestionsOf): those whose agent is merged. An agent
 * only ready, not merged, may still be needed for its own MR. No count holds
 * one back (#229); an issue no agent is free for starts a fresh one (freshOf).
 */
export const handOutOf = (queued, suggested) =>
  queued.flatMap(({ number }) => {
    const worker = suggested.get(number)?.worker;
    return worker?.isMerged ? [{ number, worker }] : [];
  });

/**
 * number → its start or handover running in the background, or what stopped
 * it: `{ isRunning, error, ended, dir, isFresh }`, `dir` the worktree of the
 * agent it hands the issue to, held until the agent moved (its issue is no
 * longer queued once wt:new made the worktree), `isFresh` for a fresh agent.
 */
const running = new Map();
/**
 * `wt:orchestrate <name> <number> <args…>` detached: wt:new takes minutes,
 * the overview goes on. Without the overview's pane, so a start doesn't move
 * the overview's tab as the orchestrator's (openOrchestrator).
 */
const runInBackground = (
  name,
  number,
  args,
  { dir = null, isFresh = false, to = null } = {},
) => {
  if (running.get(number)?.isRunning) return;
  const log = path.join(os.tmpdir(), `overview-start-${name}-${number}.log`);
  const out = fs.openSync(log, "w");
  const env = { ...process.env, NO_COLOR: "1" };
  delete env.HERDR_PANE_ID;
  delete env.HERDR_TAB_ID;
  const child = spawn(
    process.execPath,
    [ENTRY, "orchestrate", name, String(number), ...args],
    { cwd: mainDir(), env, stdio: ["ignore", out, out], detached: true },
  );
  fs.closeSync(out);
  child.unref();
  const state = { isRunning: true, error: null, ended: 0, dir, isFresh, to };
  running.set(number, state);
  child.on("exit", code => {
    state.isRunning = false;
    state.ended = Date.now();
    if (code !== 0) {
      const last = fs.readFileSync(log, "utf-8").trim().split("\n").at(-1);
      state.error = last?.replace(/^✗ /, "") || `ended with ${code}`;
    }
    screen.redraw({ isReread: true });
  });
  screen.redraw();
};
/** `worker`: `{ name }` of a free worker, or an announced handover's `{ to }`. */
const handOver = (name, number, worker, run = runInBackground) =>
  run(name, number, ["--to", worker.name ?? worker.to], {
    dir: worker.dir ?? worker.tree?.dir ?? null,
    to: worker.name ?? worker.to,
  });

/** How long a failed announced handover waits before it is tried again. */
export const RETRY_MS = 60_000;
/**
 * Whether the announced `handover` goes now: due, not running, and not
 * tried yet, or failed at least RETRY_MS ago (`state`: its run, `now` ms).
 * A try fails when the agent took up something during the minutes wt:new
 * takes (#244); the worktree it made is taken as it is on the next one.
 */
export const isHandOverNow = (handover, state, now = Date.now()) =>
  handover.isDue &&
  (!state || (!state.isRunning && now - state.ended >= RETRY_MS));

/** What an announced handover's row says after its issue and agent, the MRs merged. */
const announcedText = (handover, state, shown) => {
  if (state?.error)
    return `failed: ${red(state.error)}; tries again once ${shown(handover.to)} is idle`;
  if (handover.why) return `waits: ${handover.why}`;
  return handover.isDue ? "now" : "once it is idle";
};
/** A fresh agent for a queued issue, no idle one asked again: the overview found none that is free. */
const startFresh = (name, number, run = runInBackground) =>
  run(name, number, ["--now"], { isFresh: true });

/**
 * Which of the queued issues no agent is free for (`unsuggested`, in the
 * order they wait, `{ number }`) start a fresh agent now, memory permitting
 * (#229's threshold: its own stack fits next to it): `free` the memory free
 * now, `reserved` what the fresh agents still starting take. Returns
 * `{ starts, left }`, `starts` the numbers, `left` the memory after them.
 * Below the threshold an issue waits for an agent to get free, or a click.
 */
export const freshOf = (unsuggested, { free, reserved = 0 }) => {
  let left = free - reserved;
  const starts = [];
  for (const { number } of unsuggested) {
    if (left < THRESHOLD) break;
    starts.push(number);
    left -= THRESHOLD;
  }
  return { starts, left };
};

/**
 * The MRs of `after` (iids) a queued issue still waits for (#272): not
 * merged, or not known yet (`awaited`: iid → `{ state }`, null before GitLab
 * answered). Such an issue goes to no agent, nor starts a fresh one.
 */
export const openAfterOf = (after, awaited) =>
  (after ?? []).filter(iid => awaited?.get(iid)?.state !== "merged");
/**
 * The queued issues in the order they start: the work order of wt:plan
 * (`work`), then when they were queued (#250), then by number; the queue of
 * WORKTREES shows the same order (tree.mjs).
 */
export const queueOrder = (queued, work = []) => {
  const rank = ({ number }) => {
    const at = work.indexOf(number);
    return at === -1 ? Infinity : at;
  };
  return queued.toSorted(
    (a, b) =>
      (rank(a) === rank(b) ? 0 : rank(a) < rank(b) ? -1 : 1) ||
      (a.issue.queuedAt ?? "").localeCompare(b.issue.queuedAt ?? "") ||
      a.number - b.number,
  );
};

/** How much of its context an agent has used, for a row: `18 %`. */
const percent = share => `${Math.round(share * 100)} %`;
/** `free, shares a.mjs, 18 %` / `once its MR is merged, 18 %`: why a suggested agent, and when. */
const workerText = ({ worker, shared }) =>
  [
    worker.isMerged ? "free" : "once its MR is merged",
    shared.length ? `shares ${shared.join(", ")}` : "",
    percent(worker.share),
  ]
    .filter(Boolean)
    .join(", ");
/**
 * The rows of UP NEXT for the workers of the orchestration `name`, as
 * automatic actions (`{ key, kind, text, at, issue, who, go }`): per queued
 * issue its suggested agent, `hand over`, else `start`, a fresh agent.
 * `held`: the worktrees of the agents a handover is moving now (`holds`:
 * dir → why), whose clean-up waits until they moved. Any other merged agent
 * goes free with its clean-up (#289, worktree/free.mjs) and stays for the
 * next issue: `free`, the free agents (`{ id, name, files, share }`), are
 * suggested like the others. `suggested`: number → `{ worker, shared }`, `fresh`: number →
 * what its fresh start does, both for the queue of WORKTREES (tree.mjs).
 *
 * `shouldHandOut` (the one live overview, once GitLab answered): a queued
 * issue goes to its suggested agent by itself once that one is merged; one
 * no agent is free for starts a fresh agent by itself while `memory` (free,
 * bytes) allows (freshOf), else it waits for one to get free or a click.
 *
 * `announced` (overview/handovers.mjs): the handovers the orchestrator
 * announced (#227). Their issues and agents are no one else's; each is a
 * row of its own, done by itself once due.
 *
 * `setups` (worktree/setups.mjs): the starts running now, from any process
 * (#272). Their agents are no one else's either, their issues not queued
 * any more, and the worktrees their agents leave are held. `awaited`: iid →
 * `{ state }` of the MRs queued issues wait for (`--queue --after`), `work`:
 * the work order of wt:plan, the order the queue starts in. `shown`: the
 * name an agent is shown by (#292, handovers.mjs `agentNamesOf`), its rows
 * name it so; their keys and what they do keep the name it goes by.
 */
export const workerActionsOf = (
  name,
  issues,
  of,
  agents,
  known,
  {
    shouldHandOut = false,
    announced = [],
    run = runInBackground,
    memory = freeMemory(),
    setups = setupsOf(name),
    awaited = null,
    work = [],
    shown = who => who,
    free: freeAgents = [],
  } = {},
) => {
  // An agent a start moves takes nothing else meanwhile: the queue gave
  // the agent of #268 both #267 and #269 while wt:new ran (#272).
  const promised = new Set([
    ...announced.map(({ tree }) => tree?.dir),
    ...setups.map(setup => setup.from),
    ...[...running.values()]
      .filter(state => state.isRunning)
      .map(state => state.dir),
  ]);
  // A free agent (#289) has no worktree: it is promised by its name.
  const promisedTo = new Set([
    ...announced.map(({ to }) => to),
    ...setups.map(setup => setup.to),
    ...[...running.values()]
      .filter(state => state.isRunning)
      .map(state => state.to),
  ]);
  const free = freeWorkersOf(issues, of, agents, known, freeAgents).filter(
    worker =>
      worker.isFree ? !promisedTo.has(worker.name) : !promised.has(worker.dir),
  );
  const settingUp = new Set(setups.map(setup => setup.number));
  for (const issue of issues) {
    const state = running.get(issue.number);
    // Queued again since its start failed (by hand): the old failure is over.
    if (
      state &&
      !state.isRunning &&
      Date.parse(issue.queuedAt ?? "") > state.ended
    )
      running.delete(issue.number);
  }
  const all = queueOrder(
    issues
      .filter(
        issue =>
          issue.queued &&
          !announced.some(({ number }) => number === issue.number) &&
          // Set up by another process: its worktree's node says so.
          !(settingUp.has(issue.number) && !running.has(issue.number)),
      )
      .map(issue => ({
        number: issue.number,
        issue,
        paths: of(issue).issue?.paths ?? [],
        open: openAfterOf(issue.after, awaited),
      })),
    work,
  );
  // One that needs an open MR waits for it, for an agent and a fresh start alike.
  const queued = all.filter(({ open }) => open.length === 0);
  const suggested = suggestionsOf(queued, free);
  // Not one started already, nor one that failed: it would fail again on every read, a click retries it.
  const waiting = queued.filter(({ number }) => !running.has(number));
  if (shouldHandOut)
    for (const { number, worker } of handOutOf(waiting, suggested))
      handOver(name, number, worker, run);
  // The fresh agents still starting take their memory once their stacks are up.
  const reserved =
    [...running.values()].filter(state => state.isFresh && state.isRunning)
      .length * THRESHOLD;
  const { starts, left } = freshOf(
    waiting.filter(({ number }) => !suggested.has(number)),
    { free: memory, reserved },
  );
  if (shouldHandOut) for (const number of starts) startFresh(name, number, run);
  // One that waits for an MR is no row: nothing to do; its queue node says what it waits for (tree.mjs).
  const actions = [];
  for (const handover of announced) {
    const { number, to } = handover;
    const state = running.get(number);
    if (shouldHandOut && isHandOverNow(handover, state))
      handOver(name, number, handover, run);
    actions.push({
      key: `hand over:#${number}:${to}`,
      kind: "hand over",
      who: shown(to),
      issue: issues.find(candidate => candidate.number === number),
      at: handover.at ?? "",
      // Under way: NEXT doesn't name it (next.mjs `isClickable`).
      isRunning: Boolean(running.get(number)?.isRunning),
      text: running.get(number)?.isRunning
        ? `#${number} → ${shown(to)}: handing over …`
        : `#${number} → ${shown(to)} ${handover.open.length ? `after ${handover.open.map(iid => `!${iid}`).join(", ")}` : announcedText(handover, state, shown)} (announced)`,
      go: () => handOver(name, number, handover),
    });
  }
  // What the fresh start of each issue no agent is free for does, for its row and its queue node.
  const fresh = new Map();
  for (const { number } of queued) {
    if (suggested.has(number)) continue;
    const state = running.get(number);
    fresh.set(
      number,
      state?.isRunning
        ? "starting a fresh agent …"
        : state?.error
          ? `${state.isFresh ? "its fresh agent" : `its handover to ${shown(state.to)}`} failed: ${red(state.error)}; a click tries again`
          : starts.includes(number)
            ? "starts a fresh agent"
            : `waits for an agent to get free: ${gb(Math.max(0, left))} free, below ${gb(THRESHOLD)} for a fresh one; a click starts one on the shared stack`,
    );
  }
  for (const { number, issue } of queued) {
    const found = suggested.get(number);
    const state = running.get(number);
    // Queued until its agent moved in (#272): its handover's row meanwhile.
    if (state?.isRunning && state.to) {
      actions.push({
        key: `hand over:#${number}:${state.to}`,
        kind: "hand over",
        who: shown(state.to),
        issue,
        at: "",
        isRunning: true,
        text: `#${number} → ${shown(state.to)}: handing over …`,
        go: () => {},
      });
      continue;
    }
    if (!found) {
      actions.push({
        key: `start:#${number}`,
        kind: "start",
        who: issue.feature,
        issue,
        at: "",
        isRunning: Boolean(state?.isRunning),
        text: `#${number} queued (--queue), no agent free: ${fresh.get(number)}`,
        go: () => startFresh(name, number),
      });
      continue;
    }
    const { worker } = found;
    const error = state?.error ? `  ${red(state.error)}` : "";
    actions.push({
      key: `hand over:#${number}:${worker.name}`,
      kind: "hand over",
      who: shown(worker.name),
      issue,
      at: "",
      isRunning: Boolean(state?.isRunning),
      text: state?.isRunning
        ? `#${number} → ${shown(worker.name)}: handing over …`
        : `#${number} → ${shown(worker.name)} (${workerText(found)})${error}`,
      go: () => handOver(name, number, worker),
    });
  }
  // Why the clean-up of each held worktree waits, for its node (tree.mjs):
  // only while its agent moves out. A suggested or announced agent is freed
  // with the clean-up instead (#289) and takes its issue from there.
  const holds = new Map([
    ...[...running]
      .filter(([, state]) => state.isRunning && state.dir)
      .map(([number, state]) => [state.dir, `its agent takes #${number} now`]),
    ...setups
      .filter(setup => setup.from)
      .map(setup => [setup.from, `its agent takes #${setup.number} now`]),
  ]);
  return {
    actions,
    held: new Set(holds.keys()),
    holds,
    suggested,
    fresh,
    setups,
  };
};
