import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { movesTab } from "./layout.mjs";
import { orchestrationOf } from "./origins.mjs";

describe("wt:new and the tab running it (#245)", () => {
  it("takes an agent's tab into the new space", () => {
    assert.equal(movesTab({ isAgent: true }), true);
  });

  it("leaves a human's tab where it is", () => {
    assert.equal(movesTab({ isAgent: false }), false);
  });

  it("leaves the orchestrator's tab when wt:orchestrate starts the worktree", () => {
    assert.equal(
      movesTab({ isAgent: true, origin: { name: "breadboard", slot: 0 } }),
      false,
    );
  });

  it("leaves the orchestrator's tab when it runs wt:new from its space itself", () => {
    assert.equal(
      movesTab({ isAgent: true, orchestration: "breadboard" }),
      false,
    );
  });

  it("tells the orchestrator's space from a worktree's by its label", () => {
    const titles = new Map([["breadboard", "@monocircuit/breadboard"]]);
    const from = label => ({
      isAgent: true,
      orchestration: orchestrationOf(label, titles),
    });
    assert.equal(movesTab(from("⇶ @monocircuit/breadboard")), false);
    assert.equal(movesTab(from("217-agents-workers")), true);
    assert.equal(movesTab(from("Monocircuit/Platform")), true);
  });
});
