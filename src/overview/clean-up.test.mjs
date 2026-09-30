import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { cleanUpOf } from "./clean-up.mjs";

const merged = { state: "merged" };
const issue = (tree = {}) => ({
  number: 1,
  tree: { dir: "/w", changed: 0, ownCommits: 0, ...tree },
});

describe("cleanUpOf", () => {
  it("is due for a merged worktree with nothing left and an idle agent", () => {
    assert.deepEqual(cleanUpOf(issue(), { mr: merged, status: "idle" }), {
      key: "due",
      why: null,
    });
    // An agent herdr doesn't run is no one's work either.
    assert.equal(cleanUpOf(issue(), { mr: merged }).key, "due");
  });

  it("does nothing before the merge or without a worktree", () => {
    assert.equal(cleanUpOf(issue(), { mr: { state: "opened" } }), null);
    assert.equal(cleanUpOf({ number: 1, tree: null }, { mr: merged }), null);
  });

  it("keeps what wt:remove would refuse", () => {
    assert.deepEqual(
      cleanUpOf(issue({ changed: 2, ownCommits: 3 }), { mr: merged }),
      { key: "kept", why: "2 changed, 3 commits not in develop" },
    );
  });

  it("waits for its agent and for a handover", () => {
    assert.equal(
      cleanUpOf(issue(), { mr: merged, status: "working" }).why,
      "its agent is working",
    );
    assert.equal(
      cleanUpOf(issue(), { mr: merged, status: "blocked" }).why,
      "its agent asks you",
    );
    assert.deepEqual(
      cleanUpOf(issue(), {
        mr: merged,
        status: "idle",
        hold: "its agent takes #226 after !232",
      }),
      { key: "waits", why: "its agent takes #226 after !232" },
    );
  });
});
