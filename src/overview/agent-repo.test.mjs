import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { SPACE } from "../worktree.mjs";
import { repoOfAgents } from "./agent-repo.mjs";

const REPO = "/code/platform";
const space = (id, label, root, isLinked = false) => ({
  workspace_id: id,
  label,
  worktree: root
    ? { repo_root: root, is_linked_worktree: isLinked }
    : undefined,
});
const workspaces = [
  space("w1", "~"),
  space("w2", SPACE, REPO),
  space("w3", "144-pill", REPO, true),
  space("w5", "⇶ batch", REPO, true),
  space("w7", "other", "/code/other"),
  space("w8", "other-feature", "/code/other", true),
];
const agent = (pane, workspace) => ({ pane_id: pane, workspace_id: workspace });
const reposOf = agents => Object.fromEntries(repoOfAgents(workspaces, agents));

describe("repoOfAgents", () => {
  it("names the repo of an agent in a worktree's space", () => {
    assert.deepEqual(reposOf([agent("b", "w3")]), { b: SPACE });
  });

  it("names the repo of the orchestrator in an orchestration's space", () => {
    assert.deepEqual(reposOf([agent("e", "w5")]), { e: SPACE });
  });

  it("leaves agents in the repo's own space alone: their space says it", () => {
    assert.deepEqual(reposOf([agent("a", "w2")]), {});
  });

  it("leaves agents outside the repo alone", () => {
    assert.deepEqual(
      reposOf([agent("x", "w1"), agent("y", "w7"), agent("z", "w8")]),
      {},
    );
  });
});
