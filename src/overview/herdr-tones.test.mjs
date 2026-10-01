import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  STATE_COLOURS,
  WAIT_COLOURS,
  coloursOf,
  paintOf,
  toneOf,
  tonesOf,
} from "./herdr-tones.mjs";

describe("herdr tones", () => {
  it("paints each state as herdr's sidebar does", () => {
    assert.deepEqual(STATE_COLOURS, {
      working: "yellow",
      idle: "teal",
      done: "teal",
      blocked: "red",
    });
  });

  it("reads the colours herdr's theme wrote into its config", () => {
    const config = `[theme.custom]
accent = "#EFEFEF"
red = "#FF5C6C"    # blocked
yellow = "#FFD84D" # working
teal = "#5CE1E6"
green = "#7CFF6B"
`;
    assert.deepEqual(coloursOf(config), {
      red: "#FF5C6C",
      yellow: "#FFD84D",
      teal: "#5CE1E6",
    });
  });

  it("has a paint for every state, dim for no agent", () => {
    const tones = tonesOf({ teal: "#5CE1E6" });
    for (const state of Object.keys(STATE_COLOURS))
      assert.equal(typeof tones[state], "function");
    // Without a terminal every paint gives the plain text.
    assert.equal(toneOf(null)("no agent"), "no agent");
    assert.equal(toneOf("working")("at work"), "at work");
  });

  it("paints whom an agent waits for, each in a colour of its own (#288)", () => {
    const names = Object.values(WAIT_COLOURS);
    assert.equal(new Set(names).size, names.length);
    assert.equal(WAIT_COLOURS.work, STATE_COLOURS.working);
    assert.equal(
      paintOf("gitlab")("waits for the pipeline"),
      "waits for the pipeline",
    );
    assert.equal(paintOf(null)("no agent"), "no agent");
  });
});
