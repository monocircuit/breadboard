import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { emptyClaimOf } from "./remove.mjs";

describe("emptyClaimOf (#272)", () => {
  it("frees the claim of a branch with nothing on it, so the issue can start again", () => {
    assert.ok(
      emptyClaimOf({ isOnOrigin: true, hasOwnCommits: false, openMrs: 0 }),
    );
  });

  it("keeps a claim with commits, an open MR, or when GitLab doesn't answer", () => {
    for (const claim of [
      { isOnOrigin: false, hasOwnCommits: false, openMrs: 0 },
      { isOnOrigin: true, hasOwnCommits: true, openMrs: 0 },
      { isOnOrigin: true, hasOwnCommits: false, openMrs: 1 },
      { isOnOrigin: true, hasOwnCommits: false, openMrs: Infinity },
    ])
      assert.equal(emptyClaimOf(claim), null);
  });
});
