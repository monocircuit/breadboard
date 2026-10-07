import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  historyOf,
  countsOf,
  reuseOf,
  reuseText,
  withFailure,
  withStart,
} from "./history.mjs";

const start = (number, how, context) => ({ number, how, at: "t", context });

describe("the history of an agent", () => {
  it("starts with its first issue, fresh", () => {
    const entry = withStart(undefined, start(235, "fresh", 0));
    assert.deepEqual(entry.history, [start(235, "fresh", 0)]);
    assert.deepEqual(entry.issues, [235]);
  });

  it("adds each handover in order, once per issue", () => {
    const fresh = withStart({ name: "235-x" }, start(235, "fresh", 0));
    const handed = withStart(fresh, start(240, "handover", 0.31));
    const again = withStart(handed, start(252, "announced", 0.38));
    const retried = withStart(again, start(252, "announced", 0.4));
    assert.equal(retried.name, "235-x");
    assert.deepEqual(retried.issues, [235, 240, 252]);
    assert.deepEqual(
      retried.history.map(({ how, context }) => [how, context]),
      [
        ["fresh", 0],
        ["handover", 0.31],
        ["announced", 0.4],
      ],
    );
  });

  it("reads an entry from before it from its issues", () => {
    assert.deepEqual(historyOf({ issues: [235, 240] }), [
      { number: 235, how: "fresh", at: null, context: null },
      { number: 240, how: "handover", at: null, context: null },
    ]);
    assert.deepEqual(historyOf(undefined), []);
  });

  it("keeps a failed handover, the same failure again only counted", () => {
    const once = withFailure(
      { history: [] },
      { number: 252, reason: "it is working", at: "a" },
    );
    const twice = withFailure(once, {
      number: 252,
      reason: "it is working",
      at: "b",
    });
    const other = withFailure(twice, {
      number: 252,
      reason: "its worktree failed: no worktree",
      at: "c",
    });
    assert.deepEqual(twice.failed, [
      { number: 252, reason: "it is working", at: "b", times: 2 },
    ]);
    assert.equal(other.failed.length, 2);
    assert.deepEqual(other.history, []);
  });
});

describe("countsOf", () => {
  const known = {
    history: [
      start(235, "fresh", 0),
      start(240, "handover", 0.31),
      start(252, "announced", 0.38),
    ],
  };
  it("counts its issues, this one included, and their merged MRs", () => {
    const merged = new Set([235, 240]);
    assert.deepEqual(
      countsOf(known, 252, n => merged.has(n)),
      {
        issues: 3,
        merged: 2,
      },
    );
  });
  it("goes up with a handover", () => {
    const after = withStart(known, start(260, "handover", 0.4));
    assert.deepEqual(
      countsOf(after, 260, () => true),
      {
        issues: 4,
        merged: 4,
      },
    );
  });
  it("counts the one issue of an agent the record doesn't know", () => {
    assert.deepEqual(
      countsOf(undefined, 7, () => false),
      {
        issues: 1,
        merged: 0,
      },
    );
  });
});

describe("reuseOf", () => {
  const issue = (number, fields = {}) => ({ number, why: null, ...fields });
  it("counts agents, issues, the reused ones and the failed handovers", () => {
    const issues = [
      issue(1),
      issue(2),
      issue(3),
      // From before the history: a fresh agent of its own.
      issue(4),
      issue(5, { queued: true }),
      issue(6, { why: "failed: no worktree" }),
    ];
    const known = {
      a: {
        history: [
          start(1, "fresh", 0),
          start(2, "handover", 0.2),
          start(3, "announced", 0.4),
        ],
        failed: [{ number: 3, reason: "it is working", at: "t", times: 3 }],
      },
      b: { history: [], failed: [{ number: 5, reason: "x", at: "t" }] },
    };
    const reuse = reuseOf(issues, known);
    assert.equal(reuse.agents, 2);
    assert.equal(reuse.issues, 4);
    assert.equal(reuse.reused, 2);
    assert.ok(Math.abs(reuse.context - 0.3) < 1e-9);
    assert.equal(reuse.failed, 2);
    assert.equal(
      reuseText(reuse),
      "agents 2 · issues 4 · reused 2, started at 30% · handovers failed 2",
    );
  });
  it("counts an old record by its workers' issues", () => {
    const reuse = reuseOf([issue(1), issue(2), issue(3)], {
      a: { name: "x", issues: [1, 2] },
    });
    assert.deepEqual(reuse, {
      agents: 2,
      issues: 3,
      reused: 1,
      context: null,
      failed: 0,
    });
    assert.equal(
      reuseText(reuse),
      "agents 2 · issues 3 · reused 1 · handovers failed 0",
    );
  });
});
