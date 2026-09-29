import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { orderActions, placeIn } from "./order.mjs";

const item = (kind, rank, at = "2026-09-29T10:00:00Z") => ({
  action: { kind, at },
  rank,
});
const kinds = items =>
  items.map(({ action, rank }) => `${action.kind}@${rank}`);

describe("placeIn", () => {
  it("counts from 0 and puts an unlisted one last", () => {
    assert.equal(placeIn([5, 3], 3), 1);
    assert.equal(placeIn([5, 3], 7), Infinity);
    assert.equal(placeIn(undefined, 7), Infinity);
  });
});

describe("orderActions", () => {
  it("keeps the follow-ups out of ACTIONS and under UP NEXT", () => {
    const { actions, upNext } = orderActions([
      item("clean up", 0),
      item("waits", 2),
      item("merge", 1),
      item("pipeline", 1),
      item("end", Infinity, ""),
      item("clean up", 3),
    ]);
    assert.deepEqual(kinds(actions), ["waits@2", "merge@1"]);
    assert.deepEqual(kinds(upNext), [
      "clean up@0",
      "pipeline@1",
      "clean up@3",
      "end@Infinity",
    ]);
  });

  it("sorts ACTIONS by what blocks most, then merge order, then age", () => {
    const { actions } = orderActions([
      item("look", 0),
      item("browser-check", 0),
      item("merge", 2),
      item("merge", 1),
      item("plan", 0),
      item("decide", 3, "2026-09-29T11:00:00Z"),
      item("decide", 3, "2026-09-29T09:00:00Z"),
      item("decide", -1),
      item("waits", 4),
    ]);
    assert.deepEqual(kinds(actions), [
      "waits@4",
      "decide@-1",
      "decide@3",
      "decide@3",
      "plan@0",
      "merge@1",
      "merge@2",
      "browser-check@0",
      "look@0",
    ]);
    assert.equal(actions[2].action.at, "2026-09-29T09:00:00Z");
  });
});
