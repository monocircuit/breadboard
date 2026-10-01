import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  actionsFile,
  actionsOf,
  addAction,
  answerAction,
  answeredOf,
  takeAnswers,
} from "../worktree/actions.mjs";
import { orchestratorLabel } from "../worktree.mjs";
import { holds } from "../worktree/lock.mjs";
import {
  answerCell,
  buttonAt,
  clickAction,
  deliverAnswers,
  deliverTo,
  deliverToOrchestrator,
  deliversAnswers,
  inputIn,
  overviewLine,
  retryAnswers,
} from "./answers.mjs";
import { isOnce } from "./args.mjs";

// Without a terminal: buttons in brackets, the cell from column 3 (`│ `).
describe("answers", () => {
  const decide = { kind: "decide", options: ["A", "B"] };

  it("finds the button under a column, a column of slack after each", () => {
    // `[ A ] [ B ]` from column 3: A up to 8, B up to 14.
    assert.equal(buttonAt(decide, 1), 0);
    assert.equal(buttonAt(decide, 8), 0);
    assert.equal(buttonAt(decide, 9), 1);
    assert.equal(buttonAt(decide, 14), 1);
    assert.equal(buttonAt(decide, 15), -1);
  });

  it("counts the buttons from where their line starts", () => {
    // In NEXT the buttons start after ` NEXT  `, at column 8.
    assert.equal(buttonAt(decide, 8, 8), 0);
    assert.equal(buttonAt(decide, 14, 8), 1);
    assert.equal(buttonAt(decide, 20, 8), -1);
  });

  it("shows ✓ alone for an action without answers", () => {
    assert.equal(answerCell({ kind: "merge" }), "[ ✓ ]");
    assert.equal(answerCell(decide), "[ A ] [ B ]");
    assert.match(answerCell({ ...decide, answer: "B" }), /→ B/);
  });

  it("answers on a button, keeps the answer when it can't get in", () => {
    const file = actionsFile(
      fs.mkdtempSync(path.join(os.tmpdir(), "answers-")),
    );
    addAction(file, { kind: "decide", text: "A or B", options: ["A", "B"] });
    let isGone = false;
    const item = {
      action: actionsOf(file)[0],
      file,
      // No herdr space of that name: the answer waits for the Stop hook.
      to: { dir: path.join(os.tmpdir(), "no-such-worktree") },
      go: () => (isGone = true),
    };
    clickAction(item, 10, () => {});
    assert.equal(actionsOf(file)[0].answer, "B");
    clickAction(item, 40, () => {});
    assert.equal(isGone, true);
  });

  it("delivers nothing from a --once print: the answer stays pending", () => {
    // Under node --test stdout is a pipe: the overview runs as --once.
    assert.equal(isOnce, true);
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "answers-"));
    const file = actionsFile(dir);
    addAction(file, { kind: "decide", text: "A or B", options: ["A", "B"] });
    answerAction(file, 1, "B");
    const before = fs.readFileSync(file, "utf-8");
    assert.equal(deliversAnswers("answers-test"), false);
    retryAnswers("answers-test", [{ file, to: { dir } }]);
    assert.equal(fs.readFileSync(file, "utf-8"), before);
    assert.equal(fs.existsSync(`${file}.lock`), false);
  });

  it("lets one process at a time hold a pid file, a dead one's taken over", () => {
    const file = path.join(
      fs.mkdtempSync(path.join(os.tmpdir(), "answers-")),
      "overview.pid",
    );
    fs.writeFileSync(file, String(process.ppid));
    assert.equal(holds(file), false);
    // A pid no process has.
    fs.writeFileSync(file, "2147483646");
    assert.equal(holds(file), true);
    assert.equal(fs.readFileSync(file, "utf-8"), String(process.pid));
    assert.equal(holds(file), true);
  });
});

describe("overviewLine", () => {
  it("puts [overview] in front of a line that doesn't start with it", () => {
    assert.equal(
      overviewLine("!210 is merged: tick the boxes"),
      "[overview] !210 is merged: tick the boxes",
    );
    assert.equal(overviewLine("/clear"), "[overview] /clear");
    assert.equal(
      overviewLine('[overview] #1 decide "A or B" → B'),
      '[overview] #1 decide "A or B" → B',
    );
  });

  // A herdr on PATH that knows one space with one agent and writes down what
  // it is asked to type: the real one is never called.
  it("is how every line deliverTo and deliverToOrchestrator type starts", () => {
    const bin = fs.mkdtempSync(path.join(os.tmpdir(), "fake-herdr-"));
    const typed = path.join(bin, "typed");
    const dir = path.join(bin, "zz-fake");
    const script = `#!/usr/bin/env node
const fs = require("node:fs");
const [what, verb, ...rest] = process.argv.slice(2);
const labels = ["zz-fake", ${JSON.stringify(`⇶ zz-fake`)}];
const results = {
  workspace: { workspaces: labels.map(label => ({ label, workspace_id: label })) },
  tab: { tabs: ["zz-fake", "orchestrator"].map(label => ({ label, tab_id: label })) },
  pane: { panes: ["zz-fake", "orchestrator"].map(tab => ({ tab_id: tab, pane_id: tab, agent: "claude", agent_status: "idle" })) },
};
if (what === "agent" && verb === "prompt") fs.appendFileSync(${JSON.stringify(typed)}, rest[1] + "\\n");
console.log(JSON.stringify({ result: results[what] ?? null }));
`;
    fs.writeFileSync(path.join(bin, "herdr"), script, { mode: 0o755 });
    const before = process.env.PATH;
    process.env.PATH = `${bin}${path.delimiter}${before}`;
    try {
      assert.equal(deliverTo(dir, "!210 is merged: clean up"), true);
      assert.equal(deliverToOrchestrator("zz-fake", "/compact"), true);
    } finally {
      process.env.PATH = before;
    }
    const lines = fs.readFileSync(typed, "utf-8").trim().split("\n");
    assert.deepEqual(lines, [
      "[overview] !210 is merged: clean up",
      "[overview] /compact",
    ]);
  });
});

describe("inputIn", () => {
  const rule = "─".repeat(20);
  it("reads Claude Code's input line between its rules", () => {
    assert.equal(
      inputIn(`· Working…\n${rule}\n❯ \n${rule}\n  repo · branch`),
      "",
    );
    assert.equal(
      inputIn(`${rule}\n❯ the chip is grey\n  and too wide\n${rule}\n  repo`),
      "the chip is grey\n  and too wide",
    );
    assert.equal(inputIn("no prompt here"), null);
  });
});

// A herdr on PATH with one worktree space and the orchestrator's, one agent
// in each, whose state and screen the test sets: it writes down every call
// that acts. The real one is never called.
describe("broken and different", () => {
  const bin = fs.mkdtempSync(path.join(os.tmpdir(), "fake-herdr-"));
  const calls = path.join(bin, "calls");
  const state = path.join(bin, "state");
  const screen = path.join(bin, "screen");
  const dir = path.join(bin, "zz-draft");
  const script = `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
const [what, verb] = args;
const status = fs.readFileSync(${JSON.stringify(state)}, "utf-8");
const labels = ["zz-draft", ${JSON.stringify(orchestratorLabel("zz-draft"))}];
const tabs = ["zz-draft", "orchestrator"];
const results = {
  workspace: { workspaces: labels.map(label => ({ label, workspace_id: label })) },
  tab: { tabs: tabs.map(label => ({ label, tab_id: label })) },
  pane: { panes: tabs.map(tab => ({ tab_id: tab, pane_id: tab, workspace_id: "ws-" + tab, agent: "claude", agent_status: status })) },
};
if (what === "pane" && verb === "read") {
  process.stdout.write(fs.readFileSync(${JSON.stringify(screen)}, "utf-8"));
  process.exit(0);
}
if (verb !== "list") fs.appendFileSync(${JSON.stringify(calls)}, JSON.stringify(args) + "\\n");
console.log(JSON.stringify({ result: results[what] ?? null }));
`;
  fs.writeFileSync(path.join(bin, "herdr"), script, { mode: 0o755 });
  const rule = "─".repeat(20);

  /** A try in a fresh actions file, its agent `status`; the calls herdr got. */
  const setUp = (status, kind = "try") => {
    fs.writeFileSync(state, status);
    fs.writeFileSync(screen, `${rule}\n❯ \n${rule}\n`);
    fs.writeFileSync(calls, "");
    const file = actionsFile(fs.mkdtempSync(path.join(os.tmpdir(), "draft-")));
    addAction(file, { kind, text: "the chip" });
    return file;
  };
  const callsMade = () =>
    fs
      .readFileSync(calls, "utf-8")
      .trim()
      .split("\n")
      .filter(Boolean)
      .map(line => JSON.parse(line));
  /** `fn` with the fake herdr first on PATH. */
  const withHerdr = fn => {
    const before = process.env.PATH;
    process.env.PATH = `${bin}${path.delimiter}${before}`;
    try {
      return fn();
    } finally {
      process.env.PATH = before;
    }
  };
  // `[ looks good ] [ broken ]` from column 3: broken from column 20.
  const click = (file, to, column) =>
    withHerdr(() =>
      clickAction(
        { action: actionsOf(file)[0], file, to, go: () => {} },
        column,
        () => {},
      ),
    );

  it("focuses the agent's tab and writes the draft, without Enter, once", () => {
    const file = setUp("idle");
    click(file, { dir }, 20);
    assert.deepEqual(callsMade(), [
      ["workspace", "focus", "ws-zz-draft"],
      ["tab", "focus", "zz-draft"],
      [
        "pane",
        "send-text",
        "zz-draft",
        '[overview] #1 try "the chip" → broken: ',
      ],
    ]);
    // Delivered: off ACTIONS, nothing left for a retry or the Stop hook.
    assert.deepEqual(actionsOf(file), []);
    assert.equal(answeredOf(file)[0].answer, "broken");
    assert.deepEqual(takeAnswers(file), []);
  });

  it("writes it into a working agent's input all the same", () => {
    const file = setUp("working", "plan");
    // `[ ok ] [ different ]`: different from column 10.
    click(file, { orchestration: "zz-draft" }, 12);
    assert.deepEqual(callsMade().at(-1), [
      "pane",
      "send-text",
      "orchestrator",
      '[overview] #1 plan "the chip" → different: ',
    ]);
    assert.deepEqual(actionsOf(file), []);
  });

  it("submits every other answer as before", () => {
    const file = setUp("idle");
    click(file, { dir }, 5);
    assert.deepEqual(callsMade(), [
      [
        "agent",
        "prompt",
        "zz-draft",
        '[overview] #1 try "the chip" → looks good',
      ],
    ]);
  });

  it("writes nothing into a blocked agent's dialog, the draft once it is answered", () => {
    const file = setUp("blocked");
    click(file, { dir }, 20);
    // Only the focus: the row says why.
    assert.deepEqual(
      callsMade().map(call => call[0]),
      ["workspace", "tab"],
    );
    const [action] = actionsOf(file);
    assert.match(answerCell(action), /at a question/);
    assert.deepEqual(takeAnswers(file), []);
    fs.writeFileSync(state, "working");
    withHerdr(() => deliverAnswers(file, { dir }));
    assert.deepEqual(callsMade().at(-1), [
      "pane",
      "send-text",
      "zz-draft",
      '[overview] #1 try "the chip" → broken: ',
    ]);
    assert.deepEqual(actionsOf(file), []);
  });

  it("drops the waiting draft when the user typed meanwhile", () => {
    const file = setUp("blocked");
    click(file, { dir }, 20);
    fs.writeFileSync(state, "idle");
    fs.writeFileSync(screen, `${rule}\n❯ the chip is grey\n${rule}\n`);
    withHerdr(() => deliverAnswers(file, { dir }));
    assert.equal(
      callsMade().some(call => call[1] === "send-text"),
      false,
    );
    assert.deepEqual(actionsOf(file), []);
  });
});
