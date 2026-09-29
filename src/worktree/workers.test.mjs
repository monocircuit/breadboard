import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { briefOf } from "./orchestrator.mjs";
import {
  beforeLine,
  findWorker,
  moveWorker,
  nextIssueLine,
  refusalOf,
} from "./workers.mjs";

describe("refusalOf", () => {
  const pane = status => ({ pane_id: "p1", agent_status: status });

  it("takes an idle or done agent with room in its context", () => {
    assert.equal(refusalOf({ pane: pane("idle"), share: 0.2 }), null);
    assert.equal(refusalOf({ pane: pane("done"), share: null }), null);
  });

  it("refuses one without a pane, at work, or waiting for an answer", () => {
    assert.match(refusalOf({ pane: null, share: 0.1 }), /no claude runs it/);
    assert.equal(
      refusalOf({ pane: pane("working"), share: 0.1 }),
      "it is working",
    );
    assert.equal(
      refusalOf({ pane: pane("blocked"), share: 0.1 }),
      "it waits for an answer",
    );
    assert.match(refusalOf({ pane: pane(undefined), share: 0.1 }), /unknown/);
  });

  it("refuses one above the context limit, 50 % unless told otherwise", () => {
    assert.match(
      refusalOf({ pane: pane("idle"), share: 0.62 }),
      /62 %, above the limit of 50 %/,
    );
    assert.equal(
      refusalOf({ pane: pane("idle"), share: 0.62, limit: 0.7 }),
      null,
    );
  });
});

describe("findWorker", () => {
  const workers = [
    { name: "187-overview-button", feature: "217-agents-workers" },
    { name: "183-feed", feature: "183-feed" },
  ];

  it("finds an agent by the name it was started with, else by its worktree", () => {
    assert.equal(findWorker(workers, "187-overview-button"), workers[0]);
    assert.equal(findWorker(workers, "217-agents-workers"), workers[0]);
    assert.equal(findWorker(workers, "183-feed"), workers[1]);
    assert.equal(findWorker(workers, "999-none"), undefined);
  });
});

describe("the brief of a reused agent", () => {
  const item = {
    number: "217",
    title: "agents are workers",
    dir: "/repo/platform-217-agents-workers",
    branch: "feature/217-agents-workers",
    name: "overview-actions",
  };

  it("names its issues before in one line", () => {
    assert.equal(beforeLine([]), null);
    assert.match(
      beforeLine([{ number: 187, title: "ending an orchestration" }]),
      /^You are a reused agent: you worked on #187 \(ending an orchestration\) before\./,
    );
  });

  it("tells it to enter the worktree once, a fresh one to stay", () => {
    const reused = briefOf({ ...item, before: "You are a reused agent." });
    assert.match(
      reused,
      /`EnterWorktree\(path: "\/repo\/platform-217-agents-workers"\)` once/,
    );
    assert.match(reused, /## Before this issue\n\nYou are a reused agent\./);
    const fresh = briefOf(item);
    assert.match(fresh, /you already stand in it/);
    assert.doesNotMatch(fresh, /Before this issue/);
  });

  it("says how the next issue comes, in the line the handover types", () => {
    assert.equal(
      nextIssueLine("218", "/repo/platform-218-x"),
      '[overview] next issue #218: EnterWorktree(path: "/repo/platform-218-x"), then read .claude/brief.md',
    );
    assert.match(briefOf(item), /\[overview\] next issue #<n>: EnterWorktree/);
  });
});

describe("moveWorker", () => {
  /**
   * A herdr on PATH like the real one after a move (#222): the pane of the
   * agent, `wOld:p1` and `done`, gets the new id `wNew:p1` in the space of
   * the new worktree, and a prompt to the old id is refused. It writes down
   * what it is asked to type; the real one is never called. `hasNewId`: the
   * move answers with the pane's new id.
   */
  const withFakeHerdr = (hasNewId, test) => {
    const bin = fs.mkdtempSync(path.join(os.tmpdir(), "fake-herdr-"));
    const typed = path.join(bin, "typed");
    const script = `#!/usr/bin/env node
const fs = require("node:fs");
const [what, verb, ...rest] = process.argv.slice(2);
const moved = { pane_id: "wNew:p1", tab_id: "wNew:t9", agent: "claude", agent_status: "done", agent_session: { value: "conv-1" } };
const results = {
  "workspace list": { workspaces: [{ label: "222-next", workspace_id: "wNew" }] },
  "tab list": { tabs: [{ label: "222-next", tab_id: "wNew:t9" }] },
  "pane list": { panes: [moved] },
  "pane move": { move_result: { previous_pane_id: "wOld:p1", created_tab: { tab_id: "wNew:t9" }, ...(${hasNewId} ? { pane: moved } : {}) } },
};
if (what === "agent" && verb === "prompt") {
  if (rest[0] !== "wNew:p1") process.exit(1);
  fs.appendFileSync(${JSON.stringify(typed)}, rest[1] + "\\n");
}
console.log(JSON.stringify({ result: results[\`\${what} \${verb}\`] ?? null }));
`;
    fs.writeFileSync(path.join(bin, "herdr"), script, { mode: 0o755 });
    const [from, to] = ["221-before", "222-next"].map(feature => {
      const dir = path.join(bin, feature);
      fs.mkdirSync(path.join(dir, ".claude"), { recursive: true });
      return dir;
    });
    fs.writeFileSync(path.join(from, ".claude", "herdr-session"), "conv-1\n");
    const before = process.env.PATH;
    process.env.PATH = `${bin}${path.delimiter}${before}`;
    try {
      const worker = { dir: from, id: "conv-1", name: "221-before" };
      const pane = { pane_id: "wOld:p1", agent_status: "done" };
      test(moveWorker(worker, pane, 222, to), { from, to, typed });
    } finally {
      process.env.PATH = before;
    }
  };

  it("tells an idle agent in its moved pane, its binding moved along", () => {
    withFakeHerdr(true, (result, { from, to, typed }) => {
      assert.deepEqual(result, { isMoved: true, isTold: true });
      assert.equal(
        fs.readFileSync(typed, "utf-8"),
        `${nextIssueLine(222, to)}\n`,
      );
      assert.equal(
        fs.readFileSync(path.join(to, ".claude", "herdr-session"), "utf-8"),
        "conv-1\n",
      );
      assert.equal(
        fs.existsSync(path.join(from, ".claude", "herdr-session")),
        false,
      );
    });
  });

  it("finds the pane by its conversation when the move doesn't name it", () => {
    withFakeHerdr(false, (result, { to, typed }) => {
      assert.deepEqual(result, { isMoved: true, isTold: true });
      assert.equal(
        fs.readFileSync(typed, "utf-8"),
        `${nextIssueLine(222, to)}\n`,
      );
    });
  });
});
