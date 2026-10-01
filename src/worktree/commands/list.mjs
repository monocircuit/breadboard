import fs from "node:fs";
import path from "node:path";

import { isInside } from "../prune.mjs";
import { compose, envFileOf, isPortFree, offsetOf } from "../stack.mjs";
import { worktrees } from "../worktrees.mjs";

/**
 * Which version runs where. `●` means something answers on that port, and
 * with an own stack that is this worktree. Worktrees on the shared stack all
 * point at 3000 and which of them serves it can't be seen from outside, so
 * they say that instead of claiming it.
 */
export default async function listCommand() {
  const here = path.resolve(process.cwd());
  const rows = [];
  for (const [index, { dir, branch }] of worktrees().entries()) {
    const offset = index === 0 ? 0 : offsetOf(dir);
    const hasEnv = fs.existsSync(envFileOf(dir));
    const hasOwnPort = index === 0 || offset > 0;
    const isUp = hasEnv && hasOwnPort && !(await isPortFree(3000 + offset));
    const containers = offset
      ? compose(dir, ["ps", "-q"], { quiet: true, allowFail: true })
          .out.split("\n")
          .filter(Boolean).length
      : null;
    rows.push({
      here: isInside(here, dir) ? "→" : " ",
      name: path.basename(dir),
      branch: branch ?? "(detached)",
      app: !hasEnv
        ? "\u25cb no .env.local"
        : hasOwnPort
          ? `${isUp ? "\u25cf" : "\u25cb"} http://localhost:${3000 + offset}`
          : "\u21d2 http://localhost:3000 (shared)",
      stack:
        index === 0
          ? "the default stack (pg 5432)"
          : !hasEnv
            ? "none"
            : !offset
              ? "shared with the main checkout — only one app at a time"
              : `own +${offset} (pg ${5432 + offset}), ${containers ? `${containers} containers up` : "stopped"}`,
    });
  }
  const width = key => Math.max(...rows.map(row => row[key].length));
  console.log("");
  for (const row of rows) {
    console.log(
      `${row.here} ${row.name.padEnd(width("name"))}  ${row.branch.padEnd(width("branch"))}  ${row.app.padEnd(width("app"))}  ${row.stack}`,
    );
  }
  console.log(
    `\n  \u25cf answers  \u25cb nothing there  \u21d2 shares the port of the main checkout  \u2192 you are here\n`,
  );
}
