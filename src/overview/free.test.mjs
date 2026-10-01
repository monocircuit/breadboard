// Free agents (#289): merged → free → reused for the next issue; over the
// context limit → it goes with its worktree.
import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  freeAgentsOf,
  freeLine,
  freeTab,
  freedEntry,
  freeTabsOutside,
} from "../worktree/free.mjs";
import { removeMerged } from "./auto-actions.mjs";
import { freeRowOf, pushFreeAgents } from "./free-agents.mjs";
import { announcedOf } from "./handovers.mjs";
import { freeOnCleanUp, freeWorkersOf, workerActionsOf } from "./workers.mjs";

const at = "2026-10-01T10:00:00.000Z";
const claude = { id: "c1", used: 40_000, window: 200_000 };
const merged = {
  number: 3,
  feature: "3-done",
  tree: { dir: "/nonexistent/platform-3-done", name: "3-done", claude },
  mr: { iid: 30, state: "merged" },
};
const of = issue => ({ mr: issue.mr, issue: { paths: issue.paths ?? [] } });
const idle = new Map([["c1", { pane: "p1", tab: "t1", status: "idle" }]]);

describe("an agent merged", () => {
  it("goes free instead of closing with its worktree", () => {
    const calls = [];
    const isFreed = freeOnCleanUp(
      "o",
      merged,
      merged.mr,
      { agents: idle, workers: { c1: { name: "agent-a" } } },
      (name, what) => {
        calls.push([name, what]);
        return { isFreed: true };
      },
    );
    assert.equal(isFreed, true);
    assert.equal(calls.length, 1);
    const [[name, what]] = calls;
    assert.equal(name, "o");
    assert.deepEqual(
      { ...what, files: [] },
      {
        id: "c1",
        agent: "agent-a",
        pane: "p1",
        dir: merged.tree.dir,
        iid: 30,
        files: [],
      },
    );
  });

  it("goes with its worktree above the context limit", () => {
    const full = {
      ...merged,
      tree: { ...merged.tree, claude: { ...claude, used: 120_000 } },
    };
    let isAsked = false;
    const isFreed = freeOnCleanUp("o", full, full.mr, { agents: idle }, () => {
      isAsked = true;
      return { isFreed: true };
    });
    assert.equal(isFreed, false);
    assert.equal(isAsked, false);
  });

  it("is freed before the removal, which waits until it left", () => {
    const released = [];
    const issue = { ...merged, tree: { ...merged.tree, changed: 0 } };
    removeMerged([issue], of, idle, new Map(), candidate => {
      released.push(candidate.number);
      return true;
    });
    assert.deepEqual(released, [3]);
  });

  it("is told it is free, and where to wait", () => {
    assert.equal(
      freeLine(30, "/repo/.claude/orchestrations/o"),
      '[overview] !30 merged, your worktree is removed: you are free. EnterWorktree(path: "/repo/.claude/orchestrations/o"), then stop and wait for the next issue, it comes as one line.',
    );
    assert.equal(freeTab("agent-a"), "free agent-a");
  });
});

describe("the record of a free agent", () => {
  const entry = freedEntry(
    { name: "agent-a", files: ["a.mjs"], issues: [3] },
    { name: "agent-a", files: ["a.mjs", "b.mjs"], from: "3-done", mr: 30, at },
  );

  it("keeps what it knows and where it came from", () => {
    assert.deepEqual(entry, {
      name: "agent-a",
      files: ["a.mjs", "b.mjs"],
      free: { at, from: "3-done", mr: 30 },
    });
  });

  it("lists only the free ones", () => {
    const record = {
      workers: {
        c1: { name: "agent-a", issues: [3], ...entry },
        c2: { name: "agent-b", issues: [4], free: null },
      },
    };
    assert.deepEqual(
      freeAgentsOf(record).map(({ id, name, issues }) => [id, name, issues]),
      [["c1", "agent-a", [3]]],
    );
    assert.deepEqual(freeAgentsOf(null), []);
  });
});

describe("a free agent and the next issue", () => {
  const free = [
    { id: "c9", name: "agent-a", files: ["src/a.mjs"], share: 0.2 },
  ];
  const agents = new Map([["c9", { pane: "p9", status: "idle" }]]);

  it("is a worker while herdr sees it idle and its context is below the limit", () => {
    assert.deepEqual(
      freeWorkersOf([], of, agents, {}, free).map(({ name, isFree, dir }) => [
        name,
        isFree,
        dir,
      ]),
      [["agent-a", true, null]],
    );
    assert.deepEqual(
      freeWorkersOf([], of, new Map([["c9", { status: "working" }]]), {}, free),
      [],
    );
    assert.deepEqual(
      freeWorkersOf([], of, agents, {}, [{ ...free[0], share: 0.6 }]),
      [],
    );
  });

  it("takes the next queued issue, no fresh agent", () => {
    const calls = [];
    const queued = { number: 8, feature: "8-next", queued: true, paths: [] };
    const { actions } = workerActionsOf(
      "o",
      [queued],
      of,
      agents,
      {},
      {
        shouldHandOut: true,
        memory: 64 * 1024 ** 3,
        setups: [],
        free,
        run: (name, number, args) => calls.push([number, ...args]),
      },
    );
    assert.deepEqual(calls, [[8, "--to", "agent-a"]]);
    assert.deepEqual(
      actions.map(({ kind, who }) => [kind, who]),
      [["hand over", "agent-a"]],
    );
  });

  it("goes to the one sharing the most files, else the one with the least context", () => {
    const two = [
      { id: "c9", name: "agent-a", files: ["src/a.mjs"], share: 0.3 },
      { id: "c8", name: "agent-b", files: [], share: 0.1 },
    ];
    const both = new Map([
      ["c9", { status: "idle" }],
      ["c8", { status: "idle" }],
    ]);
    const issues = [
      { number: 8, feature: "8-a", queued: true, paths: ["src/a.mjs"] },
      { number: 9, feature: "9-z", queued: true, paths: [] },
    ];
    const { suggested } = workerActionsOf(
      "o",
      issues,
      of,
      both,
      {},
      {
        memory: 0,
        setups: [],
        free: two,
      },
    );
    assert.equal(suggested.get(8).worker.name, "agent-a");
    assert.equal(suggested.get(9).worker.name, "agent-b");
    const [alone] = issues.slice(1);
    const least = workerActionsOf(
      "o",
      [alone],
      of,
      both,
      {},
      {
        memory: 0,
        setups: [],
        free: two,
      },
    );
    assert.equal(least.suggested.get(9).worker.name, "agent-b");
  });

  it("is found by an announced handover once its worktree is gone", () => {
    const [announced] = announcedOf(
      [{ number: 12, to: "agent-a", after: [40] }],
      [],
      {
        free,
        agents,
        awaited: new Map([[40, { state: "merged" }]]),
      },
    );
    assert.equal(announced.tree.isFree, true);
    assert.equal(announced.tree.claude.id, "c9");
    assert.equal(announced.isDue, true);
  });
});

describe("the end of the orchestration", () => {
  it("closes the free agents moved out of its space, the rest go with it", () => {
    const panes = [
      {
        pane_id: "p1",
        tab_id: "t1",
        workspace_id: "w-orch",
        agent_session: { value: "c1" },
      },
      {
        pane_id: "p2",
        tab_id: "t2",
        workspace_id: "w-other",
        agent_session: { value: "c2" },
      },
      {
        pane_id: "p3",
        tab_id: "t3",
        workspace_id: "w-other",
        agent_session: { value: "c3" },
      },
      { pane_id: "p4", tab_id: "t4", workspace_id: "w-other" },
    ];
    assert.deepEqual(
      freeTabsOutside([{ id: "c1" }, { id: "c2" }], panes, "w-orch"),
      ["t2"],
    );
    // Without its space: every free agent's tab.
    assert.deepEqual(
      freeTabsOutside([{ id: "c1" }, { id: "c2" }], panes, null),
      ["t1", "t2"],
    );
  });
});

describe("AGENTS", () => {
  const agent = {
    id: "c9",
    name: "agent-a",
    share: 0.18,
    issues: [3, 7],
    free: { at, from: "7-thing", mr: 31 },
  };
  const plain = text => text.replace(/\x1b\[[0-9;]*m/g, "");

  it("shows a free agent as free, with its context and where it came from", () => {
    const { state, text } = freeRowOf(agent, "idle");
    assert.equal(state.key, "free");
    assert.equal(state.tone, "free");
    assert.equal(text, "18 % · from 7-thing (!31) · 2 issues");
  });

  it("says when it is at work on its line or asks", () => {
    assert.equal(freeRowOf(agent, "working").state.text, "at work");
    assert.equal(freeRowOf(agent, "blocked").state.text, "asks you");
  });

  it("is a box of its own only while there is a free agent", () => {
    const none = [];
    pushFreeAgents(none, "o", []);
    assert.deepEqual(none, []);
    const lines = [];
    pushFreeAgents(lines, "o", [agent], new Map([["c9", { status: "idle" }]]));
    const text = plain(lines.join("\n"));
    assert.match(text, /AGENTS/);
    assert.match(text, /free agent-a/);
    assert.match(text, /dismiss/);
  });
});
