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
import { orchestratorLabel, originOf, writeRecord } from "../origins.mjs";
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
  workersOf,
} from "../workers.mjs";
import { worktrees } from "../worktrees.mjs";

/**
 * `pnpm wt:orchestrate <name> <issue> --to <agent> [--limit <percent>]`: the
 * issue goes to an idle agent of the orchestration instead of a fresh one
 * (workers.mjs). Its worktree is made as for any issue, the brief says what
 * the agent had before, then the agent moves in and is told in one line.
 * Refused while the agent works, waits for an answer or has used more of its
 * context than the limit. A worktree an earlier try made is taken as it is.
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
  const refusal = refusalOf({
    pane: paneOfConversation(worker.id),
    share: contextShareOf(worker.id),
    limit,
  });
  if (refusal) fail(`${worker.name} gets no next issue: ${refusal}`);

  const item = readIssue(arg);
  if (!item) fail(`issue #${arg.match(/^\d+/)[0]} could not be read`);
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
      fail(`#${item.number} ${item.feature}: ${made.failed ?? "no worktree"}`);
    }
  }
  writeBrief(item.dir, brief);
  // Again: the setup took minutes, and the agent may have started on something meanwhile.
  const pane = paneOfConversation(worker.id);
  const now = refusalOf({ pane, share: null });
  if (now)
    fail(
      `the worktree of #${item.number} is ready, but ${worker.name} ${now.replace(/^it /, "")} now: run the same command again once it is idle`,
    );
  const { isMoved, isTold } = moveWorker(worker, pane, item.number, item.dir);
  recordHandover(name, worker, Number(item.number));
  step(
    isMoved
      ? `herdr: the tab of ${worker.name} moved into the space ${item.feature}`
      : `herdr: the tab of ${worker.name} stayed where it is (no space ${item.feature})`,
  );
  console.log(
    isTold
      ? `\n✓ #${item.number} ${item.feature} went to ${worker.name}, told in its prompt`
      : `\n✓ #${item.number} ${item.feature} is ready for ${worker.name}, but herdr took no line: type it into its tab\n    ${nextIssueLine(item.number, item.dir)}`,
  );
}
