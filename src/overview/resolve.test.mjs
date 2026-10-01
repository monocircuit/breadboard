import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES, isColor } from "../lib/tty.mjs";
import { button } from "./boxes.mjs";
import {
  RESOLVE,
  clickResolve,
  conflictCell,
  conflictStateOf,
  forgetResolved,
  isConflictSettled,
  noAgentOf,
  noteTold,
  resolveColumns,
  resolveLine,
} from "./resolve.mjs";

const plain = text => text.replace(ESCAPES, "").replaceAll(" ", " ");
const mrOf = hasConflicts => ({ iid: 246, state: "opened", hasConflicts });

describe("conflictStateOf", () => {
  const agents = new Map([["c", { status: "working" }]]);
  const treeOf = (more = {}) => ({
    dir: "/w",
    claude: { id: "c", prompt: "", at: null },
    pushed: "a1",
    unpushed: 0,
    ...more,
  });
  const cellOf = (mr, options) => plain(conflictCell(mr, options));

  it("says so when GitLab reports no conflicts", () => {
    assert.equal(cellOf(mrOf(false)), "no merge conflicts");
  });

  it("shows the button and how far behind develop the branch is", () => {
    const state = conflictStateOf(mrOf(true), {
      tree: treeOf(),
      agents,
      behind: 48,
    });
    assert.equal(state.canClick, true);
    assert.match(plain(state.text), /resolve merge conflicts.* 48 behind$/);
  });

  it("shows no button while GitLab checks, or before it has the pushed head", () => {
    for (const mr of [
      { ...mrOf(true), mergeCheck: "checking" },
      { ...mrOf(true), mergeCheck: "cannot_be_merged_recheck" },
      { ...mrOf(true), mergeStatus: "unchecked" },
      { ...mrOf(true), sha: "old" },
    ]) {
      const state = conflictStateOf(mr, { tree: treeOf(), agents });
      assert.equal(state.canClick, false);
      assert.equal(plain(state.text), "checking for conflicts…");
    }
    assert.equal(
      isConflictSettled({ ...mrOf(true), sha: "a1" }, treeOf()),
      true,
    );
  });

  it("greys the button out with the reason a click can't reach an agent", () => {
    const grey = (tree, given = agents) => {
      const state = conflictStateOf(mrOf(true), { tree, agents: given });
      assert.equal(state.canClick, false);
      assert.match(state.text.replace(ESCAPES, ""), /\( ?resolve|resolve/);
      return plain(state.text);
    };
    assert.match(grey(null), /no worktree, so no agent/);
    assert.match(
      grey(treeOf(), new Map([["c", { status: "blocked" }]])),
      /at a question/,
    );
    assert.match(grey(treeOf(), new Map([["d", {}]])), /isn't running/);
    assert.match(
      grey(treeOf({ unpushed: 9 })),
      /conflicts as of the pushed commits · 9 not pushed: its agent pushes first/,
    );
    if (!isColor) assert.match(grey(null), /^\( resolve merge conflicts \)/);
  });

  it("follows the order from queued to resolved, without a second button", () => {
    // GitLab has the head pushed before the click.
    const mr = { ...mrOf(true), iid: 290, sha: "a1" };
    const tree = treeOf();
    noteTold(mr, tree);
    const at = (more, now) =>
      conflictStateOf(more.mr ?? mr, {
        tree: { ...tree, ...more.tree },
        agents,
        behind: more.behind ?? 3,
        now,
      });

    let state = at({});
    assert.equal(state.canClick, false);
    assert.match(plain(state.text), /queued for its agent \(at work\)/);

    const started = {
      claude: {
        id: "c",
        prompt: resolveLine(mr),
        at: new Date(Date.now() + 1000),
      },
    };
    state = at({ tree: started });
    assert.match(plain(state.text), /its agent merges develop in/);

    // Merged in and pushed; GitLab hasn't seen the push yet.
    state = at({ tree: { pushed: "b2" }, behind: 0 });
    assert.match(plain(state.text), /pushed, GitLab checks/);
    assert.equal(state.canClick, false);

    const now = Date.now();
    state = at(
      {
        mr: { ...mr, hasConflicts: false, sha: "b2" },
        tree: { pushed: "b2" },
        behind: 0,
      },
      now,
    );
    assert.match(plain(state.text), /conflicts resolved/);
    state = at(
      {
        mr: { ...mr, hasConflicts: false, sha: "b2" },
        tree: { pushed: "b2" },
        behind: 0,
      },
      now + 120_000,
    );
    assert.equal(plain(state.text), "no merge conflicts");
  });

  it("offers the button again when conflicts are left after the push", () => {
    const mr = { ...mrOf(true), iid: 291 };
    noteTold(mr, treeOf());
    const started = treeOf({
      claude: {
        id: "c",
        prompt: resolveLine(mr),
        at: new Date(Date.now() + 1000),
      },
    });
    conflictStateOf(mr, { tree: started, agents, behind: 3 });
    const state = conflictStateOf(
      { ...mr, sha: "b2" },
      { tree: { ...started, pushed: "b2" }, agents },
    );
    assert.equal(state.canClick, true);
    assert.match(plain(state.text), /conflicts left after its push/);
  });

  it("does nothing on a click while the button isn't there", () => {
    const mr = { ...mrOf(true), iid: 292, mergeCheck: "checking" };
    clickResolve({ mr, tree: treeOf(), agents });
    assert.equal(
      plain(conflictCell(mr, { tree: treeOf(), agents })),
      "checking for conflicts…",
    );
    forgetResolved([]);
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
