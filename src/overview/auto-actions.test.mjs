import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import {
  conflictsOf,
  autoActionsOf,
  failedPipelinesOf,
  newActionsOf,
  pendingOf,
  pipelineLineOf,
  questionOf,
  sentFile,
  sentOf,
  tellFailedPipelines,
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

  it("asks for the merge of a ready draft its agent could not mark ready", () => {
    const draft = () => ({
      mr: mr({ isDraft: true, mergeStatus: "draft_status" }),
    });
    const only = issue();
    const found = autoActionsOf([only], draft, new Map(), {
      readyDrafts: new Set([only.number]),
    });
    assert.deepEqual(
      found.map(action => [action.kind, action.text, action.canMarkReady]),
      [
        [
          "merge",
          "pipeline passed, every Work box ticked, still a draft",
          true,
        ],
      ],
    );
    // Not next in the merge order, or boxes open: none.
    assert.deepEqual(kinds([only], draft), []);
  });

  it("leaves the merge out while a requested change blocks it", () => {
    const changed = issue();
    Object.assign(changed.tree, {
      change: { text: "padding", head: "a" },
      pushed: "a",
      unpushed: 0,
    });
    const green = sha => () => ({
      mr: mr({ pipeline: { status: "success", sha } }),
    });
    assert.deepEqual(kinds([changed], green("a")), []);
    // Pushed after the request and green: the merge comes back.
    changed.tree.pushed = "b";
    assert.deepEqual(kinds([changed], green("b")), [
      ["merge", "ready, pipeline passed"],
    ]);
  });

  it("asks for a push once commits sit unpushed for long", () => {
    const old = new Date(Date.now() - 11 * 60_000).toISOString();
    const late = issue();
    Object.assign(late.tree, {
      unpushed: 2,
      unpushedAt: old,
      pushed: "a",
      hasRemoteBranch: true,
    });
    assert.deepEqual(
      kinds([late], () => ({})).map(([kind, text]) => [
        kind,
        text.replace(ESCAPES, ""),
      ]),
      [["push", "2 commits not pushed for 11 min: click to tell its agent"]],
    );
    late.tree.unpushedAt = new Date().toISOString();
    assert.deepEqual(
      kinds([late], () => ({})),
      [],
    );
  });

  it("names the failed job, and why its line didn't reach an agent yet", () => {
    const of = () => ({
      mr: mr({ pipeline: { status: "failed" } }),
      jobs: [
        { name: "lint", status: "success" },
        { name: "check:vita", status: "failed", url: "j" },
      ],
    });
    assert.deepEqual(kinds([issue({ tree: null })], of), [
      ["pipeline", "!70 job check:vita failed: no agent to tell"],
    ]);
    // Not tried yet (a --once print, a second overview): the click sends it.
    assert.deepEqual(kinds([issue()], of), [
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

describe("pipelineLineOf", () => {
  const failed = {
    iid: 70,
    url: "u",
    pipeline: { id: 41, status: "failed", web_url: "p" },
  };
  const jobs = [
    { name: "lint", status: "success", url: "j1" },
    {
      name: "check:vita",
      status: "failed",
      url: "j2",
      log: ["FAIL a.test.mjs", "1 failing"],
    },
  ];

  it("names the failed job with its url and the end of its log", () => {
    assert.equal(
      pipelineLineOf(failed, jobs),
      "[overview] the pipeline of !70 failed: job check:vita, j2: look into it; its log ends: FAIL a.test.mjs ⏎ 1 failing",
    );
  });

  it("says a job develop fails too is not the MR's doing", () => {
    const develop = { jobs: [{ name: "check:vita", status: "failed" }] };
    assert.equal(
      pipelineLineOf(failed, jobs, develop),
      "[overview] the pipeline of !70 failed: job check:vita, j2: develop fails the same job: not yours, waits for the fix (say so in the MR)",
    );
    // develop passed that job: it is the MR's.
    const passed = { jobs: [{ name: "check:vita", status: "success" }] };
    assert.match(pipelineLineOf(failed, jobs, passed), /: look into it;/);
  });

  it("names develop's jobs apart when the MR fails one of its own too", () => {
    const both = [
      ...jobs,
      { name: "scan:deps", status: "failed", url: "j3", log: [] },
    ];
    const develop = { jobs: [{ name: "scan:deps", status: "failed" }] };
    assert.match(
      pipelineLineOf(failed, both, develop),
      /look into it \(develop fails scan:deps too: not yours\); its log ends: FAIL a\.test\.mjs/,
    );
  });

  it("falls back to the pipeline's url without jobs", () => {
    assert.equal(
      pipelineLineOf(failed, []),
      "[overview] the pipeline of !70 failed: p: look into it",
    );
  });
});

describe("failedPipelinesOf", () => {
  const issue = {
    branch: "feature/1-a",
    tree: { dir: "/w/a", claude: { id: "c1" } },
  };
  const of = status => () => ({
    mr: {
      iid: 221,
      state: "opened",
      pipeline: { id: 9, status, web_url: "p" },
    },
    jobs: [],
  });
  const none = () => new Set();

  it("finds the failed pipeline of an open MR whose agent wasn't told", () => {
    assert.deepEqual(failedPipelinesOf([issue], of("failed"), none), [
      {
        dir: "/w/a",
        key: "pipeline:!221:9",
        id: "c1",
        line: "[overview] the pipeline of !221 failed: p: look into it",
      },
    ]);
  });

  it("leaves out a passed one, one told already, one without an agent", () => {
    assert.deepEqual(failedPipelinesOf([issue], of("success"), none), []);
    const told = () => new Set(["pipeline:!221:9"]);
    assert.deepEqual(failedPipelinesOf([issue], of("failed"), told), []);
    const alone = { ...issue, tree: { dir: "/w/a", claude: null } };
    assert.deepEqual(failedPipelinesOf([alone], of("failed"), none), []);
  });
});

describe("tellFailedPipelines", () => {
  // A herdr on PATH with one space whose agent has the state in `status`,
  // writing down what it is asked to type: the real one is never called.
  it("sends once per pipeline, and holds the line while the agent is blocked", () => {
    const bin = fs.mkdtempSync(path.join(os.tmpdir(), "fake-herdr-"));
    const typed = path.join(bin, "typed");
    const statusFile = path.join(bin, "status");
    const dir = path.join(bin, "zz-fake");
    fs.mkdirSync(dir);
    const script = `#!/usr/bin/env node
const fs = require("node:fs");
const [what, verb, ...rest] = process.argv.slice(2);
const status = fs.readFileSync(${JSON.stringify(statusFile)}, "utf-8").trim();
const results = {
  workspace: { workspaces: [{ label: "zz-fake", workspace_id: "w" }] },
  tab: { tabs: [{ label: "zz-fake", tab_id: "t" }] },
  pane: { panes: [{ tab_id: "t", pane_id: "p", agent: "claude", agent_status: status }] },
};
if (what === "agent" && verb === "prompt") fs.appendFileSync(${JSON.stringify(typed)}, rest[1] + "\\n");
console.log(JSON.stringify({ result: results[what] ?? null }));
`;
    fs.writeFileSync(path.join(bin, "herdr"), script, { mode: 0o755 });
    const issues = [
      {
        number: 7,
        feature: "zz-fake",
        branch: "feature/7-zz-fake",
        tree: { dir, actions: [], claude: { id: "c7", transcript: "/nope" } },
      },
    ];
    const of = () => ({
      mr: {
        iid: 70,
        state: "opened",
        url: "u",
        pipeline: { id: 41, status: "failed", web_url: "p" },
      },
      jobs: [{ name: "scan:deps", status: "failed", url: "j" }],
    });
    const develop = { jobs: [{ name: "scan:deps", status: "failed" }] };
    const agentsIn = status => new Map([["c7", { status }]]);
    const typedLines = () =>
      fs.existsSync(typed)
        ? fs.readFileSync(typed, "utf-8").trim().split("\n")
        : [];
    const rows = status =>
      autoActionsOf(issues, of, agentsIn(status), { develop })
        .filter(action => action.kind === "pipeline")
        .map(action => action.text);
    const before = process.env.PATH;
    process.env.PATH = `${bin}${path.delimiter}${before}`;
    try {
      fs.writeFileSync(statusFile, "blocked");
      tellFailedPipelines(issues, of, agentsIn("blocked"), develop);
      assert.deepEqual(typedLines(), []);
      assert.deepEqual(rows("blocked"), [
        "!70 job scan:deps failed: its agent is at a question, the line follows once it answers",
      ]);
      // Still blocked: not tried again.
      fs.writeFileSync(statusFile, "idle");
      tellFailedPipelines(issues, of, agentsIn("blocked"), develop);
      assert.deepEqual(typedLines(), []);
      // Its state changed: the line gets in, its row goes.
      tellFailedPipelines(issues, of, agentsIn("idle"), develop);
      assert.deepEqual(typedLines(), [
        "[overview] the pipeline of !70 failed: job scan:deps, j: develop fails the same job: not yours, waits for the fix (say so in the MR)",
      ]);
      assert.deepEqual(rows("idle"), []);
      assert.ok(sentOf(dir).has("pipeline:!70:41"));
      // Once per pipeline.
      tellFailedPipelines(issues, of, agentsIn("working"), develop);
      assert.equal(typedLines().length, 1);
    } finally {
      process.env.PATH = before;
    }
  });
});
