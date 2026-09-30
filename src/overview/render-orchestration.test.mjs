import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES, underRows, visible } from "../lib/tty.mjs";
import {
  actionRow,
  nextActionOf,
  renderOrchestration,
} from "./render-orchestration.mjs";

// Without a terminal: buttons in brackets, the box's inside 4 narrower.
describe("ACTIONS rows", () => {
  const decide = {
    who: "235-overview-actions-text-under-action",
    action: {
      kind: "decide",
      text: "Which line of the merge hint stays in the row of ACTIONS? The old one repeats what MERGE REQUESTS shows, the new one says what the two clicks do.",
      options: [
        "keep both lines",
        "keep the new line only",
        "keep the old line only",
      ],
      recommend: "keep the new line only",
      at: new Date().toISOString(),
    },
  };
  const of = () => ({});
  const rowsAt = (width, items = [decide]) =>
    underRows(
      items.map(item => actionRow(item, of)),
      { width: width - 4, under: 2 },
    );

  it("takes at most three lines for a decide of two sentences and three options at 200 columns", () => {
    const [lines] = rowsAt(200);
    assert.ok(lines.length <= 3, lines.join("\n"));
    assert.match(lines[0], /\[ ★ keep the new line only \]/);
    assert.ok(lines[1].trimStart().startsWith("Which line"));
  });

  it("puts the text under the kind at the full width, under the buttons when that leaves too little", () => {
    const [wide] = rowsAt(200);
    assert.equal(wide[1].indexOf("Which"), wide[0].indexOf("decide"));
    const [narrow] = rowsAt(60);
    for (const line of narrow.slice(1)) {
      assert.equal(line.length - line.trimStart().length, 2);
      assert.ok(visible(line) <= 56);
    }
  });

  it("cuts a long text with … after three lines", () => {
    const long = {
      ...decide,
      action: { ...decide.action, text: decide.action.text.repeat(4) },
    };
    const [lines] = rowsAt(200, [long]);
    assert.equal(lines.length, 4);
    assert.ok(lines.at(-1).endsWith("…"));
  });

  it("aligns the kinds of all rows", () => {
    const look = {
      who: "237-overview-try-shows-whether-it",
      action: {
        kind: "look",
        text: "the new row",
        at: new Date().toISOString(),
      },
    };
    const [first, second] = rowsAt(200, [decide, look]);
    assert.equal(first[0].indexOf("decide"), second[0].indexOf("look"));
    assert.equal(second[1].indexOf("the new row"), second[0].indexOf("look"));
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

  it("shows the state as a cell after the age", () => {
    const row = actionRow(waiting, of);
    assert.equal(row.head.length, 6);
    assert.equal(
      row.head.at(-1).replace(ESCAPES, ""),
      "deploying (publish, 3 min)",
    );
    assert.match(row.text.replace(ESCAPES, ""), /^boxes on the test instance/);
    // Other kinds keep their five cells.
    const look = { who: "a", action: { kind: "look", text: "y", at } };
    assert.equal(actionRow(look, of).head.length, 5);
  });

  it("greys a try that isn't ready, its links dropped", () => {
    const { head, text } = actionRow(waiting, of);
    for (const cell of [...head.slice(1, -1), text])
      assert.doesNotMatch(cell, /\x1b\]8;;|\x1b\[3[1-6]m/);
  });

  it("NEXT skips a try that isn't ready", () => {
    const look = { who: "a", action: { kind: "look", text: "y", at } };
    assert.equal(nextActionOf([waiting, ready], [], false), ready);
    assert.equal(nextActionOf([waiting, look], [], false), look);
    assert.equal(nextActionOf([waiting], [], true), null);
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
    assert.match(view, /AGENTS/);
    assert.doesNotMatch(view, /FEED/);
  });
});
