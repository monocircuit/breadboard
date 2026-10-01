import assert from "node:assert/strict";
import { describe, it, mock } from "node:test";

import {
  clickMerge,
  mergeText,
  nextAfter,
  blockersOf,
  checksOf,
  openTriesOf,
  sharedFiles,
  shouldRedraft,
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

  it("marks a posted draft ready once every Work and acceptance box is ticked", () => {
    const draft = { ...ready, isDraft: true };
    const posted = { canMarkReady: true };
    assert.deepEqual(
      blockersOf(
        draft,
        boxes.map(found => ({ ...found, isTicked: true })),
        posted,
      ),
      [],
    );
    assert.deepEqual(
      blockersOf({ ...draft, pipeline: { status: "running" } }, boxes, posted),
      [
        "1 work box open: After the merge: the …",
        "1 acceptance box open: Two clicks merge a …",
        "pipeline running",
      ],
    );
    // A ready MR too: the boxes block whoever marked it ready.
    assert.deepEqual(blockersOf(ready, boxes.slice(2)), [
      "1 acceptance box open: Two clicks merge a …",
    ]);
  });

  describe("the human's checks (#280)", () => {
    const tryOf = (id, more = {}) => ({
      id,
      kind: "try",
      text: `t${id}`,
      ...more,
    });

    it("an open try blocks, `looks good` unblocks", () => {
      assert.deepEqual(
        blockersOf(ready, [], { tree: { actions: [tryOf(1)] } }),
        ["try #1 open"],
      );
      assert.deepEqual(
        blockersOf(ready, [], {
          tree: { actions: [tryOf(1, { answer: "looks good" })] },
        }),
        [],
      );
      assert.deepEqual(
        blockersOf(ready, [], {
          tree: { answered: [tryOf(1, { answer: "looks good" })] },
        }),
        [],
      );
      // A browser-check is a try too.
      assert.deepEqual(
        openTriesOf({ actions: [tryOf(2, { kind: "browser-check" })] }).map(
          found => found.id,
        ),
        [2],
      );
    });

    it("a `broken` blocks until a later try looks good", () => {
      const broken = tryOf(1, { answer: "broken" });
      assert.deepEqual(blockersOf(ready, [], { tree: { actions: [broken] } }), [
        "try #1 broken",
      ]);
      assert.deepEqual(
        blockersOf(ready, [], { tree: { answered: [broken] } }),
        ["try #1 broken"],
      );
      assert.deepEqual(
        blockersOf(ready, [], {
          tree: { answered: [broken], actions: [tryOf(2)] },
        }),
        ["try #1 broken", "try #2 open"],
      );
      assert.deepEqual(
        blockersOf(ready, [], {
          tree: {
            answered: [broken, tryOf(2, { answer: "looks good" })],
          },
        }),
        [],
      );
    });

    it("a try on the test instance and a box after deploy don't block, they count after the merge", () => {
      const tree = { actions: [tryOf(1, { on: "test" })] };
      const afterDeploy = {
        heading: "Acceptance criteria",
        text: "It shows on test (after deploy, try #1)",
        isTicked: false,
      };
      assert.deepEqual(checksOf([afterDeploy], tree), []);
      assert.deepEqual(blockersOf(ready, [afterDeploy], { tree }), []);
      assert.equal(toCheckOf([afterDeploy], tree), 2);
      assert.equal(toCheckOf(boxes), 0);
    });

    it("a `broken` puts a ready MR back to draft", () => {
      const open = { ...ready, title: "feat: x" };
      assert.equal(shouldRedraft(tryOf(1), "broken", open), true);
      assert.equal(shouldRedraft(tryOf(1), "looks good", open), false);
      assert.equal(
        shouldRedraft(tryOf(1), "broken", { ...open, isDraft: true }),
        false,
      );
      assert.equal(
        shouldRedraft(tryOf(1, { on: "test" }), "broken", open),
        false,
      );
      assert.equal(
        shouldRedraft({ id: 1, kind: "plan" }, "different", open),
        false,
      );
    });
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
    const done = [
      ...boxes.map(found => ({ ...found, isTicked: true })),
      {
        heading: "Acceptance criteria",
        text: "On test (after deploy, try #2)",
        isTicked: false,
      },
    ];
    assert.match(
      mergeText({ ...ready, iid: 10, isDraft: true }, "ready", done, {
        canMarkReady: true,
      }),
      /two clicks mark ready and merge · 1 to check after merge/,
    );
    assert.match(
      mergeText({ ...ready, iid: 11 }, "ready", done, {
        tree: { actions: [{ id: 1, kind: "try", text: "the row" }] },
      }),
      /ready · try #1 open/,
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
