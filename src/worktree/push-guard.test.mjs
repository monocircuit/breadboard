import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  SHIM,
  checkPush,
  installPushGuard,
  pushedBranches,
  refusalOf,
} from "./push-guard.mjs";

const SHA = "38d9f64aa1b2c3d4e5f60718293a4b5c6d7e8f90";
const ZERO = "0".repeat(40);
const mr = (iid, state) => ({
  iid,
  state,
  web_url: `https://gitlab.example/mr/${iid}`,
});

describe("pushedBranches", () => {
  it("takes the branches a push updates, not deletions or tags", () => {
    const input = [
      `refs/heads/feature/1-a ${SHA} refs/heads/feature/1-a ${ZERO}`,
      `(delete) ${ZERO} refs/heads/feature/2-b ${SHA}`,
      `refs/tags/v1 ${SHA} refs/tags/v1 ${ZERO}`,
      "",
    ].join("\n");
    assert.deepEqual(pushedBranches(input), ["feature/1-a"]);
  });
});

describe("refusalOf", () => {
  it("lets a branch without an MR through", () => {
    assert.equal(refusalOf("feature/1-a", []), null);
  });
  it("lets a branch with an open MR through, also next to a merged one", () => {
    assert.equal(refusalOf("feature/1-a", [mr(9, "opened")]), null);
    assert.equal(
      refusalOf("feature/1-a", [mr(9, "merged"), mr(12, "opened")]),
      null,
    );
  });
  it("refuses a merged MR's branch, naming the MR and what to do instead", () => {
    const refusal = refusalOf("feature/174-x", [mr(208, "merged")]);
    assert.match(refusal, /!208 of feature\/174-x is merged/);
    assert.match(refusal, /pnpm wt:new/);
    assert.match(refusal, /gitlab\.example\/mr\/208/);
  });
  it("refuses a closed MR's branch", () => {
    assert.match(
      refusalOf("fix/3-c", [mr(7, "closed"), mr(5, "merged")]),
      /!7 of fix\/3-c is closed/,
    );
  });
});

describe("checkPush", () => {
  const input = `refs/heads/b ${SHA} refs/heads/b ${ZERO}\n`;
  it("asks GitLab once per branch and returns the refusals", async () => {
    const asked = [];
    const refusals = await checkPush(`${input}${input}`, async branch => {
      asked.push(branch);
      return [mr(1, "merged")];
    });
    assert.deepEqual(asked, ["b"]);
    assert.equal(refusals.length, 1);
  });
  it("pushes unchecked when GitLab doesn't answer", async () => {
    const error = console.error;
    console.error = () => {};
    try {
      const refusals = await checkPush(input, async () => {
        throw new Error("offline");
      });
      assert.deepEqual(refusals, []);
    } finally {
      console.error = error;
    }
  });
});

const tempRepo = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "push-guard-"));
  spawnSync("git", ["init", "--quiet", dir]);
  return dir;
};

describe("installPushGuard", () => {
  it("installs the shim once and updates an older one of ours", () => {
    const dir = tempRepo();
    const file = path.join(dir, ".git", "hooks", "pre-push");
    assert.equal(installPushGuard(dir), "installed");
    assert.equal(fs.readFileSync(file, "utf-8"), SHIM);
    assert.ok(fs.statSync(file).mode & 0o100);
    assert.equal(installPushGuard(dir), "installed");
    fs.writeFileSync(file, "#!/bin/sh\n# monocircuit push guard (#184), old\n");
    assert.equal(installPushGuard(dir), "updated");
    fs.rmSync(dir, { recursive: true });
  });
  it("leaves someone else's pre-push hook alone", () => {
    const dir = tempRepo();
    const file = path.join(dir, ".git", "hooks", "pre-push");
    fs.writeFileSync(file, "#!/bin/sh\nexit 0\n");
    assert.match(installPushGuard(dir), /someone else's hook/);
    assert.equal(fs.readFileSync(file, "utf-8"), "#!/bin/sh\nexit 0\n");
    fs.rmSync(dir, { recursive: true });
  });
});

describe("the pre-push hook", () => {
  it("lets a push through when nothing is pushed to a branch", () => {
    const hook = path.join(
      import.meta.dirname,
      "..",
      "git-hooks",
      "pre-push.mjs",
    );
    const result = spawnSync("node", [hook, "origin", "url"], {
      input: `(delete) ${ZERO} refs/heads/b ${SHA}\n`,
      encoding: "utf-8",
    });
    assert.equal(result.status, 0, result.stderr);
  });
});
