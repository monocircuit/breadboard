import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { announcedOf, nextText, takesText } from "./handovers.mjs";
import { treeOf } from "./tree.mjs";
import { RETRY_MS, isHandOverNow, workerActionsOf } from "./workers.mjs";

const plain = text => text.replace(ESCAPES, "");
const agentTree = {
  dir: "/w/224",
  name: "224-x",
  changed: 0,
  ahead: 0,
  ownCommits: 0,
  actions: [],
  claude: { id: "c224" },
};
const issues = [
  {
    number: 224,
    feature: "224-x",
    branch: "feature/224-x",
    why: null,
    tree: agentTree,
  },
  {
    number: 226,
    feature: "226-y",
    branch: "feature/226-y",
    why: null,
    tree: null,
    queued: true,
  },
];
const handover = { number: 226, to: "224-x", after: [232], at: "" };

describe("announcedOf", () => {
  it("finds the agent's worktree and what it still waits for", () => {
    const [waiting] = announcedOf([handover], issues, {
      agents: new Map([["c224", { status: "idle" }]]),
      awaited: new Map([[232, { state: "opened" }]]),
    });
    assert.equal(waiting.tree, agentTree);
    assert.deepEqual(waiting.open, [232]);
    assert.equal(waiting.isDue, false);
    assert.equal(nextText(waiting), "→ next #226 after !232");
    assert.equal(takesText(waiting), "its agent takes #226 after !232");
  });

  it("is due once the MRs are merged and the agent is idle", () => {
    const merged = new Map([[232, { state: "merged" }]]);
    const due = status =>
      announcedOf([handover], issues, {
        agents: new Map([["c224", { status }]]),
        awaited: merged,
      })[0].isDue;
    assert.equal(due("idle"), true);
    assert.equal(due("done"), true);
    assert.equal(due("working"), false);
    // Before GitLab answered nothing is due.
    assert.equal(
      announcedOf([handover], issues, {
        agents: new Map([["c224", { status: "idle" }]]),
      })[0].isDue,
      false,
    );
  });

  it("says why it can't go once the MRs are merged (#244)", () => {
    const merged = new Map([[232, { state: "merged" }]]);
    const why = (status, handovers = [handover]) =>
      announcedOf(handovers, issues, {
        agents: new Map(status ? [["c224", { status }]] : []),
        awaited: merged,
      })[0].why;
    assert.equal(why("idle"), null);
    assert.equal(why("blocked"), "it waits for an answer");
    assert.equal(why("working"), "it is working");
    assert.match(why(null), /^no claude runs it/);
    assert.equal(
      why("idle", [{ ...handover, to: "gone" }]),
      "no worktree of the orchestration has an agent gone",
    );
    // Still waiting for its MRs: those are the reason.
    assert.equal(
      announcedOf([handover], issues, {
        agents: new Map([["c224", { status: "working" }]]),
        awaited: new Map([[232, { state: "opened" }]]),
      })[0].why,
      null,
    );
  });

  it("knows the agent by the name the record gives it", () => {
    const [found] = announcedOf([{ ...handover, to: "first-name" }], issues, {
      known: { c224: { name: "first-name" } },
    });
    assert.equal(found.tree, agentTree);
  });

  it("knows it by its name before a handover renamed it (#251)", () => {
    const [found] = announcedOf([{ ...handover, to: "old-name" }], issues, {
      known: { c224: { name: "224-x", was: "old-name" } },
    });
    assert.equal(found.tree, agentTree);
  });
});

describe("an announced handover in the overview", () => {
  const announced = announcedOf([handover], issues, {
    awaited: new Map([[232, { state: "opened" }]]),
  });

  it("shows on the agent's node: its state while nothing comes first, else its facts", () => {
    const { live } = treeOf(issues, () => ({}), { announced });
    const node = live.find(row => row.issue.number === 224);
    assert.equal(plain(node.state.text), "→ next #226 after !232");
    assert.doesNotMatch(plain(node.facts), /→ next/);
    const working = treeOf(issues, () => ({}), {
      announced,
      agents: new Map([["c224", { status: "working" }]]),
    }).live.find(row => row.issue.number === 224);
    assert.equal(plain(working.state.text), "at work");
    assert.match(plain(working.facts), /→ next #226 after !232$/);
  });

  it("is a row of UP NEXT of its own and holds no worktree: its agent goes free (#289)", () => {
    const { actions, held, holds } = workerActionsOf(
      "orch",
      issues,
      () => ({}),
      new Map(),
      {},
      { announced },
    );
    assert.equal(actions.length, 1);
    assert.equal(plain(actions[0].text), "#226 → 224-x after !232 (announced)");
    assert.equal(held.size, 0);
    assert.equal(holds.size, 0);
  });
});

describe("a due announced handover (#244)", () => {
  const due = announcedOf([handover], issues, {
    agents: new Map([["c224", { status: "idle" }]]),
    awaited: new Map([[232, { state: "merged" }]]),
  });
  const refresh = (options = {}) => {
    const runs = [];
    const { actions } = workerActionsOf(
      "orch",
      issues,
      () => ({}),
      new Map(),
      {},
      {
        announced: due,
        run: (...args) => runs.push(args),
        ...options,
      },
    );
    return { runs, actions };
  };

  it("is handed over on the next refresh of the live overview", () => {
    assert.deepEqual(refresh({ shouldHandOut: true }).runs, [
      ["orch", 226, ["--to", "224-x"], { dir: "/w/224", to: "224-x" }],
    ]);
    // A --once print or a second overview hands nothing over.
    assert.deepEqual(refresh().runs, []);
    assert.equal(
      plain(refresh().actions[0].text),
      "#226 → 224-x now (announced)",
    );
  });

  it("is tried again a while after a try failed, never while one runs", () => {
    const [handover] = due;
    const now = 1_000_000;
    assert.equal(isHandOverNow(handover, undefined, now), true);
    assert.equal(
      isHandOverNow(handover, { isRunning: true, ended: 0 }, now),
      false,
    );
    const failed = { isRunning: false, error: "x", ended: now - 1000 };
    assert.equal(isHandOverNow(handover, failed, now), false);
    assert.equal(
      isHandOverNow(handover, { ...failed, ended: now - RETRY_MS }, now),
      true,
    );
    assert.equal(
      isHandOverNow({ ...handover, isDue: false }, undefined, now),
      false,
    );
  });

  it("says why it waits in UP NEXT", () => {
    const [busy] = announcedOf([handover], issues, {
      agents: new Map([["c224", { status: "blocked" }]]),
      awaited: new Map([[232, { state: "merged" }]]),
    });
    const { actions } = workerActionsOf(
      "orch",
      issues,
      () => ({}),
      new Map(),
      {},
      { announced: [busy], shouldHandOut: true, run: () => assert.fail() },
    );
    assert.equal(
      plain(actions[0].text),
      "#226 → 224-x waits: it waits for an answer (announced)",
    );
  });
});
