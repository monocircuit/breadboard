import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, it } from "node:test";

import { visible } from "../lib/tty.mjs";
import { jobOrder, pushIssue, pushPipeline } from "./actions.mjs";

// Without a terminal: boxes with borders, no colours.
const NAMES = [
  "check:vita",
  "check:monolithium",
  "check:breadboard",
  "check:agents",
  "check:leaf-parity",
  "test:vita",
  "build:vita",
  "build:image",
  "scan:image:mr",
  "scan:secrets",
  "preview:deploy",
  "preview:stop",
];
const job = (name, status = "success") => ({
  name,
  status,
  url: `https://gitlab.example/jobs/${name}`,
});
const mr = { pipeline: { status: "failed" } };

let columns;
beforeEach(() => {
  columns = process.stdout.columns;
});
afterEach(() => {
  process.stdout.columns = columns;
});

describe("PIPELINE (#304)", () => {
  const jobs = NAMES.map((name, i) =>
    job(
      name,
      i === NAMES.length - 1 ? "failed" : i === 5 ? "running" : "success",
    ),
  );

  it("lists the failed jobs first, then the running ones, else in GitLab's order", () => {
    const order = jobOrder(jobs).map(j => j.name);
    assert.deepEqual(order.slice(0, 2), ["preview:stop", "test:vita"]);
    assert.deepEqual(
      order.slice(2),
      NAMES.filter(name => name !== "preview:stop" && name !== "test:vita"),
    );
  });

  for (const width of [40, 80, 120, 200, 2000])
    it(`shows every one of 12 jobs whole at ${width} columns`, () => {
      process.stdout.columns = width;
      const lines = [];
      pushPipeline(lines, mr, jobs, "/tmp");
      for (const line of lines) {
        assert.ok(visible(line) <= width, `${width}: ${line}`);
        assert.doesNotMatch(line, /…/);
      }
      const text = lines.join("\n");
      for (const name of NAMES)
        assert.match(text, new RegExp(`[✓✗◐] ${name}\\b`), name);
      // The failed one leads the first line of the jobs.
      assert.match(lines[1], /^│ ✗ preview:stop/);
      if (width <= 120) assert.ok(lines.length > 3, "wrapped");
    });
});

describe("ISSUE (#304)", () => {
  const issue = {
    iid: 304,
    url: "https://gitlab.example/issues/304",
    boxes: [
      {
        heading: "Acceptance criteria",
        isTicked: false,
        text: "Every job of the pipeline is fully visible at any width, from a narrow pane to a wide one, with eleven jobs or more and a failed one at the end",
      },
    ],
  };

  for (const width of [40, 80, 200])
    it(`shows an open box whole, wrapped under its text, at ${width} columns`, () => {
      process.stdout.columns = width;
      const lines = [];
      pushIssue(lines, issue);
      const box = lines.slice(2, -1);
      for (const line of lines) {
        assert.ok(visible(line) <= width, `${width}: ${line}`);
        assert.doesNotMatch(line, /…/);
      }
      const words = box
        .map(line => line.replace(/^│ +(○ )?|\s*│$/g, ""))
        .join(" ");
      assert.equal(words, issue.boxes[0].text);
      if (width < 200) assert.ok(box.length > 1);
      assert.ok(box.slice(1).every(line => line.startsWith("│     ")));
    });
});
