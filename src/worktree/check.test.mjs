import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, describe, it } from "node:test";

import {
  checkGate,
  checkOf,
  checkStateOf,
  failuresOf,
  failuresText,
  gateRefusalOf,
  plannedOf,
  readCheck,
  treeOf,
  workingTreeOf,
  writeCheck,
} from "./check.mjs";
import { pushedRefs } from "./push-guard.mjs";

const SHA = "38d9f64aa1b2c3d4e5f60718293a4b5c6d7e8f90";
const WORKTREE = path.join(import.meta.dirname, "..", "worktree.mjs");

const dirs = [];
after(() =>
  dirs.forEach(dir => fs.rmSync(dir, { recursive: true, force: true })),
);
const sh = (dir, ...args) => {
  const result = spawnSync("git", args, { cwd: dir, encoding: "utf-8" });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
};
/** A repo with one commit on `origin/develop` and a branch `feature/1-a` on it. */
const repo = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "breadboard-check-"));
  dirs.push(dir);
  sh(dir, "init", "-q", "-b", "feature/1-a");
  sh(dir, "config", "user.email", "test@example.com");
  sh(dir, "config", "user.name", "Test");
  fs.writeFileSync(path.join(dir, ".gitignore"), ".claude/\nnode_modules/\n");
  fs.mkdirSync(path.join(dir, "packages", "a"), { recursive: true });
  fs.writeFileSync(path.join(dir, "packages", "a", "index.js"), "1\n");
  sh(dir, "add", "-A");
  sh(dir, "commit", "-q", "-m", "base");
  sh(dir, "update-ref", "refs/remotes/origin/develop", "HEAD");
  return dir;
};
const commit = (dir, file, text) => {
  fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
  fs.writeFileSync(path.join(dir, file), text);
  sh(dir, "add", "-A");
  sh(dir, "commit", "-q", "-m", file);
  return sh(dir, "rev-parse", "HEAD");
};
const refsOf = (dir, sha) =>
  pushedRefs(
    `refs/heads/feature/1-a ${sha} refs/heads/feature/1-a ${"0".repeat(40)}`,
  );
/** A turbo that plans `packages`: one with a test, none for docs only. */
const planned =
  (...packages) =>
  () => ({
    planned: packages.map(name => ({ package: name, tasks: ["lint", "test"] })),
  });

describe("failuresOf", () => {
  it("takes the failed tasks and the names of their failing tests", () => {
    const output = [
      "@monocircuit/vita:test:  FAIL  src/timeline/frame.test.ts > frame > keeps the edge",
      "@monocircuit/vita:test:  FAIL  src/timeline/frame.test.ts > frame > keeps the edge",
      "@monocircuit/vita:test:  FAIL  src/a.test.ts > b",
      "@monocircuit/breadboard:test: ✖ refuses a stale HEAD (1.2ms)",
      "@monocircuit/breadboard:test: ✖ failing tests:",
      "@monocircuit/breadboard:lint: ✖ 2 problems (1 error, 1 warning)",
      " Tasks:    3 successful, 6 total",
      "Failed:    @monocircuit/vita#test, @monocircuit/breadboard#test, @monocircuit/breadboard#lint",
    ].join("\n");
    assert.deepEqual(failuresOf(output), [
      {
        package: "@monocircuit/vita",
        task: "test",
        tests: [
          "src/timeline/frame.test.ts > frame > keeps the edge",
          "src/a.test.ts > b",
        ],
      },
      {
        package: "@monocircuit/breadboard",
        task: "test",
        tests: ["refuses a stale HEAD"],
      },
      { package: "@monocircuit/breadboard", task: "lint", tests: [] },
    ]);
  });
  it("reads turbo's error line when there is no summary, colours and all", () => {
    const output =
      "\u001b[31m ERROR \u001b[0m @monocircuit/vita#typecheck: command (/x) pnpm run typecheck exited (2)";
    assert.deepEqual(failuresOf(output), [
      { package: "@monocircuit/vita", task: "typecheck", tests: [] },
    ]);
  });
  it("puts them in one line, the rest of the tests counted", () => {
    assert.equal(
      failuresText([
        {
          package: "@monocircuit/vita",
          task: "test",
          tests: ["a > 1", "a > 2", "a > 3"],
        },
        { package: "@monocircuit/breadboard", task: "lint", tests: [] },
      ]),
      "@monocircuit/vita (a > 1, a > 2, +1 more) · @monocircuit/breadboard lint",
    );
  });
});

describe("plannedOf", () => {
  it("keeps the tasks a package has a script for, not the root's", () => {
    const dryRun = {
      tasks: [
        { package: "//", task: "test", command: "pnpm -r test" },
        { package: "b", task: "lint", command: "eslint ." },
        { package: "b", task: "typecheck", command: "<NONEXISTENT>" },
        { package: "a", task: "test", command: "vitest run" },
        { package: "a", task: "transit", command: "<NONEXISTENT>" },
        { package: "c", task: "build", command: "tsc" },
      ],
    };
    assert.deepEqual(plannedOf(dryRun), [
      { package: "a", tasks: ["test"] },
      { package: "b", tasks: ["lint"] },
    ]);
  });
});

describe("checkStateOf", () => {
  const base = {
    head: SHA,
    tree: "t2",
    since: () => 2,
    ahead: 5,
    alive: () => true,
  };
  it("is checking while wt:check runs", () => {
    const record = { running: { pid: 1, since: "x" }, passed: { tree: "t2" } };
    assert.equal(checkStateOf({ ...base, record }).key, "checking");
    assert.equal(
      checkStateOf({ ...base, record, alive: () => false }).key,
      "tested",
    );
  });
  it("is tested when the last pass tested HEAD's tree", () => {
    assert.deepEqual(
      checkStateOf({
        ...base,
        record: { passed: { tree: "t2", packages: ["a"] } },
      }),
      {
        key: "tested",
        sha: SHA,
        packages: ["a"],
      },
    );
  });
  it("is failed when the last check of HEAD's tree failed", () => {
    const failures = [{ package: "a", task: "test", tests: ["x"] }];
    const record = {
      passed: { tree: "t1", sha: "s1" },
      failed: { tree: "t2", failures },
    };
    assert.deepEqual(checkStateOf({ ...base, record }), {
      key: "failed",
      sha: SHA,
      failures,
    });
  });
  it("counts the commits since the last pass, else since develop", () => {
    assert.deepEqual(
      checkStateOf({ ...base, record: { passed: { tree: "t1", sha: "s1" } } }),
      {
        key: "untested",
        count: 2,
      },
    );
    assert.deepEqual(checkStateOf({ ...base, record: {} }), {
      key: "untested",
      count: 5,
    });
    // The last pass isn't in HEAD's history (a rebase): since develop.
    assert.equal(
      checkStateOf({
        ...base,
        since: () => null,
        record: { passed: { tree: "t1", sha: "s1" } },
      }).count,
      5,
    );
  });
  it("says nothing for a branch without commits of its own", () => {
    assert.deepEqual(checkStateOf({ ...base, ahead: 0, record: {} }), {
      key: "none",
    });
  });
});

describe("gateRefusalOf", () => {
  const push = { branch: "feature/1-a", sha: SHA, tree: "t2" };
  it("lets a stamped commit, a docs-only one and an empty branch through", () => {
    assert.equal(
      gateRefusalOf({ ...push, record: { passed: { tree: "t2" } } }),
      null,
    );
    assert.equal(
      gateRefusalOf({ ...push, record: {}, plan: { planned: [] } }),
      null,
    );
    assert.equal(gateRefusalOf({ ...push, record: {}, isEmpty: true }), null);
  });
  it("refuses a commit never checked, naming the command and the packages", () => {
    const refusal = gateRefusalOf({
      ...push,
      record: {},
      plan: planned("@monocircuit/vita")(),
    });
    assert.match(
      refusal,
      /push refused: feature\/1-a at 38d9f64a isn't checked/,
    );
    assert.match(
      refusal,
      /pnpm wt:check: lint, typecheck and test of @monocircuit\/vita/,
    );
    assert.match(refusal, /Never --no-verify/);
  });
  it("refuses a stale stamp, naming the commit it was of", () => {
    const refusal = gateRefusalOf({
      ...push,
      record: { passed: { tree: "t1", sha: "aaaaaaaabbbb" } },
      plan: planned("a")(),
    });
    assert.match(
      refusal,
      /last check that passed was of aaaaaaaa, not of this commit/,
    );
  });
  it("names what failed when the last check of the commit failed", () => {
    const failures = [{ package: "a", task: "test", tests: ["x > y"] }];
    assert.match(
      gateRefusalOf({
        ...push,
        record: { failed: { tree: "t2", failures } },
        plan: planned("a")(),
      }),
      /its last check failed: a \(x > y\)[\s\S]*own fix MR[\s\S]*pnpm wt:wait/,
    );
  });
  it("says why when turbo can't tell, instead of locking out quietly", () => {
    assert.match(
      gateRefusalOf({
        ...push,
        record: {},
        plan: { error: "turbo isn't installed here" },
      }),
      /can't be told \(turbo isn't installed here\)[\s\S]*pnpm install, then pnpm wt:check/,
    );
  });
});

describe("checkGate", () => {
  it("lets the claim of wt:new through: nothing changed against develop", () => {
    const dir = repo();
    const head = sh(dir, "rev-parse", "HEAD");
    assert.deepEqual(
      checkGate(refsOf(dir, head), dir, { plan: () => assert.fail("no plan") }),
      [],
    );
  });
  it("refuses an unstamped commit and a stale HEAD, lets the stamped one through", () => {
    const dir = repo();
    const first = commit(dir, "packages/a/index.js", "2\n");
    assert.equal(
      checkGate(refsOf(dir, first), dir, { plan: planned("a") }).length,
      1,
    );
    writeCheck(dir, {
      passed: { sha: first, tree: treeOf(dir), packages: ["a"] },
    });
    assert.deepEqual(
      checkGate(refsOf(dir, first), dir, { plan: planned("a") }),
      [],
    );
    const second = commit(dir, "packages/a/index.js", "3\n");
    const [refusal] = checkGate(refsOf(dir, second), dir, {
      plan: planned("a"),
    });
    assert.match(
      refusal,
      new RegExp(`was of ${first.slice(0, 8)}, not of this commit`),
    );
  });
  it("passes a docs-only commit and stamps it for the overview", () => {
    const dir = repo();
    const head = commit(dir, "README.md", "docs\n");
    assert.deepEqual(
      checkGate(refsOf(dir, head), dir, { plan: planned() }),
      [],
    );
    assert.equal(readCheck(dir).passed.tree, treeOf(dir));
    assert.equal(checkOf(dir).key, "tested");
  });
});

describe("workingTreeOf", () => {
  it("is the tree the commit of every change gets, the index left as it is", () => {
    const dir = repo();
    fs.writeFileSync(path.join(dir, "packages", "a", "index.js"), "changed\n");
    fs.writeFileSync(path.join(dir, "packages", "a", "new.js"), "new\n");
    const tree = workingTreeOf(dir);
    assert.equal(
      sh(dir, "status", "--porcelain"),
      "M packages/a/index.js\n?? packages/a/new.js",
    );
    assert.notEqual(treeOf(dir), tree);
    sh(dir, "add", "-A");
    sh(dir, "commit", "-q", "-m", "all");
    assert.equal(treeOf(dir), tree);
  });
});

describe("wt:check", () => {
  /** A repo whose turbo plans package `a` and exits with `code`. */
  const withTurbo = code => {
    const dir = repo();
    commit(dir, "packages/a/index.js", "2\n");
    const bin = path.join(dir, "node_modules", ".bin");
    fs.mkdirSync(bin, { recursive: true });
    const dryRun = JSON.stringify({
      tasks: [{ package: "a", task: "test", command: "node --test" }],
    });
    fs.writeFileSync(
      path.join(bin, "turbo"),
      `#!/bin/sh
case "$*" in *--dry-run*) echo '${dryRun}'; exit 0;; esac
echo "a:test: ✖ keeps the edge (1ms)"
[ ${code} = 0 ] || echo "Failed:    a#test"
exit ${code}
`,
      { mode: 0o755 },
    );
    return dir;
  };
  const check = dir =>
    spawnSync(process.execPath, [WORKTREE, "check"], {
      cwd: dir,
      encoding: "utf-8",
    });

  it("stamps the tree it tested when everything passed", () => {
    const dir = withTurbo(0);
    const result = check(dir);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /✓ checked in \d+ s: a/);
    const { passed, failed, running } = readCheck(dir);
    assert.equal(passed.tree, treeOf(dir));
    assert.deepEqual(passed.packages, ["a"]);
    assert.equal(failed, undefined);
    assert.equal(running, undefined);
    assert.equal(checkOf(dir).key, "tested");
  });
  it("stamps nothing as passed when a test fails, and records what failed", () => {
    const dir = withTurbo(1);
    const result = check(dir);
    assert.equal(result.status, 1);
    assert.match(
      result.stderr,
      /wt:check failed after \d+ s: a \(keeps the edge\)/,
    );
    const record = readCheck(dir);
    assert.equal(record.passed, undefined);
    assert.deepEqual(record.failed.failures, [
      { package: "a", task: "test", tests: ["keeps the edge"] },
    ]);
    assert.equal(checkOf(dir).key, "failed");
    const head = sh(dir, "rev-parse", "HEAD");
    assert.match(
      checkGate(refsOf(dir, head), dir)[0],
      /its last check failed: a \(keeps the edge\)/,
    );
  });
  it("a commit after the check is untested again", () => {
    const dir = withTurbo(0);
    assert.equal(check(dir).status, 0);
    commit(dir, "packages/a/index.js", "3\n");
    assert.deepEqual(checkOf(dir), { key: "untested", count: 1 });
  });
});
