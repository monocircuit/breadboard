import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { guarded } from "./guard.mjs";
import { renderOrchestration } from "./render-orchestration.mjs";

describe("guarded", () => {
  it("keeps the last view with the error in its first line and tries again", () => {
    const log = path.join(
      fs.mkdtempSync(path.join(os.tmpdir(), "guard-")),
      "errors.log",
    );
    let isBroken = false;
    const render = guarded(() => {
      if (isBroken) throw new Error("boom");
      return "the title\nthe view";
    }, log);
    assert.equal(render(), "the title\nthe view");
    isBroken = true;
    const [line, ...rest] = render().replace(ESCAPES, "").split("\n");
    assert.match(line, /failed to draw: boom · trace in .* · trying again/);
    assert.deepEqual(rest, ["the view"]);
    assert.match(fs.readFileSync(log, "utf-8"), /Error: boom/);
    isBroken = false;
    assert.equal(render(), "the title\nthe view");
  });
});

describe("renderOrchestration", () => {
  const tree = (branch, name) => ({
    dir: `/tmp/no-such-worktree/${name}`,
    name,
    branch,
    ahead: 0,
    changed: 0,
    actions: [],
    claude: null,
    files: new Set(),
  });
  const mr = iid => ({
    iid,
    title: `MR ${iid}`,
    state: "opened",
    isDraft: true,
    url: `https://gitlab.example/mr/${iid}`,
    pipeline: { status: "running" },
    hasConflicts: false,
    mergeStatus: "draft_status",
  });

  it("draws an issue with a worktree and no MR, also beside one with the same number", () => {
    const issues = [
      // An MR, and a worktree of the same number the record doesn't know yet.
      { number: 213, feature: "213-a", branch: "feature/213-a", tree: null },
      {
        number: 213,
        feature: "213-b",
        branch: "feature/213-b",
        tree: tree("feature/213-b", "213-b"),
      },
      // A worktree, no MR yet.
      {
        number: 219,
        feature: "219-c",
        branch: "feature/219-c",
        tree: tree("feature/219-c", "219-c"),
      },
    ];
    const local = {
      issues,
      trees: issues.map(issue => issue.tree).filter(Boolean),
      plan: { work: [213, 219], merge: [219, 213] },
      ownActions: [],
      agents: new Map(),
    };
    const remote = { of: new Map([["feature/213-a", { mr: mr(220) }]]) };
    const text = renderOrchestration(local, remote, null).replace(ESCAPES, "");
    assert.match(text, /MERGE REQUESTS 1 open/);
    assert.match(text, /!220/);
  });

  it("heads its view with the title of the orchestration, else its name", () => {
    const local = { issues: [], trees: [], ownActions: [], agents: new Map() };
    const plain = view => view.replace(ESCAPES, "");
    assert.match(
      plain(
        renderOrchestration(
          { ...local, title: "@monocircuit/breadboard" },
          null,
          null,
        ),
      ),
      /⇶ @monocircuit\/breadboard/,
    );
    assert.doesNotMatch(
      plain(renderOrchestration(local, null, null)),
      /@monocircuit/,
    );
  });
});
