import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, describe, it } from "node:test";

import { writeOwnBlock } from "./own.mjs";
import { BLOCK_BEGIN, BLOCK_END } from "./session.mjs";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "workflow-own-"));
after(() => fs.rmSync(dir, { recursive: true, force: true }));
const configWith = text => {
  const file = path.join(dir, `${Math.random().toString(36).slice(2)}.toml`);
  fs.writeFileSync(file, text);
  return file;
};

describe("writeOwnBlock", () => {
  it("renews a block that is there, and keeps what is around it", async () => {
    const file = configWith(
      `[ui]\nsidebar_width = 34\n\n${BLOCK_BEGIN}\nold keys\n${BLOCK_END}\n\n# mine after it\n`,
    );
    assert.equal(await writeOwnBlock(file), true);
    const text = fs.readFileSync(file, "utf-8");
    assert.ok(text.startsWith("[ui]\nsidebar_width = 34\n\n"));
    assert.ok(text.endsWith(`${BLOCK_END}\n\n# mine after it\n`));
    assert.ok(!text.includes("old keys"));
    assert.match(text, /monocircuit\.platform\.overview/);
    assert.equal(text.split(BLOCK_BEGIN).length, 2);
  });

  it("changes nothing the second time", async () => {
    const file = configWith(`${BLOCK_BEGIN}\n${BLOCK_END}\n`);
    await writeOwnBlock(file);
    assert.equal(await writeOwnBlock(file), false);
  });

  it("never adds a block when it only renews (--reload)", async () => {
    const file = configWith("[ui]\n");
    assert.equal(await writeOwnBlock(file, { isRenewOnly: true }), false);
    assert.equal(fs.readFileSync(file, "utf-8"), "[ui]\n");
  });

  it("leaves out the sidebar rows when the config has its own", async () => {
    const file = configWith(
      `[ui.sidebar.spaces]\nrows = [["workspace"]]\n\n${BLOCK_BEGIN}\n${BLOCK_END}\n`,
    );
    await writeOwnBlock(file);
    assert.equal(
      fs.readFileSync(file, "utf-8").split("[ui.sidebar.spaces]").length,
      2,
    );
  });
});
