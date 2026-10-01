import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES, isColor } from "../lib/tty.mjs";
import { button } from "./boxes.mjs";
import {
  RESOLVE,
  clickResolve,
  conflictCell,
  forgetResolved,
  noAgentOf,
  resolveColumns,
  resolveLine,
} from "./resolve.mjs";

const plain = text => text.replace(ESCAPES, "").replaceAll(" ", " ");
const mrOf = hasConflicts => ({ iid: 246, state: "opened", hasConflicts });

describe("conflictCell", () => {
  it("says so when GitLab reports no conflicts", () => {
    assert.equal(plain(conflictCell(mrOf(false))), "no merge conflicts");
  });

  it("shows the button and how far behind develop the branch is", () => {
    assert.match(
      plain(conflictCell(mrOf(true), { behind: 48 })),
      /resolve merge conflicts.* 48 behind$/,
    );
  });

  it("says why a click didn't reach an agent, until GitLab reports none", () => {
    const mr = mrOf(true);
    clickResolve({ mr, tree: null, agents: new Map() });
    assert.match(plain(conflictCell(mr)), /no worktree, so no agent/);
    assert.doesNotMatch(
      plain(conflictCell(mr, { now: Date.now() + 60_000 })),
      /no agent/,
    );
    forgetResolved([mrOf(false)]);
    assert.doesNotMatch(plain(conflictCell(mr)), /no agent/);
  });
});

describe("noAgentOf", () => {
  const tree = { dir: "/w", claude: { id: "c" } };
  it("names what keeps the order from its agent", () => {
    assert.match(noAgentOf(null, new Map()), /no worktree/);
    assert.match(noAgentOf({ dir: "/w" }, new Map()), /no agent/);
    assert.match(noAgentOf(tree, new Map([["d", {}]])), /isn't running/);
    assert.match(
      noAgentOf(tree, new Map([["c", { status: "blocked" }]])),
      /at a question/,
    );
  });

  it("lets it go to a working or idle agent, and outside herdr", () => {
    assert.equal(noAgentOf(tree, new Map([["c", { status: "idle" }]])), null);
    assert.equal(noAgentOf(tree, new Map()), null);
  });
});

describe("resolveLine", () => {
  it("orders the merge of develop, the checks and the push", () => {
    const line = resolveLine(mrOf(true));
    assert.match(line, /^\[overview\] !246 /);
    assert.match(line, /git merge origin\/develop/);
    assert.match(line, /lint, typecheck and test/);
    assert.match(line, /push\.$/);
  });
});

describe("resolveColumns", () => {
  it("finds the chip on screen, the box's indent taken into account", () => {
    const line = `      ${button(RESOLVE, "yellow")} 4 behind`;
    // The box replaces the two columns of indent with its margin or border.
    const shown = isColor
      ? ` ${line.slice(2)}`.replace(ESCAPES, "")
      : `│ ${line.slice(2)}`.replace(ESCAPES, "");
    const chip = isColor ? ` ${RESOLVE} ` : `[ ${RESOLVE} ]`;
    const { from, to } = resolveColumns(line);
    assert.equal(shown.slice(from - 1, to), chip);
  });

  it("is null on a line without it", () => {
    assert.equal(resolveColumns("  no merge conflicts"), null);
  });
});
