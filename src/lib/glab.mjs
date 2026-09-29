// GitLab through the glab CLI, read only.
import { mainDir } from "../worktree/worktrees.mjs";
import { exec } from "./proc.mjs";

/** `glab api <endpoint>` (a GET), parsed. Async, so a sweep over many issues runs side by side. */
export const glabApi = async endpoint =>
  JSON.parse(
    (
      await exec("glab", ["api", endpoint], {
        cwd: mainDir(),
        maxBuffer: 1 << 24,
      })
    ).stdout,
  );
