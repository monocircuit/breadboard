import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, describe, it } from "node:test";

import { projectOf } from "./conversations.mjs";
import { keepConversation, whyNotEnd, worktreesLeft } from "./end.mjs";
import { mainDir } from "./worktrees.mjs";

const ID = "0f0e0d0c-0b0a-4908-8706-050403020100";

describe("keepConversation", () => {
  let config;
  before(() => {
    config = fs.mkdtempSync(path.join(os.tmpdir(), "end-test-"));
    process.env.CLAUDE_CONFIG_DIR = config;
  });
  after(() => {
    delete process.env.CLAUDE_CONFIG_DIR;
    fs.rmSync(config, { recursive: true, force: true });
  });

  it("moves a transcript from another project into the main checkout's, with its subagents", () => {
    const other = path.join(config, "projects", "-somewhere-else");
    fs.mkdirSync(path.join(other, ID, "subagents"), { recursive: true });
    fs.writeFileSync(path.join(other, `${ID}.jsonl`), "{}\n");
    assert.match(keepConversation(ID, 0), /into the main checkout/);
    const home = projectOf(mainDir());
    assert.ok(fs.existsSync(path.join(home, `${ID}.jsonl`)));
    assert.ok(fs.existsSync(path.join(home, ID, "subagents")));
    assert.ok(!fs.existsSync(path.join(other, `${ID}.jsonl`)));
  });

  it("leaves one in the main checkout's project where it is", () => {
    assert.equal(keepConversation(ID, 0), null);
    assert.ok(fs.existsSync(path.join(projectOf(mainDir()), `${ID}.jsonl`)));
  });

  it("does nothing without a transcript", () => {
    assert.equal(
      keepConversation("ffffffff-ffff-4fff-8fff-ffffffffffff", 0),
      null,
    );
  });
});

describe("whyNotEnd", () => {
  it("lets an orchestration without worktrees end", () => {
    assert.deepEqual(worktreesLeft("no-such-orchestration"), []);
    assert.equal(whyNotEnd("no-such-orchestration"), null);
  });
});
