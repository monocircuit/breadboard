import { spawn } from "node:child_process";

import { fail, git, step } from "../../lib/proc.mjs";
import {
  CI_ONLY,
  TASKS,
  baseOf,
  failuresOf,
  failuresText,
  filterOf,
  planOf,
  turboOf,
  workingTreeOf,
  writeCheck,
} from "../check.mjs";

/** Runs turbo, its output shown as it comes and kept: `{ code, output }`. */
const runTurbo = (turbo, args, cwd) =>
  new Promise(resolve => {
    const child = spawn(turbo, args, {
      cwd,
      env: { ...process.env, TURBO_TELEMETRY_DISABLED: "1" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let output = "";
    child.stdout.on("data", chunk => {
      output += chunk;
      process.stdout.write(chunk);
    });
    child.stderr.on("data", chunk => {
      output += chunk;
      process.stderr.write(chunk);
    });
    child.on("close", code => resolve({ code, output }));
  });

/**
 * `pnpm wt:check` (#298): lint, typecheck and test of every package the
 * branch changes against develop, and the packages that depend on them,
 * through turbo (its cache keeps a second run on the same tree quick). On
 * success it stamps the tree it tested (.claude/checked.json): the pre-push
 * hook lets a commit of that tree through. Uncommitted changes count, so
 * check, then commit and push.
 */
export default async function checkCommand() {
  const dir = git(["rev-parse", "--show-toplevel"]).out;
  if (!dir) fail("wt:check runs in a checkout of the repo");
  const turbo = turboOf(dir);
  if (!turbo) fail("turbo isn't installed here: pnpm install first");
  const base = baseOf(dir);
  if (!base) fail("no merge base with origin/develop: git fetch origin first");
  const sha = git(["rev-parse", "HEAD"], dir).out;
  const tree = workingTreeOf(dir);
  if (!tree)
    fail(
      "couldn't read the working tree (git add -A in a copy of the index failed)",
    );

  const filter = filterOf(base);
  const plan = planOf(dir, filter);
  if (plan.error) fail(plan.error);
  const started = Date.now();
  const stamp = { sha, tree, packages: plan.planned.map(p => p.package) };
  if (plan.planned.length === 0) {
    writeCheck(dir, {
      passed: { ...stamp, at: new Date().toISOString(), duration: 0 },
      failed: null,
      running: null,
    });
    console.log(
      `\n✓ nothing to check: the branch changes no package against develop (${base.slice(0, 8)}); ${sha.slice(0, 8)} may be pushed\n`,
    );
    return;
  }

  step(
    `${TASKS.join(", ")} of ${plan.planned.length} package${plan.planned.length === 1 ? "" : "s"}, changed against develop (${base.slice(0, 8)}) or depending on one:`,
  );
  for (const { package: name, tasks } of plan.planned)
    console.log(`  ${name}  ${tasks.join(" ")}`);
  for (const name of stamp.packages)
    if (CI_ONLY[name]) console.log(`  (${CI_ONLY[name]})`);
  console.log("");

  writeCheck(dir, {
    running: { sha, tree, pid: process.pid, since: new Date().toISOString() },
  });
  const stop = () => {
    writeCheck(dir, { running: null });
    process.exit(130);
  };
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);

  const { code, output } = await runTurbo(
    turbo,
    [
      "run",
      ...TASKS,
      `--filter=${filter}`,
      "--continue",
      "--output-logs=errors-only",
      "--log-order=grouped",
    ],
    dir,
  );
  const duration = Math.round((Date.now() - started) / 1000);
  const at = new Date().toISOString();

  if (code !== 0) {
    const failures = failuresOf(output);
    writeCheck(dir, {
      running: null,
      failed: { ...stamp, failures, at, duration },
    });
    fail(
      `wt:check failed after ${duration} s: ${failures.length ? failuresText(failures, 5) : "see turbo's output above"}
  Fix it, commit, run pnpm wt:check again: the push waits for it.
  Red on develop too: it is fixed once, in its own fix MR; wait for that with pnpm wt:wait !<mr>.`,
    );
  }
  // An edit while it ran: what passed isn't what is there now.
  if (workingTreeOf(dir) !== tree) {
    writeCheck(dir, { running: null });
    fail(
      "the files changed while wt:check ran: what passed isn't what is there now. Run it again.",
    );
  }
  writeCheck(dir, {
    running: null,
    failed: null,
    passed: { ...stamp, at, duration },
  });
  const head = git(["rev-parse", "HEAD"], dir).out;
  const isDirty = git(["status", "--porcelain"], dir).out !== "";
  console.log(
    `\n✓ checked in ${duration} s: ${stamp.packages.join(", ")}\n  ${
      isDirty
        ? "commit these changes as they are, then push: the commit is stamped as checked"
        : `${head.slice(0, 8)} may be pushed`
    }\n`,
  );
}
