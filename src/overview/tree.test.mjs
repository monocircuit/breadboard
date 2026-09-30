import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { withRemoved } from "../worktree/origins.mjs";
import { numbersOf } from "./numbering.mjs";
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
    assert.match(plain(kept.text), /^kept: 2 changed$/);
    assert.equal(
      stateOf(issue(1, { tree: tree({ ownCommits: true }) }), { mr: merged }),
      "kept",
    );
  });

  it("names what stops the clean-up of a merged worktree", () => {
    const text = (entry, facts) => plain(lifecycleOf(entry, facts).text);
    assert.equal(
      text(issue(1, { tree: tree({ ownCommits: 1 }) }), { mr: merged }),
      "kept: 1 commit not in develop",
    );
    assert.equal(
      text(issue(1), { mr: merged, status: "working" }),
      "waits: its agent is working",
    );
    assert.equal(
      text(issue(1), {
        mr: merged,
        status: "idle",
        hold: "its agent takes #226 after !232",
      }),
      "waits: its agent takes #226 after !232",
    );
    assert.equal(
      text(issue(1), { mr: merged, removal: { isRunning: true } }),
      "merged: removing …",
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

  it("numbers every node: the work order first, then the rest", () => {
    const { live } = treeOf([issue(7), issue(3), issue(9)], () => ({}), {
      work: [3, 7],
    });
    assert.deepEqual(
      live.map(row => plain(row.place + row.name)),
      ["1. 3-feature", "2. 7-feature", "3. 9-feature"],
    );
  });
});

describe("numbersOf", () => {
  const numbered = (issues, options) =>
    [...numbersOf(issues, options)]
      .toSorted(([, a], [, b]) => a - b)
      .map(([entry]) => entry.number);
  const mrOf = mrs => entry => ({ mr: mrs[entry.number] });

  it("numbers every issue without a plan: by number", () => {
    assert.deepEqual(numbered([issue(9), issue(3), issue(5)], {}), [3, 5, 9]);
  });

  it("goes by the work order, then the merge order, then the merged by time", () => {
    const of = mrOf({
      2: { state: "merged", mergedAt: "2026-09-29T12:00:00Z" },
      4: { state: "opened" },
      6: { state: "opened" },
    });
    const gone = issue(1, {
      tree: null,
      removedAt: "2026-09-29T11:00:00Z",
      mergedAt: "2026-09-29T10:00:00Z",
    });
    assert.deepEqual(
      numbered([gone, issue(2), issue(4), issue(6), issue(8), issue(5)], {
        work: [5],
        merge: [6, 4],
        of,
      }),
      [5, 6, 4, 8, 1, 2],
    );
  });

  it("gives two entries of one issue number a number each", () => {
    const numbers = numbersOf([issue(3), issue(3)], {});
    assert.deepEqual([...numbers.values()], [1, 2]);
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
    assert.match(text, /├─ 1\. 1-feature/);
    assert.match(text, /└─ 2\. 2-feature/);
  });

  it("gives a node its name and state, its facts and title below it", () => {
    const text = draw(
      [issue(1, { tree: tree({ ahead: 2, claude: null }) }), issue(2)],
      found,
    );
    const lines = text.split("\n");
    const at = lines.findIndex(line => /├─ 1\. 1-feature/.test(line));
    assert.match(lines[at], /├─ 1\. 1-feature +no agent\s*$/);
    assert.match(lines[at + 1], /^│ {5}#1 open · no MR yet · 2 ahead\s*$/);
    assert.match(lines[at + 2], /^│ {5}the title of 1\s*$/);
    // The last node: no `│` below it.
    assert.match(lines[at + 3], /└─ 2\. 2-feature +idle/);
    assert.match(lines[at + 4], /^ {6}#2 open · no MR yet · no answer yet/);
    assert.match(lines[at + 5], /^ {6}the title of 2/);
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
    assert.match(text, / {3}└─ 2\. 2-feature +merged: removed .*3:40/);
    assert.match(text, /\n {9}#2 open · !211 merged .*3:12/);
  });

  it("folds a long done and removed into one line", () => {
    const gone = [2, 3, 4, 5].map(n => issue(n, { tree: null }));
    const text = draw([issue(1), ...gone]);
    assert.match(text, /├─ 1\. 1-feature/);
    assert.match(text, /└─ ▸ done and removed \(4\)/);
    assert.doesNotMatch(text, /2-feature/);
  });

  it("shows a short one open", () => {
    const text = draw([issue(1), issue(2, { tree: null })]);
    assert.match(text, /└─ ▾ done and removed \(1\)/);
    assert.match(text, / {3}└─ 2\. 2-feature/);
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
