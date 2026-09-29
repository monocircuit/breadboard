// The push guard (#184): a git pre-push hook that refuses a push to a branch
// whose merge request is merged or closed. Such a push recreates the branch
// GitLab deleted on merge, and what it carries lands where no MR points.
import fs from "node:fs";
import path from "node:path";

import { glabApi } from "../lib/glab.mjs";
import { git } from "../lib/proc.mjs";

const ZERO = /^0+$/;
const MARKER = "monocircuit push guard (#184)";
/** The hook lives in the repo, so every worktree runs the guard of its own branch; one without it pushes unguarded. */
export const SHIM = `#!/bin/sh
# ${MARKER}, installed by pnpm wt:new and wt:mr.
hook="$(git rev-parse --show-toplevel)/packages/workflow/src/git-hooks/pre-push.mjs"
[ -f "$hook" ] || exit 0
exec node "$hook" "$@"
`;

/**
 * The branches a push updates, from the lines git hands a pre-push hook
 * (`<local ref> <local sha> <remote ref> <remote sha>`). A deletion updates
 * nothing, and a tag is no branch.
 */
export const pushedBranches = input =>
  input
    .split("\n")
    .map(line => line.trim().split(/\s+/))
    .filter(
      ([, localSha, remoteRef]) =>
        localSha &&
        !ZERO.test(localSha) &&
        remoteRef?.startsWith("refs/heads/"),
    )
    .map(([, , remoteRef]) => remoteRef.slice("refs/heads/".length));

/**
 * Why a push to `branch` is refused, or null. `mrs`: its merge requests,
 * newest first. An open one, or none yet, lets the push through; otherwise the
 * newest decides.
 */
export const refusalOf = (branch, mrs) => {
  if (mrs.length === 0 || mrs.some(mr => mr.state === "opened")) return null;
  const [last] = mrs;
  const state = last.state === "merged" ? "merged" : "closed";
  return `✗ push refused: !${last.iid} of ${branch} is ${state}, so no merge request points at this branch any more (#184).
  The push would recreate ${branch} on origin, and what it carries would reach no one.
  New work goes on a new branch: file a follow-up issue and start it with pnpm wt:new <n>-<feature>.
  ${last.web_url ?? ""}`.trimEnd();
};

/** The merge requests of `branch`, newest first. */
const mrsOf = branch =>
  glabApi(
    `projects/:fullpath/merge_requests?state=all&source_branch=${encodeURIComponent(branch)}&order_by=created_at&sort=desc`,
  );

/**
 * The refusals for a push (`input`: the hook's stdin). GitLab not answering
 * lets the push through with a warning: the guard must not block offline work.
 */
export const checkPush = async (input, lookup = mrsOf) => {
  const refusals = [];
  for (const branch of new Set(pushedBranches(input))) {
    try {
      const refusal = refusalOf(branch, await lookup(branch));
      if (refusal) refusals.push(refusal);
    } catch (error) {
      console.error(
        `! push guard: couldn't look up the merge request of ${branch}, pushing unchecked (${error.message.split("\n")[0]})`,
      );
    }
  }
  return refusals;
};

/**
 * Puts the hook into the hooks folder all worktrees share (or core.hooksPath).
 * A pre-push hook that isn't ours stays; the answer says what happened.
 */
export const installPushGuard = cwd => {
  const hooks = git(
    ["rev-parse", "--path-format=absolute", "--git-path", "hooks"],
    cwd,
  ).out;
  if (!hooks) return "no git repository";
  const file = path.join(hooks, "pre-push");
  const current = fs.existsSync(file) ? fs.readFileSync(file, "utf-8") : null;
  if (current === SHIM) return "installed";
  if (current !== null && !current.includes(MARKER))
    return `${file} is someone else's hook, the push guard is not installed`;
  fs.mkdirSync(hooks, { recursive: true });
  fs.writeFileSync(file, SHIM, { mode: 0o755 });
  return current === null ? "installed" : "updated";
};
