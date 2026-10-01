// The end of an orchestration (`wt:orchestrate <name> --end`, the ✕ of its
// overview): its space closes, the orchestrator's claude with it, and its
// record and checkout go. The conversation stays resumable from the main
// checkout, and its changelog stays where it is (changelog.mjs). Only once
// none of its worktrees is left.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  currentPane,
  herdr,
  isInHerdr,
  panesOf,
  spaces,
} from "../lib/herdr.mjs";
import { fail, step } from "../lib/proc.mjs";
import { changelogFile } from "./changelog.mjs";
import { ENTRY } from "./constants.mjs";
import {
  isConversationRunning,
  projectOf,
  transcriptOf,
} from "./conversations.mjs";
import {
  checkoutOf,
  orchestratorCheckouts,
  removeOrchestratorCheckout,
} from "./orchestrator.mjs";
import {
  orchestrationOf,
  orchestratorLabel,
  originOf,
  recordFile,
  recordOf,
} from "./origins.mjs";
import { featureOf } from "./tabs.mjs";
import { mainDir, worktrees } from "./worktrees.mjs";
import { setupsDir } from "./setups.mjs";

/** The worktrees of the orchestration `name` that are still there, by feature: while one is, it can't end. */
export const worktreesLeft = name =>
  worktrees()
    .slice(1)
    .filter(({ dir }) => originOf(dir)?.name === name)
    .map(({ dir }) => featureOf(dir));
/** Why `name` can't end yet, or null. */
export const whyNotEnd = name => {
  const left = worktreesLeft(name);
  if (left.length)
    return `worktree${left.length === 1 ? "" : "s"} left: ${left.join(", ")}`;
  const queued = (recordOf(name)?.issues ?? []).filter(issue => issue.queued);
  return queued.length
    ? `queued: ${queued.map(issue => `#${issue.number}`).join(", ")}`
    : null;
};
/** The space `⇶ <name>`, or null (none, or outside herdr). */
const spaceOfOrchestration = name =>
  isInHerdr()
    ? (spaces().find(w => orchestrationOf(w.label) === name) ?? null)
    : null;
const sleep = ms =>
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
/**
 * The transcript of a conversation into the project of the main checkout, so
 * `/resume` there lists it: one started in another directory (the checkout of
 * the orchestration, say) would wait for shareConversations until it is ten
 * minutes old. Its claude has ended; one still running after `waitMs` is left
 * alone. What moved, or null.
 */
export const keepConversation = (id, waitMs = 5000) => {
  const until = Date.now() + waitMs;
  while (isConversationRunning(id) && Date.now() < until) sleep(100);
  const transcript = transcriptOf(id);
  if (!transcript || isConversationRunning(id)) return null;
  const home = projectOf(mainDir());
  const from = path.dirname(transcript);
  fs.mkdirSync(home, { recursive: true });
  if (fs.realpathSync(from) === fs.realpathSync(home)) return null;
  // The transcript and the folder of its subagents, if any; nothing that is there already is replaced.
  for (const name of [`${id}.jsonl`, id]) {
    const source = path.join(from, name);
    const target = path.join(home, name);
    if (fs.existsSync(source) && !fs.existsSync(target))
      fs.renameSync(source, target);
  }
  return `the conversation ${id} into the main checkout's project`;
};
/**
 * Ends the orchestration `name`: its space closes (the orchestrator's claude
 * and the overview with it), each claude that ran in it is kept for
 * `/resume` in the main checkout, then its record and its checkout go (its
 * changelog stays, the log names where) — what
 * pruneOrchestrators does, without waiting for the orchestrator to end.
 * Returns `{ error }` when it can't, else `{ done }`, the lines of what it did.
 */
export const endOrchestration = name => {
  const why = whyNotEnd(name);
  if (why) return { error: `${orchestratorLabel(name)} can't end, ${why}` };
  const done = [];
  const workspace = spaceOfOrchestration(name);
  if (workspace) {
    const ids = panesOf(workspace.workspace_id)
      .filter(p => p.agent === "claude" && p.agent_session?.value)
      .map(p => p.agent_session.value);
    if (!herdr(["workspace", "close", workspace.workspace_id]))
      return { error: `herdr did not close the space ${workspace.label}` };
    done.push(`closed the space ${workspace.label}`);
    for (const id of ids) {
      const kept = keepConversation(id);
      done.push(kept ? `moved ${kept}` : `kept the conversation ${id}`);
    }
  }
  if (fs.existsSync(recordFile(name))) {
    fs.rmSync(recordFile(name), { force: true });
    done.push(
      `removed the record ${path.relative(mainDir(), recordFile(name))}`,
    );
  }
  // Its setups' claims (setups.mjs): none runs once its worktrees are gone.
  fs.rmSync(setupsDir(name), { recursive: true, force: true });
  if (orchestratorCheckouts().includes(checkoutOf(name))) {
    const kept = removeOrchestratorCheckout(name);
    done.push(kept ?? `removed the checkout of ${orchestratorLabel(name)}`);
  }
  if (fs.existsSync(changelogFile(name)))
    done.push(`kept the changelog ${changelogFile(name)}`);
  return { done };
};
/**
 * `wt:orchestrate` itself in the background, detached from its pane: the
 * space it closes may be the one it was started from (the overview's ✕, the
 * orchestrator ending itself), and closing it ends the processes in it. Its
 * output goes to `log`. Returns the child.
 */
export const endInBackground = (name, log) => {
  const out = fs.openSync(log, "w");
  const env = { ...process.env, NO_COLOR: "1" };
  // Without its pane it can't find itself in the space, so it ends it right away.
  delete env.HERDR_PANE_ID;
  delete env.HERDR_TAB_ID;
  const child = spawn(process.execPath, [ENTRY, "orchestrate", name, "--end"], {
    cwd: mainDir(),
    env,
    stdio: ["ignore", out, out],
    detached: true,
  });
  fs.closeSync(out);
  child.unref();
  return child;
};
/** Where a background end writes what it does. */
export const endLog = name =>
  path.join(os.tmpdir(), `orchestration-end-${name}.log`);
/** `pnpm wt:orchestrate <name> --end`. From inside the space it ends, in the background. */
export default function endCommand(name) {
  const why = whyNotEnd(name);
  if (why) fail(`${orchestratorLabel(name)} can't end, ${why}`);
  const workspace = spaceOfOrchestration(name);
  const pane = workspace && process.env.HERDR_PANE_ID ? currentPane() : null;
  if (pane?.workspace_id === workspace?.workspace_id && pane) {
    endInBackground(name, endLog(name));
    step(
      `${orchestratorLabel(name)} ends in the background, this tab with it (log: ${endLog(name)})`,
    );
    return;
  }
  const { error, done } = endOrchestration(name);
  if (error) fail(error);
  for (const line of done) step(line);
  console.log(`\n✓ ${orchestratorLabel(name)} ended`);
}
