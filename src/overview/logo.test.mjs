import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES, fit, visible } from "../lib/tty.mjs";
import { titleHeight, titleLines } from "./boxes.mjs";
import { LOGO, LOGO_LINES, titleLogo } from "./logo.mjs";

describe("logo", () => {
  it("is square on screen: 3 lines of 6 columns, the triangle level with the blocks", () => {
    assert.deepEqual(titleLogo(), ["▀▀▀▀▀▀", "▀▀▀▀▀▀", "▀ 🭭 ▀ "]);
    assert.equal(LOGO_LINES, 3);
    for (const line of LOGO) assert.equal(visible(line), 2 * LOGO_LINES);
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
    assert.deepEqual(lines, [
      ` ${" ".repeat(45)}12:00  ▀▀▀▀▀▀ `,
      ` ⇶ test${" ".repeat(44)}  ▀▀▀▀▀▀ `,
      ` ${" ".repeat(32)}q quit · r refresh  ▀ 🭭 ▀  `,
    ]);
  });

  it("cuts the keys first and drops the logo before it cuts the title", () => {
    assert.deepEqual(band(20), [
      "      12:00  ▀▀▀▀▀▀ ",
      " ⇶ test      ▀▀▀▀▀▀ ",
      " q quit · r  ▀ 🭭 ▀  ",
    ]);
    assert.deepEqual(band(14), [
      "        12:00 ",
      " ⇶ test       ",
      " q quit · r r ",
    ]);
  });
});
