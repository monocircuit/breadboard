import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { blockersText, rowBlockersOf } from "./mr-blockers.mjs";

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
      ["3 Work boxes open: The blockers in order, Say who acts +1 → agent"],
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
            wait: { mrs: [253, 249] },
            change: { text: "shorter title", head: "a" },
            pushed: "a",
            unpushed: 2,
          },
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
      ]).replace(ESCAPES, ""),
      "pipeline running → GitLab · draft: mark ready → you",
    );
  });
});
