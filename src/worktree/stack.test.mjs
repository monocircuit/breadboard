import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { adoptEnv } from "./stack.mjs";

const checkout = ({ hasWeb, hasOld = true, hasNew = false }) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "adopt-env-"));
  const vita = path.join(dir, "products", "vita");
  fs.mkdirSync(path.join(vita, "web"), { recursive: true });
  if (hasWeb) fs.writeFileSync(path.join(vita, "web", "package.json"), "{}");
  if (hasOld) fs.writeFileSync(path.join(vita, ".env.local"), "A=old\n");
  if (hasNew) fs.writeFileSync(path.join(vita, "web", ".env.local"), "A=new\n");
  return {
    dir,
    old: path.join(vita, ".env.local"),
    web: path.join(vita, "web", ".env.local"),
  };
};

describe("adoptEnv", () => {
  it("moves the .env.local of products/vita into web once the checkout has it", () => {
    const { dir, old, web } = checkout({ hasWeb: true });
    assert.equal(adoptEnv(dir), true);
    assert.equal(fs.existsSync(old), false);
    assert.equal(fs.readFileSync(web, "utf-8"), "A=old\n");
  });

  it("leaves it on a checkout from before the move", () => {
    const { dir, old } = checkout({ hasWeb: false });
    assert.equal(adoptEnv(dir), false);
    assert.equal(fs.existsSync(old), true);
  });

  it("never overwrites a .env.local already in web", () => {
    const { dir, web } = checkout({ hasWeb: true, hasNew: true });
    assert.equal(adoptEnv(dir), false);
    assert.equal(fs.readFileSync(web, "utf-8"), "A=new\n");
  });
});
