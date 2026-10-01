import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { visible } from "../lib/tty.mjs";
import { CELL_START } from "./boxes.mjs";
import {
  fileAt,
  fileLine,
  firstChangeIn,
  lineColumn,
  logTargets,
  matchFile,
} from "./files.mjs";

describe("a line of files a click opens (#257)", () => {
  const { text, spans } = fileLine("  ⚠ in: ", ["a.mjs", "b/c.mjs", "d", "e"]);

  it("names a few files and counts the rest", () => {
    assert.equal(visible(text), visible("  ⚠ in: a.mjs, b/c.mjs, d +1 more"));
  });

  it("knows where each file stands in the line", () => {
    assert.deepEqual(
      spans.map(({ from, to, file }) => [from, to, file]),
      [
        [8, 13, "a.mjs"],
        [15, 22, "b/c.mjs"],
        [24, 25, "d"],
        [26, 33, "e"],
      ],
    );
  });

  it("opens the file under the click, the one before it between two", () => {
    assert.equal(fileAt(spans, 9), "a.mjs");
    assert.equal(fileAt(spans, 13), "a.mjs");
    assert.equal(fileAt(spans, 16), "b/c.mjs");
    assert.equal(fileAt(spans, 30), "e");
  });

  it("opens the first file left of them", () => {
    assert.equal(fileAt(spans, 0), "a.mjs");
    assert.equal(fileAt([], 3), null);
  });

  it("maps a column on screen to the line's own, inside the box", () => {
    // The box drops the two blanks of the indent; its cell starts at CELL_START.
    assert.equal(lineColumn(CELL_START), 2);
    assert.equal(fileAt(spans, lineColumn(CELL_START + 13)), "b/c.mjs");
  });
});

describe("where a file's change is (#257)", () => {
  it("is the first line of the first hunk", () => {
    assert.equal(
      firstChangeIn(
        "diff --git a/x b/x\n@@ -10,2 +12,3 @@ ctx\n+a\n@@ -40 +44 @@\n",
      ),
      12,
    );
  });

  it("is the line before the gap of a hunk that only removes", () => {
    assert.equal(firstChangeIn("@@ -5,2 +4,0 @@\n-a\n-b\n"), 4);
    assert.equal(firstChangeIn("@@ -1,2 +0,0 @@\n"), 1);
  });

  it("is the top without a hunk", () => {
    assert.equal(firstChangeIn(""), 1);
  });
});

describe("file:line in the log of a failed job (#257)", () => {
  it("finds a path with a line and column", () => {
    assert.deepEqual(
      logTargets([" ❯ src/overview/files.test.mjs:12:5", "plain words"]),
      [{ file: "src/overview/files.test.mjs", line: 12 }, null],
    );
  });

  it("finds TypeScript's (line,col)", () => {
    assert.deepEqual(
      logTargets(["src/app/page.tsx(31,7): error TS2322: Type …"]),
      [{ file: "src/app/page.tsx", line: 31 }],
    );
  });

  it("gives ESLint's problems the file above them", () => {
    assert.deepEqual(
      logTargets([
        "/builds/monocircuit/platform/products/vita/web/src/a.ts",
        "  12:5  error  'x' is defined but never used  no-unused-vars",
        "  20:1  warning  Unexpected console  no-console",
        "✖ 2 problems (1 error, 1 warning)",
      ]),
      [
        {
          file: "/builds/monocircuit/platform/products/vita/web/src/a.ts",
          line: null,
        },
        {
          file: "/builds/monocircuit/platform/products/vita/web/src/a.ts",
          line: 12,
        },
        {
          file: "/builds/monocircuit/platform/products/vita/web/src/a.ts",
          line: 20,
        },
        null,
      ],
    );
  });

  it("skips a bare name without a line, and urls", () => {
    assert.deepEqual(
      logTargets([
        "ERR_PNPM_RECURSIVE_RUN_FIRST_FAIL package.json",
        "see https://gitlab.com/monocircuit/platform/-/jobs/1.json",
      ]),
      [null, null],
    );
  });
});

describe("the file of the checkout a logged path means (#257)", () => {
  const files = [
    "products/vita/web/src/a.ts",
    "packages/monolithium/src/a.ts",
    "products/vita/web/src/app/page.tsx",
    "package.json",
  ];

  it("is the path itself", () => {
    assert.equal(matchFile(files, "./package.json"), "package.json");
  });

  it("ends in a path within a package", () => {
    assert.equal(
      matchFile(files, "src/app/page.tsx"),
      "products/vita/web/src/app/page.tsx",
    );
  });

  it("is the end of an absolute path on the runner", () => {
    assert.equal(
      matchFile(
        files,
        "/builds/monocircuit/platform/products/vita/web/src/a.ts",
      ),
      "products/vita/web/src/a.ts",
    );
  });

  it("is none for a file the checkout doesn't have", () => {
    assert.equal(matchFile(files, "src/gone.ts"), null);
  });
});
