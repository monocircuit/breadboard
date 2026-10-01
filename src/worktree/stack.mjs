// The vita dev stacks: ports per worktree, docker compose, creating and
// handing over a stack.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";

import { currentPane, isAgent, isInHerdr, spaces } from "../lib/herdr.mjs";
import { fail, git, run, step } from "../lib/proc.mjs";
import { PORTS, VITA } from "./constants.mjs";
import { sessionFile } from "./conversations.mjs";
import { orchestratorLabel } from "./origins.mjs";
import { hasOwnCommits, isInside } from "./prune.mjs";
import { featureOf } from "./tabs.mjs";
import { mainDir, worktrees } from "./worktrees.mjs";
import { ENTRY } from "./constants.mjs";

export const readEnv = file =>
  fs.existsSync(file)
    ? Object.fromEntries(
        fs
          .readFileSync(file, "utf-8")
          .split("\n")
          .filter(l => l.includes("=") && !l.trimStart().startsWith("#"))
          .map(l => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
      )
    : null;
/** The offset of a worktree's own stack, 0 for the shared one. */
export const offsetOf = dir => {
  const env = readEnv(path.join(dir, VITA, ".env.local"));
  return env?.COMPOSE_PROJECT_NAME && env.POSTGRES_PORT
    ? Number(env.POSTGRES_PORT) - 5432
    : 0;
};
export const isPortFree = port =>
  new Promise(resolve => {
    const server = net.createServer();
    server.once("error", () => resolve(false));
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true)));
  });
/** The smallest offset whose ports are free and not claimed by a (stopped) stack of another worktree. */
const freeOffset = async () => {
  const claimed = new Set(worktrees().map(w => offsetOf(w.dir)));
  for (let offset = 1; offset < 50; offset++) {
    if (claimed.has(offset)) continue;
    const free = await Promise.all(
      Object.keys(PORTS).map(p => isPortFree(Number(p) + offset)),
    );
    if (free.every(Boolean)) return offset;
  }
  fail("no free ports found for a stack");
};
/** The .env.local of the main checkout, with every port moved by `offset` and an own compose project. */
const stackEnv = (source, feature, offset) => {
  const skip = new Set([
    "COMPOSE_PROJECT_NAME",
    ...Object.values(PORTS).filter(Boolean),
  ]);
  const lines = fs
    .readFileSync(source, "utf-8")
    .split("\n")
    .filter(l => !skip.has(l.slice(0, l.indexOf("="))))
    .map(line => {
      if (!line.includes("=") || line.trimStart().startsWith("#")) return line;
      let value = line.slice(line.indexOf("=") + 1);
      for (const port of Object.keys(PORTS)) {
        value = value.replace(
          new RegExp(`(localhost|127\\.0\\.0\\.1):${port}\\b`, "g"),
          `$1:${Number(port) + offset}`,
        );
      }
      return `${line.slice(0, line.indexOf("="))}=${value}`;
    });
  while (lines.at(-1) === "") lines.pop();
  return [
    ...lines,
    "",
    `# --- own dev stack of the worktree ${feature}, written by packages/breadboard/src/worktree.mjs ---`,
    `COMPOSE_PROJECT_NAME=vita-${feature}`,
    ...Object.entries(PORTS)
      .filter(([, variable]) => variable)
      .map(([port, variable]) => `${variable}=${Number(port) + offset}`),
    "",
  ].join("\n");
};
export const compose = (dir, args, options) =>
  run(
    "docker",
    [
      "compose",
      "--env-file",
      ".env.local",
      "-f",
      "docker-compose.local.yml",
      ...args,
    ],
    {
      cwd: path.join(dir, VITA),
      ...options,
    },
  );
/** Moves a worktree from the shared stack to an own one and starts it. */
export const createStack = async (dir, feature) => {
  const source = path.join(mainDir(), VITA, ".env.local");
  const target = path.join(dir, VITA, ".env.local");
  if (!fs.existsSync(source))
    fail(`${source} is missing, set up the main checkout first`);
  if (offsetOf(dir))
    fail(`${feature} has its own stack already (pnpm wt:list)`);
  if (
    fs.existsSync(target) &&
    fs.readFileSync(target, "utf-8") !== fs.readFileSync(source, "utf-8")
  ) {
    fail(
      `${target} differs from the one of the main checkout, move it away first`,
    );
  }

  const offset = await freeOffset();
  step(
    `own stack on offset ${offset}: app ${3000 + offset}, Postgres ${5432 + offset}, Keycloak ${8080 + offset}, SeaweedFS ${8333 + offset}`,
  );
  fs.writeFileSync(target, stackEnv(source, feature, offset));

  step("pnpm dev:setup (containers, migrations, bucket, Keycloak, seed)");
  run("pnpm", ["dev:setup"], { cwd: path.join(dir, VITA) });
  return offset;
};
/**
 * `34-…` claims issue 34, and a branch of someone else carrying that number
 * means the issue is taken. Found after the work it is worth nothing, so it is
 * looked up before the worktree exists.
 */
export const claimedElsewhere = feature => {
  const number = feature.match(/^(\d+)-/)?.[1];
  if (!number) return null;
  return git(["ls-remote", "--heads", "origin", `*${number}-*`])
    .out.split("\n")
    .map(line => line.split("refs/heads/")[1])
    .find(Boolean);
};
/** What `pnpm stack:<action>` runs in products/vita/web of the worktree it names. */
export const STACK_ACTIONS = {
  up: ["pnpm", ["stack:up"]],
  down: ["pnpm", ["stack:down"]],
  reset: ["pnpm", ["stack:reset"]],
  ps: [
    "docker",
    [
      "compose",
      "--env-file",
      ".env.local",
      "-f",
      "docker-compose.local.yml",
      "ps",
    ],
  ],
};
/**
 * The worktree whose stack answers on `port`: the app port `3000+n` or the port
 * of one of its services (Postgres `5432+n`, …). Offset 0 is the main checkout,
 * whose stack the worktrees on the shared one use too. The offsets stay below
 * 50 (freeOffset), so the ranges of the bases don't overlap.
 */
export const worktreeOnPort = port => {
  const offset = Object.keys(PORTS)
    .map(base => port - Number(base))
    .find(candidate => candidate >= 0 && candidate < 50);
  if (offset === undefined) return null;
  const all = worktrees();
  return offset === 0
    ? all[0]
    : (all.find(w => offsetOf(w.dir) === offset) ?? null);
};
/**
 * The worktree an agent comes from when it starts the next piece of work in
 * the same conversation, or null: the space its tab stands in (a `cd` in its
 * shell says nothing, the session stays where it was), else where it ran this.
 */
export const leavingWorktree = () => {
  if (!isAgent()) return null;
  const pane = isInHerdr() ? currentPane() : null;
  const label =
    pane && spaces().find(w => w.workspace_id === pane.workspace_id)?.label;
  const here = path.resolve(process.env.INIT_CWD ?? process.cwd());
  return (
    worktrees()
      .slice(1)
      .find(({ dir }) => featureOf(dir) === label || isInside(here, dir)) ??
    null
  );
};
/**
 * The conversation left `previous` for the new worktree. Its binding goes, or
 * a restart of herdr would resume it in both spaces. When the work there is
 * done — the branch merged and gone from origin, nothing uncommitted — the
 * worktree goes too, space and all, like wt:prune would; else it stays as it
 * is. Returns a line for the summary.
 */
export const handOver = previous => {
  fs.rmSync(sessionFile(previous.dir), { force: true });
  const feature = featureOf(previous.dir);
  const isClean = !git(["status", "--porcelain"], previous.dir).out;
  const isOnOrigin =
    previous.branch &&
    git(["ls-remote", "--exit-code", "--heads", "origin", previous.branch]).ok;
  const isDone =
    previous.branch &&
    isClean &&
    !isOnOrigin &&
    !hasOwnCommits(previous.branch, previous.dir);
  if (!isDone) {
    const why = !isClean
      ? "uncommitted changes"
      : isOnOrigin
        ? "its branch is still on origin"
        : "commits not in develop";
    return `${feature} stays (${why}): its space keeps overview and nvim`;
  }
  const removed = spawnSync(
    process.execPath,
    [ENTRY, "remove", feature, "--close-space"],
    {
      cwd: mainDir(),
      encoding: "utf-8",
    },
  );
  return removed.status === 0
    ? `${feature} removed, its work is in develop`
    : `${feature} could not be removed: ${removed.stderr.trim().split("\n").at(-1)}`;
};
export const summary = (
  dir,
  branch,
  offset,
  tab,
  stale = 0,
  silent = 0,
  left = null,
  origin = null,
) => {
  const port = 3000 + offset;
  // wt:orchestrate reads it: it starts the agent and says the rest.
  if (origin) {
    console.log(
      `\n✓ ${branch} pushed, worktree ready for ${orchestratorLabel(origin.name)}${tab ? `, herdr: ${tab}` : ""}${offset ? `, own stack +${offset}` : ""}\n`,
    );
    return;
  }
  // Merged worktrees and their issues are nobody's job until someone starts the next thing.
  const sweep = [
    stale
      ? `\n  ${stale} worktree${stale === 1 ? "" : "s"}/branch${stale === 1 ? "" : "es"} of merged work left: pnpm wt:prune`
      : "",
    silent
      ? `\n  ${silent} issue${silent === 1 ? "" : "s"} of merged work without a comment since the merge: pnpm wt:issues`
      : "",
  ].join("");
  // An agent pays for every line it reads. It gets the facts it acts on.
  if (isAgent()) {
    // From one worktree EnterWorktree refuses another of ours: the session leaves the old one first.
    const enter = left
      ? `ExitWorktree(action: "keep"), then EnterWorktree(path: "${dir}")\n  ${left.line}`
      : `EnterWorktree(path: "${dir}")`;
    console.log(`
✓ ${branch} pushed, worktree ready${tab ? `, herdr: ${tab}` : ""}
  ${enter}${offset ? `\n  own stack +${offset}: app ${port}, Postgres ${5432 + offset}` : ""}${sweep}
`);
    return;
  }
  console.log(`${tab ? `\n▸ herdr: ${tab}` : ""}
✓ ${path.basename(dir)} is ready

  directory  ${dir}
  branch     ${branch} (from origin/develop)
  stack      ${offset ? `own: Postgres ${5432 + offset}, Keycloak ${8080 + offset}, SeaweedFS ${8333 + offset}` : "shared with the main checkout (pnpm wt:stack for an own one)"}

  cd ${dir} && claude
  agent: EnterWorktree(path: "${dir}")   ← move the session, don't cd
  vita:  cd ${VITA} && pnpm dev${offset ? ` --port ${port}` : ""}   → http://localhost:${port}${offset ? "" : " (only one checkout at a time)"}${sweep}
`);
};
