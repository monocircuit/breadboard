import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { actionsFile, actionsOf } from "../worktree/actions.mjs";
import { spaceLabel } from "../worktree/spaces.mjs";
import {
  clickTry,
  isOnApp,
  pageOf,
  shortPage,
  TRY_TAB,
  trySpot,
} from "./try.mjs";

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

describe("clickTry", () => {
  it("says on the row when its terminal can't open, instead of doing nothing (#290)", () => {
    // Outside herdr: no space to open the tab try in.
    const action = { kind: "try", run: "pnpm overview" };
    const tree = { dir: "/nowhere-290" };
    assert.match(trySpot(action, tree), /opens a terminal/);
    clickTry(action, tree, () => assert.fail("no silent fallback"));
    assert.match(
      trySpot(action, tree).replace(ESCAPES, ""),
      /no space of its worktree in herdr: run pnpm overview there yourself/,
    );
  });
});

// A herdr on PATH with the space of one worktree, whose tabs it keeps in a
// file and whose panes idle as the test says; every call that acts is written
// down. xdg-open writes down the page. The real ones are never called.
describe("a click on a try (#295)", () => {
  const bin = fs.mkdtempSync(path.join(os.tmpdir(), "fake-herdr-"));
  const dir = path.join(bin, "zz-try");
  const label = spaceLabel(dir);
  const calls = path.join(bin, "calls");
  const tabs = path.join(bin, "tabs");
  const idle = path.join(bin, "idle");
  const opened = path.join(bin, "opened");
  const script = `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
const [what, verb] = args;
const tabs = JSON.parse(fs.readFileSync(${JSON.stringify(tabs)}, "utf-8"));
const save = () => fs.writeFileSync(${JSON.stringify(tabs)}, JSON.stringify(tabs));
const isIdle = fs.readFileSync(${JSON.stringify(idle)}, "utf-8") === "yes";
let result = null;
if (what === "workspace" && verb === "list")
  result = { workspaces: [{ label: ${JSON.stringify(label)}, workspace_id: "ws" }] };
else if (what === "tab" && verb === "list")
  result = { tabs: tabs.map(t => ({ ...t, pane_count: 1 })) };
else if (what === "pane" && verb === "list")
  result = { panes: tabs.map(t => ({ tab_id: t.tab_id, pane_id: "p-" + t.tab_id })) };
else if (what === "pane" && verb === "process-info")
  result = { process_info: { shell_pid: 1, foreground_process_group_id: isIdle ? 1 : 2 } };
else {
  fs.appendFileSync(${JSON.stringify(calls)}, JSON.stringify(args) + "\\n");
  if (what === "tab" && verb === "create") {
    const tab = { tab_id: "t" + (tabs.length + Math.random()).toString(36).slice(2, 6), label: args[args.indexOf("--label") + 1] };
    tabs.push(tab);
    save();
    result = { tab, root_pane: { pane_id: "p-" + tab.tab_id } };
  } else if (what === "tab" && verb === "close") {
    tabs.splice(tabs.findIndex(t => t.tab_id === args[2]), 1);
    save();
    result = {};
  } else result = {};
}
console.log(JSON.stringify({ result }));
`;
  fs.writeFileSync(path.join(bin, "herdr"), script, { mode: 0o755 });
  fs.writeFileSync(
    path.join(bin, "xdg-open"),
    `#!/bin/sh\necho "$1" >> ${JSON.stringify(opened)}\n`,
    { mode: 0o755 },
  );

  const setUp = (tabsThere, isIdle = false) => {
    fs.rmSync(path.join(dir, ".claude"), { recursive: true, force: true });
    fs.writeFileSync(tabs, JSON.stringify(tabsThere));
    fs.writeFileSync(idle, isIdle ? "yes" : "no");
    fs.writeFileSync(calls, "");
    fs.writeFileSync(opened, "");
  };
  const callsMade = () =>
    fs
      .readFileSync(calls, "utf-8")
      .split("\n")
      .filter(Boolean)
      .map(line => JSON.parse(line));
  const tabsNow = () => JSON.parse(fs.readFileSync(tabs, "utf-8"));
  /** `fn` with the fakes first on PATH, inside herdr, no browser to group tabs in. */
  const withFakes = fn => {
    const before = { ...process.env };
    process.env.PATH = `${bin}${path.delimiter}${before.PATH}`;
    process.env.HERDR_ENV = "1";
    process.env.XDG_CONFIG_HOME = bin;
    try {
      return fn();
    } finally {
      for (const key of ["PATH", "HERDR_ENV", "XDG_CONFIG_HOME"])
        if (before[key] === undefined) delete process.env[key];
        else process.env[key] = before[key];
    }
  };
  const tree = { dir, offset: 0, hasOwnPort: false, hasEnv: true, isUp: true };
  const click = action =>
    withFakes(() => clickTry(action, tree, () => assert.fail("no fallback")));
  const spot = action => trySpot(action, tree).replace(ESCAPES, "");
  const older = { id: 3, kind: "try", run: "pnpm overview --orchestration a" };
  const newer = { id: 5, kind: "try", run: "pnpm overview --orchestration b" };

  it("closes an older try's tab and runs the newer one there", () => {
    setUp([{ tab_id: "root", label: "root" }]);
    click(older);
    assert.equal(spot(older), `runs in the tab try ${older.run}`);
    const first = tabsNow().find(t => t.label === TRY_TAB).tab_id;
    fs.writeFileSync(calls, "");
    click(newer);
    const now = tabsNow().filter(t => t.label === TRY_TAB);
    assert.equal(now.length, 1);
    assert.notEqual(now[0].tab_id, first);
    const made = callsMade();
    assert.deepEqual(made[0], ["tab", "close", first]);
    assert.equal(made[1][1], "create");
    assert.deepEqual(made[2], ["pane", "run", `p-${now[0].tab_id}`, newer.run]);
    assert.deepEqual(made.slice(3), [
      ["workspace", "focus", "ws"],
      ["tab", "focus", now[0].tab_id],
    ]);
    assert.equal(
      spot(newer),
      `replaced the try of #3, runs in the tab try ${newer.run}`,
    );
    assert.equal(spot(older), `opens a terminal: ${older.run}`);
  });

  it("only focuses the try that runs when it is clicked again", () => {
    setUp([]);
    click(newer);
    const tab = tabsNow()[0].tab_id;
    fs.writeFileSync(calls, "");
    click(newer);
    assert.deepEqual(callsMade(), [
      ["workspace", "focus", "ws"],
      ["tab", "focus", tab],
    ]);
    assert.equal(spot(newer), `focused, runs in the tab try ${newer.run}`);
    // Once its command ended, the click runs it again in the same tab.
    fs.writeFileSync(idle, "yes");
    fs.writeFileSync(calls, "");
    click(newer);
    assert.deepEqual(callsMade()[0], ["pane", "run", `p-${tab}`, newer.run]);
    assert.equal(spot(newer), `runs again in the tab try ${newer.run}`);
  });

  it("replaces a try tab the overview didn't start", () => {
    setUp([{ tab_id: "stray", label: TRY_TAB }]);
    click(newer);
    assert.deepEqual(callsMade()[0], ["tab", "close", "stray"]);
    assert.deepEqual(
      tabsNow().map(t => t.label),
      [TRY_TAB],
    );
    assert.equal(spot(newer), `replaced what ran in the tab try ${newer.run}`);
  });

  it("opens the page of the try clicked, not an older one", async () => {
    setUp([]);
    const first = { id: 6, kind: "try", url: "/atlas/a" };
    const second = { id: 7, kind: "try", url: "/atlas/b" };
    await click(first);
    await click(second);
    // The browser starts detached: its page is written down a moment later.
    for (let i = 0; i < 50; i++) {
      if (fs.readFileSync(opened, "utf-8").split("\n").length > 2) break;
      await new Promise(resolve => setTimeout(resolve, 20));
    }
    assert.deepEqual(fs.readFileSync(opened, "utf-8").trim().split("\n"), [
      "http://localhost:3000/atlas/a",
      "http://localhost:3000/atlas/b",
    ]);
    assert.equal(spot(second), "opened :3000/atlas/b");
    assert.equal(spot(first), "opens :3000/atlas/a");
  });
});
