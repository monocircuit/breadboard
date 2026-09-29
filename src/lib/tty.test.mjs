import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { fit, table, tableRows, visible, wrap } from "./tty.mjs";

const RED = "\x1b[31m";
const RESET = "\x1b[0m";
const link = (url, text) => `\x1b]8;;${url}\x1b\\${text}\x1b]8;;\x1b\\`;

describe("visible", () => {
  it("counts neither colours nor links", () => {
    assert.equal(visible(`${RED}abc${RESET}`), 3);
    assert.equal(visible(link("https://example.org", "!191")), 4);
  });
});

describe("fit", () => {
  it("leaves a line that fits as it is", () => {
    assert.equal(fit(`${RED}abc${RESET}`, 3), `${RED}abc${RESET}`);
  });

  it("cuts to the width, the ellipsis included, and keeps the colours", () => {
    const cut = fit(`${RED}abcdef${RESET}`, 4);
    assert.equal(visible(cut), 4);
    assert.ok(cut.startsWith(`${RED}abc`));
    assert.ok(cut.endsWith("…"));
  });
});

describe("table", () => {
  it("pads every column but the last to its widest cell", () => {
    assert.deepEqual(
      table([
        ["a", "bb", "x"],
        ["ccc", "d", "yy"],
      ]),
      ["a    bb  x", "ccc  d   yy"],
    );
  });

  it("measures painted cells by what shows", () => {
    const [first] = table([
      [`${RED}a${RESET}`, "b"],
      ["cc", "d"],
    ]);
    assert.equal(visible(first), "a   b".length);
  });
});

describe("wrap", () => {
  it("leaves a text that fits as it is", () => {
    assert.deepEqual(wrap("a short one", 20), ["a short one"]);
  });

  it("breaks between words, never inside one that fits", () => {
    assert.deepEqual(wrap("the quick brown fox jumps", 10), [
      "the quick",
      "brown fox",
      "jumps",
    ]);
  });

  it("cuts a word longer than a line", () => {
    assert.deepEqual(wrap("abcdefghij kl", 4), ["abcd", "efgh", "ij", "kl"]);
  });

  it("counts neither colours nor links and carries them over a break", () => {
    const lines = wrap(
      `${RED}aaa bbb${RESET} ${link("https://x.org", "ccc ddd")}`,
      7,
    );
    assert.deepEqual(lines.map(visible), [7, 7]);
    assert.deepEqual(
      lines.map(line =>
        line.replaceAll(/\x1b\[[0-9;]*m|\x1b\]8;;[^\x1b]*\x1b\\/g, ""),
      ),
      ["aaa bbb", "ccc ddd"],
    );
    const [, second] = wrap(`${RED}aaa bbb${RESET}`, 4);
    assert.equal(second, `${RED}bbb${RESET}`);
    const [first, rest] = wrap(link("https://x.org", "aaa bbb"), 4);
    assert.ok(first.endsWith("\x1b]8;;\x1b\\"));
    assert.ok(rest.startsWith("\x1b]8;;https://x.org\x1b\\"));
  });

  it("stops after the most lines, the last one ending in …", () => {
    const lines = wrap("aa bb cc dd ee ff", 5, 2);
    assert.equal(lines.length, 2);
    assert.equal(lines[0], "aa bb");
    assert.equal(visible(lines[1]), 5);
    assert.ok(lines[1].endsWith("…"));
  });
});

describe("tableRows", () => {
  it("wraps the last cell under where it starts, the other cells in their columns", () => {
    assert.deepEqual(
      tableRows(
        [
          ["a", "one two three four five six"],
          ["bb", "short"],
        ],
        { width: 24 },
      ),
      [["a   one two three four", "    five six"], ["bb  short"]],
    );
  });

  it("stops a row after its most lines", () => {
    const [[first, second, ...rest]] = tableRows(
      [["a", "one two three four five six seven eight nine ten"]],
      {
        width: 20,
        maxLines: 2,
      },
    );
    assert.equal(first, "a  one two three");
    assert.equal(visible(second), 20);
    assert.ok(second.endsWith("…"));
    assert.equal(rest.length, 0);
  });

  it("wraps nothing without a width", () => {
    assert.deepEqual(tableRows([["a", "x ".repeat(50)]]), [
      [`a  ${"x ".repeat(50)}`],
    ]);
  });
});
