import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { cleanUpOf } from "./clean-up.mjs";
import {
  freeWorkersOf,
  freshOf,
  handOutOf,
  pathsOf,
  sharedOf,
  suggestionsOf,
  workerActionsOf,
} from "./workers.mjs";

describe("pathsOf", () => {
  it("finds the files an issue names, with a folder or without, each once", () => {
    assert.deepEqual(
      pathsOf(
        "Read `packages/breadboard/src/overview/render-orchestration.mjs`, overview/boxes.mjs and `tty.mjs`; tty.mjs again, orchestrate.md.",
      ),
      [
        "packages/breadboard/src/overview/render-orchestration.mjs",
        "overview/boxes.mjs",
        "tty.mjs",
        "orchestrate.md",
      ],
    );
    assert.deepEqual(pathsOf(null), []);
    assert.deepEqual(pathsOf("version 1.2 of e.g. nothing"), []);
  });
});

describe("sharedOf", () => {
  it("matches a named file to a worktree's file by its end", () => {
    const files = [
      "packages/breadboard/src/overview/boxes.mjs",
      "packages/breadboard/src/lib/tty.mjs",
    ];
    assert.deepEqual(
      sharedOf(
        ["overview/boxes.mjs", "tty.mjs", "xtty.mjs", "view.mjs"],
        files,
      ),
      ["overview/boxes.mjs", "tty.mjs"],
    );
  });
});

describe("suggestionsOf", () => {
  const a = { name: "a", share: 0.3, files: ["src/x.mjs", "src/y.mjs"] };
  const b = { name: "b", share: 0.1, files: ["src/x.mjs"] };
  const c = { name: "c", share: 0.05, files: ["src/z.mjs"] };

  it("picks the agent sharing most files, the one with more room on a tie", () => {
    const suggested = suggestionsOf(
      [{ number: 1, paths: ["x.mjs", "y.mjs"] }],
      [a, b, c],
    );
    assert.equal(suggested.get(1).worker, a);
    assert.deepEqual(suggested.get(1).shared, ["x.mjs", "y.mjs"]);
    assert.equal(
      suggestionsOf([{ number: 2, paths: ["x.mjs"] }], [a, b]).get(2).worker,
      b,
    );
  });

  it("gives each agent one issue, in the order they wait, and an only ready one none it shares nothing with", () => {
    const suggested = suggestionsOf(
      [
        { number: 1, paths: ["x.mjs"] },
        { number: 2, paths: ["x.mjs"] },
        { number: 3, paths: ["x.mjs"] },
        { number: 4, paths: ["w.mjs"] },
      ],
      [a, b],
    );
    assert.equal(suggested.get(1).worker, b);
    assert.equal(suggested.get(2).worker, a);
    assert.equal(suggested.has(3), false);
    assert.equal(suggested.has(4), false);
  });

  it("gives an issue nothing shares to the merged agent with the most room, after those that share (#265)", () => {
    const full = { name: "full", share: 0.4, files: [], isMerged: true };
    const roomy = { name: "roomy", share: 0.1, files: [], isMerged: true };
    const knows = { ...c, isMerged: true };
    const suggested = suggestionsOf(
      [
        { number: 1, paths: ["new.mjs"] },
        { number: 2, paths: ["z.mjs"] },
        { number: 3, paths: [] },
        { number: 4, paths: [] },
      ],
      [full, knows, roomy],
    );
    assert.equal(suggested.get(2).worker, knows);
    assert.equal(suggested.get(1).worker, roomy);
    assert.deepEqual(suggested.get(1).shared, []);
    assert.equal(suggested.get(3).worker, full);
    assert.equal(suggested.has(4), false);
  });
});

describe("freshOf", () => {
  const GIB = 1024 ** 3;
  it("starts fresh agents while their stack and agent fit, the starting ones counted", () => {
    assert.deepEqual(
      freshOf([{ number: 1 }, { number: 2 }, { number: 3 }], {
        free: 4 * GIB,
      }).starts,
      [1, 2],
    );
    assert.deepEqual(
      freshOf([{ number: 1 }], { free: 4 * GIB, reserved: 3 * GIB }).starts,
      [],
    );
  });
});

describe("handOutOf", () => {
  it("hands every queued issue to its merged agent, no count holding one back", () => {
    const merged = name => ({ name, isMerged: true });
    const ready = { name: "r", isMerged: false };
    const suggested = new Map([
      [1, { worker: merged("a") }],
      [2, { worker: ready }],
      [3, { worker: merged("b") }],
      [4, { worker: merged("c") }],
      [5, { worker: merged("d") }],
      [6, { worker: merged("e") }],
    ]);
    const queued = [1, 2, 3, 4, 5, 6, 7].map(number => ({ number }));
    assert.deepEqual(
      handOutOf(queued, suggested).map(({ number }) => number),
      [1, 3, 4, 5, 6],
    );
  });
});

describe("workerActionsOf", () => {
  it("offers a queued issue nothing fits as a start, one that fits as a hand over", () => {
    const claude = { id: "c1", used: 20_000, window: 200_000 };
    const issues = [
      {
        number: 3,
        feature: "3-done",
        tree: { dir: "/nonexistent/platform-3-done", name: "3-done", claude },
        mr: { state: "merged" },
      },
      { number: 8, feature: "8-fits", queued: true, paths: ["src/a.mjs"] },
      { number: 9, feature: "9-new", queued: true, paths: ["src/z.mjs"] },
    ];
    const of = issue => ({ mr: issue.mr, issue: { paths: issue.paths } });
    const { actions, held, suggested } = workerActionsOf(
      "o",
      issues,
      of,
      new Map([["c1", { status: "idle" }]]),
      { c1: { name: "3-done", files: ["src/a.mjs"] } },
    );
    assert.deepEqual(
      actions.map(({ kind, who }) => [kind, who]),
      [
        ["hand over", "3-done"],
        ["start", "9-new"],
      ],
    );
    assert.deepEqual([...held], ["/nonexistent/platform-3-done"]);
    assert.equal(suggested.get(8).worker.name, "3-done");
  });
});

describe("workerActionsOf without shared files (#265)", () => {
  const GIB = 1024 ** 3;
  const claude = { id: "c1", used: 36_000, window: 200_000 };
  const done = {
    number: 30,
    feature: "30-done",
    tree: { dir: "/nonexistent/platform-30-done", name: "30-done", claude },
    mr: { state: "merged" },
  };
  const of = issue => ({ mr: issue.mr, issue: { paths: issue.paths } });
  const idle = new Map([["c1", { status: "idle" }]]);
  const calls = [];
  const run = (name, number, args) => calls.push([number, ...args]);

  it("hands a queued issue nothing shares to the free agent and holds its clean-up", () => {
    calls.length = 0;
    const queued = { number: 31, feature: "31-new", queued: true, paths: [] };
    const { actions, holds } = workerActionsOf(
      "o",
      [done, queued],
      of,
      idle,
      {},
      { shouldHandOut: true, run, memory: 0 },
    );
    assert.deepEqual(calls, [[31, "--to", "30-done"]]);
    assert.equal(actions[0].text, "#31 → 30-done (free, 18 %)");
    const hold = holds.get(done.tree.dir);
    assert.ok(hold);
    assert.equal(
      cleanUpOf(done, { mr: done.mr, status: "idle", hold }).key,
      "waits",
    );
  });

  it("closes the free agent with its clean-up once nothing is queued", () => {
    const { holds } = workerActionsOf(
      "o",
      [done],
      of,
      idle,
      {},
      {
        memory: 0,
      },
    );
    assert.equal(holds.size, 0);
    assert.equal(
      cleanUpOf(done, {
        mr: done.mr,
        status: "idle",
        hold: holds.get(done.tree.dir),
      }).key,
      "due",
    );
  });

  it("starts a fresh agent by itself while memory allows, else says what it waits for", () => {
    calls.length = 0;
    const issues = [
      { number: 32, feature: "32-a", queued: true, paths: [] },
      { number: 33, feature: "33-b", queued: true, paths: [] },
    ];
    const { actions, fresh } = workerActionsOf(
      "o",
      issues,
      of,
      idle,
      {},
      {
        shouldHandOut: true,
        run,
        memory: 2 * GIB,
      },
    );
    assert.deepEqual(calls, [[32, "--now"]]);
    assert.equal(fresh.get(32), "starts a fresh agent");
    assert.match(fresh.get(33), /^waits for an agent to get free: /);
    assert.deepEqual(
      actions.map(({ kind, who }) => [kind, who]),
      [
        ["start", "32-a"],
        ["start", "33-b"],
      ],
    );
  });
});

describe("freeWorkersOf", () => {
  const claude = { id: "c1", used: 40_000, window: 200_000 };
  const issue = mr => ({
    number: 5,
    tree: { dir: "/nonexistent/platform-5-x", name: "5-x", claude },
    mr,
  });
  const of = item => ({ mr: item.mr });
  const agents = status => new Map([["c1", { status, tab: "t1" }]]);
  const merged = { state: "merged" };

  it("takes an idle agent whose MR is merged or ready and green", () => {
    const [worker] = freeWorkersOf([issue(merged)], of, agents("idle"), {
      c1: { name: "3-first", files: ["src/a.mjs"] },
    });
    assert.equal(worker.name, "3-first");
    assert.equal(worker.share, 0.2);
    assert.deepEqual(worker.files, ["src/a.mjs"]);
    const ready = {
      state: "opened",
      isDraft: false,
      pipeline: { status: "success" },
    };
    assert.equal(freeWorkersOf([issue(ready)], of, agents("done")).length, 1);
  });

  it("leaves out one at work, with a draft or a red pipeline, or above the limit", () => {
    assert.equal(
      freeWorkersOf([issue(merged)], of, agents("working")).length,
      0,
    );
    assert.equal(
      freeWorkersOf(
        [
          issue({
            state: "opened",
            isDraft: true,
            pipeline: { status: "success" },
          }),
        ],
        of,
        agents("idle"),
      ).length,
      0,
    );
    const full = { ...issue(merged) };
    full.tree = { ...full.tree, claude: { ...claude, used: 150_000 } };
    assert.equal(freeWorkersOf([full], of, agents("idle")).length, 0);
  });
});
