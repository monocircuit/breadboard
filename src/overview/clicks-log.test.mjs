import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { clickLine, logClick } from "./clicks-log.mjs";

describe("the log of clicks (#303)", () => {
  const at = new Date("2026-10-01T12:00:00Z");

  it("says where a click landed and what ran, rows counted from 1", () => {
    assert.equal(
      clickLine(
        {
          row: 16,
          column: 5,
          viewRow: 30,
          ran: "button looks good",
          line: "looks good  broken",
        },
        { at, pid: 7 },
      ),
      "2026-10-01T12:00:00.000Z pid 7 row 17 col 5 view 31 → button looks good | looks good  broken",
    );
  });

  it("appends a line per click", () => {
    const file = path.join(
      fs.mkdtempSync(path.join(os.tmpdir(), "clicks-")),
      "clicks.log",
    );
    logClick({ row: 0, column: 1, ran: "nothing" }, file);
    logClick({ row: 1, column: 2, ran: "line", line: "x" }, file);
    const lines = fs.readFileSync(file, "utf-8").trim().split("\n");
    assert.equal(lines.length, 2);
    assert.match(lines[0], /row 1 col 1 → nothing$/);
    assert.match(lines[1], /row 2 col 2 → line \| x$/);
  });
});
