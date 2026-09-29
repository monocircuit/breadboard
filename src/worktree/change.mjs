// A change requested for an MR blocks it until the change is pushed and green
// (#228), and work that isn't pushed shows: what `wt:action change` records,
// what the overview reads of each worktree.
import { git } from "../lib/proc.mjs";

/** Past this long an unpushed commit is no longer a push about to happen: it goes to NEXT. */
export const UNPUSHED_MS = 10 * 60_000;

/**
 * How much of a worktree's branch is on origin: `{ pushed, unpushed,
 * unpushedAt, hasRemoteBranch }`. `pushed` the commit of `origin/<branch>`
 * (null without one), `unpushed` how many commits of HEAD it doesn't have
 * (without a remote branch: the ones develop doesn't have), `unpushedAt` when
 * the oldest of them was made. From the refs git has, no fetch.
 */
export const pushStateOf = (dir, branch) => {
  const remote = `refs/remotes/origin/${branch}`;
  const pushed =
    branch && branch !== "(detached)"
      ? git(["rev-parse", "--verify", "--quiet", remote], dir).out || null
      : null;
  const base = pushed ? remote : "origin/develop";
  const times = git(["log", "--format=%ct", `${base}..HEAD`], dir)
    .out.split("\n")
    .filter(Boolean);
  return {
    pushed,
    unpushed: times.length,
    unpushedAt: times.length
      ? new Date(Number(times.at(-1)) * 1000).toISOString()
      : null,
    hasRemoteBranch: Boolean(pushed),
  };
};

/** The commit origin has of `branch` in `dir` right now, null without a remote branch. */
export const remoteHeadOf = (dir, branch) =>
  git(["ls-remote", "--heads", "origin", branch], dir).out.split(/\s/)[0] ||
  null;

/**
 * Whether the change requested for a worktree still blocks its MR: until a
 * commit after the request is on origin, nothing is left unpushed and the
 * pipeline of that commit passed. `tree` with `change` and pushStateOf's
 * fields, `mr` as the overview reads it (its pipeline with `sha`).
 */
export const isChangeOpen = (tree, mr) => {
  const { change } = tree ?? {};
  if (!change) return false;
  if (!tree.pushed || tree.pushed === change.head || tree.unpushed > 0)
    return true;
  const pipeline = mr?.pipeline;
  if (pipeline?.status !== "success") return true;
  return Boolean(pipeline.sha) && pipeline.sha !== tree.pushed;
};

/** Whether the unpushed commits of a worktree are old enough to need someone. */
export const isUnpushedLong = (tree, now = Date.now()) =>
  tree?.unpushed > 0 &&
  Boolean(tree.unpushedAt) &&
  now - Date.parse(tree.unpushedAt) >= UNPUSHED_MS;
