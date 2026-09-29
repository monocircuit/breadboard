import fs from "node:fs";
import path from "node:path";

import { isInHerdr } from "../../lib/herdr.mjs";
import { fail, flagValue, run, step } from "../../lib/proc.mjs";
import { VITA } from "../constants.mjs";
import { syncStackTabs } from "../layout.mjs";
import { isInside } from "../prune.mjs";
import { STACK_ACTIONS, offsetOf, worktreeOnPort } from "../stack.mjs";
import { dirOf, mainDir, worktrees } from "../worktrees.mjs";

/**
 * `pnpm stack:down --port 3002`: the stack of another worktree — an agent's —
 * without looking for its directory. The port is the one `wt:list` and the
 * herdr tab show; the name (`pnpm stack:down 40-worktree-ports`) works too.
 * Without either it is the stack of the checkout you are in.
 */
export default async function controlCommand(action, flags) {
  const usage = `usage: pnpm stack:${Object.keys(STACK_ACTIONS).join("|")} [--port <port> | <feature>]`;
  if (!STACK_ACTIONS[action]) fail(usage);
  const port = flagValue("--port");
  const argv = process.argv.slice(4);
  const name = argv.find(
    (arg, index) => !arg.startsWith("--") && argv[index - 1] !== "--port",
  );
  if (flags.has("--port") && !port) fail(usage);

  let tree;
  if (port) {
    if (!/^\d+$/.test(port)) fail(`--port ${port} is not a port`);
    tree = worktreeOnPort(Number(port));
    if (!tree) fail(`no worktree has a stack on port ${port} (pnpm wt:list)`);
  } else if (name) {
    tree = worktrees().find(
      w => w.dir === dirOf(name) || path.basename(w.dir) === name,
    );
    if (!tree) fail(`${name} is not a worktree (pnpm wt:list)`);
  } else {
    const here = path.resolve(process.cwd());
    tree = worktrees().findLast(w => isInside(here, w.dir)) ?? worktrees()[0];
  }

  const { dir } = tree;
  if (!fs.existsSync(path.join(dir, VITA, ".env.local")))
    fail(`${path.basename(dir)} has no .env.local, so no stack`);
  const offset = dir === mainDir() ? 0 : offsetOf(dir);
  // A worktree on the shared stack acts on the one of the main checkout, and
  // that one belongs to every worktree on it: say so, don't do it quietly.
  const whose =
    dir !== mainDir() && !offset
      ? ", that is the shared stack of the main checkout"
      : "";
  step(
    `stack ${action}: ${path.basename(dir)} (app ${3000 + offset}, Postgres ${5432 + offset})${whose}`,
  );
  const [command, args] = STACK_ACTIONS[action];
  run(command, args, { cwd: path.join(dir, VITA) });
  // The overview of the main space would notice too, a few seconds later.
  if (isInHerdr() && action !== "ps") {
    const changed = await syncStackTabs(dir);
    if (changed.length) step(`herdr: ${changed.join(" ")}`);
  }
}
