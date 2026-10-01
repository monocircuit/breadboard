import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES, visible } from "../lib/tty.mjs";
import { blockersText, mrRowLines, rowBlockersOf } from "./mr-blockers.mjs";

/** An open MR, ready and green unless `over` says otherwise. */
const mrOf = over => ({
  iid: 242,
  state: "opened",
  isDraft: false,
  pipeline: { status: "success", sha: "b" },
  hasConflicts: false,
  mergeStatus: "mergeable",
  ...over,
});
const work = (text, isTicked = false) => ({ heading: "Work", text, isTicked });
const reasonsOf = (over, rest = {}) =>
  rowBlockersOf({ mr: mrOf(over), ...rest }).map(
    ({ text, who }) => `${text} → ${who}`,
  );

describe("rowBlockersOf", () => {
  it("has none for a ready, green MR", () => {
    assert.deepEqual(reasonsOf({}, { boxes: [work("Tests", true)] }), []);
  });

  it("says who acts on the pipeline", () => {
    assert.deepEqual(reasonsOf({ pipeline: { status: "running" } }), [
      "pipeline running → GitLab",
    ]);
    assert.deepEqual(reasonsOf({ pipeline: null }), [
      "no pipeline yet → GitLab",
    ]);
    assert.deepEqual(
      reasonsOf(
        { pipeline: { status: "failed" } },
        {
          jobs: [
            { name: "check:lint", status: "success" },
            { name: "check:test", status: "failed" },
          ],
        },
      ),
      ["pipeline failed at check:test → agent"],
    );
  });

  it("names the first open Work boxes and counts the rest, the draft once", () => {
    assert.deepEqual(
      reasonsOf(
        { isDraft: true, mergeStatus: "draft_status" },
        {
          boxes: [
            work("Row layout: first line as now", true),
            work("The blockers in order"),
            work("Say who acts"),
            work("Tests"),
            {
              heading: "Acceptance criteria",
              text: "Checked",
              isTicked: false,
            },
          ],
        },
      ),
      [
        "3 Work boxes open: The blockers in order, Say who acts +1 → agent",
        "1 acceptance box open: Checked → agent",
      ],
    );
  });

  it("names open tries for you and open acceptance boxes, the draft only after them (#280)", () => {
    const tree = {
      actions: [
        { id: 1, kind: "try", text: "the row" },
        { id: 2, kind: "try", text: "on test", on: "test" },
      ],
      answered: [],
    };
    const acceptance = (text, isTicked = false) => ({
      heading: "Acceptance criteria",
      text,
      isTicked,
    });
    assert.deepEqual(
      reasonsOf(
        { isDraft: true, mergeStatus: "draft_status" },
        {
          tree,
          boxes: [
            work("Tests", true),
            acceptance("No merge while a try is open"),
            acceptance("Looks good unblocks"),
            acceptance("On the test instance (after deploy, try #2)"),
          ],
        },
      ),
      [
        "try #1 open → you",
        "2 acceptance boxes open: No merge while a …, Looks good unblocks → agent",
      ],
    );
    assert.deepEqual(
      reasonsOf(
        { isDraft: true, mergeStatus: "draft_status" },
        { tree: { actions: [], answered: [] }, boxes: [work("Tests", true)] },
      ),
      ["draft: mark ready → you"],
    );
  });

  it("gives a ticked, green draft to the user to mark ready", () => {
    const boxes = [work("Tests", true)];
    const draft = { isDraft: true, mergeStatus: "draft_status" };
    assert.deepEqual(reasonsOf(draft, { boxes }), ["draft: mark ready → you"]);
    assert.deepEqual(
      reasonsOf({ ...draft, pipeline: { status: "running" } }, { boxes }),
      ["pipeline running → GitLab", "draft: mark ready → agent"],
    );
    assert.deepEqual(
      reasonsOf(
        { ...draft, pipeline: { status: "running" } },
        { boxes, tree: { actions: [{ kind: "merge" }] } },
      ),
      ["pipeline running → GitLab", "draft: mark ready → you"],
    );
  });

  it("puts conflicts, MRs it waits for, a change and unpushed commits in order", () => {
    assert.deepEqual(
      reasonsOf(
        { hasConflicts: true, mergeStatus: "conflict" },
        {
          behind: 4,
          after: [{ iid: 253, why: "shares tty.mjs" }],
          tree: {
            wait: { mrs: [253, 249, 248] },
            change: { text: "shorter title", head: "a" },
            pushed: "a",
            unpushed: 2,
          },
          done: new Set([248]),
        },
      ),
      [
        "conflicts with develop (4 behind): merge develop in → agent",
        "after !253 (shares tty.mjs) → !253",
        "waits for !249 → !249",
        "change requested: shorter title → agent",
        "2 commits not pushed → agent",
      ],
    );
  });

  it("names what else GitLab holds it for", () => {
    assert.deepEqual(reasonsOf({ mergeStatus: "discussions_not_resolved" }), [
      "open discussions → you",
    ]);
    assert.deepEqual(reasonsOf({ mergeStatus: "checking" }), [
      "GitLab is checking → GitLab",
    ]);
  });
});

describe("blockersText", () => {
  it("joins the reasons, each with who acts", () => {
    assert.equal(
      blockersText([
        { text: "pipeline running", who: "GitLab" },
        { text: "draft: mark ready", who: "you" },
      ])
        .replace(ESCAPES, "")
        .replaceAll("\u00a0", " "),
      "pipeline running → GitLab · draft: mark ready → you",
    );
  });
});

describe("mrRowLines", () => {
  const plain = rows =>
    rows.map(row => row.map(line => line.replace(ESCAPES, "")));
  const row = (iid, over = {}) => ({
    graph: "●",
    cells: [`!${iid}`, "✓ passed"],
    title: `feat(breadboard): the change of !${iid}, told in a title that goes on and on for a while`,
    ...over,
  });
  const lacking = blockersText([
    { text: "pipeline failed at check:test", who: "agent" },
    {
      text: "3 Work boxes open: Row layout: first line …, The blockers, in the … +1",
      who: "agent",
    },
    { text: "draft: mark ready", who: "you" },
    { text: "after !253 (shares render-orchestration.mjs +2)", who: "!253" },
    { text: "change requested: a shorter title for the row", who: "agent" },
    { text: "2 commits not pushed", who: "agent" },
  ]);

  it("keeps a mergeable MR on one line, with the whole title where it fits", () => {
    const [lines] = plain(
      mrRowLines([row(256, { status: "ready to merge" })], { width: 196 }),
    );
    assert.equal(lines.length, 1);
    assert.match(lines[0], /and on for a while {2}ready to merge$/);
  });

  it("puts what it lacks under the title, at most three lines at 200 columns", () => {
    const [lines] = plain(
      mrRowLines([row(256, { text: lacking })], { width: 196 }),
    );
    assert.ok(lines.length <= 3);
    const under = lines[0].indexOf("feat(");
    assert.ok(
      lines
        .slice(1)
        .every(
          line =>
            line.indexOf("pipeline") === -1 ||
            line.indexOf("pipeline") === under,
        ),
    );
    assert.ok(lines.every(line => visible(line) <= 196));
  });

  it("wraps at a narrow width without parting a reason from who acts", () => {
    const [lines] = plain(
      mrRowLines([row(256, { text: lacking })], { width: 80 }),
    );
    // Whole, no `…` at the edge (#273).
    assert.ok(lines.length <= 7);
    assert.ok(lines.every(line => visible(line) <= 80));
    assert.ok(lines.slice(1).every(line => !/^\s*→/.test(line)));
    assert.match(lines.join(" "), /2 commits not\s+pushed\s→\sagent$/);
  });

  it("gives a row without a status the room of the statuses for its title", () => {
    const [open, merged] = plain(
      mrRowLines(
        [
          row(256),
          { ...row(254), title: "fix: short", status: "merged 2 h ago" },
        ],
        { width: 116 },
      ),
    );
    assert.ok(!open[0].includes("…"), open[0]);
    assert.ok(visible(open[0]) <= 116);
    assert.match(merged[0], /fix: short +merged 2 h ago$/);
  });

  it("carries the lanes that go on below on its further lines", () => {
    const rows = plain(
      mrRowLines(
        [
          row(254, { graph: "● │", below: "│ │", text: lacking }),
          { graph: "├─┘" },
          row(256, { status: "ready to merge" }),
        ],
        { width: 120 },
      ),
    );
    assert.ok(rows[0].slice(1).every(line => line.startsWith("  │ │")));
    assert.deepEqual(rows[1], ["  ├─┘"]);
    assert.equal(rows[2].length, 1);
  });
});
