// git pre-push hook: refuses a push to the branch of a merged or closed MR
// (#184). Installed as a shim by pnpm wt:new and wt:mr, see push-guard.mjs.
import fs from "node:fs";

import { checkPush } from "../worktree/push-guard.mjs";

const refusals = await checkPush(fs.readFileSync(0, "utf-8"));
if (refusals.length > 0) {
  console.error(refusals.join("\n\n"));
  process.exit(1);
}
