import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  graphOf,
  holdUpOf,
  mergeOrderOf,
  needsOf,
  progressOf,
} from "./merge-order.mjs";

/** An open MR `!iid` of issue `number`; `mr` overrides its state. */
const entry = (number, { needs = [], files = [], ...mr } = {}) => ({
  number,
  needs,
  files: new Set(files),
  mr: {
    iid: number + 100,
    state: "opened",
    isDraft: false,
    pipeline: { status: "running" },
    hasConflicts: false,
    mergedAt: null,
    ...mr,
  },
});
const green = { pipeline: { status: "success" } };
const failed = { pipeline: { status: "failed" } };
const draft = { isDraft: true };
const numbersOf = order => order.open.map(item => item.number);

describe("needsOf", () => {
  it("finds the prerequisites an issue names", () => {
    assert.deepEqual(
      needsOf(
        "Builds on #180. Needs #181 and #183, after #186 / #187.\nRelated: #179",
      ),
      [180, 181, 183, 186, 187],
    );
  });

  it("finds none in a text without them", () => {
    assert.deepEqual(needsOf("Related: #179, #182"), []);
    assert.deepEqual(needsOf(null), []);
  });
});

describe("progressOf", () => {
  it("ranks ready and green before coming, blocked and draft", () => {
    assert.equal(progressOf(entry(1, green).mr), 0);
    assert.equal(progressOf(entry(1).mr), 1);
    assert.equal(progressOf(entry(1, failed).mr), 2);
    assert.equal(progressOf(entry(1, { ...green, hasConflicts: true }).mr), 2);
    assert.equal(progressOf(entry(1, { ...green, ...draft }).mr), 3);
  });
});

describe("mergeOrderOf", () => {
  it("keeps the plan when no fact speaks against it", () => {
    const order = mergeOrderOf([entry(1), entry(2), entry(3)], [3, 1, 2]);
    assert.deepEqual(numbersOf(order), [3, 1, 2]);
    assert.equal(order.open[0].reason, "as planned");
    assert.equal(order.isOffPlan, false);
  });

  it("orders by number without a plan, and says it leaves the plan", () => {
    const order = mergeOrderOf([entry(2), entry(1)]);
    assert.deepEqual(numbersOf(order), [1, 2]);
    assert.equal(order.open[0].reason, "not in the plan: by number");
    assert.equal(order.isOffPlan, true);
  });

  it("puts an issue after the one it needs", () => {
    const order = mergeOrderOf([entry(1, { needs: [2] }), entry(2)], [1, 2]);
    assert.deepEqual(numbersOf(order), [2, 1]);
    assert.equal(order.open[1].reason, "after #2 (needs it)");
    assert.equal(order.isOffPlan, true);
  });

  it("ignores a need that is merged or not in the orchestration", () => {
    const order = mergeOrderOf(
      [entry(1, { needs: [2, 9] }), entry(2, { state: "merged" })],
      [1],
    );
    assert.deepEqual(numbersOf(order), [1]);
  });

  it("falls back to the plan when the needs go in a circle", () => {
    const order = mergeOrderOf(
      [entry(1, { needs: [2] }), entry(2, { needs: [1] })],
      [1, 2],
    );
    assert.deepEqual(numbersOf(order), [1, 2]);
  });

  it("merges the one further along first when two share files", () => {
    const shared = ["packages/workflow/src/overview/render-orchestration.mjs"];
    const order = mergeOrderOf(
      [
        entry(1, { files: shared, ...draft }),
        entry(2, { files: shared, ...green }),
      ],
      [1, 2],
    );
    assert.deepEqual(numbersOf(order), [2, 1]);
    assert.equal(
      order.open[1].reason,
      "after !102: shares render-orchestration.mjs",
    );
  });

  it("keeps the plan between two sharing files that are equally far along", () => {
    const order = mergeOrderOf(
      [
        entry(1, { files: ["a.mjs"] }),
        entry(2, { files: ["a.mjs", "b.mjs", "c.mjs"] }),
      ],
      [2, 1],
    );
    assert.deepEqual(numbersOf(order), [2, 1]);
    assert.equal(order.open[1].reason, "after !102: shares a.mjs");
  });

  it("lets a need win over file overlap", () => {
    const order = mergeOrderOf(
      [
        entry(1, { files: ["a.mjs"], ...green, needs: [2] }),
        entry(2, { files: ["a.mjs"], ...draft }),
      ],
      [1, 2],
    );
    assert.deepEqual(numbersOf(order), [2, 1]);
  });

  it("moves a ready, green MR up past blocked ones that share no files", () => {
    const order = mergeOrderOf(
      [entry(1, failed), entry(2, draft), entry(3, green), entry(4)],
      [1, 2, 3, 4],
    );
    assert.deepEqual(numbersOf(order), [3, 1, 2, 4]);
    assert.equal(order.open[0].reason, "ready while !101, !102 are blocked");
    assert.equal(order.isOffPlan, true);
  });

  it("doesn't move a ready MR past one whose pipeline is still running", () => {
    const order = mergeOrderOf(
      [entry(1), entry(2, failed), entry(3, green)],
      [1, 2, 3],
    );
    assert.deepEqual(numbersOf(order), [1, 3, 2]);
    assert.equal(order.open[0].reason, "as planned");
  });

  it("doesn't move a ready MR past what it needs", () => {
    const order = mergeOrderOf(
      [entry(1, failed), entry(2, { ...green, needs: [1] })],
      [1, 2],
    );
    assert.deepEqual(numbersOf(order), [1, 2]);
  });

  it("lists the merged ones in the order they were merged, the closed after", () => {
    const order = mergeOrderOf(
      [
        entry(1, { state: "merged", mergedAt: "2026-09-29T10:00:00Z" }),
        entry(2, { state: "merged", mergedAt: "2026-09-28T10:00:00Z" }),
        entry(3, { state: "closed" }),
        entry(4),
      ],
      [1, 2, 4, 5],
    );
    assert.deepEqual(order.merged, [2, 1, 3]);
    assert.deepEqual(order.numbers, [4, 5, 2, 1, 3]);
    assert.deepEqual(order.planned, [2, 1, 4, 5]);
    assert.equal(order.isOffPlan, false);
  });
});

describe("holdUpOf", () => {
  const work = (text, isTicked = false) => ({
    heading: "Work",
    text,
    isTicked,
  });

  it("is empty for a ready, green MR with its work done", () => {
    assert.deepEqual(
      holdUpOf({ ...entry(1, green), boxes: [work("a", true)] }),
      [],
    );
  });

  it("names everything that blocks, in one list", () => {
    const blocked = {
      ...entry(1, { ...draft, ...failed, hasConflicts: true }),
      boxes: [
        work("UP NEXT row with a click that writes"),
        work("b"),
        { heading: "Acceptance criteria", text: "c", isTicked: false },
      ],
      behind: 13,
    };
    assert.deepEqual(
      holdUpOf(blocked, [{ iid: 215, why: "shares view.mjs" }]),
      [
        "draft",
        "2 work boxes open: UP NEXT row with … +1",
        "pipeline failed",
        "conflicts with develop (13 behind)",
        "after !215 (shares view.mjs)",
      ],
    );
  });

  it("is what mergeOrderOf puts on each open MR, with what it comes after", () => {
    const order = mergeOrderOf(
      [entry(1, { needs: [2], ...green }), entry(2, green)],
      [2, 1],
    );
    assert.deepEqual(
      order.open.map(item => item.holdUp),
      [[], ["after !102 (needs #2)"]],
    );
  });
});

describe("graphOf", () => {
  const item = (number, ...after) => ({
    number,
    after: after.map(pred => ({ number: pred })),
  });
  const drawn = open =>
    graphOf(open).map(row =>
      row.number ? `${row.graph} ${row.number} / ${row.below}` : row.graph,
    );

  it("puts independent MRs side by side and joins them into the one after both", () => {
    assert.deepEqual(drawn([item(1), item(2), item(3, 1, 2), item(4, 3)]), [
      "● 1 / │",
      "│ ● 2 / │ │",
      "├─┘",
      "● 3 / │",
      "● 4 / ",
    ]);
  });

  it("forks a lane for each MR after the same one", () => {
    assert.deepEqual(drawn([item(1), item(2, 1), item(3, 1), item(4, 2, 3)]), [
      "● 1 / │",
      "├─┐",
      "● │ 2 / │ │",
      "│ ● 3 / │ │",
      "├─┘",
      "● 4 / ",
    ]);
  });

  it("ends the lane of an MR nothing comes after", () => {
    assert.deepEqual(drawn([item(1), item(2), item(3, 1)]), [
      "● 1 / │",
      "│ ● 2 / │",
      "● 3 / ",
    ]);
  });
});
