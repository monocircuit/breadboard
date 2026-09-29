import fs from "node:fs";
import path from "node:path";

import { isInHerdr } from "../../lib/herdr.mjs";
import { fail, git, run, step } from "../../lib/proc.mjs";
import { sessionFile } from "../conversations.mjs";
import { releaseSpace } from "../layout.mjs";
import { markRemoved, originOf, pruneOrchestrators } from "../origins.mjs";
import { hasOwnCommits, ownFiles } from "../prune.mjs";
import { compose, offsetOf } from "../stack.mjs";
import { dirOf, mainDir, worktrees } from "../worktrees.mjs";

/** When the MR of `branch` was merged, null when it wasn't or GitLab does not answer. */
const mergedAtOf = branch => {
  const { ok: isOk, out } = run(
    "glab",
    [
      "api",
      `projects/:fullpath/merge_requests?state=merged&source_branch=${encodeURIComponent(branch)}`,
    ],
    { cwd: mainDir(), quiet: true, allowFail: true },
  );
  try {
    return (isOk && JSON.parse(out)[0]?.merged_at) || null;
  } catch {
    return null;
  }
};

export default function removeCommand(feature, flags) {
  if (!feature)
    fail("usage: pnpm wt:remove <feature> [--keep-branch] [--close-space]");
  const dir = dirOf(feature);
  const worktree = worktrees().find(w => w.dir === dir);
  if (!worktree) fail(`${dir} is not a worktree (pnpm wt:list)`);
  if (path.resolve(process.cwd()).startsWith(dir))
    fail(`run it from outside ${dir}`);

  if (git(["status", "--porcelain"], dir).out)
    fail(`${feature} has uncommitted changes, commit or discard them first`);

  const branch = worktree.branch;
  let isMerged = false;
  if (branch) {
    git(["fetch", "--prune"]);
    isMerged = !hasOwnCommits(branch, dir);
    if (!isMerged && !flags.has("--keep-branch")) {
      fail(
        `${branch} has commits that are not in develop. Merge its MR first, or keep the branch with --keep-branch`,
      );
    }
  }

  // The origin goes with the directory: read before, noted in the record after.
  const origin = originOf(dir);
  if (offsetOf(dir)) {
    step(`stop the own stack of ${feature} and delete its data`);
    compose(dir, ["down", "-v"]);
  }
  step(`git worktree remove ${dir}`);
  // Its binding goes first: the overview only rewrites a binding that is there.
  fs.rmSync(sessionFile(dir), { force: true });
  if (!run("git", ["worktree", "remove", dir], { allowFail: true }).ok) {
    // git unregisters it and deletes the files, then fails on a directory something
    // wrote into meanwhile (an older overview: the binding). Nothing of anyone's work
    // left (the same test as wt:prune's): done; anything else stops here.
    const isGone = !worktrees().some(w => w.dir === dir);
    const files = isGone && fs.existsSync(dir) ? ownFiles(dir) : [];
    if (!isGone || files.length > 0) {
      fail(
        `git worktree remove ${dir} failed${files.length ? `, left in it: ${files.join(", ")}` : ""}`,
      );
    }
    // Again if the writer strikes once more in between.
    fs.rmSync(dir, {
      recursive: true,
      force: true,
      maxRetries: 10,
      retryDelay: 100,
    });
  }
  if (branch && isMerged && !flags.has("--keep-branch")) {
    step(`delete ${branch} (all of it is in develop)`);
    run("git", ["branch", "-D", branch]);
  }
  // The overview of the orchestration lists it under "done and removed", with the times.
  if (origin && branch)
    markRemoved(origin.name, {
      branch,
      feature,
      removedAt: new Date().toISOString(),
      mergedAt: isMerged ? mergedAtOf(branch) : null,
    });
  const tab = releaseSpace(feature, { isWhole: flags.has("--close-space") });
  if (tab) step(`herdr: ${tab}`);
  // Its origin went with the directory; the orchestration's space goes with its last worktree.
  if (isInHerdr())
    for (const line of pruneOrchestrators()) step(`herdr: ${line}`);
  console.log(
    `\n✓ ${feature} removed${branch && !isMerged ? `, ${branch} kept` : ""}`,
  );
}
