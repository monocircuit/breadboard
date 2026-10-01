import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  ensurePlugin,
  ideFlag,
  IDE_LUA,
  isIdeOn,
  isOff,
  PLUGIN,
  pluginDir,
} from "./ide.mjs";
import { editorCommand, ideCommand } from "./nvim.mjs";

const git = (args, cwd) => {
  const result = spawnSync("git", args, { cwd, encoding: "utf-8" });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
};
/** A repository in a temporary directory with one commit. */
const repo = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ide-"));
  git(["init", "--quiet"], dir);
  fs.mkdirSync(path.join(dir, "lua", "claudecode"), { recursive: true });
  fs.writeFileSync(path.join(dir, "lua", "claudecode", "init.lua"), "");
  git(["add", "."], dir);
  git(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "a"], dir);
  return dir;
};

describe("the switch of the IDE integration (#258)", () => {
  it("is off for false, off, no and 0, in any case", () => {
    for (const value of ["false", "OFF", "no", "0", " false\n"])
      assert.ok(isOff(value), value);
  });

  it("is on without a value and for anything else", () => {
    for (const value of ["", "true", "on", "1"]) assert.ok(!isOff(value));
  });

  it("follows git config breadboard.ide of the checkout", () => {
    const dir = repo();
    assert.ok(isIdeOn(dir));
    assert.equal(ideFlag(dir), " --ide");
    git(["config", "breadboard.ide", "false"], dir);
    assert.ok(!isIdeOn(dir));
    assert.equal(ideFlag(dir), "");
  });
});

describe("the plugin claudecode.nvim (#258)", () => {
  it("lies in .claude/nvim of the main checkout, one directory per tag", () => {
    assert.equal(
      pluginDir("/repos/platform"),
      `/repos/platform/.claude/nvim/claudecode.nvim-${PLUGIN.tag}`,
    );
  });

  it("is cloned at its tag when that points at the pinned commit", () => {
    const source = repo();
    git(["tag", "v9"], source);
    const main = fs.mkdtempSync(path.join(os.tmpdir(), "main-"));
    const plugin = {
      url: source,
      tag: "v9",
      commit: git(["rev-parse", "HEAD"], source),
    };
    const dir = ensurePlugin(main, plugin);
    assert.equal(dir, pluginDir(main, plugin));
    assert.ok(fs.existsSync(path.join(dir, "lua", "claudecode", "init.lua")));
    assert.equal(ensurePlugin(main, { ...plugin, url: "/nowhere" }), dir);
  });

  it("is refused, and nothing of it stays, when the tag points elsewhere", () => {
    const source = repo();
    git(["tag", "v9"], source);
    const main = fs.mkdtempSync(path.join(os.tmpdir(), "main-"));
    const plugin = { url: source, tag: "v9", commit: "0".repeat(40) };
    assert.equal(ensurePlugin(main, plugin), null);
    assert.deepEqual(fs.readdirSync(path.join(main, ".claude", "nvim")), []);
  });

  it("is null when it can't be cloned", () => {
    const main = fs.mkdtempSync(path.join(os.tmpdir(), "main-"));
    assert.equal(
      ensurePlugin(main, { url: "/nowhere", tag: "v9", commit: "0" }),
      null,
    );
  });
});

describe("the nvim tab with the IDE integration (#258)", () => {
  const plugin = "/repos/platform/.claude/nvim/claudecode.nvim-v0.3.0";

  it("starts ide.lua with the plugin after changes.lua", () => {
    const command = editorCommand("/repos/platform-258-x", { ide: plugin });
    assert.ok(
      command.endsWith(
        ` -c 'luafile ${path.join(path.dirname(IDE_LUA), "changes.lua")}' -c 'lua dofile("${IDE_LUA}")("${plugin}")'`,
      ),
    );
  });

  it("opens the clicked file after the integration started", () => {
    assert.ok(
      editorCommand("/r", { ide: plugin, file: "a.mjs", line: 3 }).endsWith(
        `("${plugin}")' +3 'a.mjs'`,
      ),
    );
  });

  it("is left out without a plugin", () => {
    assert.doesNotMatch(editorCommand("/r"), /ide\.lua/);
  });

  it("writes the paths as Lua strings", () => {
    assert.equal(
      ideCommand('/a "b"'),
      `lua dofile(${JSON.stringify(IDE_LUA)})("/a \\"b\\"")`,
    );
  });
});
