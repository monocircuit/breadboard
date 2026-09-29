// The waits of an orchestration's agents (`pnpm wt:wait`) whose command is
// gone — the agent restarted, the background task died: the overview, which
// reads GitLab anyway, asks for their MRs and types the line the command
// would have printed. So a wait is never lost, and no model polls for it.
import {
  behindOf,
  clearWait,
  factsOf,
  isWaiting,
  lineOf,
  verdictOf,
  waitOf,
} from "../worktree/wait.mjs";
import { deliverTo, deliversAnswers } from "./answers.mjs";
import { glab } from "./io.mjs";

/** dir → iid → since when it has conflicts, over the reads. */
const conflicts = new Map();

/**
 * The line of each wait in `dirs` that ended while its command was gone,
 * typed into its agent's pane; delivered, the wait goes. Only by the
 * overview that delivers the answers of the orchestration `name` (never a
 * `--once` print). `ask`: iid → the MR from GitLab, for the tests.
 */
export const deliverWaits = async (
  name,
  dirs,
  ask = iid => glab(`projects/:fullpath/merge_requests/${iid}`),
) => {
  const orphans = dirs
    .map(dir => ({ dir, wait: waitOf(dir) }))
    .filter(({ wait }) => wait && !isWaiting(wait));
  if (orphans.length === 0 || !deliversAnswers(name)) return [];
  const delivered = [];
  for (const { dir, wait } of orphans) {
    const found = await Promise.all(
      wait.mrs.map(iid => ask(iid).then(factsOf, () => null)),
    );
    if (!conflicts.has(dir)) conflicts.set(dir, new Map());
    const verdict = verdictOf(
      wait,
      found.filter(Boolean),
      Date.now(),
      conflicts.get(dir),
    );
    if (verdict.status === "waiting") continue;
    const line = lineOf(
      wait,
      verdict,
      verdict.status === "merged" ? behindOf(dir) : null,
    );
    // A new wt:wait may have started while GitLab answered.
    if (waitOf(dir)?.since !== wait.since || !deliverTo(dir, line)) continue;
    clearWait(dir);
    conflicts.delete(dir);
    delivered.push({ dir, line });
  }
  return delivered;
};
