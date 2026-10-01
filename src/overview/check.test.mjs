import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { visible } from "../lib/tty.mjs";
import { checkFact, checkRow } from "./check.mjs";

const SHA = "38d9f64aa1b2c3d4e5f60718293a4b5c6d7e8f90";
const plain = text => text.replace(/\u001b\[[0-9;]*m/g, "");

describe("checkFact", () => {
  it("shows each state of a worktree's HEAD", () => {
    assert.equal(
      plain(checkFact({ key: "tested", sha: SHA, packages: [] })),
      "✓ tested 38d9f64a",
    );
    assert.equal(plain(checkFact({ key: "checking" })), "checking…");
    assert.equal(
      plain(checkFact({ key: "untested", count: 2 })),
      "untested: 2 commits",
    );
    assert.equal(
      plain(checkFact({ key: "untested", count: 1 })),
      "untested: 1 commit",
    );
    assert.equal(plain(checkFact({ key: "untested", count: 0 })), "untested");
  });
  it("names the failing package and its tests in one line", () => {
    const failures = [
      {
        package: "@monocircuit/vita",
        task: "test",
        tests: ["frame.test.ts > a", "frame.test.ts > b", "c"],
      },
    ];
    assert.equal(
      plain(checkFact({ key: "failed", failures })),
      "✗ tests failed: @monocircuit/vita (frame.test.ts > a, frame.test.ts > b, +1 more)",
    );
    assert.equal(
      plain(
        checkFact({
          key: "failed",
          failures: [{ package: "a", task: "lint", tests: [] }],
        }),
      ),
      "✗ check failed: a lint",
    );
  });
  it("says nothing for a branch without work of its own", () => {
    assert.equal(checkFact({ key: "none" }), "");
    assert.equal(checkFact(undefined), "");
  });
});

describe("checkRow", () => {
  it("adds what was tested, or what to run", () => {
    assert.match(
      plain(checkRow({ key: "tested", sha: SHA, packages: ["a", "b"] })),
      /tested 38d9f64a · a, b$/,
    );
    assert.match(
      plain(checkRow({ key: "untested", count: 1 })),
      /pnpm wt:check before the push$/,
    );
    assert.equal(visible(checkRow({ key: "none" })), 0);
  });
});
