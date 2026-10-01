import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { HOVER } from "../lib/mouse.mjs";
import { buttonsOn, hovered } from "./boxes.mjs";

const ISLAND = "\x1b[48;2;38;38;38m";
const CYAN = "\x1b[1;30;46m";
const GRAY = "\x1b[2;37;100m";
/** An island's line as closeBox draws it: ` ✓ ` and ` later ` as buttons at column 2. */
const line = `${ISLAND} ${CYAN} ✓ \x1b[0m${ISLAND} ${CYAN} later \x1b[0m${ISLAND} rest \x1b[0m`;

describe("buttonsOn", () => {
  it("finds the buttons of a drawn line by their columns, a column of slack after each", () => {
    assert.deepEqual(
      buttonsOn(line).buttons.map(({ from, to }) => [from, to]),
      [
        [2, 5],
        [6, 13],
      ],
    );
  });

  it("leaves out a gray button: it can't be clicked yet", () => {
    assert.deepEqual(buttonsOn(`${GRAY} ✕ \x1b[0m`).buttons, []);
  });
});

describe("hovered", () => {
  it("lights the button under the pointer and nothing else", () => {
    const hover = hovered(line, 8);
    assert.equal(hover.key, "button 6");
    assert.equal(
      hover.line,
      `${ISLAND} ${CYAN} ✓ \x1b[0m${ISLAND} \x1b[1;30;106m later \x1b[0m${ISLAND} rest \x1b[0m`,
    );
  });

  it("puts the line on a lighter ground off its buttons", () => {
    const hover = hovered(line, 16);
    assert.equal(hover.key, "line");
    assert.ok(!hover.line.includes(ISLAND));
    assert.ok(hover.line.startsWith(HOVER));
  });
});
