import assert from "node:assert/strict";
import fs from "node:fs";
import { describe, it } from "node:test";

import {
  orchestrationOf,
  orchestratorLabel,
  recordFile,
  recordOf,
  titleOf,
  updateRecord,
} from "./origins.mjs";

describe("the title of an orchestration (#234)", () => {
  // A record of its own in the main checkout's .claude/orchestrations, gone after.
  const name = "zz-title-test";

  it("is recorded by --title, changed by a later one, and keeps the issues", () => {
    try {
      updateRecord(name, { issues: [{ number: 1 }] });
      assert.equal(titleOf(name), name);
      assert.equal(orchestratorLabel(name), `⇶ ${name}`);
      updateRecord(name, { title: "@monocircuit/breadboard" });
      assert.equal(titleOf(name), "@monocircuit/breadboard");
      assert.equal(orchestratorLabel(name), "⇶ @monocircuit/breadboard");
      updateRecord(name, { title: "breadboard, the dev workflow" });
      assert.equal(recordOf(name).title, "breadboard, the dev workflow");
      assert.deepEqual(recordOf(name).issues, [{ number: 1 }]);
    } finally {
      fs.rmSync(recordFile(name), { force: true });
    }
  });

  it("falls back to the name without a record or a title", () => {
    assert.equal(
      titleOf("zz-no-such-orchestration"),
      "zz-no-such-orchestration",
    );
    assert.equal(
      orchestratorLabel("zz-no-such-orchestration"),
      "⇶ zz-no-such-orchestration",
    );
  });
});

describe("orchestrationOf", () => {
  const titles = new Map([
    ["breadboard", "@monocircuit/breadboard"],
    ["batch", "batch"],
  ]);

  it("finds the name behind a space labelled with its title", () => {
    assert.equal(
      orchestrationOf("⇶ @monocircuit/breadboard", titles),
      "breadboard",
    );
  });

  it("still reads a space from before the title, and an untitled one", () => {
    assert.equal(orchestrationOf("⇶ breadboard", titles), "breadboard");
    assert.equal(orchestrationOf("⇶ batch", titles), "batch");
    assert.equal(orchestrationOf("⇶ other", titles), "other");
  });

  it("knows no orchestration in any other label", () => {
    assert.equal(orchestrationOf("217-agents-workers", titles), null);
    assert.equal(orchestrationOf(undefined, titles), null);
  });
});
