import fs from "node:fs";

import { git, step } from "../../lib/proc.mjs";
import { prunable } from "../prune.mjs";
import removeCommand from "./remove.mjs";

/**
 * After a merge nobody remembers `wt:remove`, so worktrees and branches pile
 * up. This closes what GitLab is done with: a worktree whose branch is gone
 * from origin and whose commits are all in `develop`, and the local branches
 * one left behind.
 *
 * Everything else is only listed. A branch with own commits — a closed merge
 * request, work nobody pushed — is the only copy of it there is.
 */
export default function pruneCommand(_feature, flags) {
  const isDry = flags.has("--dry-run");
  step("git fetch --prune");
  git(["fetch", "--prune"]);

  const { trees, branches, folders } = prunable();
  const report = [];
  const say = (what, line) => report.push(`${what.padEnd(32)} ${line}`);

  for (const { name, why } of trees) {
    if (why) {
      say(name, `kept, ${why}`);
    } else if (isDry) {
      say(name, "would be removed");
    } else {
      step(`pnpm wt:remove ${name}`);
      removeCommand(name, new Set());
      say(name, "removed");
    }
  }

  for (const { name, why } of branches) {
    if (why) {
      say(name, `kept, ${why}`);
    } else if (isDry) {
      say(name, "would be deleted");
    } else {
      say(
        name,
        git(["branch", "-D", name]).ok
          ? "deleted, all of it is in develop"
          : "kept, git refused",
      );
    }
  }

  for (const { name, dir, why } of folders) {
    if (why) {
      say(name, `kept, ${why}`);
    } else if (isDry) {
      say(name, "would be deleted, a leftover folder");
    } else {
      fs.rmSync(dir, { recursive: true, force: true });
      say(name, "deleted, a leftover folder");
    }
  }

  console.log(`\n✓ ${isDry ? "nothing was touched (--dry-run)" : "pruned"}`);
  for (const line of report) console.log(`  ${line}`);
  if (report.length === 0) console.log("  nothing to do");
  console.log("");
}
