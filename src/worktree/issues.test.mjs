import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { boxesOf, unfilled } from "./issues.mjs";

describe("boxesOf", () => {
  it("reads every box with its state, text and the heading above it", () => {
    const text = [
      "## Phase 1 — Move",
      "",
      "- [ ] move the scripts",
      "- [x] the paths",
      "",
      "## Acceptance criteria",
      "* [X] lint green",
    ].join("\n");
    assert.deepEqual(boxesOf(text), [
      {
        line: 2,
        isTicked: false,
        text: "move the scripts",
        heading: "Phase 1 — Move",
      },
      { line: 3, isTicked: true, text: "the paths", heading: "Phase 1 — Move" },
      {
        line: 6,
        isTicked: true,
        text: "lint green",
        heading: "Acceptance criteria",
      },
    ]);
  });

  it("skips boxes inside a code block", () => {
    const text = ["```md", "- [ ] an example", "```", "- [ ] real"].join("\n");
    assert.deepEqual(
      boxesOf(text).map(box => box.text),
      ["real"],
    );
  });

  it("gives a box before any heading the heading null", () => {
    assert.equal(boxesOf("- [ ] first")[0].heading, null);
  });
});

describe("unfilled", () => {
  it("finds template comments and empty bullets, with their line", () => {
    const text = [
      "What and why.",
      "<!-- By topic, not by file. -->",
      "-",
      "- a real bullet",
    ].join("\n");
    assert.deepEqual(unfilled(text), [
      "     2  <!-- By topic, not by file. -->",
      "     3  - (empty bullet)",
    ]);
  });

  it("reports a comment over several lines once", () => {
    const text = ["<!-- one", "two -->", "done"].join("\n");
    assert.equal(unfilled(text).length, 1);
  });

  it("ignores comments in code spans and code blocks", () => {
    const text = [
      "the template has `<!-- comments -->`",
      "```",
      "<!-- in a block -->",
      "-",
      "```",
    ].join("\n");
    assert.deepEqual(unfilled(text), []);
  });
});
