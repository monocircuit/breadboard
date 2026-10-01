import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { agentStateOf, sidebarLineOf } from "./agent-state.mjs";

const mr = (extra = {}) => ({
  iid: 192,
  state: "opened",
  isDraft: true,
  pipeline: { status: "success", sha: "a" },
  ...extra,
});
const tree = (extra = {}) => ({
  dir: "/w",
  claude: { id: "c1", answer: "Pushed, the pipeline runs." },
  actions: [],
  wait: null,
  change: null,
  pushed: "a",
  unpushed: 0,
  ...extra,
});
const at = (extra, facts) => agentStateOf(tree(extra), facts);
const textOf = (extra, facts) => at(extra, facts).text;

describe("agentStateOf (#288)", () => {
  it("says the agent is at work, whom it waits for else", () => {
    assert.deepEqual(at({}, { status: "working", mr: mr() }), {
      key: "working",
      text: "at work",
      tone: "work",
    });
  });

  it("asks you when blocked or stopped on a question", () => {
    assert.equal(textOf({}, { status: "blocked" }), "asks you");
    const asking = { claude: { id: "c1", answer: "Done.\n\nShall I merge?" } };
    assert.equal(at(asking, { status: "idle" }).key, "asks");
    assert.equal(at(asking, { status: "idle" }).tone, "you");
  });

  it("waits for the pipeline while it runs or GitLab has none for the push yet", () => {
    for (const status of ["running", "pending", "created"])
      assert.equal(
        textOf(
          {},
          { status: "idle", mr: mr({ pipeline: { status, sha: "a" } }) },
        ),
        "waits for the pipeline",
      );
    // Pushed b, the pipeline GitLab shows is still the one of a.
    const behind = at({ pushed: "b" }, { status: "idle", mr: mr() });
    assert.equal(behind.text, "waits for the pipeline");
    assert.equal(behind.tone, "gitlab");
  });

  it("says a failed pipeline", () => {
    const failed = at(
      {},
      { status: "idle", mr: mr({ pipeline: { status: "failed", sha: "a" } }) },
    );
    assert.deepEqual(failed, {
      key: "failed",
      text: "pipeline failed",
      tone: "you",
    });
  });

  it("waits for your merge once the pipeline of its MR passed, a draft too", () => {
    assert.equal(
      textOf({}, { status: "idle", mr: mr() }),
      "waits for you: merge",
    );
    assert.equal(
      textOf({}, { status: "done", mr: mr({ isDraft: false }) }),
      "waits for you: merge",
    );
  });

  it("names the open actions of the user's, a try by its number", () => {
    const actions = [
      { id: 3, kind: "decide" },
      { id: 4, kind: "try" },
      { id: 5, kind: "browser-check" },
      { id: 6, kind: "look", answer: "done" },
      { id: 7, kind: "merge" },
    ];
    assert.equal(
      textOf({ actions }, { status: "idle", mr: mr() }),
      "waits for you: decide, try #4, try #5, merge",
    );
  });

  it("waits for other MRs (wt:wait), then for an announced handover", () => {
    const wait = { mrs: [271, 272] };
    const waiting = at({ wait }, { status: "idle", merged: new Set([272]) });
    assert.equal(waiting.text, "waits for !271 · 1 merged");
    assert.equal(waiting.tone, "gitlab");
    const handover = { number: 226, open: [232] };
    assert.equal(
      textOf({}, { status: "idle", handover }),
      "→ next #226 after !232",
    );
    assert.equal(
      textOf({ wait }, { status: "idle", handover }),
      "waits for !271, !272",
    );
  });

  it("says a requested change, then commits not pushed", () => {
    const change = { text: "less padding", head: "a" };
    assert.equal(
      textOf({ change }, { status: "idle", mr: mr() }),
      "change requested",
    );
    assert.equal(
      textOf({ unpushed: 2 }, { status: "idle" }),
      "not pushed: 2 commits",
    );
    assert.equal(
      textOf({ unpushed: 1 }, { status: "idle" }),
      "not pushed: 1 commit",
    );
  });

  it("is free only with nothing open", () => {
    assert.deepEqual(at({}, { status: "idle" }), {
      key: "free",
      text: "free",
      tone: "free",
    });
    assert.equal(
      textOf({}, { status: "idle", mr: mr({ state: "merged" }) }),
      "free",
    );
    assert.equal(
      textOf(
        {},
        {
          status: "idle",
          mr: mr({ state: "merged" }),
          handover: { number: 9, open: [] },
        },
      ),
      "→ next #9 now",
    );
  });

  it("takes the first that holds, in this order", () => {
    const everything = {
      claude: { id: "c1", answer: "Shall I?" },
      actions: [{ id: 1, kind: "decide" }],
      wait: { mrs: [271] },
      change: { text: "x", head: "a" },
      unpushed: 1,
    };
    const facts = {
      status: "working",
      mr: mr({ pipeline: { status: "running", sha: "a" } }),
      handover: { number: 9, open: [271] },
    };
    const order = [];
    let extra = everything;
    let current = facts;
    // Take away what wins, one after the other.
    const steps = [
      () => (current = { ...current, status: "idle" }),
      () => (extra = { ...extra, claude: { id: "c1", answer: "Done." } }),
      () =>
        (current = {
          ...current,
          mr: mr({ pipeline: { status: "failed", sha: "a" } }),
        }),
      () => (current = { ...current, mr: null }),
      () => (extra = { ...extra, actions: [] }),
      () => (extra = { ...extra, wait: null }),
      () => (current = { ...current, handover: null }),
      () => (extra = { ...extra, change: null }),
      () => (extra = { ...extra, unpushed: 0 }),
    ];
    order.push(at(extra, current).key);
    for (const step of steps) {
      step();
      order.push(at(extra, current).key);
    }
    assert.deepEqual(order, [
      "working",
      "asks",
      "pipeline",
      "failed",
      "waits",
      "waitsForMr",
      "next",
      "change",
      "unpushed",
      "free",
    ]);
  });
});

describe("sidebarLineOf", () => {
  it("is the state with the MR behind it", () => {
    assert.equal(
      sidebarLineOf(tree(), { status: "idle", mr: mr() }),
      "waits for you: merge · !192",
    );
    assert.equal(sidebarLineOf(tree(), { status: "working" }), "at work");
  });
});
