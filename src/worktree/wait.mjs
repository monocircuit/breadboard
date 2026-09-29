// An agent that waits for other MRs (`pnpm wt:wait !213 !217 --then "…"`):
// the wait recorded in its worktree, what the answers of GitLab mean for it,
// and the one line it goes on with. The command asks GitLab from node, the
// overview reads the record: no model polls, no orchestrator has to remember.
import fs from "node:fs";
import path from "node:path";

import { git } from "../lib/proc.mjs";

/** How often the command asks GitLab. */
export const POLL_MS = 60_000;
/** How long it waits by default before it gives up, in hours. */
export const DEFAULT_HOURS = 8;
/** How long an awaited MR may have conflicts with develop before the wait ends. */
export const CONFLICT_MS = 30 * 60_000;

/**
 * Where a worktree keeps its wait, in its ignored `.claude/`: `{ mrs, then,
 * since, until, pid }`, `pid` the command that waits (gone after a restart
 * of the agent: the overview then delivers the line).
 */
export const waitFile = dir => path.join(dir, ".claude", "wait.json");
/** The wait of the worktree `dir`, or null when it waits for nothing. */
export const waitOf = dir => {
  try {
    const wait = JSON.parse(fs.readFileSync(waitFile(dir), "utf-8"));
    return Array.isArray(wait?.mrs) && wait.mrs.length ? wait : null;
  } catch {
    return null;
  }
};
export const recordWait = (dir, wait) => {
  fs.mkdirSync(path.dirname(waitFile(dir)), { recursive: true });
  fs.writeFileSync(waitFile(dir), `${JSON.stringify(wait, null, 2)}\n`);
  return wait;
};
export const clearWait = dir => fs.rmSync(waitFile(dir), { force: true });
/** Whether the command of `wait` still runs. */
export const isWaiting = wait => {
  if (!wait?.pid) return false;
  try {
    process.kill(wait.pid, 0);
    return true;
  } catch (error) {
    // EPERM: it runs, as someone else.
    return error.code === "EPERM";
  }
};

export const USAGE =
  'usage: pnpm wt:wait !<mr> [!<mr>…] [--then "<what to do>"] [--hours <h>]   (run_in_background)\n       pnpm wt:wait --cancel';
/**
 * The command line after `wait`: `{ mrs, then, hours }`, or `{ error }`.
 * MRs as `!213` or `213`, each once, in the given order.
 */
export const parseWait = argv => {
  const mrs = [];
  let then = null;
  let hours = DEFAULT_HOURS;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const [flag, inline] = arg.startsWith("--") ? arg.split(/=(.*)/s) : [];
    if (flag === "--then" || flag === "--hours") {
      const value = inline ?? argv[++i];
      if (
        value === undefined ||
        (inline === undefined && value.startsWith("--"))
      )
        return { error: `${flag} needs a value` };
      if (flag === "--then") then = value.trim() || null;
      else {
        hours = Number(value);
        if (!(hours > 0))
          return { error: `--hours takes a number above 0, not ${value}` };
      }
    } else if (/^!?\d+$/.test(arg)) {
      const iid = Number(arg.replace("!", ""));
      if (!mrs.includes(iid)) mrs.push(iid);
    } else return { error: `${arg} is no MR (!213) and no flag` };
  }
  if (mrs.length === 0) return { error: "name the MRs to wait for: !213 !217" };
  return { mrs, then, hours };
};

/** `!213`, `!213 and !217`, `!213, !217 and !219`. */
export const listOf = mrs => {
  const names = mrs.map(iid => `!${iid}`);
  return names.length < 2
    ? (names[0] ?? "")
    : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
};
/** What the overview shows of a wait: `waits for !213, !217`. */
export const waitsText = wait =>
  `waits for ${wait.mrs.map(iid => `!${iid}`).join(", ")}`;

/**
 * Where a wait stands, from the awaited MRs as GitLab has them (`{ iid,
 * state, hasConflicts }`, `state` as GitLab says it; one missing counts as
 * not answered yet), at `now`. `conflictsSince`: iid → since when it has
 * conflicts, kept by the caller between asks. One of
 * - `{ status: "merged", mrs }`: every one merged, go on;
 * - `{ status: "closed", mr }`: one closed without a merge;
 * - `{ status: "conflicts", mr }`: one had conflicts for CONFLICT_MS;
 * - `{ status: "timeout" }`: past `wait.until`;
 * - `{ status: "waiting", open }`: the ones not merged yet.
 */
export const verdictOf = (
  wait,
  found,
  now = Date.now(),
  conflictsSince = new Map(),
) => {
  const byIid = new Map(found.map(mr => [mr.iid, mr]));
  const closed = wait.mrs.find(iid => byIid.get(iid)?.state === "closed");
  if (closed) return { status: "closed", mr: closed };
  const open = wait.mrs.filter(iid => byIid.get(iid)?.state !== "merged");
  if (open.length === 0) return { status: "merged", mrs: wait.mrs };
  for (const iid of open) {
    if (!byIid.get(iid)?.hasConflicts) conflictsSince.delete(iid);
    else if (!conflictsSince.has(iid)) conflictsSince.set(iid, now);
    else if (now - conflictsSince.get(iid) >= CONFLICT_MS)
      return { status: "conflicts", mr: iid };
  }
  if (wait.until && now >= new Date(wait.until).getTime())
    return { status: "timeout", open };
  return { status: "waiting", open };
};

/**
 * The line the agent goes on with, for a verdict that ends the wait:
 * `!213 and !217 are merged: merge develop in (3 behind), then <then>`.
 * `behind`: how many commits of develop the worktree lacks, null unknown.
 */
export const lineOf = (wait, verdict, behind = null) => {
  const all = listOf(wait.mrs);
  const then = wait.then ? `, then ${wait.then}` : "";
  switch (verdict.status) {
    case "merged": {
      const merge =
        behind === 0
          ? "develop is in already"
          : `merge develop in${behind ? ` (${behind} behind)` : ""}`;
      return `${all} ${wait.mrs.length > 1 ? "are" : "is"} merged: ${merge}${then}`;
    }
    case "closed":
      return `!${verdict.mr} was closed without a merge: the wait for ${all} ends, find out what replaces it (glab mr view ${verdict.mr}) before you go on`;
    case "conflicts":
      return `!${verdict.mr} has had conflicts with develop for ${CONFLICT_MS / 60_000} min: the wait for ${all} ends, tell the orchestrator (SendMessage) instead of waiting on`;
    case "timeout":
      return `waited ${wait.hours ?? DEFAULT_HOURS} h for ${all}, ${listOf(verdict.open)} not merged yet: the wait ends, tell the orchestrator or wait again (pnpm wt:wait)`;
    default:
      return null;
  }
};

/** How many commits of develop the worktree `dir` lacks, fetched first; null when git can't say. */
export const behindOf = dir => {
  git(["fetch", "--quiet", "origin", "develop"], dir);
  const count = git(["rev-list", "--count", "HEAD..origin/develop"], dir);
  return count.ok ? Number(count.out) : null;
};
/** An MR of the GitLab API as verdictOf takes it. */
export const factsOf = mr => ({
  iid: mr.iid,
  state: mr.state,
  hasConflicts: Boolean(mr.has_conflicts),
});
