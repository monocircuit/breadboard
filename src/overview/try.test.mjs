import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { actionsFile, actionsOf } from "../worktree/actions.mjs";
import { isOnApp, pageOf, shortPage, trySpot } from "./try.mjs";

const ENTRY = path.join(import.meta.dirname, "..", "worktree.mjs");
/** A repository with a linked worktree, where wt:action keeps its actions. */
const worktreeOf = () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "try-"));
  const main = path.join(root, "repo");
  const dir = path.join(root, "repo-1");
  const git = (cwd, args) =>
    spawnSync("git", args, { cwd, stdio: "ignore" }).status;
  fs.mkdirSync(main);
  git(main, ["init", "-q"]);
  git(main, [
    "-c",
    "user.name=t",
    "-c",
    "user.email=t@t",
    "commit",
    "-q",
    "--allow-empty",
    "-m",
    "x",
  ]);
  git(main, ["worktree", "add", "-q", dir, "-b", "f"]);
  return dir;
};
const action = (dir, ...args) => {
  const env = { ...process.env };
  delete env.HERDR_PANE_ID;
  return spawnSync(process.execPath, [ENTRY, "action", ...args], {
    cwd: dir,
    env,
    encoding: "utf-8",
  });
};

describe("try", () => {
  it("puts a path on the worktree's app, a url stays", () => {
    assert.equal(pageOf("/atlas/x", 2), "http://localhost:3002/atlas/x");
    assert.equal(pageOf("/", 0), "http://localhost:3000/");
    assert.equal(pageOf("https://example.com/a", 2), "https://example.com/a");
  });

  it("knows a page on the worktree's port", () => {
    assert.ok(isOnApp("http://localhost:3002/atlas", 2));
    assert.ok(isOnApp("http://localhost:3002", 2));
    assert.ok(!isOnApp("http://localhost:30021/atlas", 2));
    assert.ok(!isOnApp("http://localhost:3000/atlas", 2));
  });

  it("shortens a page on localhost", () => {
    assert.equal(shortPage("http://localhost:3002/atlas"), ":3002/atlas");
    assert.equal(shortPage("http://localhost:3002/"), ":3002");
    assert.equal(shortPage("https://example.com/a"), "https://example.com/a");
  });

  it("shows what the click will do", () => {
    const tree = {
      dir: "/nowhere",
      offset: 2,
      hasOwnPort: true,
      hasEnv: true,
      isUp: false,
    };
    const spot = (action, over = {}) =>
      trySpot(action, { ...tree, ...over }).replace(ESCAPES, "");
    assert.equal(
      spot({ run: "pnpm overview --orchestration a" }),
      "opens a terminal: pnpm overview --orchestration a",
    );
    assert.equal(
      spot({ url: "/atlas" }),
      "opens :3002/atlas after starting the stack",
    );
    assert.equal(spot({ url: "/atlas" }, { isUp: true }), "opens :3002/atlas");
    // The shared stack: nothing of its own to start.
    assert.equal(
      spot({ url: "/atlas" }, { offset: 0, hasOwnPort: false }),
      "opens :3000/atlas",
    );
  });

  it("takes a try from the command line, browser-check as try --url", () => {
    const dir = worktreeOf();
    const run = action(
      dir,
      "try",
      "the overview scrolls",
      "--run",
      "pnpm overview --orchestration a",
    );
    assert.equal(run.status, 0, run.stderr);
    assert.match(
      run.stdout,
      /opens a terminal in this worktree: pnpm overview/,
    );
    assert.equal(
      action(dir, "browser-check", "atlas", "--url", "/atlas").status,
      0,
    );
    assert.deepEqual(
      actionsOf(actionsFile(dir)).map(a => [a.kind, a.text, a.url, a.run]),
      [
        [
          "try",
          "the overview scrolls",
          null,
          "pnpm overview --orchestration a",
        ],
        ["try", "atlas", "/atlas", null],
      ],
    );
  });

  it("takes a try on the test instance as a path, only for a try", () => {
    const dir = worktreeOf();
    const test = action(dir, "try", "boxes", "--on", "test", "--url", "/atlas");
    assert.equal(test.status, 0, test.stderr);
    assert.match(test.stdout, /on the test instance/);
    assert.deepEqual(
      actionsOf(actionsFile(dir)).map(a => [a.kind, a.url, a.on]),
      [["try", "/atlas", "test"]],
    );
    assert.equal(
      trySpot(actionsOf(actionsFile(dir))[0], null).replace(ESCAPES, ""),
      "opens /atlas on the test instance",
    );
    // A hostname, a command, another place or kind: refused.
    for (const args of [
      ["try", "x", "--on", "test", "--url", "https://example.com/a"],
      ["try", "x", "--on", "test", "--run", "ls"],
      ["try", "x", "--on", "prod", "--url", "/a"],
      ["look", "x", "--on", "test", "--url", "/a"],
    ])
      assert.equal(action(dir, ...args).status, 1, args.join(" "));
    assert.equal(actionsOf(actionsFile(dir)).length, 1);
  });

  it("refuses a try without one spot, and a command on another kind", () => {
    const dir = worktreeOf();
    assert.equal(action(dir, "try", "none").status, 1);
    assert.equal(
      action(dir, "try", "both", "--url", "/a", "--run", "ls").status,
      1,
    );
    assert.equal(action(dir, "look", "x", "--run", "ls").status, 1);
    assert.equal(action(dir, "browser-check", "x").status, 1);
    assert.deepEqual(actionsOf(actionsFile(dir)), []);
  });
});
