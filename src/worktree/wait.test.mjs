import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  CONFLICT_MS,
  DEFAULT_HOURS,
  clearWait,
  isWaiting,
  lineOf,
  listOf,
  parseWait,
  recordWait,
  verdictOf,
  waitOf,
  waitsText,
} from "./wait.mjs";

const wait = { mrs: [213, 217], then: "open the MR", hours: 8 };
const mr = (iid, state, hasConflicts = false) => ({ iid, state, hasConflicts });

describe("parseWait", () => {
  it("takes MRs with and without !, each once, and --then", () => {
    assert.deepEqual(parseWait(["!213", "217", "!213", "--then", "go on"]), {
      mrs: [213, 217],
      then: "go on",
      hours: DEFAULT_HOURS,
    });
  });
  it("takes --flag=value and --hours", () => {
    assert.deepEqual(parseWait(["!5", "--then=a b", "--hours=0.5"]), {
      mrs: [5],
      then: "a b",
      hours: 0.5,
    });
  });
  it("refuses no MR, a stray word, a flag without its value, a bad --hours", () => {
    assert.ok(parseWait([]).error);
    assert.ok(parseWait(["--then", "x"]).error);
    assert.ok(parseWait(["!5", "soon"]).error);
    assert.ok(parseWait(["!5", "--then"]).error);
    assert.ok(parseWait(["!5", "--then", "--hours", "2"]).error);
    assert.ok(parseWait(["!5", "--hours", "0"]).error);
  });
});

describe("verdictOf", () => {
  it("waits while one is open, goes on once all are merged", () => {
    assert.deepEqual(verdictOf(wait, [mr(213, "merged"), mr(217, "opened")]), {
      status: "waiting",
      open: [217],
    });
    assert.equal(
      verdictOf(wait, [mr(213, "merged"), mr(217, "merged")]).status,
      "merged",
    );
  });
  it("counts an MR GitLab didn't answer for as not merged", () => {
    assert.equal(verdictOf(wait, [mr(213, "merged")]).status, "waiting");
  });
  it("ends when one is closed without a merge", () => {
    assert.deepEqual(verdictOf(wait, [mr(213, "merged"), mr(217, "closed")]), {
      status: "closed",
      mr: 217,
    });
  });
  it("waits out conflicts for a while, then ends", () => {
    const since = new Map();
    const found = [mr(213, "opened", true), mr(217, "opened")];
    assert.equal(verdictOf(wait, found, 0, since).status, "waiting");
    assert.equal(
      verdictOf(wait, found, CONFLICT_MS - 1, since).status,
      "waiting",
    );
    assert.deepEqual(verdictOf(wait, found, CONFLICT_MS, since), {
      status: "conflicts",
      mr: 213,
    });
  });
  it("starts the conflict time again once they are resolved", () => {
    const since = new Map();
    verdictOf(wait, [mr(213, "opened", true)], 0, since);
    verdictOf(wait, [mr(213, "opened")], 1, since);
    assert.equal(
      verdictOf(wait, [mr(213, "opened", true)], CONFLICT_MS, since).status,
      "waiting",
    );
  });
  it("ends past its limit", () => {
    const until = { ...wait, until: new Date(1000).toISOString() };
    assert.deepEqual(verdictOf(until, [mr(213, "merged")], 1000), {
      status: "timeout",
      open: [217],
    });
  });
});

describe("the lines", () => {
  it("lists MRs as a sentence", () => {
    assert.equal(listOf([1]), "!1");
    assert.equal(listOf([1, 2]), "!1 and !2");
    assert.equal(listOf([1, 2, 3]), "!1, !2 and !3");
    assert.equal(waitsText(wait), "waits for !213, !217");
  });
  it("says what to do once they are merged", () => {
    const merged = { status: "merged", mrs: wait.mrs };
    assert.equal(
      lineOf(wait, merged, 3),
      "!213 and !217 are merged: merge develop in (3 behind), then open the MR",
    );
    assert.equal(
      lineOf({ mrs: [5] }, merged, 0),
      "!5 is merged: develop is in already",
    );
    assert.equal(
      lineOf({ mrs: [5] }, merged),
      "!5 is merged: merge develop in",
    );
  });
  it("says why an early end came", () => {
    assert.match(
      lineOf(wait, { status: "closed", mr: 217 }),
      /^!217 was closed without a merge/,
    );
    assert.match(
      lineOf(wait, { status: "conflicts", mr: 213 }),
      /^!213 has had conflicts/,
    );
    assert.match(
      lineOf(wait, { status: "timeout", open: [217] }),
      /^waited 8 h for !213 and !217, !217 not merged yet/,
    );
    assert.equal(lineOf(wait, { status: "waiting", open: [217] }), null);
  });
});

describe("the record", () => {
  it("is written, read and cleared in the worktree", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wait-"));
    assert.equal(waitOf(dir), null);
    recordWait(dir, { ...wait, pid: process.pid });
    assert.deepEqual(waitOf(dir).mrs, [213, 217]);
    assert.equal(isWaiting(waitOf(dir)), true);
    clearWait(dir);
    assert.equal(waitOf(dir), null);
  });
  it("knows a command that is gone", () => {
    assert.equal(isWaiting({ pid: 2 ** 22 + 1 }), false);
    assert.equal(isWaiting({}), false);
  });
});
