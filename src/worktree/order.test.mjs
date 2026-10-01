import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { groupMove, workOrderOf } from "./order.mjs";

const REPO = "/platform/.git";
const main = { workspace_id: "w1", label: "Monocircuit/Platform" };
const space = (id, label, repo = REPO) => ({
  workspace_id: id,
  label,
  worktree: { is_linked_worktree: true, repo_key: repo },
});
const mainSpace = {
  ...main,
  worktree: { is_linked_worktree: false, repo_key: REPO },
};
const other = { workspace_id: "w2", label: "Dash" };

/** The lookups of two orchestrations: breadboard (#249 → #240) and vita. */
const lookups = {
  originOf: label =>
    ({
      "240-overview": "breadboard",
      "249-agents": "breadboard",
      "250-canvas": "vita",
    })[label] ?? null,
  workOrderOf: name =>
    ({ breadboard: ["249-agents", "240-overview"], vita: ["250-canvas"] })[
      name
    ] ?? [],
  orchestrationOf: label =>
    ({ "⇶ @monocircuit/breadboard": "breadboard", "⇶ vita": "vita" })[label] ??
    null,
};
const order = (list, move) => {
  const moved = new Set(move.workspace_ids);
  const rest = list.filter(w => !moved.has(w.workspace_id));
  const at = move.before_workspace_id
    ? rest.findIndex(w => w.workspace_id === move.before_workspace_id)
    : rest.length;
  const byId = new Map(list.map(w => [w.workspace_id, w]));
  return [
    ...rest.slice(0, at),
    ...move.workspace_ids.map(id => byId.get(id)),
    ...rest.slice(at),
  ].map(w => w.label);
};

describe("the order of the spaces of an orchestration (#255)", () => {
  it("puts the worktrees of an orchestration right below its space, in the work order", () => {
    const list = [
      mainSpace,
      other,
      space("w3", "⇶ @monocircuit/breadboard"),
      space("w4", "link-only-dot"),
      space("w5", "agents-mark-ready"),
      space("w6", "240-overview"),
      space("w7", "249-agents"),
    ];
    const move = groupMove(list, lookups);
    assert.deepEqual(order(list, move), [
      "Monocircuit/Platform",
      "Dash",
      "link-only-dot",
      "agents-mark-ready",
      "⇶ @monocircuit/breadboard",
      "249-agents",
      "240-overview",
    ]);
  });

  it("moves nothing when the order stands", () => {
    const list = [
      mainSpace,
      space("w4", "link-only-dot"),
      space("w3", "⇶ @monocircuit/breadboard"),
      space("w7", "249-agents"),
      space("w6", "240-overview"),
      other,
    ];
    assert.equal(groupMove(list, lookups), null);
  });

  it("keeps a later orchestration below an earlier one", () => {
    const list = [
      mainSpace,
      space("w3", "⇶ @monocircuit/breadboard"),
      space("w8", "⇶ vita"),
      space("w9", "250-canvas"),
      space("w6", "240-overview"),
      other,
    ];
    assert.deepEqual(order(list, groupMove(list, lookups)), [
      "Monocircuit/Platform",
      "⇶ @monocircuit/breadboard",
      "240-overview",
      "⇶ vita",
      "250-canvas",
      "Dash",
    ]);
  });

  it("puts the worktrees of an orchestration that ended back into the plain block", () => {
    const list = [
      mainSpace,
      space("w8", "⇶ vita"),
      space("w9", "250-canvas"),
      space("w6", "240-overview"),
    ];
    assert.deepEqual(order(list, groupMove(list, lookups)), [
      "Monocircuit/Platform",
      "240-overview",
      "⇶ vita",
      "250-canvas",
    ]);
  });

  it("leaves the spaces of another repo where they are", () => {
    const list = [
      mainSpace,
      space("w3", "⇶ @monocircuit/breadboard"),
      space("w10", "feature-x", "/other/.git"),
      space("w4", "link-only-dot"),
    ];
    assert.deepEqual(order(list, groupMove(list, lookups)), [
      "Monocircuit/Platform",
      "link-only-dot",
      "⇶ @monocircuit/breadboard",
      "feature-x",
    ]);
  });

  it("never puts the group in front of the main space", () => {
    const list = [
      space("w4", "240-overview"),
      space("w3", "⇶ @monocircuit/breadboard"),
      mainSpace,
      other,
    ];
    const move = groupMove(list, lookups);
    assert.equal(move.before_workspace_id, "w2");
  });

  it("does nothing without the main space", () => {
    assert.equal(groupMove([other, space("w3", "a")], lookups), null);
  });
});

describe("the work order of an orchestration", () => {
  const record = {
    issues: [
      { number: 240, feature: "240-overview" },
      { number: 249, feature: "249-agents" },
      { number: 251, feature: "251-stays" },
    ],
  };

  it("follows the plan, the rest in the order they were started", () => {
    assert.deepEqual(workOrderOf({ ...record, plan: { work: [249, 240] } }), [
      "249-agents",
      "240-overview",
      "251-stays",
    ]);
  });

  it("is the order they were started without a plan", () => {
    assert.deepEqual(workOrderOf(record), [
      "240-overview",
      "249-agents",
      "251-stays",
    ]);
  });

  it("is empty without a record", () => {
    assert.deepEqual(workOrderOf(null), []);
  });
});
