import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { leftIn, leftText } from "../worktree/conversations.mjs";
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

  it("waits while a try on the test instance is open", () => {
    const tries = [{ id: 1, kind: "try", on: "test", url: "/atlas" }];
    assert.deepEqual(cleanUpOf(issue({ actions: tries }), { mr: merged }), {
      key: "waits",
      why: "its try on the test instance is open",
    });
    // Answered, or on the worktree's app: nothing to keep it for.
    assert.equal(
      cleanUpOf(issue({ actions: [{ ...tries[0], answer: "looks good" }] }), {
        mr: merged,
      }).key,
      "due",
    );
    assert.equal(
      cleanUpOf(issue({ actions: [{ id: 1, kind: "try", url: "/a" }] }), {
        mr: merged,
      }).key,
      "due",
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

describe("the clean-up after a handover (#252)", () => {
  // The merge fired the handover: the conversation is bound to /w/246 now, the claude process still stands in /w/242.
  const agent = { pid: 7, id: "c242", name: "242-x", cwd: "/w/242" };
  const old = left =>
    issue({ dir: "/w/242", left: leftIn("/w/242", left, null) });

  it("waits while the handed-over agent still stands in its old worktree", () => {
    assert.deepEqual(cleanUpOf(old([agent]), { mr: merged }), {
      key: "waits",
      why: "242-x has not entered its next worktree yet",
    });
    // Also from a folder deeper in it.
    assert.equal(
      cleanUpOf(old([{ ...agent, cwd: "/w/242/packages/x" }]), { mr: merged })
        .key,
      "waits",
    );
  });

  it("goes once the agent entered its next worktree", () => {
    assert.equal(
      cleanUpOf(old([{ ...agent, cwd: "/w/246" }]), { mr: merged }).key,
      "due",
    );
  });

  it("does not wait for the worktree's own agent, it closes with the space", () => {
    assert.deepEqual(leftIn("/w/242", [agent], "c242"), []);
    // `/w/2420` is not inside `/w/242`.
    assert.deepEqual(
      leftIn("/w/242", [{ ...agent, cwd: "/w/2420" }], null),
      [],
    );
  });

  it("names a claude without a name by its pid", () => {
    assert.equal(
      leftText([agent, { ...agent, name: null, pid: 9 }]),
      "242-x, claude 9",
    );
  });
});
