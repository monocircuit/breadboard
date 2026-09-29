import assert from "node:assert/strict";
import { describe, it, mock } from "node:test";

import {
  clickMerge,
  mergeText,
  nextAfter,
  blockersOf,
  sharedFiles,
  toCheckOf,
  isNotYet,
  mergeStatusText,
  reasonOf,
} from "./merge.mjs";

describe("merge", () => {
  const ready = {
    iid: 7,
    state: "opened",
    isDraft: false,
    url: "https://gitlab.example/mr/7",
    pipeline: { status: "success" },
  };

  const box = (heading, text, isTicked) => ({ heading, text, isTicked });
  const boxes = [
    box("Work", "`merge` button: the first click arms it", true),
    box("Work", "After the merge: the agent gets told", false),
    box("Acceptance criteria", "Two clicks merge a ready MR", false),
  ];

  it("names what blocks a merge, as the overview reads the MR", () => {
    assert.deepEqual(blockersOf(ready), []);
    assert.deepEqual(blockersOf({ ...ready, isDraft: true }), [
      "draft: its agent posts the merge",
    ]);
    assert.deepEqual(
      blockersOf({ ...ready, pipeline: { status: "running" } }),
      ["pipeline running"],
    );
    assert.deepEqual(blockersOf({ ...ready, pipeline: null }), [
      "pipeline not there yet",
    ]);
    assert.deepEqual(blockersOf({ ...ready, state: "merged" }), [
      "it is merged",
    ]);
  });

  it("marks a posted draft ready once every Work box is ticked", () => {
    const draft = { ...ready, isDraft: true };
    const posted = { canMarkReady: true };
    // The acceptance criteria don't block.
    assert.deepEqual(
      blockersOf(
        draft,
        boxes.filter(found => found.isTicked || found.heading !== "Work"),
        posted,
      ),
      [],
    );
    assert.deepEqual(
      blockersOf({ ...draft, pipeline: { status: "running" } }, boxes, posted),
      ["1 work box open: After the merge: the …", "pipeline running"],
    );
    assert.equal(toCheckOf(boxes), 1);
  });

  it("names what GitLab refuses", () => {
    const fresh = {
      state: "opened",
      draft: false,
      head_pipeline: { status: "success" },
      detailed_merge_status: "mergeable",
    };
    assert.deepEqual(blockersOf(fresh), []);
    assert.deepEqual(
      blockersOf({ ...fresh, detailed_merge_status: "checking" }),
      ["GitLab is checking"],
    );
    // GitLab's word counts even when has_conflicts says nothing (!220).
    assert.deepEqual(
      blockersOf({
        ...fresh,
        has_conflicts: false,
        detailed_merge_status: "conflict",
      }),
      ["conflicts with develop"],
    );
    assert.deepEqual(
      blockersOf({
        ...fresh,
        detailed_merge_status: "discussions_not_resolved",
      }),
      ["open discussions"],
    );
    assert.deepEqual(
      blockersOf({
        ...fresh,
        has_conflicts: true,
        detailed_merge_status: "conflict",
      }),
      ["conflicts with develop"],
    );
    assert.deepEqual(
      blockersOf({ ...fresh, detailed_merge_status: "not_approved" }),
      ["needs an approval"],
    );
  });

  it("finds the next open MR by the merge order", () => {
    const issue = (number, state, hasTree = true) => ({
      number,
      tree: hasTree ? { dir: `/w/${number}` } : null,
      mr: state && { state },
    });
    const issues = [
      issue(1, "merged"),
      issue(2, "opened"),
      issue(3, "opened"),
      issue(4, "opened", false),
      issue(5, null),
    ];
    const of = found => ({ mr: found.mr });
    assert.equal(nextAfter(issues, 1, [1, 3, 2], of).number, 3);
    assert.equal(nextAfter(issues, 3, [1, 3, 2], of).number, 2);
    // Without an order, by number.
    assert.equal(nextAfter(issues, 1, [], of).number, 2);
    assert.equal(nextAfter(issues, 2, [2], of).number, 3);
    assert.equal(nextAfter([issues[0]], 1, [1], of), null);
  });

  it("names the files both touched", () => {
    assert.deepEqual(
      sharedFiles(
        ["b.mjs", "a.mjs", "a.mjs", "c.mjs"],
        new Set(["a.mjs", "b.mjs"]),
      ),
      ["a.mjs", "b.mjs"],
    );
    assert.deepEqual(sharedFiles(["c.mjs"], new Set(["a.mjs"])), []);
  });

  it("arms on the first click and disarms by itself", () => {
    mock.timers.enable({ apis: ["setTimeout"] });
    try {
      assert.match(mergeText(ready, "ready"), /two clicks merge/);
      clickMerge({ issue: { tree: null }, mr: ready, next: null });
      assert.match(mergeText(ready, "ready"), /click again to merge !7/);
      mock.timers.tick(5000);
      assert.match(mergeText(ready, "ready"), /two clicks merge/);
    } finally {
      mock.timers.reset();
    }
  });

  it("greys a blocked row with everything that blocks it in one line", () => {
    assert.match(
      mergeText({ ...ready, iid: 8, isDraft: true }, "ready"),
      /ready · draft: its agent posts the merge/,
    );
    assert.match(
      mergeText({ ...ready, iid: 9, isDraft: true }, "ready", boxes, {
        canMarkReady: true,
      }),
      /ready · draft · 1 work box open: After the merge: the …/,
    );
    const done = boxes.map(found => ({
      ...found,
      isTicked: found.heading === "Work",
    }));
    assert.match(
      mergeText({ ...ready, iid: 10, isDraft: true }, "ready", done, {
        canMarkReady: true,
      }),
      /two clicks mark ready and merge · 1 to check after merge/,
    );
  });
});

describe("reasonOf", () => {
  it("says in words what a refused merge means, never a bare HTTP code", () => {
    const refused = { stderr: "glab: 405 Method Not Allowed (HTTP 405)" };
    assert.equal(
      reasonOf(refused),
      "GitLab is still checking the MR (it refused the merge for now)",
    );
    assert.equal(isNotYet(refused), true);
    assert.equal(isNotYet({ stderr: "HTTP 422" }), true);
    assert.equal(
      reasonOf({ stderr: "glab: 403 Forbidden (HTTP 403)" }),
      "GitLab refuses: no right to merge it as you",
    );
    assert.doesNotMatch(reasonOf({ stderr: "boom (HTTP 500)" }), /HTTP/);
  });
});

describe("mergeStatusText", () => {
  it("leaves out what another blocker already says", () => {
    assert.equal(mergeStatusText("mergeable"), null);
    assert.equal(mergeStatusText("draft_status", { isDraft: true }), null);
    assert.equal(
      mergeStatusText("ci_must_pass", { isPipelineBlocked: true }),
      null,
    );
    assert.equal(mergeStatusText("need_rebase"), "needs a rebase onto develop");
    assert.equal(mergeStatusText("some_new_thing"), "GitLab: some new thing");
  });
});
