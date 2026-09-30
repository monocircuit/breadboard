import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { underRows, visible } from "../lib/tty.mjs";
import { actionRow } from "./render-orchestration.mjs";

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
