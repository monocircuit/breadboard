import { spawnSync } from "node:child_process";
import fs from "node:fs";

import { fail, flagValue, quote, step } from "../../lib/proc.mjs";
import { isInHerdr } from "../../lib/herdr.mjs";
import { ENTRY, NAME } from "../constants.mjs";
import endCommand from "../end.mjs";
import { startsOf } from "../fit.mjs";
import { freeMemory } from "../memory.mjs";
import {
  BRIEF_PROMPT,
  briefOf,
  makeWorktree,
  openOrchestrator,
  readIssue,
  recordEntry,
  startAgent,
} from "../orchestrator.mjs";
import {
  ORIGIN_SLOTS,
  orchestratorLabel,
  originToken,
  recordOf,
  slotOf,
  updateRecord,
  writeRecord,
} from "../origins.mjs";
import { afterArgs, parseAfter } from "../handovers.mjs";
import { idleWorkersNow } from "../workers.mjs";
import handOverCommand, { announceCommand } from "./handover.mjs";

/**
 * One session orchestrates several issues: its own space `⇶ <name>` (its
 * tab moves there), a worktree per issue started with `wt:new --origin`,
 * and in each worktree's space a claude briefed on its issue: the FEED of
 * the overview shows what it does, SendMessage carries only decisions. The
 * worktrees one after the other: two at once would pick the same free ports.
 *
 * `--to <agent>` hands one issue to an idle agent of it instead
 * (handover.mjs), with `--after !<mr>…` once those MRs are merged (the
 * overview does it, handovers.mjs), `--end` ends it once its worktrees are gone (end.mjs).
 *
 * An issue is its number (the name comes from its title) or `<n>-<feature>`.
 * A worktree counts as made when it is there: wt:new may still fail after
 * the setup, in its summary (#130), and the rest must not stop because of it.
 */
export default async function orchestrateCommand(name, flags) {
  const usage =
    "usage: pnpm wt:orchestrate <name> <issue|n-feature>… [--shared] [--peer <session>] [--brief <file>] [--queue] [--dry-run]\n       pnpm wt:orchestrate <name> <issue> --to <agent> [--after !<mr>…] [--limit <percent>] [--dry-run]\n       pnpm wt:orchestrate <name> --end";
  // The end: its space, the orchestrator's claude, record and checkout (worktree/end.mjs).
  if (flags.has("--end")) {
    if (!name || !NAME.test(name)) fail(usage);
    return endCommand(name);
  }
  const argv = process.argv.slice(3);
  // The MRs an announced handover waits for (handovers.mjs): no issues.
  const after = afterArgs(argv);
  const positional = argv.filter(
    (arg, i) =>
      !arg.startsWith("--") &&
      !after?.at.includes(i) &&
      !["--peer", "--brief", "--to", "--limit"].includes(argv[i - 1]),
  );
  const wanted = positional.slice(1);
  if (!name || !NAME.test(name) || wanted.length === 0) fail(usage);
  if (
    wanted.some(
      arg => !/^\d+$/.test(arg) && !(NAME.test(arg) && /^\d+-/.test(arg)),
    )
  )
    fail(usage);
  const isDry = flags.has("--dry-run");
  const extraFile = flagValue("--brief");
  if (extraFile && !fs.existsSync(extraFile))
    fail(`${extraFile} does not exist`);
  if (flags.has("--cap"))
    fail(
      "--cap is gone (#229): every issue starts at once, the machine's memory is the only limit; --queue queues them",
    );
  // Left out, as the first run said: an issue started later from the queue gets the same brief.
  const record = recordOf(name);
  const peer = flagValue("--peer") ?? record?.peer;
  const extra = extraFile
    ? fs.readFileSync(extraFile, "utf-8")
    : (record?.brief ?? "");
  // The next issue to an agent that has one done (workers.mjs).
  if (flags.has("--to")) {
    if (wanted.length !== 1) fail("--to hands over one issue at a time");
    if (after) {
      const parsed = parseAfter(after.values);
      if (parsed.error) fail(parsed.error);
      return announceCommand(name, wanted[0], { after: parsed.after, isDry });
    }
    return handOverCommand(name, wanted[0], {
      peer,
      extra,
      isShared: flags.has("--shared"),
      isDry,
    });
  }

  // The issues first, all of them: a typo stops here, before any worktree exists.
  const plan = wanted.map(arg => {
    const item = readIssue(arg);
    if (!item) fail(`issue #${arg.match(/^\d+/)[0]} could not be read`);
    return item;
  });
  const slot = slotOf(name);
  const label = orchestratorLabel(name);

  console.log(
    `\n${label}  colour slot ${slot + 1} (${ORIGIN_SLOTS[slot].colour}, sidebar token $${originToken(slot)})`,
  );
  const width = Math.max(...plan.map(item => item.branch.length));
  for (const item of plan) {
    item.isSkipped = Boolean(item.why);
    console.log(
      `  #${item.number.padEnd(4)} ${item.branch.padEnd(width)}  ${item.why ? `skipped: ${item.why}` : item.title}`,
    );
  }
  if (plan.every(item => item.isSkipped))
    fail(
      "nothing to start: every issue is closed, claimed or has its worktree already",
    );
  // No count holds one back (#229): each to an idle agent that knows its
  // files, else to a fresh one at once, on the shared stack when memory is
  // short (memory.mjs). `--queue` queues all of them for the overview,
  // `--now` (the overview's own start) looks for no idle one: it looked already.
  const toStart = plan.filter(item => !item.isSkipped);
  const isQueued = flags.has("--queue");
  const idle =
    isQueued || flags.has("--now") || !isInHerdr()
      ? []
      : await idleWorkersNow(name);
  const starts = startsOf(toStart, idle, {
    isShared: flags.has("--shared"),
    free: freeMemory(),
  });
  const queued = isQueued ? toStart : [];
  for (const item of isQueued ? [] : toStart) {
    Object.assign(item, starts.get(item));
    console.log(
      `  #${item.number.padEnd(4)} ${
        item.worker
          ? `→ ${item.worker.name} (idle, shares ${item.shared.join(", ")})`
          : `→ a fresh agent${item.stack.why ? `: ${item.stack.why}` : item.stack.isShared ? " on the shared stack" : ""}`
      }`,
    );
  }
  if (isDry) {
    const first = plan.find(item => !item.isSkipped) ?? plan[0];
    console.log(
      `\n▸ nothing was done (--dry-run). The brief of ${first.feature} (.claude/brief.md), started with\n  claude --session-id <new id> --name ${first.feature} ${quote(BRIEF_PROMPT)}\n`,
    );
    console.log(briefOf({ ...first, name, peer, extra }));
    return;
  }

  const opened = await openOrchestrator(name);
  if (opened) step(`herdr: ${opened}`);
  updateRecord(name, {
    peer: flagValue("--peer"),
    brief: extraFile ? extra : undefined,
  });
  // After the space, which keeps it from being pruned; the overview in there reads it from now on.
  writeRecord(
    name,
    plan.map(item =>
      queued.includes(item)
        ? { ...recordEntry(item), queued: true }
        : recordEntry(item),
    ),
  );
  if (queued.length)
    step(
      `queued ${queued.map(item => `#${item.number}`).join(", ")} (--queue). The overview of ${label} hands each to an idle agent that knows its code once its MR is merged (UP NEXT), a click there starts a fresh one; by hand: pnpm wt:orchestrate ${name} <issue> --to <agent>`,
    );

  const results = [];
  for (const item of isQueued ? [] : toStart) {
    if (item.worker) {
      // handover.mjs, in its own process: its fail() must not stop the rest.
      const args = ["orchestrate", name, item.number, "--to", item.worker.name];
      if (flags.has("--shared")) args.push("--shared");
      step(`pnpm wt:${args.join(" ")}`);
      const handed = spawnSync(process.execPath, [ENTRY, ...args], {
        stdio: "inherit",
      });
      results.push(
        handed.status === 0
          ? `✓ #${item.number} ${item.feature}: went to ${item.worker.name}, idle and knowing its files`
          : `✗ #${item.number} ${item.feature}: the handover to ${item.worker.name} failed (see above); a fresh agent: pnpm wt:orchestrate ${name} ${item.number} --now`,
      );
      continue;
    }
    if (item.stack.why) step(`memory: ${item.stack.why}`);
    step(
      `pnpm wt:new ${item.feature} --origin ${name}${item.stack.isShared ? " --shared" : ""}`,
    );
    const made = makeWorktree(item, name, { isShared: item.stack.isShared });
    const { isThere, failed } = made;
    if (!isThere || failed) {
      results.push(
        `✗ #${item.number} ${item.feature}: ${failed ?? "no worktree"}`,
      );
      if (!isThere)
        writeRecord(name, [
          recordEntry(item, `failed: ${failed ?? "no worktree"}`),
        ]);
      continue;
    }
    const agent = startAgent(item.dir, briefOf({ ...item, name, peer, extra }));
    const note =
      made.status === 0 ? "" : " (wt:new failed after the setup, see above)";
    results.push(
      agent.isStarted
        ? `✓ #${item.number} ${item.feature}: claude started in its tab, named ${item.feature}${note}`
        : `✓ #${item.number} ${item.feature}: worktree ready${note}, start its agent:\n      ${agent.command}`,
    );
  }

  const started = plan.filter(
    item =>
      !item.isSkipped &&
      results.some(line => line.startsWith(`✓ #${item.number} `)),
  );
  console.log(`\n${label}`);
  for (const line of results) console.log(`  ${line}`);
  if (started.length) {
    console.log(`
  A folder claude has not seen asks to be trusted first: answer it in the agent's tab.
  The overview's FEED shows what each agent does: look there, don't ask and don't relay it.
  An agent messages you${peer ? ` (${peer})` : ""} only for a decision, a changed plan or another agent's work.
  Say hello first, so it has your address:
${started.map(item => `    SendMessage(to: "${item.worker?.name ?? item.feature}", message: "…")`).join("\n")}

  The overview of ${label} answers the user's "what now?": keep it true.
    pnpm wt:plan ${name} --work <issue>… --merge <issue>… --why "<one line>"
                                  the order now, again whenever it changes (a dependency, a failed pipeline)
    pnpm wt:action decide "<question>" --orchestration ${name}
                                  a step that is the user's and no agent's (--done <id> --orchestration ${name} after)
  The agents leave their own actions (merge, try, decide, look, plan) whenever they wait for the user.
`);
  }
}
