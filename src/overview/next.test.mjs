import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  namedIn,
  nextOf,
  nothingText,
  onceText,
  staleOf,
  updatedPlan,
  whyOf,
} from "./next.mjs";

const at = "2026-10-01T10:00:00Z";
const row = (kind, extra = {}) => ({
  who: "a",
  go: () => {},
  ...extra,
  action: { kind, text: kind, at, ...extra.action },
});
const ready = { isReady: true, iid: 286, holdUp: [] };
const held = {
  isReady: false,
  iid: 286,
  holdUp: ["draft", "pipeline running"],
};

describe("nextOf", () => {
  it("names the first action not answered and not a waiting try", () => {
    const answered = row("decide", { action: { answer: "Go" } });
    const waiting = row("try", { tryState: { isReady: false } });
    const look = row("look");
    const next = nextOf({
      actions: [answered, waiting, look],
      upNext: [row("order")],
      merge: ready,
      isWaiting: item => item.tryState?.isReady === false,
    });
    assert.deepEqual(next, { kind: "action", item: look });
  });

  it("names the first clickable UP NEXT row before a ready merge", () => {
    const busy = row("hand over", { action: { isRunning: true } });
    const order = row("order");
    assert.deepEqual(
      nextOf({ actions: [], upNext: [busy, order], merge: ready }),
      { kind: "up next", item: order },
    );
  });

  it("names a ready merge when nothing else is to do", () => {
    const busy = row("hand over", { action: { isRunning: true } });
    assert.deepEqual(nextOf({ actions: [], upNext: [busy], merge: ready }), {
      kind: "merge",
      merge: ready,
    });
  });

  it("names nothing for a draft or a running pipeline", () => {
    assert.equal(nextOf({ actions: [], upNext: [], merge: held }), null);
    assert.equal(nextOf({ actions: [], upNext: [], merge: null }), null);
  });
});

describe("nothingText", () => {
  it("says nothing is for the user, who is at work and what the next merge waits for", () => {
    assert.equal(
      nothingText({ atWork: 2, merge: held }),
      "nothing for you · 2 agents at work · next merge: !286 once its pipeline passes",
    );
    assert.equal(
      nothingText({ atWork: 1 }),
      "nothing for you · 1 agent at work",
    );
    assert.equal(nothingText(), "nothing for you");
  });

  it("names what an MR waits for in words", () => {
    assert.equal(onceText(["after !223 (shares view.mjs)"]), "!223 is merged");
    assert.equal(
      onceText([
        "draft",
        "pipeline failed",
        "conflicts with develop (3 behind)",
      ]),
      "its conflicts are resolved",
    );
    assert.equal(onceText(["draft"]), "its agent posts the merge");
    assert.equal(onceText(["something new"]), "something new");
  });
});

describe("the plan's why and staleness", () => {
  const states = new Map([
    [256, "merged"],
    [262, "closed"],
    [273, "open"],
    [278, "open"],
  ]);
  const plan = {
    work: [273, 278],
    merge: [256, 273, 278],
    why: "#273 first: #278 builds on its NEXT",
  };

  it("finds the issues a why names", () => {
    assert.deepEqual(namedIn("#256 then #262, #256 again"), [256, 262]);
    assert.deepEqual(namedIn(null), []);
  });

  it("holds a plan whose issues are open or merged and whose why names open ones", () => {
    assert.deepEqual(staleOf({ plan, states }), []);
  });

  it("is stale with an issue closed or gone, a why naming done ones, or the order off", () => {
    const stale = staleOf({
      plan: {
        work: [262, 299],
        merge: [256, 273],
        why: "#256 then the overall changelog; #262 touches workers",
      },
      states,
      isOffPlan: true,
    });
    assert.deepEqual(stale, [
      "#262 closed",
      "#299 no longer in the orchestration",
      "its why names #256, #262, done or gone",
      "the merge order left the plan's",
    ]);
  });

  it("shows the why only under a merge, never once the plan is stale", () => {
    const merge = { kind: "merge", merge: ready };
    const mergeAction = { kind: "action", item: row("merge") };
    assert.equal(whyOf(plan, [], merge), plan.why);
    assert.equal(whyOf(plan, [], mergeAction), plan.why);
    assert.equal(whyOf(plan, [], { kind: "action", item: row("look") }), null);
    assert.equal(whyOf(plan, [], null), null);
    assert.equal(whyOf(plan, ["#262 closed"], merge), null);
  });

  it("updates a stale plan to the computed order, dropping the done why", () => {
    assert.deepEqual(
      updatedPlan({
        plan: { work: [262, 273, 299], merge: [262], why: "#262 first" },
        states,
        planned: [256, 278, 273, 262],
      }),
      { work: [273], merge: [256, 278, 273], why: "" },
    );
    assert.equal(updatedPlan({ plan, states, planned: [] }).why, plan.why);
  });
});
