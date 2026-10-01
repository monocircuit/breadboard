import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { keyReader, mouseOf } from "./mouse.mjs";

describe("mouseOf", () => {
  it("reads a move, with no button and with one held, never as a click", () => {
    assert.deepEqual(mouseOf("\x1b[<35;26;19M"), {
      kind: "move",
      row: 19,
      column: 26,
    });
    assert.equal(mouseOf("\x1b[<32;4;2M").kind, "move");
    assert.equal(mouseOf("\x1b[<48;4;2M").kind, "move");
  });

  it("reads a click of the left button alone", () => {
    assert.deepEqual(mouseOf("\x1b[<0;7;3M"), {
      kind: "click",
      row: 3,
      column: 7,
    });
  });

  it("reads the wheel, with a modifier too", () => {
    assert.equal(mouseOf("\x1b[<64;1;1M").by, -1);
    assert.equal(mouseOf("\x1b[<65;1;1M").by, 1);
    assert.equal(mouseOf("\x1b[<81;1;1M").by, 1);
  });

  it("ignores releases, other buttons, modified clicks and keys", () => {
    for (const key of ["\x1b[<0;7;3m", "\x1b[<2;7;3M", "\x1b[<16;7;3M", "j"])
      assert.equal(mouseOf(key), null);
  });
});

describe("keyReader", () => {
  it("splits a chunk into keys and reports", () => {
    const keysOf = keyReader();
    assert.deepEqual(keysOf("j\x1b[<35;1;2M\x1b[O"), [
      "j",
      "\x1b[<35;1;2M",
      "\x1b[O",
    ]);
  });

  it("keeps a report cut off at the end of a chunk for the next", () => {
    const keysOf = keyReader();
    assert.deepEqual(keysOf("\x1b[<35;1;2M\x1b[<35;1"), ["\x1b[<35;1;2M"]);
    assert.deepEqual(keysOf("0;2Mk"), ["\x1b[<35;10;2M", "k"]);
  });
});
