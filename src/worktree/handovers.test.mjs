import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  afterArgs,
  handoversOf,
  parseAfter,
  withHandover,
  withoutHandover,
} from "./handovers.mjs";

describe("afterArgs", () => {
  it("takes the MRs behind --after, not the issue that follows", () => {
    const argv = ["orch", "--to", "224-x", "--after", "!232", "!233", "226"];
    assert.deepEqual(afterArgs(argv), {
      values: ["!232", "!233"],
      at: [3, 4, 5],
    });
    assert.deepEqual(afterArgs(["orch", "226", "--after=!232,!233"]), {
      values: ["!232,!233"],
      at: [2],
    });
    assert.equal(afterArgs(["orch", "226", "--to", "x"]), null);
  });
});

describe("parseAfter", () => {
  it("reads MRs with or without their !, each once", () => {
    assert.deepEqual(parseAfter(["!232,233", "!232"]), { after: [232, 233] });
    assert.match(parseAfter(["226-x"]).error, /no MR/);
    assert.match(parseAfter([]).error, /--after !232/);
  });
});

describe("the record's handovers", () => {
  it("keeps one per issue, a new one replacing the old, and drops a done one", () => {
    const record = { issues: [], plan: { work: [1] } };
    const first = withHandover(record, { number: 226, to: "a", after: [232] });
    const again = withHandover(first, { number: 226, to: "b", after: [233] });
    assert.deepEqual(handoversOf(again), [
      { number: 226, to: "b", after: [233] },
    ]);
    assert.deepEqual(again.plan, record.plan);
    assert.deepEqual(handoversOf(withoutHandover(again, 226)), []);
    assert.deepEqual(handoversOf(null), []);
  });
});
