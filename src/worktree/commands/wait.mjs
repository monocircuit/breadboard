import { setTimeout as sleep } from "node:timers/promises";

import { glabApi } from "../../lib/glab.mjs";
import { fail, git } from "../../lib/proc.mjs";
import {
  POLL_MS,
  USAGE,
  behindOf,
  clearWait,
  factsOf,
  lineOf,
  listOf,
  parseWait,
  recordWait,
  verdictOf,
  waitOf,
  waitsText,
} from "../wait.mjs";
import { mainDir } from "../worktrees.mjs";

/**
 * `pnpm wt:wait !213 !217 --then "…"`, meant for `run_in_background`: waits
 * until every named MR is merged, asking GitLab every minute from node, and
 * ends with the line to go on with — Claude Code wakes the agent when a
 * background command exits. It ends early when one is closed, stays in
 * conflict, or the limit passes. The wait is recorded in the worktree for
 * the overview, which delivers the line itself when this command is gone.
 */
export default async function waitCommand() {
  const dir = git(["rev-parse", "--show-toplevel"]).out;
  if (!dir || dir === mainDir()) fail("wt:wait is for a worktree's agent");
  const argv = process.argv.slice(3);
  if (argv.includes("--cancel")) {
    const wait = waitOf(dir);
    clearWait(dir);
    console.log(wait ? `\n✓ no longer ${waitsText(wait)}\n` : "\n  no wait\n");
    return;
  }
  const parsed = parseWait(argv);
  if (parsed.error) fail(`${parsed.error}\n${USAGE}`);
  const since = new Date();
  const wait = recordWait(dir, {
    mrs: parsed.mrs,
    then: parsed.then,
    hours: parsed.hours,
    since: since.toISOString(),
    until: new Date(since.getTime() + parsed.hours * 3_600_000).toISOString(),
    pid: process.pid,
  });
  console.log(`▸ ${waitsText(wait)}, asking GitLab every minute`);

  // A typo waits forever: an MR GitLab doesn't know ends it at once.
  const unknown = await Promise.all(
    wait.mrs.map(iid =>
      glabApi(`projects/:fullpath/merge_requests/${iid}`).then(
        () => null,
        error => (/\b404\b/.test(error.stderr ?? "") ? iid : null),
      ),
    ),
  );
  if (unknown.some(Boolean)) {
    clearWait(dir);
    fail(`GitLab has no MR ${listOf(unknown.filter(Boolean))}`);
  }

  const conflictsSince = new Map();
  for (;;) {
    // A newer wt:wait in this worktree took over: this one steps back quietly.
    if (waitOf(dir)?.pid !== process.pid) return;
    const found = await Promise.all(
      wait.mrs.map(iid =>
        glabApi(`projects/:fullpath/merge_requests/${iid}`).then(
          factsOf,
          () => null,
        ),
      ),
    );
    const verdict = verdictOf(
      wait,
      found.filter(Boolean),
      Date.now(),
      conflictsSince,
    );
    if (verdict.status !== "waiting") {
      const line = lineOf(
        wait,
        verdict,
        verdict.status === "merged" ? behindOf(dir) : null,
      );
      if (waitOf(dir)?.pid === process.pid) clearWait(dir);
      console.log(line);
      process.exitCode = verdict.status === "merged" ? 0 : 1;
      return;
    }
    await sleep(POLL_MS);
  }
}
