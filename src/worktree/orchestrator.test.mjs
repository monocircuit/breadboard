import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { slugOf } from "./orchestrator.mjs";

describe("slugOf", () => {
  it("keeps up to five words without the small ones", () => {
    assert.equal(
      slugOf("Split worktree.mjs and overview.mjs into modules, with tests"),
      "split-worktree-mjs-overview-mjs",
    );
  });

  it("stays within 40 characters, cutting only between words", () => {
    const slug = slugOf(
      "Internationalisation of the notification preferences dialogue",
    );
    assert.ok(slug.length <= 40);
    assert.equal(slug, "internationalisation-notification");
  });

  it("drops accents and punctuation", () => {
    assert.equal(slugOf("Crème brûlée: the fix!"), "creme-brulee-fix");
  });

  it("names an issue without words `issue`", () => {
    assert.equal(slugOf("the — of"), "issue");
  });
});
