import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  historyOf,
  historyText,
  ordinal,
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

describe("historyText", () => {
  const known = {
    a: {
      history: [
        start(235, "fresh", 0),
        start(240, "handover", 0.31),
        start(252, "announced", 0.38),
      ],
    },
    b: { history: [start(236, "fresh", 0)] },
  };
  it("says which issue of its agent it is, the chain for a reused one", () => {
    assert.equal(
      historyText(known, 252, { id: "a" }),
      "agent: 3rd issue (#235 → #240 at 31% → #252 at 38%)",
    );
    assert.match(historyText(known, 235, null), /^agent: 1st issue \(/);
    assert.equal(historyText(known, 236, { id: "b" }), "agent: 1st issue");
  });
  it("counts an agent the record doesn't know as fresh, nothing without one", () => {
    assert.equal(historyText({}, 7, { id: "c" }), "agent: 1st issue");
    assert.equal(historyText({}, 7, null), "");
  });
  it("leaves out a context it doesn't know", () => {
    assert.equal(
      historyText({ a: { issues: [1, 2] } }, 2, null),
      "agent: 2nd issue (#1 → #2)",
    );
  });
});

describe("ordinal", () => {
  it("names the place", () => {
    assert.deepEqual([1, 2, 3, 4, 11, 12, 13, 21, 22, 101].map(ordinal), [
      "1st",
      "2nd",
      "3rd",
      "4th",
      "11th",
      "12th",
      "13th",
      "21st",
      "22nd",
      "101st",
    ]);
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
