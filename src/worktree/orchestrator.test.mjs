import assert from "node:assert/strict";
import fs from "node:fs";
import { describe, it } from "node:test";

import { editorCommand } from "./nvim.mjs";
import {
  ORCHESTRATION_LUA,
  changesCommands,
  slugOf,
  startEntry,
} from "./orchestrator.mjs";

describe("slugOf", () => {
  it("keeps up to five words without the small ones", () => {
    assert.equal(
      slugOf("Split worktree.mjs and overview.mjs into modules, with tests"),
      "split-worktree-mjs-overview-mjs",
    );
  });

  it("stays within 40 characters, cutting only between words", () => {
    const slug = slugOf(
      "Internationalisation of the notification preferences dialogue",
    );
    assert.ok(slug.length <= 40);
    assert.equal(slug, "internationalisation-notification");
  });

  it("drops accents and punctuation", () => {
    assert.equal(slugOf("Crème brûlée: the fix!"), "creme-brulee-fix");
  });

  it("names an issue without words `issue`", () => {
    assert.equal(slugOf("the — of"), "issue");
  });
});

describe("changesCommands", () => {
  it("loads orchestration.lua and lists the orchestration's changes", () => {
    assert.ok(fs.existsSync(ORCHESTRATION_LUA));
    assert.deepEqual(changesCommands("breadboard"), [
      `luafile ${ORCHESTRATION_LUA}`,
      "OrchestrationChanges breadboard",
    ]);
  });

  it("starts the tab on the orchestration's own socket, without changes.lua", () => {
    const line = editorCommand("/repo", {
      name: "breadboard",
      commands: changesCommands("breadboard"),
    });
    assert.match(line, /--listen '\.claude\/nvim-breadboard\.sock'/);
    assert.match(line, /-c 'OrchestrationChanges breadboard'$/);
    assert.doesNotMatch(line, /changes\.lua/);
  });
});

describe("startEntry (#272)", () => {
  const item = {
    number: "269",
    title: "ui",
    feature: "269-ui",
    branch: "feature/269-ui",
  };

  it("keeps a queued issue queued at its place, with why it is back", () => {
    const known = { queued: true, queuedAt: "2026-10-01T09:00:00Z" };
    assert.deepEqual(startEntry(item, known), {
      number: 269,
      title: "ui",
      feature: "269-ui",
      branch: "feature/269-ui",
      why: null,
      queued: true,
      queuedAt: "2026-10-01T09:00:00Z",
    });
    assert.equal(
      startEntry(item, known, "its claude did not start").requeued,
      "its claude did not start",
    );
  });

  it("marks one that was never queued as failed", () => {
    assert.equal(
      startEntry(item, null, "no worktree").why,
      "failed: no worktree",
    );
    assert.equal(startEntry(item, undefined).why, null);
  });
});
