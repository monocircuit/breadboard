import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  fit,
  fitParts,
  flow,
  flowGroups,
  table,
  tableRows,
  underRows,
  visible,
  wrap,
} from "./tty.mjs";

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

describe("underRows", () => {
  const rows = [
    {
      head: ["", "[ ok ]", "try", "vita"],
      text: "one two three four five six",
    },
    { head: ["", "[ ✓ ]", "merge", "db"], text: "" },
  ];

  it("aligns the heads and puts the text under the column it names", () => {
    const [first, second] = underRows(rows, { width: 80, under: 2 });
    assert.deepEqual(first, [
      "  [ ok ]  try    vita",
      "          one two three four five six",
    ]);
    assert.deepEqual(second, ["  [ ✓ ]   merge  db"]);
  });

  it("wraps the text at the full width, the last of its lines cut with …", () => {
    const long = { head: ["", "x"], text: "word ".repeat(40).trim() };
    const [lines] = underRows([long], { width: 30, under: 1, maxLines: 2 });
    assert.equal(lines.length, 3);
    for (const line of lines.slice(1)) {
      assert.ok(line.startsWith("  word"));
      assert.ok(visible(line) <= 30);
    }
    assert.ok(lines.at(-1).endsWith("…"));
  });
});

describe("fitParts", () => {
  it("drops the parts that don't fit whole, without …", () => {
    const title = `ACTIONS 4 for you · ${RED}a button answers${RESET} · auto ones go`;
    assert.equal(fitParts(title, 80), title);
    assert.equal(
      fitParts(title, 30).replace(/\x1b\[[0-9;]*m|\x1b\]8;;\x1b\\/g, ""),
      "ACTIONS 4 for you",
    );
    // A first part too wide alone is cut with … still.
    assert.equal(fitParts("ACTIONS 4 for you", 10), "ACTIONS 4…");
  });
});

describe("flow", () => {
  const parts = ["✓ check:vita", "✓ check:monolithium", "✗ scan:image:mr"];

  it("keeps the parts on one line when they fit", () => {
    assert.deepEqual(flow(parts, 80, { gap: "   " }), [
      "✓ check:vita   ✓ check:monolithium   ✗ scan:image:mr",
    ]);
  });

  it("wraps between parts, never inside one, the next lines indented", () => {
    assert.deepEqual(flow(parts, 36, { gap: "   ", indent: "  " }), [
      "✓ check:vita   ✓ check:monolithium",
      "  ✗ scan:image:mr",
    ]);
    assert.deepEqual(flowGroups(parts, 36, { gap: "   ", indent: "  " }), [
      [0, 1],
      [2],
    ]);
  });

  it("counts the indent of a continued line", () => {
    assert.deepEqual(
      flowGroups(["aaaa", "bbbb", "cccc"], 9, { indent: "  " }),
      [[0, 1], [2]],
    );
    assert.deepEqual(
      flowGroups(["aaaa", "bb", "cc", "dd"], 7, { indent: "  " }),
      [
        [0, 1],
        [2, 3],
      ],
    );
  });

  it("measures painted parts by what shows, colours and links kept", () => {
    const painted = [`${RED}✗${RESET} ${link("u", "check:vita")}`, "✓ b"];
    const lines = flow(painted, 14);
    assert.deepEqual(lines, [painted[0], painted[1]]);
  });

  it("cuts only a part wider than a line alone", () => {
    assert.deepEqual(flow(["a-very-long-job-name", "b"], 10), [
      "a-very-lo…",
      "b",
    ]);
  });
});
