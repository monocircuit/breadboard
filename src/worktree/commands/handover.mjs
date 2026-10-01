import { isInHerdr } from "../../lib/herdr.mjs";
import { fail, flagValue, step } from "../../lib/proc.mjs";
import { boundSession, contextShareOf } from "../conversations.mjs";
import {
  briefOf,
  makeWorktree,
  readIssue,
  recordEntry,
  writeBrief,
} from "../orchestrator.mjs";
import {
  orchestratorLabel,
  originOf,
  recordOf,
  writeRecord,
} from "../origins.mjs";
import {
  CONTEXT_LIMIT,
  beforeLine,
  findWorker,
  issuesBefore,
  moveWorker,
  nextIssueLine,
  paneOfConversation,
  recordHandover,
  refusalOf,
  renameLine,
  workersOf,
} from "../workers.mjs";
import { announceHandover, dropHandover, handoversOf } from "../handovers.mjs";
import { recordFailure, recordStart } from "../history.mjs";
import { worktrees } from "../worktrees.mjs";

/**
 * `pnpm wt:orchestrate <name> <issue> --to <agent> [--limit <percent>]`: the
 * issue goes to an idle agent of the orchestration instead of a fresh one
 * (workers.mjs). Its worktree is made as for any issue, the brief says what
 * the agent had before, then the agent moves in, is renamed after the
 * issue's feature (#251) and is told in one line.
 * Refused while the agent works, waits for an answer or has used more of its
 * context than the limit. A worktree an earlier try made is taken as it is.
 * Its history (#264) learns the issue, or why the handover failed.
 */
export default function handOverCommand(
  name,
  arg,
  { peer, extra, isShared, isDry },
) {
  const label = orchestratorLabel(name);
  if (!isInHerdr())
    fail("--to works inside herdr only: the agents are found through it");
  const who = flagValue("--to");
  const percent = flagValue("--limit");
  const limit = percent === undefined ? CONTEXT_LIMIT : Number(percent) / 100;
  if (!who || !(limit > 0 && limit <= 1))
    fail(
      "usage: pnpm wt:orchestrate <name> <issue> --to <agent> [--limit <percent>]",
    );
  const workers = workersOf(name);
  const worker = findWorker(workers, who);
  if (!worker)
    fail(
      `${label} has no agent ${who}; its agents: ${workers.map(w => w.name).join(", ") || "none"}`,
    );
  const share = contextShareOf(worker.id);
  const refusal = refusalOf({
    pane: paneOfConversation(worker.id),
    share,
    limit,
  });
  const item = readIssue(arg);
  if (!item) fail(`issue #${arg.match(/^\d+/)[0]} could not be read`);
  const number = Number(item.number);
  /** The handover fails: in the agent's history (not on a dry run), then said. */
  const failed = reason => {
    if (!isDry) recordFailure(name, worker.id, number, reason);
    fail(`${worker.name} gets no next issue: ${reason}`);
  };
  if (refusal) failed(refusal);
  // A worktree of this orchestration without an agent: an earlier try made it.
  const isMade =
    worktrees().some(({ dir }) => dir === item.dir) &&
    originOf(item.dir)?.name === name &&
    !boundSession(item.dir);
  if (item.why && !isMade) fail(`#${item.number}: ${item.why}`);
  const brief = briefOf({
    ...item,
    name,
    peer,
    extra,
    before: beforeLine(issuesBefore(name, worker)),
  });
  if (isDry) {
    console.log(
      `\n▸ nothing was done (--dry-run). ${worker.name} would get\n  ${nextIssueLine(item.number, item.dir)}\n  with the brief\n`,
    );
    console.log(brief);
    return;
  }

  if (!isMade) {
    writeRecord(name, [recordEntry(item, null)]);
    step(`pnpm wt:new ${item.feature} --origin ${name}`);
    const made = makeWorktree(item, name, { isShared });
    if (!made.isThere) {
      writeRecord(name, [
        recordEntry(item, `failed: ${made.failed ?? "no worktree"}`),
      ]);
      recordFailure(
        name,
        worker.id,
        number,
        `its worktree failed: ${made.failed ?? "no worktree"}`,
      );
      fail(`#${item.number} ${item.feature}: ${made.failed ?? "no worktree"}`);
    }
  }
  writeBrief(item.dir, brief);
  // Again: the setup took minutes, and the agent may have started on something meanwhile.
  const pane = paneOfConversation(worker.id);
  const now = refusalOf({ pane, share: null });
  if (now) {
    recordFailure(name, worker.id, number, `after the setup, ${now}`);
    fail(
      `the worktree of #${item.number} is ready, but ${worker.name} ${now.replace(/^it /, "")} now: run the same command again once it is idle`,
    );
  }
  const { isMoved, isTold } = moveWorker(worker, pane, item.number, item.dir);
  recordHandover(name, worker, number, item.dir);
  // Announced before (--after): done now, by the overview or by hand.
  const isAnnounced = handoversOf(recordOf(name)).some(
    handover => handover.number === number,
  );
  recordStart(name, worker.id, {
    number,
    how: isAnnounced ? "announced" : "handover",
    context: share,
  });
  dropHandover(name, number);
  step(
    isMoved
      ? `herdr: the tab of ${worker.name} moved into the space ${item.feature}`
      : `herdr: the tab of ${worker.name} stayed where it is (no space ${item.feature})`,
  );
  const renamed =
    worker.name === item.feature ? "" : `, renamed from ${worker.name}`;
  console.log(
    isTold
      ? `\n✓ #${item.number} went to the agent ${item.feature}${renamed}, told in its prompt; SendMessage(to: "${item.feature}")`
      : `\n✓ #${item.number} ${item.feature} is ready for ${worker.name}, but herdr took no line: type these into its tab\n    ${renameLine(item.dir)}\n    ${nextIssueLine(item.number, item.dir)}`,
  );
}

/**
 * `pnpm wt:orchestrate <name> <issue> --to <agent> --after !<mr>…`: the
 * handover announced, not done (handovers.mjs). The record keeps it, the
 * issue queued; the overview of the orchestration hands the issue over once
 * the MRs are merged and the agent is idle, and holds the clean-up of the
 * agent's worktree until then. The agent may be at work now.
 */
export function announceCommand(name, arg, { after, isDry }) {
  const label = orchestratorLabel(name);
  const who = flagValue("--to");
  const workers = workersOf(name);
  const worker = findWorker(workers, who);
  if (!worker)
    fail(
      `${label} has no agent ${who}; its agents: ${workers.map(w => w.name).join(", ") || "none"}`,
    );
  const item = readIssue(arg);
  if (!item) fail(`issue #${arg.match(/^\d+/)[0]} could not be read`);
  const number = Number(item.number);
  const known = recordOf(name)?.issues.find(issue => issue.number === number);
  // Queued already is fine: it waits for an agent either way.
  if (item.why && !known?.queued) fail(`#${number}: ${item.why}`);
  const mrs = after.map(iid => `!${iid}`).join(", ");
  if (isDry) {
    console.log(
      `\n▸ nothing was done (--dry-run). ${worker.name} would take #${number} once ${mrs} ${after.length > 1 ? "are" : "is"} merged.`,
    );
    return;
  }
  writeRecord(name, [
    {
      ...recordEntry(item),
      queued: true,
      // Its place in WORKTREES' queue (#250): a queued one keeps it.
      queuedAt: known?.queuedAt ?? new Date().toISOString(),
    },
  ]);
  announceHandover(name, { number, to: worker.name, after });
  console.log(
    `\n✓ #${number} ${item.feature} goes to ${worker.name} once ${mrs} ${after.length > 1 ? "are" : "is"} merged: the overview of ${label} hands it over then (its worktree is kept until), shown on ${worker.name}'s node as → next #${number}`,
  );
}
