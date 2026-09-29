import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { stateOf } from "./agent-state.mjs";

const mr = (extra = {}) => ({
  iid: 192,
  isDraft: false,
  pipeline: { status: "success" },
  ...extra,
});

describe("stateOf", () => {
  it("puts an agent that asks something first, over a failed pipeline", () => {
    assert.equal(
      stateOf({
        agentStatus: "blocked",
        mr: mr({ pipeline: { status: "failed" } }),
      }),
      "needs you · !192",
    );
  });

  it("puts a failed pipeline over work in progress", () => {
    assert.equal(
      stateOf({
        agentStatus: "working",
        mr: mr({ pipeline: { status: "failed" } }),
      }),
      "pipeline failed · !192",
    );
  });

  it("says working, with the MR when there is one", () => {
    assert.equal(stateOf({ agentStatus: "working", mr: null }), "working");
    assert.equal(
      stateOf({ agentStatus: "working", mr: mr() }),
      "working · !192",
    );
  });

  it("names an idle agent's MR as draft or review", () => {
    assert.equal(
      stateOf({ agentStatus: "idle", mr: mr({ isDraft: true }) }),
      "draft · !192",
    );
    assert.equal(stateOf({ agentStatus: "idle", mr: mr() }), "review · !192");
  });

  it("has nothing to say for an idle agent without an MR", () => {
    assert.equal(stateOf({ agentStatus: "idle", mr: null }), null);
    assert.equal(stateOf({ agentStatus: "unknown", mr: null }), null);
  });
});
