import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES, fit, visible } from "../lib/tty.mjs";
import { titleHeight, titleLines } from "./boxes.mjs";
import { LOGO, LOGO_LINES, titleLogo } from "./logo.mjs";

describe("logo", () => {
  it("is about square on screen: 3 lines of 5 columns, the triangle level with the blocks", () => {
    assert.deepEqual(titleLogo(), ["▀▀▀▀▀", "▀▀▀▀▀", "▀ 🭭 ▀"]);
    assert.equal(LOGO_LINES, 3);
    // The bars end where the bottom line ends.
    for (const line of LOGO) assert.equal(visible(line), 5);
  });
});

describe("titleLines", () => {
  const band = width =>
    titleLines("⇶ test", "12:00", "q quit · r refresh", width).map(line =>
      line.replace(ESCAPES, ""),
    );

  it("draws a band of the logo's height, the logo inside at its right end", () => {
    const lines = band(60);
    assert.equal(lines.length, titleHeight());
    for (const line of lines) {
      assert.equal(visible(line), 60);
      // The triangle is two UTF-16 units, one column: a view's fit keeps the line whole.
      assert.equal(fit(line, 60), line);
    }
    // A line above and below the logo, and two columns right of it: a line
    // is about two columns wide, the same space on three sides.
    assert.deepEqual(lines, [
      " ".repeat(60),
      ` ${" ".repeat(45)}12:00  ▀▀▀▀▀  `,
      ` ⇶ test${" ".repeat(44)}  ▀▀▀▀▀  `,
      ` ${" ".repeat(32)}q quit · r refresh  ▀ 🭭 ▀  `,
      " ".repeat(60),
    ]);
  });

  it("cuts the keys first and drops the logo before it cuts the title", () => {
    assert.deepEqual(band(20), [
      " ".repeat(20),
      "      12:00  ▀▀▀▀▀  ",
      " ⇶ test      ▀▀▀▀▀  ",
      " q quit · r  ▀ 🭭 ▀  ",
      " ".repeat(20),
    ]);
    assert.deepEqual(band(13), [
      " ".repeat(13),
      "       12:00 ",
      " ⇶ test      ",
      " q quit · r  ",
      " ".repeat(13),
    ]);
  });
});
