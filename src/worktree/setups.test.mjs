import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, describe, it } from "node:test";

import {
  claimSetup,
  setupAt,
  setupText,
  setupsDir,
  setupsOf,
} from "./setups.mjs";

describe("claimSetup and setupsOf (#272)", () => {
  const cwd = process.cwd();
  let repo;
  before(() => {
    // A checkout of its own: the claims go next to the records of its main checkout.
    repo = fs.mkdtempSync(path.join(os.tmpdir(), "setups-test-"));
    spawnSync("git", ["init", "--quiet"], { cwd: repo });
    process.chdir(repo);
  });
  after(() => {
    process.chdir(cwd);
    fs.rmSync(repo, { recursive: true, force: true });
  });

  it("claims an issue once: a second start of it gets the running one", () => {
    const first = claimSetup("o", 267, {
      dir: "/x/platform-267-a",
      to: "268-b",
      from: "/x/platform-268-b",
    });
    assert.ok(first.release);
    const second = claimSetup("o", 267, { dir: "/x/platform-267-a" });
    assert.equal(second.holder.to, "268-b");
    assert.equal(second.holder.pid, process.pid);
    assert.match(setupText(second.holder), /^a handover of #267 to 268-b /);
    assert.deepEqual(
      setupsOf("o").map(({ number, from }) => [number, from]),
      [[267, "/x/platform-268-b"]],
    );
    first.release();
    assert.deepEqual(setupsOf("o"), []);
    const again = claimSetup("o", 267, { dir: "/x/platform-267-a" });
    assert.ok(again.release);
    again.release();
  });

  it("drops the claim of a process that ended without its release", () => {
    fs.mkdirSync(setupsDir("o"), { recursive: true });
    const file = path.join(setupsDir("o"), "269.json");
    fs.writeFileSync(
      file,
      JSON.stringify({ number: 269, dir: "/x/269", pid: 1, at: "" }),
    );
    assert.deepEqual(setupsOf("o", { alive: () => false }), []);
    assert.ok(!fs.existsSync(file));
    fs.writeFileSync(
      file,
      JSON.stringify({ number: 269, dir: "/x/269", pid: 999_999_999 }),
    );
    const claim = claimSetup("o", 269, { dir: "/x/269" });
    assert.ok(claim.release, "a dead pid's claim is taken over");
    claim.release();
  });

  it("finds the setup of a worktree it makes or moves an agent out of", () => {
    const setups = [
      { number: 1, dir: "/x/1", from: "/x/0", to: "0-a", pid: 1, at: "" },
    ];
    assert.equal(setupAt(setups, "/x/1").number, 1);
    assert.equal(setupAt(setups, "/x/0").number, 1);
    assert.equal(setupAt(setups, "/x/2"), undefined);
  });
});
