// Which open merge requests a worktree waits for: the branches its work
// builds on (a stack, !194 on !193 on !192), in the order they have to be
// merged. Read from git, nobody writes the stack down.
import { sh } from "../lib/proc.mjs";

const git = (args, dir) => sh("git", args, dir);

/**
 * The open MRs whose branch the worktree `dir` builds on: commits of the
 * branch (its `origin/` ref) are in HEAD and not in `develop` yet. Through
 * their merge base, so a base that got more commits since still counts.
 * Ordered by how many of those commits there are, the base of the stack
 * first: that is the merge order. `mrs`: `{ iid, branch, … }`, the
 * worktree's own branch left out by the caller or here by `branch`.
 */
export const waitingFor = (dir, branch, mrs, develop = "origin/develop") => {
  const waiting = [];
  for (const mr of mrs) {
    if (mr.branch === branch) continue;
    const ref = git(
      ["rev-parse", "--verify", "--quiet", `refs/remotes/origin/${mr.branch}`],
      dir,
    ).stdout?.trim();
    if (!ref) continue;
    const base = git(["merge-base", ref, "HEAD"], dir).stdout?.trim();
    if (!base) continue;
    const count =
      Number(
        git(["rev-list", "--count", `${develop}..${base}`], dir).stdout?.trim(),
      ) || 0;
    if (count > 0) waiting.push({ ...mr, count });
  }
  return waiting.sort((a, b) => a.count - b.count);
};
