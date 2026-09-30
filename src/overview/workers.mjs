// Agents are workers (#217), in the overview of an orchestration: under UP
// NEXT, for each queued issue the idle agent that knows its code best (a
// click hands it over, worktree/workers.mjs); its clean-up waits meanwhile.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { red } from "../lib/tty.mjs";
import { CONTEXT_LIMIT, ENTRY, filesOf, mainDir } from "../worktree.mjs";
import { suggestionsOf } from "../worktree/fit.mjs";
import { takesText } from "./handovers.mjs";
import { screen } from "./screen.mjs";

export { pathsOf, sharedOf, suggestionsOf } from "../worktree/fit.mjs";

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
 */
export const freeWorkersOf = (issues, of, agents, known = {}) =>
  issues.flatMap(issue => {
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
  });

/**
 * Which queued issues (in the order they wait) go to their suggested agent
 * now (`suggested`, suggestionsOf): those whose agent is merged. An agent
 * only ready, not merged, may still be needed for its own MR. No count holds
 * one back (#229); an issue nothing fits waits for a click (start fresh).
 */
export const handOutOf = (queued, suggested) =>
  queued.flatMap(({ number }) => {
    const worker = suggested.get(number)?.worker;
    return worker?.isMerged ? [{ number, worker }] : [];
  });

/** number → its start or handover running in the background, or what stopped it. */
const running = new Map();
/**
 * `wt:orchestrate <name> <number> <args…>` detached: wt:new takes minutes,
 * the overview goes on. Without the overview's pane, so a start doesn't move
 * the overview's tab as the orchestrator's (openOrchestrator).
 */
const runInBackground = (name, number, args) => {
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
  const state = { isRunning: true, error: null };
  running.set(number, state);
  child.on("exit", code => {
    state.isRunning = false;
    if (code !== 0) {
      const last = fs.readFileSync(log, "utf-8").trim().split("\n").at(-1);
      state.error = last?.replace(/^✗ /, "") || `ended with ${code}`;
    }
    screen.redraw({ isReread: true });
  });
  screen.redraw();
};
/** `worker`: `{ name }` of a free worker, or an announced handover's `{ to }`. */
const handOver = (name, number, worker) =>
  runInBackground(name, number, ["--to", worker.name ?? worker.to]);
/** A fresh agent for a queued issue, no idle one asked again: the overview found none that fits. */
const startFresh = (name, number) => runInBackground(name, number, ["--now"]);

/** How much of its context an agent has used, for a row: `18 %`. */
const percent = share => `${Math.round(share * 100)} %`;
/**
 * The rows of UP NEXT for the workers of the orchestration `name`, as
 * automatic actions (`{ key, kind, text, at, issue, who, go }`): per queued
 * issue its suggested agent, `hand over`, else `start`, a fresh agent on a
 * click. `held`: the worktrees of the suggested and announced agents
 * (`holds`: dir → why), whose clean-up waits, it would close the agent with
 * the space. An agent nothing fits closes with its clean-up, its
 * conversation kept for `/resume`. `suggested`: number → `{ worker, shared
 * }`, for AGENTS (agents.mjs).
 *
 * `shouldHandOut` (the one live overview, once GitLab answered): a queued
 * issue goes to its suggested agent by itself once that one is merged.
 *
 * `announced` (overview/handovers.mjs): the handovers the orchestrator
 * announced (#227). Their issues and agents are no one else's; each is a
 * row of its own, done by itself once due, and its agent's worktree is held.
 */
export const workerActionsOf = (
  name,
  issues,
  of,
  agents,
  known,
  { shouldHandOut = false, announced = [] } = {},
) => {
  const promised = new Set(announced.map(({ tree }) => tree?.dir));
  const free = freeWorkersOf(issues, of, agents, known).filter(
    worker => !promised.has(worker.dir),
  );
  const queued = issues
    .filter(
      issue =>
        issue.queued &&
        !announced.some(({ number }) => number === issue.number),
    )
    .map(issue => ({
      number: issue.number,
      issue,
      paths: of(issue).issue?.paths ?? [],
    }));
  const suggested = suggestionsOf(queued, free);
  if (shouldHandOut) {
    // Not one started already, nor one that failed: it would fail again on every read, a click retries it.
    const waiting = queued.filter(({ number }) => !running.has(number));
    for (const { number, worker } of handOutOf(waiting, suggested))
      handOver(name, number, worker);
  }
  const actions = [];
  for (const handover of announced) {
    const { number, to } = handover;
    const state = running.get(number);
    if (shouldHandOut && handover.isDue && !state)
      handOver(name, number, handover);
    actions.push({
      key: `hand over:#${number}:${to}`,
      kind: "hand over",
      who: to,
      issue: issues.find(candidate => candidate.number === number),
      at: handover.at ?? "",
      text: running.get(number)?.isRunning
        ? `#${number} → ${to}: handing over …`
        : `#${number} → ${to} ${handover.open.length ? `after ${handover.open.map(iid => `!${iid}`).join(", ")}` : "once it is idle"} (announced)${state?.error ? `  ${red(state.error)}` : ""}`,
      go: () => handOver(name, number, handover),
    });
  }
  for (const { number, issue } of queued) {
    const found = suggested.get(number);
    const state = running.get(number);
    const error = state?.error ? `  ${red(state.error)}` : "";
    if (!found) {
      actions.push({
        key: `start:#${number}`,
        kind: "start",
        who: issue.feature,
        issue,
        at: "",
        text: state?.isRunning
          ? `#${number}: starting a fresh agent …`
          : `#${number} queued (--queue), no idle agent knows its files: a click starts a fresh one${error}`,
        go: () => startFresh(name, number),
      });
      continue;
    }
    const { worker, shared } = found;
    actions.push({
      key: `hand over:#${number}:${worker.name}`,
      kind: "hand over",
      who: worker.name,
      issue,
      at: "",
      text: state?.isRunning
        ? `#${number} → ${worker.name}: handing over …`
        : `#${number} → ${worker.name} (shares ${shared.join(", ")}, ${percent(worker.share)})${error}`,
      go: () => handOver(name, number, worker),
    });
  }
  // Why the clean-up of each held worktree waits, for its node (tree.mjs).
  const holds = new Map([
    ...[...suggested].map(([number, { worker }]) => [
      worker.dir,
      `its agent is suggested for #${number}`,
    ]),
    ...announced
      .filter(({ tree }) => tree)
      .map(handover => [handover.tree.dir, takesText(handover)]),
  ]);
  return { actions, held: new Set(holds.keys()), holds, suggested };
};
