// Agents are workers (#217), in the overview of an orchestration: under UP
// NEXT, for each queued issue the idle agent that knows its code best (a
// click hands it over, worktree/workers.mjs); its clean-up waits meanwhile.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { red } from "../lib/tty.mjs";
import { CAP, CONTEXT_LIMIT, ENTRY, filesOf, mainDir } from "../worktree.mjs";
import { screen } from "./screen.mjs";

/** A file path an issue names: with a folder or not, with an extension of the repo's files. */
const PATH =
  /(?:[\w@.-]+\/)*[\w@-][\w@.-]*\.(?:mjs|cjs|js|jsx|ts|tsx|json|md|css|scss|sql|toml|ya?ml)\b/g;
/** The file paths `text` (an issue's description) names, each once. */
export const pathsOf = text => [...new Set(String(text ?? "").match(PATH))];
/** The paths of `paths` a file of `files` is: the same, or `files` has it under a folder. */
export const sharedOf = (paths, files) =>
  paths.filter(name =>
    files.some(file => file === name || file.endsWith(`/${name}`)),
  );
/**
 * For each issue (`{ number, paths }`, in the order they wait), the free
 * worker (`{ name, share, files }`) that shares the most of its paths, the
 * one with the most room in its context on a tie: `number → { worker,
 * shared }`. Each worker gets one issue; one sharing nothing gets none.
 */
export const suggestionsOf = (issues, workers) => {
  const taken = new Set();
  const suggested = new Map();
  for (const issue of issues) {
    const [best] = workers
      .filter(worker => !taken.has(worker))
      .map(worker => ({ worker, shared: sharedOf(issue.paths, worker.files) }))
      .filter(({ shared }) => shared.length > 0)
      .toSorted(
        (a, b) =>
          b.shared.length - a.shared.length || a.worker.share - b.worker.share,
      );
    if (!best) continue;
    taken.add(best.worker);
    suggested.set(issue.number, best);
  }
  return suggested;
};
/**
 * An agent whose work waits for no one but the human: its MR merged, or
 * ready with its pipeline passed. Only such an agent is suggested, and only
 * a merged one gets a queued issue by itself.
 */
const isDoneMr = mr =>
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
 * How many agents of an orchestration are at work: one per worktree whose
 * MR isn't done yet. An agent idle between two turns still holds its issue.
 */
export const atWorkOf = (issues, of) =>
  issues.filter(issue => issue.tree && !isDoneMr(of(issue).mr)).length;
/**
 * Which queued issues (in the order they wait) start now, `slots` of them:
 * each to the agent suggested for it when that one is merged (`suggested`,
 * suggestionsOf), else to a fresh agent (`worker: null`). An agent only
 * ready, not merged, may still be needed for its own MR.
 */
export const handOutOf = (queued, suggested, slots) =>
  queued.slice(0, Math.max(0, slots)).map(({ number }) => {
    const worker = suggested.get(number)?.worker;
    return { number, worker: worker?.isMerged ? worker : null };
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
/** A fresh agent for a queued issue, past the cap's check: the overview counted already. */
const startFresh = (name, number) => runInBackground(name, number, ["--now"]);

/** How much of its context an agent has used, for a row: `18 %`. */
const percent = share => `${Math.round(share * 100)} %`;
/**
 * The rows of UP NEXT for the workers of the orchestration `name`: per
 * queued issue its suggested agent, `hand over`, as automatic actions
 * (`{ key, kind, text, at, issue, who, go }`), and `held`, the worktrees of
 * the suggested agents. Their clean-up waits: it would close the agent with
 * the space. An agent nothing fits closes with its clean-up, its
 * conversation kept for `/resume`.
 *
 * `shouldHandOut` (the one live overview, once GitLab answered): queued
 * issues start by themselves while fewer than `cap` agents are at work.
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
  { cap = CAP, shouldHandOut = false, announced = [] } = {},
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
    const starting = [...running.values()].filter(s => s.isRunning).length;
    // Not one started already, nor one that failed: it would fail again on every read, a click retries it.
    const waiting = queued.filter(({ number }) => !running.has(number));
    const slots = cap - atWorkOf(issues, of) - starting;
    for (const { number, worker } of handOutOf(waiting, suggested, slots)) {
      if (worker) handOver(name, number, worker);
      else startFresh(name, number);
    }
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
    if (!found) continue;
    const { worker, shared } = found;
    const state = running.get(number);
    actions.push({
      key: `hand over:#${number}:${worker.name}`,
      kind: "hand over",
      who: worker.name,
      issue,
      at: "",
      text: state?.isRunning
        ? `#${number} → ${worker.name}: handing over …`
        : `#${number} → ${worker.name} (shares ${shared.join(", ")}, ${percent(worker.share)})${state?.error ? `  ${red(state.error)}` : ""}`,
      go: () => handOver(name, number, worker),
    });
  }
  return {
    actions,
    held: new Set([
      ...[...suggested.values()].map(({ worker }) => worker.dir),
      ...[...promised].filter(Boolean),
    ]),
  };
};
