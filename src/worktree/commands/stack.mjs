import { isInHerdr } from "../../lib/herdr.mjs";
import { fail, git } from "../../lib/proc.mjs";
import { syncStackTabs } from "../layout.mjs";
import { openApp, worktreeSpace } from "../spaces.mjs";
import { createStack, summary } from "../stack.mjs";
import { featureOf } from "../tabs.mjs";
import { mainDir } from "../worktrees.mjs";

export default async function stackCommand() {
  const top = git(["rev-parse", "--show-toplevel"]).out;
  if (top === mainDir())
    fail("run it inside a worktree, the main checkout has the default stack");
  const feature = featureOf(top);
  const offset = await createStack(top, feature);
  const changed = isInHerdr()
    ? await syncStackTabs(top, worktreeSpace(top, { hasWorkTab: false }))
    : [];
  openApp(top, offset);
  summary(
    top,
    git(["branch", "--show-current"], top).out,
    offset,
    changed.length ? `stack tabs ${changed.join(" ")}` : null,
  );
}
