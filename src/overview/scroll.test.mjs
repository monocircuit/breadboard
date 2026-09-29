import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { titleHeight, titleLines } from "./boxes.mjs";
import { KEYS, screen } from "./screen.mjs";
import {
  moreMark,
  scrollBy,
  scrollWindow,
  viewRowOf,
  windowOf,
} from "./scroll.mjs";

const view = count => Array.from({ length: count }, (_, i) => `line ${i}`);

describe("scrollWindow", () => {
  it("shows all of a view that fits", () => {
    const shown = scrollWindow(view(5), 10, { sticky: 1, offset: 3 });
    assert.deepEqual(shown.lines, view(5));
    assert.deepEqual([shown.offset, shown.above, shown.below], [0, 0, 0]);
  });

  it("keeps the sticky lines on top and scrolls the rest", () => {
    const shown = scrollWindow(view(20), 6, { sticky: 2, offset: 3 });
    assert.deepEqual(shown.lines, [
      "line 0",
      "line 1",
      "line 5",
      "line 6",
      "line 7",
      "line 8",
    ]);
    assert.deepEqual([shown.above, shown.below], [3, 11]);
  });

  it("clamps an offset past the end, as when a view got shorter", () => {
    const shown = scrollWindow(view(10), 6, { sticky: 1, offset: 50 });
    assert.equal(shown.offset, 4);
    assert.equal(shown.lines.at(-1), "line 9");
    assert.equal(shown.below, 0);
  });

  it("clamps a negative offset to the top", () => {
    assert.equal(scrollWindow(view(10), 6, { offset: -4 }).offset, 0);
  });

  it("never shows more sticky lines than the pane has rows", () => {
    const shown = scrollWindow(view(10), 2, { sticky: 4 });
    assert.deepEqual(shown.lines, ["line 0", "line 1"]);
  });
});

describe("viewRowOf", () => {
  it("leaves a sticky row where it is", () => {
    assert.equal(viewRowOf(1, { sticky: 3, offset: 7 }), 1);
  });

  it("moves a scrolled row by the offset", () => {
    assert.equal(viewRowOf(3, { sticky: 3, offset: 7 }), 10);
  });

  it("hits the line the window shows on that row", () => {
    const lines = view(30);
    const position = { sticky: 2, offset: 9 };
    const shown = scrollWindow(lines, 8, position).lines;
    for (const [row, line] of shown.entries())
      assert.equal(lines[viewRowOf(row, position)], line);
  });
});

describe("moreMark", () => {
  it("is empty when nothing is cut off", () => {
    assert.equal(moreMark(0, 0), "");
  });

  it("counts what is below and above", () => {
    assert.equal(moreMark(0, 12), "↓ 12 more");
    assert.equal(moreMark(3, 12), "↑ 3 ↓ 12 more");
  });
});

describe("windowOf", () => {
  beforeEach(() => {
    screen.sticky = titleHeight();
    screen.scroll = 0;
  });

  // The band and two rows of the view under it.
  const rows = () => titleHeight() + 2;

  it("puts the mark by the time in the title band and keeps the position clamped", () => {
    const text = [...titleLines("⇶ test", "12:00"), ...view(20)].join("\n");
    screen.scroll = 100;
    const shown = windowOf(text, rows());
    assert.equal(screen.scroll, 18);
    assert.match(shown[1].replace(ESCAPES, ""), /↑ 18 more\s+12:00/);
    assert.equal(shown.length, rows());
    assert.equal(shown.at(-1), "line 19");
  });

  it("scrolls to the top and the bottom", () => {
    const text = [...titleLines("⇶ test", "12:00"), ...view(20)].join("\n");
    scrollBy(Infinity);
    windowOf(text, rows());
    assert.equal(screen.scroll, 18);
    scrollBy(-Infinity);
    windowOf(text, rows());
    assert.equal(screen.scroll, 0);
  });
});

describe("KEYS", () => {
  it("names the scroll keys", () => {
    for (const key of ["wheel", "j k", "PgUp PgDn", "g G"])
      assert.ok(KEYS.includes(key), key);
  });
});
