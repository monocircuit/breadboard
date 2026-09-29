import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  atWorkOf,
  freeWorkersOf,
  handOutOf,
  pathsOf,
  sharedOf,
  suggestionsOf,
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

  it("gives each agent one issue, in the order they wait, and none sharing nothing", () => {
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
});

describe("atWorkOf and handOutOf", () => {
  const tree = { dir: "/nonexistent" };
  const of = issue => ({ mr: issue.mr });

  it("counts a worktree as at work until its MR is merged or ready and green", () => {
    const issues = [
      { tree, mr: null },
      { tree, mr: { state: "opened", isDraft: true } },
      {
        tree,
        mr: {
          state: "opened",
          isDraft: false,
          pipeline: { status: "success" },
        },
      },
      { tree, mr: { state: "merged" } },
      { tree: null, queued: true },
    ];
    assert.equal(atWorkOf(issues, of), 2);
  });

  it("starts as many queued issues as there are slots, each to its merged agent or a fresh one", () => {
    const merged = { name: "m", isMerged: true };
    const ready = { name: "r", isMerged: false };
    const suggested = new Map([
      [1, { worker: merged }],
      [2, { worker: ready }],
    ]);
    const queued = [{ number: 1 }, { number: 2 }, { number: 3 }];
    assert.deepEqual(handOutOf(queued, suggested, 2), [
      { number: 1, worker: merged },
      { number: 2, worker: null },
    ]);
    assert.deepEqual(handOutOf(queued, suggested, 0), []);
    assert.deepEqual(handOutOf(queued, suggested, -1), []);
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
