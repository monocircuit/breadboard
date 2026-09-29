import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { THRESHOLD, costOf, freeMemory, gb, stackFor } from "./memory.mjs";
import { startsOf } from "./fit.mjs";

const GIB = 1024 ** 3;

describe("freeMemory", () => {
  it("reads MemAvailable, else asks node", () => {
    const file = path.join(
      fs.mkdtempSync(path.join(os.tmpdir(), "memory-")),
      "meminfo",
    );
    fs.writeFileSync(
      file,
      "MemTotal:       16000000 kB\nMemAvailable:    2097152 kB\n",
    );
    assert.equal(freeMemory(file), 2 * GIB);
    assert.equal(typeof freeMemory("/nonexistent/meminfo"), "number");
  });
});

describe("stackFor", () => {
  it("gives a fresh agent its own stack while it fits, the shared one below the threshold", () => {
    assert.deepEqual(stackFor({ free: 4 * GIB }), {
      isShared: false,
      why: null,
    });
    const short = stackFor({ free: 4 * GIB, reserved: 3 * GIB });
    assert.equal(short.isShared, true);
    assert.equal(
      short.why,
      `1.0 GB free, below ${gb(THRESHOLD)} (own stack ~1.0 GB + agent ~0.5 GB): on the shared stack`,
    );
  });
  it("keeps --shared without a reason", () => {
    assert.deepEqual(stackFor({ isShared: true, free: 0 }), {
      isShared: true,
      why: null,
    });
  });
  it("counts a stack only when it is an own one", () => {
    assert.equal(costOf({ isShared: false }), 1.5 * GIB);
    assert.equal(costOf({ isShared: true }), 0.5 * GIB);
  });
});

describe("startsOf", () => {
  const item = (number, description = "") => ({ number, description });

  it("starts a fifth issue at once, next to four at work", () => {
    const items = [1, 2, 3, 4, 5].map(number => item(number));
    const starts = startsOf(items, [], { free: 64 * GIB });
    assert.equal(starts.size, 5);
    for (const one of items)
      assert.deepEqual(starts.get(one), {
        stack: { isShared: false, why: null },
      });
  });
  it("gives an issue to the idle agent that knows its files", () => {
    const worker = { name: "3-x", share: 0.1, files: ["src/a.mjs"] };
    const [fits, other] = [item(8, "touches `src/a.mjs`"), item(9)];
    const starts = startsOf([fits, other], [worker], { free: 64 * GIB });
    assert.deepEqual(starts.get(fits), { worker, shared: ["src/a.mjs"] });
    assert.equal(starts.get(other).stack.isShared, false);
  });
  it("puts the fresh ones past the free memory on the shared stack, still at once", () => {
    const items = [1, 2, 3].map(number => item(number));
    const starts = startsOf(items, [], { free: 3.5 * GIB });
    assert.deepEqual(
      items.map(one => starts.get(one).stack.isShared),
      [false, false, true],
    );
  });
});
