import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ancestorsOf, paneOfProcess } from "./herdr.mjs";

describe("ancestorsOf", () => {
  const stats = {
    40: "40 (node) S 30 40 30 0",
    30: "30 (pnpm (dev) x) S 20 30 20 0",
    20: "20 (claude) S 1 20 20 0",
  };
  it("walks up the parents, the name's spaces and parentheses aside", () => {
    assert.deepEqual(
      ancestorsOf(40, id => stats[id]),
      [40, 30, 20],
    );
  });

  it("stops where a parent can't be read", () => {
    assert.deepEqual(
      ancestorsOf(40, id => {
        if (id === 30) throw new Error("gone");
        return stats[id];
      }),
      [40, 30],
    );
  });
});

describe("paneOfProcess", () => {
  const shells = { "w1:p1": 10, "w2:p4": 20, "w3:p2": 30 };
  const panes = () => [
    { pane_id: "w1:p1" },
    { pane_id: "w2:p4" },
    { pane_id: "w3:p2" },
  ];
  const shellOf = id => shells[id] ?? null;

  it("takes herdr's answer when its shell is an ancestor", () => {
    const current = { pane_id: "w2:p4" };
    assert.equal(
      paneOfProcess({
        ancestors: [99, 50, 20],
        current,
        panes: () => assert.fail("no scan needed"),
        shellOf,
      }),
      current,
    );
  });

  it("finds the pane by the process when HERDR_PANE_ID is stale (the focused pane came back)", () => {
    assert.deepEqual(
      paneOfProcess({
        ancestors: [99, 50, 30],
        current: { pane_id: "w1:p1" },
        panes,
        shellOf,
      }),
      { pane_id: "w3:p2" },
    );
  });

  it("is null when no pane's shell is an ancestor (a process outside herdr's panes)", () => {
    assert.equal(
      paneOfProcess({
        ancestors: [99, 50],
        current: { pane_id: "w1:p1" },
        panes,
        shellOf,
      }),
      null,
    );
  });
});
