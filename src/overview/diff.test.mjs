import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { areaOf } from "./diff.mjs";

describe("areaOf", () => {
  it("names a package's file by the folder below src", () => {
    assert.equal(
      areaOf("products/vita/src/server/db/schema.ts"),
      "vita/server",
    );
    assert.equal(
      areaOf("packages/breadboard/src/lib/tty.mjs"),
      "breadboard/lib",
    );
  });

  it("names a file at a package's top, or directly in src, by the package", () => {
    assert.equal(areaOf("products/vita/package.json"), "vita");
    assert.equal(areaOf("packages/breadboard/src/overview.mjs"), "breadboard");
  });

  it("names a file outside the packages by its top-level folder or itself", () => {
    assert.equal(areaOf("scripts/check-agents.mjs"), "scripts");
    assert.equal(areaOf("AGENTS.md"), "AGENTS.md");
  });
});
