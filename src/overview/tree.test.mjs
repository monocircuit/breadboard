import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { withRemoved } from "../worktree/origins.mjs";
import { lifecycleOf, pushWorktrees, treeOf } from "./tree.mjs";

const plain = text => text.replace(ESCAPES, "");
const tree = (fields = {}) => ({
  dir: "/w",
  changed: 0,
  ahead: 0,
  ownCommits: false,
  actions: [],
  claude: { id: "c1" },
  ...fields,
});
const issue = (number, fields = {}) => ({
  number,
  feature: `${number}-feature`,
  branch: `feature/${number}-feature`,
  why: null,
  tree: tree(),
  ...fields,
});
const merged = { state: "merged", iid: 1, url: "u", mergedAt: null };
const stateOf = (entry, facts) => lifecycleOf(entry, facts).key;

describe("lifecycleOf", () => {
  it("tells a merged worktree the clean-up takes from one it keeps", () => {
    assert.equal(stateOf(issue(1), { mr: merged }), "removable");
    const kept = lifecycleOf(issue(1, { tree: tree({ changed: 2 }) }), {
      mr: merged,
    });
    assert.equal(kept.key, "kept");
    assert.match(plain(kept.text), /merged: kept, 2 changed/);
    assert.equal(
      stateOf(issue(1, { tree: tree({ ownCommits: true }) }), { mr: merged }),
      "kept",
    );
  });

  it("reads the agent's state and its open actions", () => {
    assert.equal(stateOf(issue(1), { status: "blocked" }), "asks");
    assert.equal(stateOf(issue(1), { status: "working" }), "working");
    const asked = issue(1, { tree: tree({ actions: [{ kind: "decide" }] }) });
    assert.equal(stateOf(asked, { status: "working" }), "waits");
    const answered = issue(1, {
      tree: tree({ actions: [{ kind: "decide", answer: "A" }] }),
    });
    assert.equal(stateOf(answered, { status: "working" }), "working");
  });

  it("is ready once the MR is ready and its pipeline passed", () => {
    const mr = {
      state: "opened",
      isDraft: false,
      pipeline: { status: "success" },
    };
    assert.equal(stateOf(issue(1), { mr, status: "idle" }), "ready");
    assert.equal(
      stateOf(issue(1), { mr: { ...mr, isDraft: true }, status: "idle" }),
      "idle",
    );
  });

  it("is removed without a worktree, skipped with a reason", () => {
    assert.equal(stateOf(issue(1, { tree: null })), "removed");
    const skipped = lifecycleOf(issue(1, { tree: null, why: "claimed" }));
    assert.equal(skipped.key, "skipped");
    assert.equal(plain(skipped.text), "claimed");
  });
});

describe("treeOf", () => {
  it("puts the gone ones under removed, the last removed first", () => {
    // Today's: one of another day shows its date instead of the time.
    const today = hour => {
      const at = new Date();
      at.setHours(hour, 0, 0, 0);
      return at.toISOString();
    };
    const { live, removed } = treeOf(
      [
        issue(1),
        issue(2, { tree: null, removedAt: today(10) }),
        issue(3, { tree: null, why: "failed at the start" }),
        issue(4, { tree: null, removedAt: today(12) }),
      ],
      () => ({}),
    );
    assert.deepEqual(
      live.map(row => row.issue.number),
      [1, 3],
    );
    assert.deepEqual(
      removed.map(row => row.issue.number),
      [4, 2],
    );
    assert.match(plain(removed[0].state.text), /^removed \d/);
  });

  it("shows the work order number in front of the feature", () => {
    const { live } = treeOf([issue(7)], () => ({}), new Map(), [3, 7]);
    assert.equal(plain(live[0].place + live[0].name), "2. 7-feature");
    const [, outside] = treeOf(
      [issue(3), issue(9)],
      () => ({}),
      new Map(),
      [3, 7],
    ).live;
    assert.equal(plain(outside.place + outside.name), "   9-feature");
  });
});

describe("pushWorktrees", () => {
  const draw = (issues, of = () => ({})) => {
    const lines = [];
    pushWorktrees(lines, {
      name: "orch",
      issues,
      of,
      agents: new Map(),
      work: [],
      hasRemote: true,
    });
    return lines
      .map(line => plain(line).replace(/^│ /, "").replace(/ │$/, ""))
      .join("\n");
  };
  const found = ({ number }) => ({
    issue: {
      state: "opened",
      url: "i",
      title: `the title of ${number}`,
    },
  });

  it("draws the orchestration at the root and a branch per worktree", () => {
    const text = draw([issue(1), issue(2)]);
    assert.match(text, /⇶ orch/);
    assert.match(text, /├─ 1-feature/);
    assert.match(text, /└─ 2-feature/);
  });

  it("gives a node its name and state, its facts and title below it", () => {
    const text = draw(
      [issue(1, { tree: tree({ ahead: 2, claude: null }) }), issue(2)],
      found,
    );
    const lines = text.split("\n");
    const at = lines.findIndex(line => /├─ 1-feature/.test(line));
    assert.match(lines[at], /├─ 1-feature +no agent\s*$/);
    assert.match(lines[at + 1], /^│ {2}#1 open · no MR yet · 2 ahead\s*$/);
    assert.match(lines[at + 2], /^│ {2}the title of 1\s*$/);
    // The last node: no `│` below it.
    assert.match(lines[at + 3], /└─ 2-feature +idle/);
    assert.match(lines[at + 4], /^ {3}#2 open · no MR yet · no answer yet/);
    assert.match(lines[at + 5], /^ {3}the title of 2/);
  });

  it("gives a removed node its times", () => {
    const gone = issue(2, {
      tree: null,
      removedAt: "2026-09-29T15:40:00",
      mergedAt: "2026-09-29T15:12:00",
    });
    const text = draw([issue(1), gone], ({ number }) =>
      number === 2
        ? {
            ...found({ number: 2 }),
            mr: { state: "merged", iid: 211, url: "m" },
          }
        : found({ number: 1 }),
    );
    assert.match(text, / {3}└─ 2-feature +removed .*3:40/);
    assert.match(text, /\n {6}#2 open · !211 merged .*3:12/);
  });

  it("folds a long done and removed into one line", () => {
    const gone = [2, 3, 4, 5].map(n => issue(n, { tree: null }));
    const text = draw([issue(1), ...gone]);
    assert.match(text, /├─ 1-feature/);
    assert.match(text, /└─ ▸ done and removed \(4\)/);
    assert.doesNotMatch(text, /2-feature/);
  });

  it("shows a short one open", () => {
    const text = draw([issue(1), issue(2, { tree: null })]);
    assert.match(text, /└─ ▾ done and removed \(1\)/);
    assert.match(text, / {3}└─ 2-feature/);
  });
});

describe("withRemoved", () => {
  const entry = {
    branch: "feature/5-x",
    feature: "5-x",
    removedAt: "2026-09-29T15:40:00Z",
    mergedAt: "2026-09-29T15:12:00Z",
  };

  it("notes the times on the issue's entry", () => {
    const record = {
      plan: { work: [5] },
      issues: [{ number: 5, branch: "feature/5-x", why: null }],
    };
    const next = withRemoved(record, entry);
    assert.deepEqual(next.plan, record.plan);
    assert.equal(next.issues.length, 1);
    assert.equal(next.issues[0].removedAt, entry.removedAt);
    assert.equal(next.issues[0].mergedAt, entry.mergedAt);
  });

  it("adds an entry for a worktree started by hand", () => {
    const next = withRemoved({ issues: [] }, entry);
    assert.equal(next.issues[0].number, 5);
    assert.equal(next.issues[0].feature, "5-x");
  });
});
