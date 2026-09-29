// The git worktrees of the repo and the main checkout.
import path from "node:path";

import { git } from "../lib/proc.mjs";

/**
 * All worktrees, the main checkout first. Asked from the checkout of this
 * script, not the working directory: the overview of a worktree lives on while
 * `wt:remove` deletes the directory it runs in. After the first answer from the
 * main checkout, which no removal deletes: `wt:new` hands a conversation over
 * and removes the worktree whose copy of this script is running.
 */
let knownMain = null;
export const allWorktrees = () => {
  const list = git(
    ["worktree", "list", "--porcelain"],
    knownMain ?? import.meta.dirname,
  )
    .out.split("\n\n")
    .map(block => {
      const lines = block.split("\n");
      const get = key =>
        lines.find(l => l.startsWith(`${key} `))?.slice(key.length + 1);
      return {
        dir: get("worktree"),
        branch: get("branch")?.replace("refs/heads/", ""),
      };
    })
    .filter(w => w.dir);
  knownMain ??= list[0]?.dir ?? null;
  return list;
};
/**
 * The worktrees without the checkouts of the orchestrations
 * (orchestratorCheckout): they carry no work, only the place of a space in
 * herdr's sidebar.
 */
export const worktrees = () => {
  const list = allWorktrees();
  const orchestrations = `${path.join(list[0]?.dir ?? "", ".claude", "orchestrations")}${path.sep}`;
  return list.filter(({ dir }) => !dir.startsWith(orchestrations));
};
export const mainDir = () => worktrees()[0].dir;
export const dirOf = feature => `${mainDir()}-${feature}`;
