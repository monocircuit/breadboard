import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { eventsOf } from "./notify.mjs";

const state = (extra = {}) => ({
  isAsking: false,
  mr: { iid: 7, status: "running" },
  hasRemote: true,
  ...extra,
});
const merged = async () => true;
const closed = async () => false;
const bodies = async (...args) =>
  (await eventsOf(...args)).map(event => event.body);

describe("eventsOf", () => {
  it("notifies nothing for the state found at the start", async () => {
    assert.deepEqual(
      await bodies(undefined, state({ isAsking: true }), merged),
      [],
    );
  });

  it("notifies an agent that starts asking, once", async () => {
    assert.deepEqual(await bodies(state(), state({ isAsking: true }), merged), [
      "its agent asks you something",
    ]);
    assert.deepEqual(
      await bodies(
        state({ isAsking: true }),
        state({ isAsking: true }),
        merged,
      ),
      [],
    );
  });

  it("notifies a pipeline that turns failed, not one that stays failed", async () => {
    const failed = state({ mr: { iid: 7, status: "failed" } });
    assert.deepEqual(await bodies(state(), failed, merged), [
      "the pipeline of !7 failed",
    ]);
    assert.deepEqual(await bodies(failed, failed, merged), []);
  });

  it("takes a failed pipeline of another MR for news of none", async () => {
    assert.deepEqual(
      await bodies(
        state({ mr: { iid: 6, status: "success" } }),
        state({ mr: { iid: 7, status: "failed" } }),
        merged,
      ),
      [],
    );
  });

  it("notifies an MR that left the open ones merged, not closed", async () => {
    assert.deepEqual(await bodies(state(), state({ mr: null }), merged), [
      "!7 is merged",
    ]);
    assert.deepEqual(await bodies(state(), state({ mr: null }), closed), []);
  });

  it("says nothing about MRs before GitLab answered", async () => {
    assert.deepEqual(
      await bodies(state({ hasRemote: false, mr: null }), state(), merged),
      [],
    );
    assert.deepEqual(
      await bodies(state({ hasRemote: false }), state({ mr: null }), merged),
      [],
    );
  });
});
