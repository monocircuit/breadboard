import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, describe, it } from "node:test";

import { waitingFor } from "./waiting.mjs";

// A repo with a stack: develop ← a ← b ← c (each merges the one below), an
// unrelated branch x, and origin/<name> refs for them as a fetch leaves them.
let dir;
const git = (...args) => {
  const result = spawnSync("git", args, { cwd: dir, encoding: "utf-8" });
  if (result.status !== 0)
    throw new Error(`git ${args.join(" ")}: ${result.stderr}`);
  return result.stdout.trim();
};
const commit = name => {
  fs.writeFileSync(path.join(dir, name), name);
  git("add", name);
  git("commit", "-q", "-m", name);
};
const publish = branch =>
  git("update-ref", `refs/remotes/origin/${branch}`, branch);

before(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "workflow-waiting-"));
  git("init", "-q", "-b", "develop");
  git("config", "user.email", "test@example.org");
  git("config", "user.name", "test");
  commit("base");
  publish("develop");
  git("switch", "-q", "-c", "a");
  commit("a1");
  commit("a2");
  publish("a");
  git("switch", "-q", "-c", "b");
  commit("b1");
  publish("b");
  git("switch", "-q", "-c", "c");
  commit("c1");
  publish("c");
  git("switch", "-q", "-c", "x", "develop");
  commit("x1");
  publish("x");
  git("switch", "-q", "c");
});
after(() => fs.rmSync(dir, { recursive: true, force: true }));

const mrs = ["x", "c", "b", "a", "gone"].map((branch, i) => ({
  iid: 10 + i,
  branch,
}));

describe("waitingFor", () => {
  it("lists the branches the work builds on, base first, not its own nor an unrelated one", () => {
    assert.deepEqual(
      waitingFor(dir, "c", mrs).map(mr => [mr.branch, mr.count]),
      [
        ["a", 2],
        ["b", 3],
      ],
    );
  });

  it("still counts a base that got more commits since", () => {
    git("switch", "-q", "a");
    commit("a3");
    publish("a");
    git("switch", "-q", "c");
    assert.deepEqual(
      waitingFor(dir, "c", mrs).map(mr => mr.branch),
      ["a", "b"],
    );
  });

  it("drops a branch once develop has it", () => {
    git("update-ref", "refs/remotes/origin/develop", "refs/remotes/origin/b");
    assert.deepEqual(waitingFor(dir, "c", mrs), []);
    git("update-ref", "refs/remotes/origin/develop", "develop");
  });

  it("has nothing for a branch on develop alone", () => {
    git("switch", "-q", "x");
    assert.deepEqual(waitingFor(dir, "x", mrs), []);
    git("switch", "-q", "c");
  });
});
