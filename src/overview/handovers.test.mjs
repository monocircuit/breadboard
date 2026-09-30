import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { announcedOf, nextText, takesText } from "./handovers.mjs";
import { treeOf } from "./tree.mjs";
import { workerActionsOf } from "./workers.mjs";

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

  it("shows on the agent's node", () => {
    const { live } = treeOf(issues, () => ({}), { announced });
    const node = live.find(row => row.issue.number === 224);
    assert.match(plain(node.facts), /→ next #226 after !232$/);
  });

  it("is a row of UP NEXT of its own and holds the agent's worktree", () => {
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
    assert.ok(held.has("/w/224"));
    assert.equal(holds.get("/w/224"), "its agent takes #226 after !232");
  });
});
