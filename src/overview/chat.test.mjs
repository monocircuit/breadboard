import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { chatText, clickChat } from "./chat.mjs";

const ID = "00000000-0000-4000-8000-000000000287";

describe("chatText", () => {
  it("says a click only focuses a live orchestrator", () => {
    assert.match(chatText({ state: "alive", sessionId: ID }), /focuses it/);
  });

  it("says which conversation a click resumes once its claude is gone", () => {
    assert.match(
      chatText({ state: "idle", sessionId: ID }),
      /claude is gone · a click resumes 00000000…/,
    );
    assert.match(
      chatText({ state: "missing", sessionId: ID }),
      /tab is gone · a click resumes/,
    );
  });

  it("says a fresh claude starts when none is recorded", () => {
    assert.match(
      chatText({ state: "missing", sessionId: null }),
      /fresh claude, none is recorded/,
    );
  });
});

describe("clickChat", () => {
  it("restores as a click, and shows what happened", () => {
    const asked = [];
    const said = clickChat("zz-chat", (name, options) => {
      asked.push([name, options]);
      return "focused the orchestrator";
    });
    assert.deepEqual(asked, [["zz-chat", { isClick: true }]]);
    assert.equal(said, "focused the orchestrator");
  });
});
