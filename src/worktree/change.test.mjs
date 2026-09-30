import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  actionsFile,
  actionsOf,
  addAction,
  changeOf,
  requestChange,
} from "./actions.mjs";
import {
  UNPUSHED_MS,
  isChangeOpen,
  isUnpushedLong,
  pushStateOf,
} from "./change.mjs";

const temp = prefix => fs.mkdtempSync(path.join(os.tmpdir(), prefix));
const change = {
  text: "padding",
  at: "2026-09-29T10:00:00Z",
  mr: 7,
  head: "a",
};
const green = sha => ({ pipeline: { status: "success", sha } });

describe("requestChange", () => {
  it("withdraws the merge actions and keeps the rest", () => {
    const file = actionsFile(temp("change-"));
    addAction(file, { kind: "merge", text: "!7 ready" });
    addAction(file, { kind: "try", text: "look", url: "/atlas" });
    const withdrawn = requestChange(file, change);
    assert.deepEqual(
      withdrawn.map(action => action.kind),
      ["merge"],
    );
    assert.deepEqual(
      actionsOf(file).map(action => action.kind),
      ["try"],
    );
    assert.deepEqual(changeOf(file), change);
  });

  it("ends with the merge its agent posts again", () => {
    const file = actionsFile(temp("change-"));
    requestChange(file, change);
    addAction(file, { kind: "try", text: "look", url: "/atlas" });
    assert.deepEqual(changeOf(file), change);
    addAction(file, { kind: "merge", text: "!7 built" });
    assert.equal(changeOf(file), null);
  });
});

describe("isChangeOpen", () => {
  const tree = fields => ({ change, pushed: "b", unpushed: 0, ...fields });

  it("is closed without a change", () => {
    assert.equal(isChangeOpen({ change: null }, green("b")), false);
    assert.equal(isChangeOpen(null, green("b")), false);
  });

  it("blocks until a commit after the request is pushed", () => {
    assert.equal(isChangeOpen(tree({ pushed: "a" }), green("a")), true);
    assert.equal(isChangeOpen(tree({ unpushed: 1 }), green("b")), true);
    assert.equal(isChangeOpen(tree({ pushed: null }), green("b")), true);
  });

  it("blocks until the pipeline of that commit passed", () => {
    assert.equal(
      isChangeOpen(tree(), { pipeline: { status: "running", sha: "b" } }),
      true,
    );
    assert.equal(isChangeOpen(tree(), green("a")), true);
    assert.equal(isChangeOpen(tree(), green("b")), false);
  });
});

describe("isUnpushedLong", () => {
  const now = Date.parse("2026-09-29T12:00:00Z");
  const at = ms => new Date(now - ms).toISOString();

  it("is long past UNPUSHED_MS", () => {
    assert.equal(
      isUnpushedLong({ unpushed: 2, unpushedAt: at(UNPUSHED_MS) }, now),
      true,
    );
    assert.equal(
      isUnpushedLong({ unpushed: 2, unpushedAt: at(UNPUSHED_MS - 1) }, now),
      false,
    );
    assert.equal(
      isUnpushedLong({ unpushed: 0, unpushedAt: at(UNPUSHED_MS) }, now),
      false,
    );
  });
});

describe("pushStateOf", () => {
  const git = (dir, ...args) =>
    execFileSync("git", args, { cwd: dir, encoding: "utf-8" }).trim();
  const commit = (dir, name) => {
    fs.writeFileSync(path.join(dir, name), name);
    git(dir, "add", name);
    git(dir, "commit", "-qm", name);
  };
  const repo = () => {
    const origin = temp("origin-");
    git(origin, "init", "-q", "--bare", "-b", "develop");
    const dir = temp("clone-");
    git(dir, "init", "-q", "-b", "develop");
    git(dir, "config", "user.email", "t@t");
    git(dir, "config", "user.name", "t");
    git(dir, "config", "commit.gpgsign", "false");
    git(dir, "remote", "add", "origin", origin);
    commit(dir, "base");
    git(dir, "push", "-q", "origin", "develop");
    git(dir, "switch", "-qc", "fix/9-thing");
    return dir;
  };

  it("counts the commits origin's branch doesn't have", () => {
    const dir = repo();
    commit(dir, "one");
    git(dir, "push", "-q", "origin", "fix/9-thing");
    commit(dir, "two");
    commit(dir, "three");
    const state = pushStateOf(dir, "fix/9-thing");
    assert.equal(state.unpushed, 2);
    assert.equal(state.hasRemoteBranch, true);
    assert.equal(state.pushed, git(dir, "rev-parse", "HEAD~2"));
    assert.ok(Date.parse(state.unpushedAt) <= Date.now());
  });

  it("counts against develop when origin has no branch", () => {
    const dir = repo();
    commit(dir, "one");
    assert.deepEqual(
      { ...pushStateOf(dir, "fix/9-thing"), unpushedAt: null },
      { pushed: null, unpushed: 1, unpushedAt: null, hasRemoteBranch: false },
    );
  });

  it("finds nothing unpushed once all is on origin", () => {
    const dir = repo();
    commit(dir, "one");
    git(dir, "push", "-q", "origin", "fix/9-thing");
    assert.equal(pushStateOf(dir, "fix/9-thing").unpushed, 0);
    assert.equal(pushStateOf(dir, "fix/9-thing").unpushedAt, null);
  });
});
