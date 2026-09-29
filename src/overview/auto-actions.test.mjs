import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  conflictsOf,
  autoActionsOf,
  newActionsOf,
  pendingOf,
  questionOf,
  sentFile,
} from "./auto-actions.mjs";

const entry = (type, content, extra = {}) =>
  JSON.stringify({ type, message: { content }, ...extra });

describe("pendingOf", () => {
  it("reads the question of an AskUserQuestion that waits", () => {
    const lines = [
      entry("user", "go"),
      entry("assistant", [
        { type: "text", text: "Two ways:" },
        {
          type: "tool_use",
          name: "AskUserQuestion",
          input: { questions: [{ question: "Which one?\nmore" }] },
        },
      ]),
    ];
    assert.equal(pendingOf(lines), "Which one?");
  });

  it("names the tool of a permission prompt", () => {
    const lines = [
      entry("assistant", [
        {
          type: "tool_use",
          name: "Bash",
          input: { command: "git push", description: "Push the branch" },
        },
      ]),
    ];
    assert.equal(pendingOf(lines), "may it run Bash: Push the branch?");
  });

  it("is null once the call has its result, or the last answer is text", () => {
    const call = entry("assistant", [
      { type: "tool_use", name: "Bash", input: {} },
    ]);
    const result = entry("user", [{ type: "tool_result", content: "ok" }]);
    assert.equal(pendingOf([call, result]), null);
    assert.equal(
      pendingOf([entry("assistant", [{ type: "text", text: "done" }])]),
      null,
    );
  });

  it("skips subagents, harness lines and a line cut in the middle", () => {
    const call = entry("assistant", [
      { type: "tool_use", name: "Edit", input: { file_path: "a.mjs" } },
    ]);
    const lines = [
      call,
      entry("assistant", [{ type: "text", text: "x" }], { isSidechain: true }),
      JSON.stringify({ type: "system" }),
      '{"type":"assi',
    ];
    assert.equal(pendingOf(lines), "may it run Edit: a.mjs?");
  });
});

describe("questionOf", () => {
  it("finds the question an answer ends on", () => {
    assert.equal(
      questionOf("Did X.\n\nOption A or B?\nA is cheaper."),
      "Option A or B?",
    );
  });

  it("is null for an answer without one at its end", () => {
    assert.equal(questionOf("Why? Because.\n\nAll done."), null);
    assert.equal(questionOf(null), null);
  });
});

const issue = (extra = {}) => ({
  number: 7,
  feature: "7-thing",
  branch: "feature/7-thing",
  tree: {
    dir: "/w/7",
    actions: [],
    claude: {
      id: "c7",
      transcript: "/nope",
      answer: "Working on it.",
      at: null,
    },
  },
  ...extra,
});
const mr = (extra = {}) => ({
  iid: 70,
  state: "opened",
  isDraft: false,
  url: "u",
  pipeline: { status: "success", web_url: "p" },
  mergeStatus: "mergeable",
  ...extra,
});
const kinds = (issues, of, agents = new Map()) =>
  autoActionsOf(issues, of, agents).map(action => [action.kind, action.text]);

describe("autoActionsOf", () => {
  it("asks for the merge of a ready MR whose pipeline passed", () => {
    assert.deepEqual(
      kinds([issue()], () => ({ mr: mr() })),
      [["merge", "ready, pipeline passed"]],
    );
  });

  it("asks for no merge while GitLab says anything but mergeable", () => {
    for (const mergeStatus of ["conflict", "checking", "ci_must_pass"])
      assert.deepEqual(
        kinds([issue()], () => ({ mr: mr({ mergeStatus }) })),
        [],
      );
  });

  it("leaves the merge out for a draft, a running pipeline or one the agent posted", () => {
    assert.deepEqual(
      kinds([issue()], () => ({ mr: mr({ isDraft: true }) })),
      [],
    );
    assert.deepEqual(
      kinds([issue()], () => ({
        mr: mr({ pipeline: { status: "running" } }),
      })),
      [],
    );
    const posted = issue();
    posted.tree.actions = [{ kind: "merge" }];
    assert.deepEqual(
      kinds([posted], () => ({ mr: mr() })),
      [],
    );
  });

  it("names the failed job, with a click for the agent when there is one", () => {
    const of = () => ({
      mr: mr({ pipeline: { status: "failed" } }),
      jobs: [
        { name: "lint", status: "success" },
        { name: "check:vita", status: "failed", url: "j" },
      ],
    });
    assert.deepEqual(kinds([issue()], of), [
      ["pipeline", "!70 job check:vita failed"],
    ]);
    const agents = new Map([["c7", { pane: "p1", status: "working" }]]);
    assert.deepEqual(kinds([issue()], of, agents), [
      ["pipeline", "!70 job check:vita failed: click to tell its agent"],
    ]);
  });

  it("cleans up after the merge: the open boxes through the agent, the worktree goes by itself", () => {
    const of = () => ({
      mr: mr({ state: "merged" }),
      issue: { iid: 7, boxes: [{ isTicked: true }, { isTicked: false }] },
    });
    const agents = new Map([["c7", { pane: "p1", status: "working" }]]);
    const actions = autoActionsOf([issue()], of, agents);
    assert.deepEqual(
      actions.map(action => action.key),
      ["clean up:feature/7-thing:boxes"],
    );
    assert.match(actions[0].text, /1 open boxes/);
    // Its worktree gone, nothing to remove.
    assert.deepEqual(kinds([issue({ tree: null })], of), []);
  });

  it("hides a row whose line got in, until a new pipeline fails", () => {
    const sent = issue();
    sent.tree.dir = fs.mkdtempSync(path.join(os.tmpdir(), "sent-"));
    fs.mkdirSync(path.dirname(sentFile(sent.tree.dir)));
    fs.writeFileSync(
      sentFile(sent.tree.dir),
      JSON.stringify({ sent: ["pipeline:!70:41", "boxes:!70"] }),
    );
    const agents = new Map([["c7", { pane: "p1", status: "working" }]]);
    const failed = id => () => ({
      mr: mr({ pipeline: { id, status: "failed" } }),
    });
    assert.deepEqual(kinds([sent], failed(41), agents), []);
    assert.deepEqual(kinds([sent], failed(42), agents), [
      ["pipeline", "!70 failed: click to tell its agent"],
    ]);
    const merged = () => ({
      mr: mr({ state: "merged" }),
      issue: { iid: 7, boxes: [{ isTicked: false }] },
    });
    assert.deepEqual(
      autoActionsOf([sent], merged, agents).map(action => action.key),
      [],
    );
  });

  it("shows an agent that waits, and none that works", () => {
    const asking = issue();
    asking.tree.claude.answer = "Done with A.\n\nShall I do B too?";
    const idle = new Map([["c7", { pane: "p1", status: "idle" }]]);
    assert.deepEqual(
      kinds([asking], () => ({}), idle),
      [["waits", "Shall I do B too?"]],
    );
    const working = new Map([["c7", { pane: "p1", status: "working" }]]);
    assert.deepEqual(
      kinds([asking], () => ({}), working),
      [],
    );
    // Blocked without a transcript to read: the first line of its last answer.
    const blocked = new Map([["c7", { pane: "p1", status: "blocked" }]]);
    assert.deepEqual(
      kinds([asking], () => ({}), blocked),
      [["waits", "Done with A."]],
    );
  });
});

describe("newActionsOf", () => {
  const actions = [{ key: "a" }, { key: "b" }];
  it("finds nothing new at the start", () => {
    assert.deepEqual(newActionsOf(null, actions), []);
  });
  it("finds the actions whose key wasn't there", () => {
    assert.deepEqual(newActionsOf(new Set(["a"]), actions), [{ key: "b" }]);
  });
});

describe("conflictsOf", () => {
  const issue = { branch: "feature/1-a", tree: { dir: "/w/a" } };
  const of = hasConflicts => () => ({
    mr: { iid: 221, state: "opened", hasConflicts },
  });
  const none = () => new Set();
  const told = () => new Set(["conflicts:221"]);

  it("tells the agent of an MR that has conflicts, once", () => {
    assert.deepEqual(conflictsOf([issue], of(true), none), [
      {
        dir: "/w/a",
        key: "conflicts:221",
        line: "[overview] !221 has conflicts with develop: merge develop in (git fetch origin, git merge origin/develop)",
      },
    ]);
    assert.deepEqual(conflictsOf([issue], of(true), told), []);
  });

  it("forgets it once the conflicts are gone, so the next ones are told again", () => {
    assert.deepEqual(conflictsOf([issue], of(false), told), [
      { dir: "/w/a", key: "conflicts:221", isGone: true },
    ]);
    assert.deepEqual(conflictsOf([issue], of(false), none), []);
  });

  it("leaves out an MR without a worktree or not open", () => {
    assert.deepEqual(
      conflictsOf([{ ...issue, tree: null }], of(true), none),
      [],
    );
    assert.deepEqual(
      conflictsOf(
        [issue],
        () => ({ mr: { iid: 221, state: "merged", hasConflicts: true } }),
        none,
      ),
      [],
    );
  });
});
