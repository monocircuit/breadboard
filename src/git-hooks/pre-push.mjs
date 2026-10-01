// git pre-push hook: refuses a push to the branch of a merged or closed MR
// (#184), and one of a commit `pnpm wt:check` didn't pass (#298). Installed
// as a shim by pnpm wt:new and wt:mr, see push-guard.mjs.
import fs from "node:fs";

import { git } from "../lib/proc.mjs";
import { checkGate } from "../worktree/check.mjs";
import { checkPush, pushedRefs } from "../worktree/push-guard.mjs";

const input = fs.readFileSync(0, "utf-8");
const dir = git(["rev-parse", "--show-toplevel"]).out;
const refusals = [
  ...(await checkPush(input)),
  ...checkGate(pushedRefs(input), dir),
];
if (refusals.length > 0) {
  console.error(refusals.join("\n\n"));
  process.exit(1);
}
