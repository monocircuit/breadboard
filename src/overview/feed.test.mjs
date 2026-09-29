import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { feedRowsOf } from "./feed.mjs";

const plain = cells => cells.map(cell => cell.replace(ESCAPES, ""));
const issue = (feature, tree) => ({ feature, tree });

describe("feedRowsOf", () => {
  it("has a row per issue with a worktree, with state and line", () => {
    const rows = feedRowsOf(
      [
        issue("180-answers", {
          dir: "/a",
          note: { text: "hook written", at: new Date().toISOString() },
          claude: { id: "c1", answer: "Old", answerAt: new Date(0) },
        }),
        issue("181-auto", { dir: "/b", note: null, claude: null }),
        issue("182-merged", null),
      ],
      new Map([["c1", { status: "working" }]]),
    );
    assert.equal(rows.length, 2);
    const [first, second] = rows.map(row => plain(row.cells));
    assert.deepEqual(first.slice(1), [
      "◐",
      "180-answers",
      "just now",
      "› hook written",
    ]);
    assert.equal(second.at(-1), "no agent");
  });

  it("shows the Stop hook's line without a transcript", () => {
    const [row] = feedRowsOf([
      issue("183-feed", {
        dir: "/c",
        note: null,
        stopped: { text: "Stage 3 pushed.", at: new Date().toISOString() },
        claude: null,
      }),
    ]);
    assert.equal(plain(row.cells).at(-1), '" Stage 3 pushed.');
  });
});
