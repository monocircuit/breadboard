// No push without passing tests (#298): `pnpm wt:check` runs lint, typecheck
// and test of every package the branch changes (and their dependents) and
// stamps what it tested; the pre-push hook lets only a stamped commit
// through; the overview shows per worktree whether its HEAD is tested.
//
// The stamp names a tree, not a commit: a check of uncommitted changes holds
// for the commit made of them, and a commit with another tree (develop merged
// in, one more change) is untested again.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { git } from "../lib/proc.mjs";
import { mainDir } from "./worktrees.mjs";

/** What wt:check runs, through turbo. */
export const TASKS = ["lint", "typecheck", "test"];
/** What runs in CI only, and why: said in the output when vita is checked. */
export const CI_ONLY = {
  "@monocircuit/vita":
    "vita's integration tests (test:int) need the stack: they run in CI only (check:vita:int)",
};

/**
 * Where a worktree keeps its checks, in its ignored `.claude/`: `{ passed,
 * failed, running }`. `passed`: the last check that passed `{ sha, tree,
 * packages, at, duration }`; `failed`: the last that failed, with `failures`
 * `[{ package, task, tests }]`; `running`: `{ sha, tree, pid, since }` while
 * one runs.
 */
export const checkFile = dir => path.join(dir, ".claude", "checked.json");
export const readCheck = dir => {
  try {
    return JSON.parse(fs.readFileSync(checkFile(dir), "utf-8")) ?? {};
  } catch {
    return {};
  }
};
/** Sets the given parts of the record (null clears one), keeps the others. */
export const writeCheck = (dir, parts) => {
  const record = { ...readCheck(dir), ...parts };
  for (const [key, value] of Object.entries(record))
    if (value == null) delete record[key];
  fs.mkdirSync(path.dirname(checkFile(dir)), { recursive: true });
  fs.writeFileSync(checkFile(dir), `${JSON.stringify(record, null, 2)}\n`);
  return record;
};

/** The tree of a commit, null for none. */
export const treeOf = (dir, rev = "HEAD") =>
  git(["rev-parse", "--verify", "--quiet", `${rev}^{tree}`], dir).out || null;

/**
 * The tree of the working copy as it is, uncommitted and untracked files
 * included (not ignored ones): what a commit of everything would have. Built
 * in a copy of the index, the real one stays as it is.
 */
export const workingTreeOf = dir => {
  const real = git(
    ["rev-parse", "--path-format=absolute", "--git-path", "index"],
    dir,
  ).out;
  const copy = path.join(
    os.tmpdir(),
    `breadboard-check-${process.pid}-${Date.now()}`,
  );
  try {
    if (real && fs.existsSync(real)) fs.copyFileSync(real, copy);
    const env = { ...process.env, GIT_INDEX_FILE: copy };
    const add = spawnSync("git", ["add", "-A"], {
      cwd: dir,
      env,
      encoding: "utf-8",
    });
    if (add.status !== 0) return null;
    const tree = spawnSync("git", ["write-tree"], {
      cwd: dir,
      env,
      encoding: "utf-8",
    });
    return tree.status === 0 ? tree.stdout.trim() : null;
  } finally {
    fs.rmSync(copy, { force: true });
  }
};

/** Where `rev` left develop: the commit the branch is compared against. */
export const baseOf = (dir, rev = "HEAD") =>
  git(["merge-base", "origin/develop", rev], dir).out || null;

/** The turbo of the worktree, else the main checkout's; null before any `pnpm install`. */
export const turboOf = dir =>
  [dir, mainDir()]
    .filter(Boolean)
    .map(root => path.join(root, "node_modules", ".bin", "turbo"))
    .find(file => fs.existsSync(file)) ?? null;

/** The turbo filter of the packages a branch changes and their dependents. */
export const filterOf = (base, sha) => `...[${base}${sha ? `...${sha}` : ""}]`;

/**
 * The packages turbo would check, from its dry run as JSON: `[{ package,
 * tasks }]`, only tasks a package has a script for, the root (`//`) left out.
 */
export const plannedOf = dryRun =>
  Object.values(
    (dryRun.tasks ?? [])
      .filter(
        task =>
          TASKS.includes(task.task) &&
          task.package !== "//" &&
          task.command !== "<NONEXISTENT>",
      )
      .reduce((all, task) => {
        all[task.package] ??= { package: task.package, tasks: [] };
        all[task.package].tasks.push(task.task);
        return all;
      }, {}),
  ).sort((a, b) => a.package.localeCompare(b.package));

/**
 * What turbo would check in `dir` for `filter`: `{ planned }`, or `{ error }`
 * when it can't tell (no turbo, no install).
 */
export const planOf = (dir, filter) => {
  const turbo = turboOf(dir);
  if (!turbo)
    return { error: "turbo isn't installed here: pnpm install first" };
  const result = spawnSync(
    turbo,
    ["run", ...TASKS, `--filter=${filter}`, "--dry-run=json"],
    {
      cwd: dir,
      encoding: "utf-8",
      env: { ...process.env, TURBO_TELEMETRY_DISABLED: "1" },
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  try {
    return { planned: plannedOf(JSON.parse(result.stdout)) };
  } catch {
    const why = (result.stderr || result.stdout || "")
      .trim()
      .split("\n")
      .at(-1);
    return {
      error: `turbo can't tell which packages changed${why ? `: ${why}` : ""}`,
    };
  }
};

const ANSI = /\u001b\[[0-9;]*m/g;
/**
 * What failed in turbo's output: `[{ package, task, tests }]`, `tests` the
 * names of the failing tests (vitest's `FAIL  file > name`, node --test's
 * `✖ name`), each once.
 */
export const failuresOf = output => {
  const text = output.replace(ANSI, "");
  const failed = new Map();
  const add = (id, test) => {
    const [pkg, task] = id.split("#");
    if (!pkg || !task) return;
    if (!failed.has(id)) failed.set(id, { package: pkg, task, tests: [] });
    if (test && !failed.get(id).tests.includes(test))
      failed.get(id).tests.push(test);
  };
  for (const line of text.split("\n")) {
    const summary = line.match(/^\s*Failed:\s+(.+)$/);
    if (summary) for (const id of summary[1].split(/,\s*/)) add(id.trim());
    const exited = line.match(/(?:^|\s)(\S+#\S+): command .* exited \(\d+\)/);
    if (exited) add(exited[1]);
  }
  for (const line of text.split("\n")) {
    const prefixed = line.match(/^(\S+?):(\w[\w:-]*): (.*)$/);
    if (!prefixed) continue;
    const id = `${prefixed[1]}#${prefixed[2]}`;
    if (!failed.has(id) || failed.get(id).task !== "test") continue;
    const body = prefixed[3];
    const vitest = body.match(/^\s*FAIL\s+(\S+\.\w+ > .+?)\s*$/);
    const node = body.match(/^\s*✖ (.+?)(?: \([\d.]+m?s\))?\s*$/);
    if (vitest) add(id, vitest[1]);
    else if (node && node[1] !== "failing tests:") add(id, node[1]);
  }
  return [...failed.values()];
};

/** `@monocircuit/vita (frame.test.ts > keeps …, +2 more)`: what failed, in one line. */
export const failuresText = (failures, count = 2) =>
  failures
    .map(({ package: name, task, tests }) => {
      if (task !== "test" || tests.length === 0) return `${name} ${task}`;
      const shown = tests.slice(0, count).join(", ");
      const rest =
        tests.length > count ? `, +${tests.length - count} more` : "";
      return `${name} (${shown}${rest})`;
    })
    .join(" · ");

/** Whether the process of a running check is still there. */
export const isAlive = pid => {
  if (!pid) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === "EPERM";
  }
};

/**
 * Where the HEAD of a worktree stands with its checks, for the overview:
 * - `checking`: wt:check runs (`since`)
 * - `tested`: the last check that passed tested HEAD's tree (`sha`, `packages`)
 * - `failed`: the last check of HEAD's tree failed (`failures`)
 * - `untested`: none of them; `count` commits since the last one that passed
 *   (since develop without one), 0 when only uncommitted work was checked
 * - `none`: nothing of its own yet (no commit ahead of develop)
 * `record` readCheck's, `head` and `tree` HEAD's commit and tree, `since(sha)`
 * how many commits HEAD has after `sha` (null when it isn't an ancestor),
 * `ahead` how many it has after develop.
 */
export const checkStateOf = ({
  record,
  head,
  tree,
  since,
  ahead,
  alive = isAlive,
}) => {
  const { passed, failed, running } = record ?? {};
  if (running && alive(running.pid))
    return { key: "checking", since: running.since };
  if (passed && passed.tree === tree)
    return { key: "tested", sha: head, packages: passed.packages ?? [] };
  if (failed && failed.tree === tree)
    return { key: "failed", sha: head, failures: failed.failures ?? [] };
  if (!passed && !ahead) return { key: "none" };
  const count = passed ? (since(passed.sha) ?? ahead) : ahead;
  return { key: "untested", count };
};

/** checkStateOf for the worktree `dir`, from git and its record. */
export const checkOf = dir => {
  const head = git(["rev-parse", "--verify", "--quiet", "HEAD"], dir).out;
  if (!head) return { key: "none" };
  const count = range => {
    const out = git(["rev-list", "--count", range], dir);
    return out.ok ? Number(out.out) : null;
  };
  return checkStateOf({
    record: readCheck(dir),
    head,
    tree: treeOf(dir),
    since: sha =>
      git(["merge-base", "--is-ancestor", sha, "HEAD"], dir).ok
        ? count(`${sha}..HEAD`)
        : null,
    ahead: count("origin/develop..HEAD") ?? 0,
  });
};

/**
 * Why the push of `sha` to `branch` is refused, or null. `record` the
 * worktree's checks, `tree` the commit's tree, `plan` planOf for its
 * changes against develop: `{ planned }` (none: docs only) or `{ error }`;
 * `isEmpty`: nothing changed against develop (a claim, wt:new).
 */
export const gateRefusalOf = ({ branch, sha, tree, record, plan, isEmpty }) => {
  if (isEmpty) return null;
  if (record?.passed?.tree && record.passed.tree === tree) return null;
  if (plan?.planned && plan.planned.length === 0) return null;
  const short = sha.slice(0, 8);
  const packages = plan?.planned?.map(p => p.package).join(", ");
  const why =
    record?.failed?.tree === tree
      ? `its last check failed: ${failuresText(record.failed.failures ?? [])}.
  Fix it, commit, run pnpm wt:check again.`
      : plan?.error
        ? `which packages it changes can't be told (${plan.error}).
  Run pnpm install, then pnpm wt:check.`
        : record?.passed
          ? `the last check that passed was of ${record.passed.sha.slice(0, 8)}, not of this commit.
  Run pnpm wt:check (it runs only what changed, a second run is quick).`
          : `no check of it ran in this worktree yet.
  Run pnpm wt:check: lint, typecheck and test of ${packages ?? "the packages it changes"}.`;
  return `✗ push refused: ${branch} at ${short} isn't checked (#298), ${why}
  Push once it passed. Never --no-verify: a failing check is fixed, not pushed.`;
};

/**
 * The refusals of the check gate for a push (`refs`: pushedRefs of the
 * hook's input) from the worktree `dir`. A docs-only commit passes, and is
 * stamped when it is HEAD: the overview shows it tested.
 */
export const checkGate = (refs, dir, { plan = planOf } = {}) => {
  const record = readCheck(dir);
  const refusals = [];
  for (const { branch, sha } of refs) {
    const tree = treeOf(dir, sha);
    const base = baseOf(dir, sha);
    const isEmpty =
      Boolean(base) && git(["diff", "--quiet", base, sha], dir).ok;
    const isStamped = record.passed?.tree && record.passed.tree === tree;
    const planned =
      isEmpty || isStamped
        ? null
        : base
          ? plan(dir, filterOf(base, sha))
          : { error: "no merge base with origin/develop: git fetch origin" };
    const refusal = gateRefusalOf({
      branch,
      sha,
      tree,
      record,
      plan: planned,
      isEmpty,
    });
    if (refusal) refusals.push(refusal);
    else if (planned?.planned?.length === 0 && tree === treeOf(dir))
      writeCheck(dir, {
        passed: {
          sha,
          tree,
          packages: [],
          at: new Date().toISOString(),
          duration: 0,
        },
      });
  }
  return refusals;
};
