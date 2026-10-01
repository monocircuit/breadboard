import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES, visible } from "../lib/tty.mjs";
import {
  blockOf,
  isInert,
  nextFor,
  renderOrchestration,
} from "./render-orchestration.mjs";

// Without a terminal: buttons in brackets, chips as their label alone.
describe("ACTIONS blocks", () => {
  const at = new Date().toISOString();
  const decide = {
    who: "270-desktop-timeline-drawn-skia-pixi",
    action: {
      kind: "decide",
      text: "Does the desktop timeline move to Skia (phases 2–4 after #194/#269)? Skia held 60 fps where pixi dropped to 41, the cost is +0.3–0.7 s cold start.",
      options: ["Go", "No-go"],
      descriptions: [
        "phases 2–4 build the Skia painter, compressed wasm and woff2 font before it becomes the default",
        "pixi stays on the desktop, the iPad timeline needs its own renderer and the prototype goes",
      ],
      recommend: "Go",
      at,
    },
  };
  const of = () => ({});
  const plain = line => line.replace(ESCAPES, "");
  const linesOf = (item, width) =>
    blockOf(item, of, width).map(({ line }) => plain(line));

  for (const width of [96, 196])
    it(`shows a decide's options numbered, in full, one under the other, at ${width} columns`, () => {
      const block = blockOf(decide, of, width);
      const lines = block.map(({ line }) => plain(line));
      for (const line of lines) assert.ok(visible(line) <= width, line);
      assert.ok(
        lines.every(line => !line.endsWith("…")),
        lines.join("\n"),
      );
      assert.match(
        lines[0],
        /^decide {2}270-desktop-timeline-drawn-skia-pixi {2}/,
      );
      assert.ok(lines[1].startsWith("Does the desktop timeline"));
      const go = lines.indexOf("1. Go (Recommended)");
      const noGo = lines.indexOf("2. No-go");
      assert.ok(go > 1 && noGo > go, lines.join("\n"));
      const shown = lines.join(" ").replaceAll(/\s+/g, " ");
      for (const description of decide.action.descriptions)
        assert.ok(shown.includes(description), description);
      // The description under its label, after the number.
      assert.ok(lines[go + 1].startsWith("   phases 2–4"));
      // Its label and its description answer the option, the rest does it.
      assert.equal(block[go].on, 0);
      assert.equal(block[go + 1].on, 0);
      assert.equal(block[noGo].on, 1);
      assert.equal(block[0].on, "go");
    });

  it("shows an option stored before #273 split at its first colon, its answer the whole text", () => {
    const old = {
      ...decide,
      action: {
        ...decide.action,
        options: ["Go: phases 2–4", "No-go: pixi stays"],
        descriptions: undefined,
        recommend: "Go: phases 2–4",
      },
    };
    const lines = linesOf(old, 96);
    assert.ok(lines.includes("1. Go (Recommended)"));
    assert.ok(lines.includes("   phases 2–4"));
  });

  it("puts the buttons of a try, look and plan on a line after the text", () => {
    const look = {
      who: "a",
      action: { kind: "look", text: "the new row", at },
    };
    const block = blockOf(look, of, 96);
    assert.deepEqual(
      block.map(({ line, on }) => [plain(line).trimEnd(), on]),
      [
        ["look  a  just now", "go"],
        ["the new row", "go"],
        ["[ done ]", "buttons"],
      ],
    );
  });

  it("marks an automatic action in its chip and gives it no buttons", () => {
    const auto = {
      who: "a",
      isAuto: true,
      action: { kind: "merge", text: "!282 ready", at },
    };
    const lines = linesOf(auto, 96);
    assert.match(lines[0], /^merge · auto {2}a/);
    assert.equal(lines.length, 2);
  });

  it("parts the blocks with a line in the view", () => {
    const tree = {
      dir: "/nowhere/273-blocks",
      name: "273-blocks",
      actions: [
        { id: 1, ...decide.action },
        { id: 2, kind: "look", text: "the new row", at },
      ],
    };
    const view = renderOrchestration(
      {
        issues: [
          {
            number: 273,
            feature: "273-blocks",
            branch: "feature/273-blocks",
            tree,
          },
        ],
        agents: new Map(),
        ownActions: [],
        trees: [tree],
      },
      null,
    );
    const lines = plain(view).split("\n");
    const start = lines.findIndex(line => line.includes("ACTIONS"));
    const end = lines.findIndex((line, i) => i > start && line.startsWith("╰"));
    const dividers = lines
      .slice(start, end)
      .filter(line => /^│ ─+ │$/.test(line));
    assert.equal(dividers.length, 1);
    for (const line of lines) assert.ok(visible(line) <= 120, line);
  });
});

describe("a try's state", () => {
  const at = new Date().toISOString();
  const tryOf = (tryState, text = "boxes on the test instance") => ({
    who: "237-overview-try-shows-whether-it",
    action: { kind: "try", text, url: "/atlas", on: "test", at },
    tryState,
  });
  const waiting = tryOf({
    isReady: false,
    label: "deploying (publish, 3 min)",
    tone: "waits",
  });
  const ready = tryOf({ isReady: true, label: "ready", tone: "ready" }, "x");
  const of = () => ({});

  it("shows the state in the head after the age", () => {
    const [head, text] = blockOf(waiting, of, 96).map(({ line }) =>
      line.replace(ESCAPES, ""),
    );
    assert.ok(head.endsWith("deploying (publish, 3 min)"), head);
    assert.match(text, /^boxes on the test instance/);
  });

  it("greys a try that isn't ready, its links dropped, without buttons", () => {
    const block = blockOf(waiting, of, 96);
    assert.equal(block.length, 2);
    assert.doesNotMatch(block[1].line, /\x1b\]8;;|\x1b\[3[1-6]m/);
    assert.equal(blockOf(ready, of, 96).at(-1).on, "buttons");
  });

  it("NEXT skips a try that isn't ready", () => {
    const look = { who: "a", action: { kind: "look", text: "y", at } };
    assert.equal(nextFor([waiting, ready], [], null).item, ready);
    assert.equal(nextFor([waiting, look], [], null).item, look);
    assert.equal(nextFor([waiting], [], { isReady: false }), null);
  });
});

// Without a terminal the view is a --once print: nothing is notified or delivered.
describe("the orchestration view", () => {
  it("has no FEED section", () => {
    const tree = {
      dir: "/nowhere/240-overview-drop-feed",
      name: "240-overview-drop-feed",
      claude: { id: "c1", answer: "Pushed stage 1." },
      actions: [],
    };
    const view = renderOrchestration(
      {
        issues: [
          {
            number: 240,
            feature: "240-overview-drop-feed",
            branch: "feature/240-overview-drop-feed",
            tree,
          },
        ],
        agents: new Map([["c1", { status: "done" }]]),
        ownActions: [],
        trees: [tree],
      },
      null,
    );
    assert.doesNotMatch(view, /AGENTS/);
    assert.match(view, /WORKTREES/);
    assert.doesNotMatch(view, /FEED/);
  });
});

describe("MERGE REQUESTS", () => {
  const issueOf = (number, feature) => ({
    number,
    feature,
    branch: `fix/${feature}`,
    tree: { dir: `/nowhere/${feature}`, name: feature, actions: [] },
  });
  const mrOf = iid => ({
    iid,
    url: `https://gitlab.example/mr/${iid}`,
    title: `MR ${iid}`,
    state: "opened",
    pipeline: null,
  });

  it("draws open MRs with rows of the graph alone, which have no issue (#306)", () => {
    const issues = [issueOf(1, "1-a"), issueOf(2, "2-b"), issueOf(3, "3-c")];
    const needs = { 1: [], 2: [1], 3: [1] };
    const remote = {
      of: new Map(
        issues.map(issue => [
          issue.branch,
          {
            mr: mrOf(10 + issue.number),
            issue: {
              iid: issue.number,
              needs: needs[issue.number],
              boxes: [],
            },
          },
        ]),
      ),
      develop: null,
    };
    const view = renderOrchestration(
      {
        issues,
        agents: new Map(),
        ownActions: [],
        trees: issues.map(issue => issue.tree),
      },
      remote,
    );
    const plain = view.replace(ESCAPES, "");
    assert.match(plain, /MERGE REQUESTS/);
    for (const iid of [11, 12, 13]) assert.match(plain, new RegExp(`!${iid}`));
  });
});

describe("isInert", () => {
  it("takes no click on what can't do anything yet (#290)", () => {
    assert.equal(isInert({ tryState: { isReady: false } }), true);
    assert.equal(isInert({ action: { isRunning: true } }), true);
  });

  it("keeps the click where it does something", () => {
    assert.equal(isInert({ tryState: { isReady: true } }), false);
    // A failed deployment opens its job.
    assert.equal(
      isInert({ tryState: { isReady: false, url: "https://j" } }),
      false,
    );
    assert.equal(isInert({ action: { kind: "merge" } }), false);
  });
});
