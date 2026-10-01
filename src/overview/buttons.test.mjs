// Every drawn button works on every click (#303): views with every kind of
// button, at several widths and every scroll position, each button clicked
// at each of its cells. A new button is covered without a test of its own.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { actionsFile, actionsOf } from "../worktree/actions.mjs";
import { buttonsOn, titleHeight } from "./boxes.mjs";
import { missedButtons } from "./every-button.mjs";
import { painter } from "./frame.mjs";
import { renderOrchestration } from "./render-orchestration.mjs";
import { screen } from "./screen.mjs";

const plain = text => text.replace(ESCAPES, "");
const columnsBefore = process.stdout.columns;
afterEach(() => {
  process.stdout.columns = columnsBefore;
  screen.scroll = 0;
});

/** A herdr on PATH that fails every call: no answer gets into a pane, none is typed anywhere. */
const withoutHerdr = fn => {
  const bin = fs.mkdtempSync(path.join(os.tmpdir(), "no-herdr-"));
  fs.writeFileSync(path.join(bin, "herdr"), "#!/bin/sh\nexit 1\n", {
    mode: 0o755,
  });
  const before = process.env.PATH;
  process.env.PATH = `${bin}${path.delimiter}${before}`;
  try {
    return fn();
  } finally {
    process.env.PATH = before;
  }
};

/** The try of #292 that lost its `looks good` (#303): long, wrapped at any width. */
const TRY = {
  id: 3,
  kind: "try",
  text: "WORKTREES: every worktree with an agent has a line under its name, ● Name · N issues · N MRs merged; an agent that was handed over keeps its name with higher counts; AGENTS lists free agents by name (read-only --once view of this branch)",
  url: null,
  run: "pnpm overview --orchestration breadboard --once",
  options: [],
  recommend: null,
};

/** A worktree of `number` with `actions` in an actions file of its own. */
const treeOf = (number, actions) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `buttons-${number}-`));
  const file = actionsFile(dir);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify({ actions }));
  return {
    dir,
    name: `${number}-feature`,
    branch: `feature/${number}-feature`,
    ahead: 0,
    changed: 0,
    actions,
    claude: { id: `claude-${number}` },
    files: new Set(),
  };
};
const issueOf = tree => ({
  number: Number.parseInt(tree.name),
  feature: tree.name,
  branch: tree.branch,
  title: `the issue of ${tree.name}, long enough to wrap in a narrow pane`,
  tree,
});

/** An orchestration with every kind of action, an MR with conflicts and a free agent. */
const orchestration = () => {
  const at = new Date().toISOString();
  const tries = treeOf(292, [{ ...TRY, at }]);
  const asks = treeOf(296, [
    {
      id: 1,
      kind: "decide",
      text: "Which mapping? One for both, the other later.",
      options: ["one", "two"],
      recommend: "one",
      at,
    },
    { id: 2, kind: "plan", text: "the plan in one line", at },
    { id: 3, kind: "look", text: "the new row", at },
  ]);
  const conflicts = treeOf(294, []);
  const issues = [tries, asks, conflicts].map(issueOf);
  const mr = {
    iid: 300,
    url: "https://gitlab.example/mr/300",
    title: "the MR with conflicts",
    state: "opened",
    hasConflicts: true,
    mergeStatus: "cannot_be_merged",
    pipeline: { status: "success" },
  };
  return {
    local: {
      issues,
      trees: issues.map(issue => issue.tree),
      plan: { work: [292, 296, 294], merge: [], why: "" },
      ownActions: [],
      agents: new Map(),
      freeAgents: [{ id: "free-1", name: "Ada", issues: 2, merged: 1 }],
      recordFile: path.join(os.tmpdir(), "no-such-record.json"),
    },
    remote: { of: new Map([[conflicts.branch, { mr, issue: { boxes: [] } }]]) },
  };
};

/** A painter on `render`, drawn: its rows on screen, a frame a click resolves against. */
const drawn = (render, rows) => {
  let pending = null;
  const shown = [];
  const frame = painter({
    render,
    rows: () => rows,
    write: text => {
      for (const part of text.split(/(?=\x1b\[\d+;1H)/)) {
        const at = part.match(/^\x1b\[(\d+);1H(\x1b\[2K|\x1b\[J)([\s\S]*)$/);
        if (!at) continue;
        if (at[2] === "\x1b[J") shown.length = Number(at[1]) - 1;
        else shown[Number(at[1]) - 1] = at[3];
      }
    },
    now: () => 0,
    later: paint => {
      pending = paint;
      return 1;
    },
  });
  const flush = () => {
    const paint = pending;
    pending = null;
    paint?.();
  };
  frame.changed();
  flush();
  return { frame, flush, shown: () => shown };
};

describe("every drawn button (#303)", () => {
  for (const width of [60, 100, 160])
    it(`runs its own click on each of its cells, at ${width} columns and every scroll position`, () => {
      process.stdout.columns = width;
      const { local, remote } = orchestration();
      let text = "";
      const render = () => {
        screen.clicks = [];
        screen.upTarget = null;
        screen.sticky = titleHeight();
        text = renderOrchestration(local, remote, null);
        return text;
      };
      const { frame, flush, shown } = drawn(render, 24);
      const length = text.split("\n").length;
      const clicked = { count: 0 };
      const labels = new Set();
      for (let offset = 0; offset < length; offset++) {
        screen.scroll = offset;
        frame.scrolled();
        flush();
        assert.deepEqual(
          missedButtons(frame, shown(), clicked),
          [],
          `offset ${offset}`,
        );
        for (const line of shown())
          for (const { label } of buttonsOn(line).buttons) labels.add(label);
      }
      assert.ok(clicked.count > 0, "the view has buttons");
      // Every kind is drawn: answers, options' neighbours, the MR's and an agent's.
      for (const label of [
        "looks good",
        "broken",
        "ok",
        "different",
        "done",
        "resolve merge conflicts",
        "✕ dismiss",
      ])
        assert.ok(
          [...labels].some(shownLabel =>
            shownLabel.replaceAll(" ", " ").startsWith(label),
          ),
          `${label} is drawn: ${[...labels].join(", ")}`,
        );
    });
});

describe("looks good on a try (#303)", () => {
  for (const width of [60, 100, 160])
    it(`records the answer on the first click, in NEXT and in ACTIONS, at any cell, at ${width} columns`, () => {
      process.stdout.columns = width;
      const at = new Date().toISOString();
      for (const section of ["NEXT", "ACTIONS"])
        for (let cell = 0; ; cell++) {
          const tree = treeOf(292, [{ ...TRY, at }]);
          const local = {
            issues: [issueOf(tree)],
            trees: [tree],
            plan: { work: [292], merge: [], why: "" },
            ownActions: [],
            agents: new Map(),
          };
          const render = () => {
            screen.clicks = [];
            screen.upTarget = null;
            screen.sticky = titleHeight();
            return renderOrchestration(local, { of: new Map() }, null);
          };
          const { frame, flush, shown } = drawn(render, 200);
          const rows = shown();
          const heading = rows.findIndex(line =>
            plain(line).includes("ACTIONS"),
          );
          const looksGood = line =>
            buttonsOn(line).buttons.find(({ label }) => label === "looks good");
          const row = rows.findIndex(
            (line, i) =>
              looksGood(line) &&
              (section === "NEXT" ? i < heading : i > heading),
          );
          assert.ok(row > 0, `${section} shows the button`);
          const { from, to } = looksGood(rows[row]);
          const column = from + cell;
          if (column > to) break;
          withoutHerdr(() => frame.clickAt(row, column));
          assert.equal(
            actionsOf(actionsFile(tree.dir))[0]?.answer,
            "looks good",
            `${section}, column ${column}`,
          );
          // Shown at once, from the same read: the answer, no buttons.
          frame.changed();
          flush();
          const after = shown();
          assert.match(after.map(plain).join("\n"), /→ looks good/);
          assert.ok(!after.some(looksGood), "no button left");
        }
    });
});
