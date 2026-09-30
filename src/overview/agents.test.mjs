import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { agentCountsOf, agentRowsOf, agentStateOf } from "./agents.mjs";

const claude = id => ({ id, used: 50_000, window: 200_000 });
const tree = (name, id, extra = {}) => ({
  dir: `/nonexistent/platform-${name}`,
  name,
  claude: claude(id),
  ...extra,
});
const of = issue => ({ mr: issue.mr ?? null });

describe("agentStateOf", () => {
  it("names what an agent waits for before what it does", () => {
    assert.equal(agentStateOf({}, "blocked").key, "asks");
    assert.deepEqual(
      agentStateOf({ actions: [{ kind: "try" }, { kind: "merge" }] }, "idle"),
      { key: "yours", text: "waits for you: try, merge" },
    );
    assert.deepEqual(agentStateOf({ wait: { mrs: [232] } }, "idle"), {
      key: "waits",
      text: "waits for !232",
    });
    assert.equal(agentStateOf({}, "working").text, "at work");
    assert.equal(
      agentStateOf({}, "idle", { state: "merged" }).text,
      "idle, its MR done",
    );
    assert.equal(
      agentStateOf({ actions: [{ kind: "try", answer: "ok" }] }, "idle").key,
      "idle",
    );
  });
});

describe("agentRowsOf", () => {
  const issues = [
    { number: 1, feature: "1-a", tree: tree("1-a", "c1") },
    { number: 2, feature: "2-b", tree: tree("2-b", "c2") },
    {
      number: 3,
      feature: "3-c",
      tree: tree("3-c", "c3"),
      mr: { state: "merged" },
    },
    { number: 4, feature: "4-d", tree: tree("4-d", "c4") },
    {
      number: 5,
      feature: "5-e",
      tree: tree("5-e", "c5", { wait: { mrs: [7] } }),
    },
    { number: 6, feature: "6-f", queued: true },
    { number: 8, feature: "8-g", queued: true },
    { number: 9, feature: "9-h", queued: true },
    { number: 10, feature: "10-i", why: "#10 is closed" },
  ];
  const status = new Map(
    ["c1", "c2", "c4"].map(id => [id, { status: "working" }]),
  );
  status.set("c3", { status: "idle" });
  const worker = { name: "3-c" };

  it("shows every agent with its issue, state, context and what it gets next", () => {
    const { agents } = agentRowsOf(issues, of, status, {
      known: { c1: { name: "first" } },
      handovers: [{ number: 8, to: "4-d", after: [240] }],
      suggested: new Map([[6, { worker }]]),
    });
    assert.deepEqual(
      agents.map(row => [
        row.name,
        `#${row.issue.number}`,
        row.state.text,
        row.share,
        row.next,
      ]),
      [
        ["first", "#1", "at work", 0.25, ""],
        ["2-b", "#2", "at work", 0.25, ""],
        [
          "3-c",
          "#3",
          "idle, its MR done",
          0.25,
          "next #6 once its MR is merged",
        ],
        ["4-d", "#4", "at work", 0.25, "next #8 after !240"],
        ["5-e", "#5", "waits for !7", 0.25, ""],
      ],
    );
    assert.equal(agentCountsOf(agents), "3 at work · 1 idle · 1 waiting");
  });

  it("keeps herdr's state of each agent for its colour, a waiting one's too", () => {
    const { agents } = agentRowsOf(issues, of, status);
    assert.deepEqual(
      agents.map(row => [row.name, row.status, row.state.key]),
      [
        ["1-a", "working", "work"],
        ["2-b", "working", "work"],
        ["3-c", "idle", "idle"],
        ["4-d", "working", "work"],
        // Waits for !7 as text, coloured by what herdr says: no pane, no state.
        ["5-e", null, "waits"],
      ],
    );
  });

  it("leaves the queued issues to WORKTREES' queue", () => {
    const { waiting } = agentRowsOf(issues, of, status, {
      handovers: [{ number: 8, to: "4-d", after: [240] }],
      suggested: new Map([[6, { worker }]]),
    });
    assert.deepEqual(waiting, []);
  });

  it("names a worktree whose agent never started", () => {
    const { agents, waiting } = agentRowsOf(
      [
        {
          number: 1,
          feature: "1-a",
          tree: { dir: "/x", name: "1-a", claude: null },
        },
      ],
      of,
      new Map(),
    );
    assert.equal(agents.length, 0);
    assert.equal(waiting[0].text, "no agent runs in its worktree");
  });
});
