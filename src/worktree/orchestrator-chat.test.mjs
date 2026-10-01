import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  chatOf,
  chatOfPane,
  chatPaneOf,
  chatStateOf,
  hasChat,
  restoreChat,
  stayReason,
} from "./orchestrator-chat.mjs";
import { orchestratorLabel } from "./origins.mjs";

const ID = "00000000-0000-4000-8000-000000000287";
const OTHER = "00000000-0000-4000-8000-000000000288";

describe("stayReason (the move rule)", () => {
  it("moves a session that is nobody's into a space without an orchestrator", () => {
    assert.equal(
      stayReason({ name: "a", here: null, isWorktree: false, hasLive: false }),
      null,
    );
  });

  it("leaves the orchestrator of another orchestration where it is", () => {
    assert.equal(
      stayReason({ name: "a", here: "b", isWorktree: false, hasLive: false }),
      "it is the orchestrator of b",
    );
  });

  it("leaves a worktree's agent where it is", () => {
    assert.match(
      stayReason({ name: "a", here: null, isWorktree: true, hasLive: false }),
      /worktree/,
    );
  });

  it("never makes a second orchestrator tab, even from the main space", () => {
    assert.match(
      stayReason({ name: "a", here: null, isWorktree: false, hasLive: true }),
      /has its orchestrator/,
    );
  });

  it("has nothing to move for the orchestrator in its own space", () => {
    assert.equal(
      stayReason({ name: "a", here: "a", isWorktree: false, hasLive: true }),
      null,
    );
  });
});

describe("hasChat (one orchestrator tab per space)", () => {
  const tabs = [
    { tab_id: "t1", label: "root" },
    { tab_id: "t2", label: "orchestrator" },
  ];
  it("counts a claude in the tab orchestrator", () => {
    assert.equal(
      hasChat({
        tabs,
        panes: [{ tab_id: "t2", pane_id: "p2", agent: "claude" }],
        isIdle: () => false,
      }),
      true,
    );
  });

  it("counts anything running in it: a script never replaces it", () => {
    assert.equal(
      hasChat({
        tabs,
        panes: [{ tab_id: "t2", pane_id: "p2", agent: null }],
        isIdle: () => false,
      }),
      true,
    );
  });

  it("does not count a spare shell, whose claude died", () => {
    assert.equal(
      hasChat({
        tabs,
        panes: [{ tab_id: "t2", pane_id: "p2", agent: null }],
        isIdle: () => true,
      }),
      false,
    );
  });

  it("counts the one claude of a space without the tab", () => {
    assert.equal(
      hasChat({
        tabs: [{ tab_id: "t3", label: "5" }],
        panes: [
          {
            tab_id: "t3",
            pane_id: "p3",
            agent: "claude",
            agent_session: { value: ID },
          },
        ],
      }),
      true,
    );
  });

  it("does not count a space without the tab", () => {
    assert.equal(
      hasChat({ tabs: tabs.slice(0, 1), panes: [], isIdle: () => true }),
      false,
    );
  });
});

describe("chatStateOf", () => {
  const tabs = [
    { tab_id: "t1", label: "overview" },
    { tab_id: "t2", label: "orchestrator" },
  ];
  it("finds the recorded conversation wherever it runs", () => {
    const running = {
      tab_id: "x5",
      pane_id: "px",
      agent_session: { value: ID },
    };
    assert.deepEqual(
      chatStateOf({ tabs, panes: [], all: [running], sessionId: ID }),
      { state: "alive", pane: running, tab: null },
    );
  });

  it("takes a fresh claude in the tab as alive", () => {
    const pane = { tab_id: "t2", pane_id: "p2", agent: "claude" };
    assert.equal(
      chatStateOf({ tabs, panes: [pane], sessionId: ID }).state,
      "alive",
    );
  });

  it("is idle once the claude died and its shell is left", () => {
    const pane = { tab_id: "t2", pane_id: "p2", agent: null };
    assert.equal(
      chatStateOf({ tabs, panes: [pane], isIdle: () => true }).state,
      "idle",
    );
  });

  it("is busy when something else runs there, missing without the tab", () => {
    const pane = { tab_id: "t2", pane_id: "p2", agent: null };
    assert.equal(
      chatStateOf({ tabs, panes: [pane], isIdle: () => false }).state,
      "busy",
    );
    assert.equal(
      chatStateOf({ tabs: tabs.slice(0, 1), panes: [] }).state,
      "missing",
    );
  });
});

describe("chatPaneOf and chatOfPane (what is recorded)", () => {
  const claude = (tab, value) => ({
    tab_id: tab,
    pane_id: `p-${tab}`,
    agent: "claude",
    agent_session: { value },
    cwd: "/repo",
  });
  it("takes the claude of the tab orchestrator", () => {
    assert.equal(
      chatPaneOf({
        tabs: [
          { tab_id: "t2", label: "orchestrator" },
          { tab_id: "t3", label: "5" },
        ],
        panes: [claude("t3", OTHER), claude("t2", ID)],
      }).agent_session.value,
      ID,
    );
  });

  it("takes the one claude of a space without the tab (a fresh chat in a tab `5`)", () => {
    assert.equal(
      chatPaneOf({
        tabs: [{ tab_id: "t3", label: "5" }],
        panes: [claude("t3", OTHER)],
      }).agent_session.value,
      OTHER,
    );
    assert.equal(
      chatPaneOf({
        tabs: [],
        panes: [claude("t3", OTHER), claude("t4", ID)],
      }),
      null,
    );
  });

  it("records the claude's own cwd, else herdr's", () => {
    assert.deepEqual(chatOfPane(claude("t2", ID), [{ id: ID, cwd: "/wt" }]), {
      sessionId: ID,
      cwd: "/wt",
    });
    assert.deepEqual(chatOfPane(claude("t2", ID), []), {
      sessionId: ID,
      cwd: "/repo",
    });
    assert.equal(chatOfPane({ agent: null }, []), null);
  });

  it("reads only a well-formed record", () => {
    assert.deepEqual(
      chatOf("x", { orchestrator: { sessionId: ID, cwd: "/r" } }),
      {
        sessionId: ID,
        cwd: "/r",
      },
    );
    assert.equal(chatOf("x", { orchestrator: { sessionId: "nope" } }), null);
    assert.equal(chatOf("x", { issues: [] }), null);
  });
});

// A herdr on PATH holding one space `⇶ zz-chat` in a state file: it answers
// the lists from it, adds what `tab create` makes and writes down every call
// that acts. The real one is never called.
describe("restoreChat (fake herdr)", () => {
  const bin = fs.mkdtempSync(path.join(os.tmpdir(), "fake-herdr-"));
  const stateFile = path.join(bin, "state.json");
  const calls = path.join(bin, "calls");
  const script = `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
const [what, verb] = args;
const state = JSON.parse(fs.readFileSync(${JSON.stringify(stateFile)}, "utf-8"));
const flag = name => args[args.indexOf(name) + 1];
const answer = result => console.log(JSON.stringify({ result }));
if (verb !== "list" && verb !== "process-info")
  fs.appendFileSync(${JSON.stringify(calls)}, JSON.stringify(args) + "\\n");
if (what === "workspace" && verb === "list") answer({ workspaces: state.spaces });
else if (what === "tab" && verb === "list")
  answer({ tabs: state.tabs.filter(t => t.workspace_id === flag("--workspace")) });
else if (what === "pane" && verb === "list")
  answer({ panes: args.includes("--workspace") ? state.panes.filter(p => p.workspace_id === flag("--workspace")) : state.panes });
else if (what === "pane" && verb === "process-info") {
  const pane = state.panes.find(p => p.pane_id === flag("--pane"));
  answer({ process_info: { shell_pid: 7, foreground_process_group_id: pane?.idle ? 7 : 8 } });
} else if (what === "tab" && verb === "create") {
  const n = state.tabs.length + 1;
  const tab = { tab_id: "w1:t" + n, label: flag("--label"), workspace_id: flag("--workspace"), pane_count: 1 };
  const pane = { pane_id: "w1:p" + n, tab_id: tab.tab_id, workspace_id: tab.workspace_id, idle: true };
  state.tabs.push(tab);
  state.panes.push(pane);
  fs.writeFileSync(${JSON.stringify(stateFile)}, JSON.stringify(state));
  answer({ tab, root_pane: pane });
} else answer(null);
`;
  fs.writeFileSync(path.join(bin, "herdr"), script, { mode: 0o755 });
  const label = orchestratorLabel("zz-chat");
  const chat = { sessionId: ID, cwd: bin };
  const tab = (id, name) => ({
    tab_id: id,
    label: name,
    workspace_id: "w1",
    pane_count: 1,
  });
  /** The space with `tabs` and `panes` (`idle`: its shell idles), no calls yet. */
  const setUp = (tabs, panes) => {
    fs.writeFileSync(
      stateFile,
      JSON.stringify({
        spaces: [{ workspace_id: "w1", label }],
        tabs,
        panes: panes.map(pane => ({ workspace_id: "w1", ...pane })),
      }),
    );
    fs.writeFileSync(calls, "");
  };
  const callsMade = () =>
    fs
      .readFileSync(calls, "utf-8")
      .trim()
      .split("\n")
      .filter(Boolean)
      .map(line => JSON.parse(line));
  const restore = options => {
    const before = { path: process.env.PATH, env: process.env.HERDR_ENV };
    process.env.PATH = `${bin}${path.delimiter}${before.path}`;
    process.env.HERDR_ENV = "1";
    try {
      return restoreChat("zz-chat", { chat, ...options });
    } finally {
      process.env.PATH = before.path;
      if (before.env === undefined) delete process.env.HERDR_ENV;
      else process.env.HERDR_ENV = before.env;
    }
  };
  const base = [tab("w1:t1", "root"), tab("w1:t2", "overview")];

  it("only focuses a live orchestrator: no second tab", () => {
    setUp(
      [...base, tab("w1:t3", "orchestrator")],
      [
        {
          pane_id: "w1:p3",
          tab_id: "w1:t3",
          agent: "claude",
          agent_session: { value: ID },
        },
      ],
    );
    assert.equal(restore({ isClick: true }), "focused the orchestrator");
    assert.deepEqual(callsMade(), [
      ["workspace", "focus", "w1"],
      ["tab", "focus", "w1:t3"],
    ]);
  });

  it("resumes the same conversation in the tab whose claude died", () => {
    setUp(
      [...base, tab("w1:t3", "orchestrator")],
      [{ pane_id: "w1:p3", tab_id: "w1:t3", agent: null, idle: true }],
    );
    assert.match(restore({ isClick: true }), /resumed .* in its tab/);
    const made = callsMade();
    assert.deepEqual(made[0].slice(0, 3), ["pane", "run", "w1:p3"]);
    assert.match(made[0][3], new RegExp(`claude --resume ${ID}`));
    assert.deepEqual(made.slice(1), [
      ["workspace", "focus", "w1"],
      ["tab", "focus", "w1:t3"],
    ]);
    assert.equal(made.filter(call => call[1] === "create").length, 0);
  });

  it("makes the missing tab, running the conversation, and focuses it", () => {
    setUp(base, []);
    assert.match(restore({ isClick: true }), /in a new tab orchestrator/);
    const made = callsMade();
    assert.deepEqual(
      made
        .filter(call => call[1] === "create")
        .map(call => call[call.indexOf("--label") + 1]),
      ["orchestrator"],
    );
    const run = made.find(call => call[1] === "run");
    assert.match(run[3], new RegExp(`claude --resume ${ID}`));
    assert.deepEqual(made.at(-1), ["tab", "focus", "w1:t3"]);
  });

  it("focuses the recorded conversation where it runs instead of resuming it twice", () => {
    setUp(
      [...base, tab("w1:t3", "5")],
      [
        {
          pane_id: "w1:p3",
          tab_id: "w1:t3",
          agent: "claude",
          agent_session: { value: ID },
        },
      ],
    );
    assert.equal(restore({ isClick: true }), "focused the orchestrator");
    assert.deepEqual(callsMade(), [
      ["workspace", "focus", "w1"],
      ["tab", "focus", "w1:t3"],
    ]);
  });

  it("takes a lone claude in a tab of its own as the orchestrator: renamed, recorded, focused", () => {
    setUp(
      [...base, tab("w1:t3", "5")],
      [
        {
          pane_id: "w1:p3",
          tab_id: "w1:t3",
          agent: "claude",
          agent_session: { value: OTHER },
          cwd: "/repo",
        },
      ],
    );
    const remembered = [];
    assert.equal(
      restore({
        isClick: true,
        remember: (name, found) => remembered.push([name, found.sessionId]),
      }),
      "focused the orchestrator",
    );
    assert.deepEqual(remembered, [["zz-chat", OTHER]]);
    assert.deepEqual(callsMade(), [
      ["tab", "rename", "w1:t3", "orchestrator"],
      ["workspace", "focus", "w1"],
      ["tab", "focus", "w1:t3"],
    ]);
  });

  it("after a herdr restart resumes it in its idle tab, unfocused", () => {
    setUp(
      [...base, tab("w1:t3", "orchestrator")],
      [{ pane_id: "w1:p3", tab_id: "w1:t3", agent: null, idle: true }],
    );
    assert.match(restore(), /resumed/);
    const made = callsMade();
    assert.equal(made.length, 1);
    assert.deepEqual(made[0].slice(0, 3), ["pane", "run", "w1:p3"]);
  });

  it("after a restart makes no tab and starts nothing without a recorded one", () => {
    setUp(base, []);
    assert.equal(restore(), null);
    setUp(
      [...base, tab("w1:t3", "orchestrator")],
      [{ pane_id: "w1:p3", tab_id: "w1:t3", agent: null, idle: true }],
    );
    assert.equal(restore({ chat: null }), null);
    assert.deepEqual(callsMade(), []);
  });
});
