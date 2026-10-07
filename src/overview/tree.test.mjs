import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { withRemoved } from "../worktree/origins.mjs";
import { numbersOf } from "./numbering.mjs";
import { MEMORY, THRESHOLD } from "../worktree/memory.mjs";
import {
  foldLine,
  headOf,
  lifecycleOf,
  nodeLines,
  pushWorktrees,
  toggleQueue,
  treeOf,
} from "./tree.mjs";

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
  it("waits for your merge only once no try or acceptance box is open (#280)", () => {
    const green = {
      iid: 9,
      state: "opened",
      isDraft: false,
      pipeline: { status: "success" },
    };
    assert.equal(
      plain(lifecycleOf(issue(1), { mr: green, status: "idle" }).text),
      "waits for you: merge",
    );
    assert.equal(
      plain(
        lifecycleOf(issue(1), {
          mr: green,
          status: "idle",
          checks: ["try #1 broken"],
        }).text,
      ),
      "checks open: try #1 broken",
    );
  });

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

  it("says what the agent waits for, in the words of agentStateOf (#288)", () => {
    const text = (entry, facts) => plain(lifecycleOf(entry, facts).text);
    assert.equal(text(issue(1), { status: "blocked" }), "asks you");
    assert.equal(text(issue(1), { status: "working" }), "at work");
    const asked = issue(1, {
      tree: tree({
        actions: [{ kind: "try", id: 4 }, { kind: "merge" }],
        wait: { mrs: [271] },
      }),
    });
    assert.equal(
      text(asked, { status: "idle" }),
      "waits for you: try #4, merge",
    );
    const waiting = issue(1, { tree: tree({ wait: { mrs: [271] } }) });
    assert.equal(stateOf(waiting, { status: "idle" }), "waitsForMr");
    assert.equal(text(waiting, { status: "idle" }), "waits for !271");
    assert.equal(text(issue(1), { status: "idle" }), "free");
    assert.equal(
      text(issue(1, { tree: tree({ claude: null }) }), {}),
      "no agent",
    );
  });

  it("waits for the user's merge once the pipeline of its draft passed, not idle", () => {
    const mr = {
      state: "opened",
      isDraft: true,
      pipeline: { status: "success" },
    };
    const state = lifecycleOf(issue(1), { mr, status: "idle" });
    assert.equal(plain(state.text), "waits for you: merge");
    assert.equal(state.tone, "you");
  });

  it("is removed without a worktree, skipped with a reason", () => {
    assert.equal(stateOf(issue(1, { tree: null })), "removed");
    const skipped = lifecycleOf(issue(1, { tree: null, why: "claimed" }));
    assert.equal(skipped.key, "skipped");
    assert.equal(plain(skipped.text), "claimed");
  });
});

describe("treeOf", () => {
  it("shows a requested change and what isn't pushed in the facts", () => {
    const { live } = treeOf(
      [
        issue(1, {
          tree: tree({
            change: { text: "less padding", head: "a" },
            pushed: null,
            unpushed: 2,
            hasRemoteBranch: false,
          }),
        }),
      ],
      () => ({ mr: { state: "opened", iid: 5, url: "u", isDraft: true } }),
    );
    assert.match(plain(live[0].facts), /change requested: less padding/);
    assert.match(
      plain(live[0].facts),
      /2 commits not pushed \(no remote branch\)/,
    );
  });

  it("doesn't wait for a merge while a requested change blocks the MR", () => {
    const green = {
      state: "opened",
      isDraft: false,
      pipeline: { status: "success", sha: "a" },
    };
    const changed = issue(1, {
      tree: tree({ change: { text: "x", head: "a" }, pushed: "a" }),
    });
    assert.equal(stateOf(changed, { mr: green }), "change");
    assert.equal(stateOf(issue(1), { mr: green }), "waits");
  });

  it("names the wait once: in the state, else in the facts", () => {
    const waiting = issue(1, { tree: tree({ wait: { mrs: [271] } }) });
    const factsAt = status =>
      plain(
        treeOf([waiting], () => ({}), {
          agents: new Map([["c1", { status }]]),
        }).live[0].facts,
      );
    assert.doesNotMatch(factsAt("idle"), /waits for !271/);
    assert.match(factsAt("blocked"), /waits for !271/);
  });

  it("names the agent when it isn't the worktree's, its old name for a day (#260)", () => {
    const renamed = "2026-09-30T10:00:00.000Z";
    const entry = issue(1, { tree: tree({ name: "1-feature" }) });
    const factsOf = (known, now = Date.parse(renamed) + 60_000) =>
      plain(treeOf([entry], () => ({}), { known, now }).live[0].facts);
    assert.doesNotMatch(factsOf({ c1: { name: "1-feature" } }), /agent \S/);
    assert.doesNotMatch(factsOf({}), /agent /);
    const known = { c1: { name: "2-other", was: "1-feature", renamed } };
    assert.match(factsOf(known), /agent 2-other was 1-feature/);
    const later = factsOf(known, Date.parse(renamed) + 2 * 86_400_000);
    assert.match(later, /agent 2-other/);
    assert.doesNotMatch(later, /was 1-feature/);
  });

  it("names its agent on the line under its name, with its issues and merged MRs (#292)", () => {
    const agentsOf = (issues, known, of = () => ({})) =>
      treeOf(issues, of, { known })
        .live.toSorted((a, b) => a.issue.number - b.issue.number)
        .map(row => plain(row.agent));
    const [fresh] = agentsOf(
      [issue(1, { tree: tree({ name: "1-feature" }) })],
      {
        c1: { displayName: "Mira Albrecht" },
      },
    );
    assert.equal(fresh, "● Mira Albrecht · 1 issue · 0 MRs merged");
    const known = {
      c1: {
        name: "3-third",
        displayName: "Mira Albrecht",
        history: [
          { number: 1, how: "fresh", at: null, context: 0 },
          { number: 3, how: "handover", at: null, context: 0.31 },
        ],
      },
    };
    const [gone, third] = agentsOf(
      [
        issue(1, { tree: tree({ name: "1-feature", claude: null }) }),
        issue(3, { tree: tree({ name: "3-third" }) }),
      ],
      known,
      ({ number }) => (number === 1 ? { mr: merged } : {}),
    );
    // Its old worktree has no agent any more; the handover kept its name, its counts went up.
    assert.equal(gone, "");
    assert.equal(third, "● Mira Albrecht · 2 issues · 1 MR merged");
  });

  it("gives an agent the record has no name for one of its own, no one else's", () => {
    const rows = treeOf(
      [
        issue(1, { tree: tree({ dir: "/a", claude: { id: "a" } }) }),
        issue(2, { tree: tree({ dir: "/b", claude: { id: "b" } }) }),
      ],
      () => ({}),
    ).live;
    const names = rows.map(row => plain(row.agent).split(" · ")[0]);
    assert.match(names[0], /^● \S+ \S+$/);
    assert.notEqual(names[0], names[1]);
  });

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

  it("queues the issues without an agent: the work order, then when queued", () => {
    const queued = (number, queuedAt) =>
      issue(number, { tree: null, queued: true, queuedAt });
    const { live, queued: queue } = treeOf(
      [
        issue(1),
        queued(5, "2026-09-30T10:00:00Z"),
        queued(4, "2026-09-30T09:00:00Z"),
        queued(6, "2026-09-30T08:00:00Z"),
        queued(7, "2026-09-30T07:00:00Z"),
      ],
      () => ({}),
      {
        work: [1, 6],
        announced: [{ number: 4, to: "2-feature", open: [256] }],
        suggested: new Map([[5, { worker: { name: "1-feature" } }]]),
      },
    );
    assert.deepEqual(
      live.map(row => row.issue.number),
      [1],
    );
    assert.deepEqual(
      queue.map(row => [row.issue.number, plain(row.state.text)]),
      [
        [6, "start: waits for an agent"],
        [7, "start: waits for an agent"],
        [4, "→ 2-feature after !256"],
        [5, "→ 1-feature once its MR is merged"],
      ],
    );
  });

  it("says what a queued issue waits for: its MR, or why it is back (#272)", () => {
    const { queued: queue } = treeOf(
      [
        issue(267, { tree: null, queued: true, after: [282, 283] }),
        issue(269, {
          tree: null,
          queued: true,
          requeued: "after the setup, 268-x no claude runs it",
        }),
      ],
      () => ({}),
      {
        merged: new Set([283]),
        fresh: new Map([[269, "starts a fresh agent"]]),
      },
    );
    assert.deepEqual(
      queue.map(row => plain(row.state.text)),
      [
        "waits for !282 to be merged",
        "back in the queue: after the setup, 268-x no claude runs it; starts a fresh agent",
      ],
    );
  });

  it("shows a worktree a start sets up as such, not as one without an agent (#272)", () => {
    const made = issue(267, { tree: tree({ dir: "/w-267", claude: null }) });
    const setup = { number: 267, dir: "/w-267", to: "268-x", pid: 1 };
    const { live } = treeOf([made], () => ({}), { setups: [setup] });
    assert.equal(live[0].state.key, "setup");
    assert.equal(
      plain(live[0].state.text),
      "being set up for 268-x (wt:new) …, leave it",
    );
    assert.equal(
      plain(lifecycleOf(made, {}).text),
      "no agent",
      "without a setup it has none",
    );
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

describe("headOf", () => {
  const rows = (...tones) =>
    tones.map(tone => ({ state: { key: tone, tone } }));

  it("counts the agents by whom they wait for, then queued and removed, then the memory", () => {
    const head = headOf(
      {
        live: [
          ...rows("work", "gitlab", "you", "gitlab", "you", "you", "free"),
          { state: { key: "removable", tone: null } },
        ],
        queued: rows("queued"),
        removed: rows(...Array(14).fill("removed")),
      },
      THRESHOLD + MEMORY.stack,
    );
    assert.equal(
      plain(head),
      "1 at work · 2 wait for GitLab · 3 wait for you · 1 free · 1 will be removed · 1 queued · 14 removed · 2.5 GB free (shared stack below 1.5 GB)",
    );
  });

  it("leaves out what is none, warns below the threshold", () => {
    const head = plain(
      headOf({ live: rows("you"), queued: [], removed: [] }, MEMORY.agent),
    );
    assert.equal(
      head,
      "1 wait for you · 0.5 GB free: fresh agents on the shared stack",
    );
    assert.equal(
      plain(headOf({ live: rows("free"), queued: [], removed: [] })),
      "1 free",
    );
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
    assert.match(
      text,
      /│  agents 2 · issues 2 · reused 0 · handovers failed 0/,
    );
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
    assert.match(lines[at + 3], /└─ 2\. 2-feature +free/);
    assert.match(lines[at + 4], /^ {6}● \S+ \S+ · 1 issue · 0 MRs merged\s*$/);
    assert.match(lines[at + 5], /^ {6}#2 open · no MR yet · no answer yet/);
    assert.match(lines[at + 6], /^ {6}the title of 2/);
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

  it("shows the queue as a node of the tree, its issues under it", () => {
    const text = draw([
      issue(1),
      issue(2, { tree: null, queued: true }),
      issue(3, { tree: null }),
    ]);
    const lines = text.split("\n");
    const at = lines.findIndex(line => /queued \(1\)/.test(line));
    assert.match(
      lines[at],
      /^\s*├─ ▾ queued \(1\) · in the order they start\s*$/,
    );
    assert.match(lines[at - 4], /├─ 1\. 1-feature/);
    assert.match(lines[at - 1], /^│ {5}/);
    assert.match(
      lines[at + 1],
      /^│ {2}└─ 2\. 2-feature +start: waits for an agent/,
    );
    assert.match(lines[at + 2], /^│ {8}#2/);
    assert.match(text, /└─ ▾ done and removed \(1\)/);
    assert.match(text, /WORKTREES.*1 queued/);
  });

  it("ends the tree with the queue when nothing is removed", () => {
    const lines = draw([
      issue(1),
      issue(2, { tree: null, queued: true }),
    ]).split("\n");
    const at = lines.findIndex(line => /queued \(1\)/.test(line));
    assert.match(lines[at], /^\s*└─ ▾ queued \(1\)/);
    assert.match(lines[at + 1], /^ {3}└─ 2\. 2-feature/);
  });

  it("folds the queue into its node", () => {
    toggleQueue();
    try {
      const text = draw([issue(1), issue(2, { tree: null, queued: true })]);
      assert.match(text, /└─ ▸ queued \(1\) · in the order they start/);
      assert.doesNotMatch(text, /2-feature/);
    } finally {
      toggleQueue();
    }
  });

  it("draws no row wider than the box, at any width", () => {
    const columns = process.stdout.columns;
    try {
      for (const width of [30, 50, 80, 120]) {
        process.stdout.columns = width;
        const issues = [
          issue(1),
          issue(2, { tree: null, queued: true }),
          issue(3, { tree: null }),
        ];
        const text = draw(issues);
        for (const line of text.split("\n"))
          assert.ok([...line].length <= width, `${width}: ${line}`);
        assert.doesNotMatch(text, /─…/);
        if (width >= 50) assert.doesNotMatch(text, /[▾▸] [^\n]*…/);
        if (width >= 80) assert.doesNotMatch(text, /…/);
      }
    } finally {
      process.stdout.columns = columns;
    }
  });

  it("cuts a fold line to the width", () => {
    const line = plain(
      foldLine({
        branch: "├─ ",
        isOpen: false,
        text: "queued (12) · in the order they start",
        width: 20,
      }),
    );
    assert.equal([...line].length, 20);
    assert.match(line, /^├─ ▸ queued/);
  });

  it("draws no divider with nothing queued", () => {
    assert.doesNotMatch(draw([issue(1), issue(2)]), /queued/);
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

describe("nodeLines", () => {
  const row = {
    place: "4. ",
    name: "194-shared-vita-core-without-dom",
    state: {
      text: "→ 191-move-vita-into-products-vita once its MR is merged",
    },
    facts: "#194 open",
    title: "",
  };

  it("puts a state too long for beside the name under it, whole, at the right end", () => {
    const lines = nodeLines(row, { branch: "├─ ", stem: "│  ", width: 80 }).map(
      plain,
    );
    assert.equal(lines[0], "├─ 4. 194-shared-vita-core-without-dom");
    assert.ok(lines[1].endsWith(row.state.text), lines[1]);
    assert.equal([...lines[1]].length, 80);
    assert.ok(lines.every(line => !line.includes("…")));
    const wide = nodeLines(row, { branch: "├─ ", stem: "│  ", width: 120 });
    assert.ok(plain(wide[0]).endsWith(row.state.text));
  });
});
