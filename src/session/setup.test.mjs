import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { withStatusEntry } from "./setup.mjs";

const RUN = "/repo/packages/breadboard/herdr/run.mjs";
const ENTRY = `{ type = "command", command = "node ${RUN} status", interval_seconds = 10, timeout_seconds = 3 }`;

describe("withStatusEntry", () => {
  it("puts the status in front of the config's own tab bar", () => {
    const text =
      '[ui]\nsidebar_width = 34\ntab_bar_right = [{ type = "zoom" }, { type = "hostname" }]\n\n[keys]\n';
    assert.equal(
      withStatusEntry(text, RUN),
      `[ui]\nsidebar_width = 34\ntab_bar_right = [${ENTRY}, { type = "zoom" }, { type = "hostname" }]\n\n[keys]\n`,
    );
  });

  it("gives [ui] a tab bar when it has none, and leaves a tab bar elsewhere alone", () => {
    const text = '[ui]\naccent = "cyan"\n\n[other]\ntab_bar_right = []\n';
    const result = withStatusEntry(text, RUN);
    assert.ok(
      result.startsWith(
        `[ui]\ntab_bar_right = [${ENTRY}, { type = "zoom" }]\naccent = "cyan"`,
      ),
    );
    assert.ok(result.endsWith("[other]\ntab_bar_right = []\n"));
  });

  it("adds [ui] to a config without one", () => {
    assert.equal(
      withStatusEntry("[keys]", RUN),
      `[keys]\n\n[ui]\ntab_bar_right = [${ENTRY}, { type = "zoom" }]`,
    );
  });

  it("leaves a config that runs the status already", () => {
    const text = `[ui]\ntab_bar_right = [{ type = "command", command = "node ${RUN} status" }]`;
    assert.equal(withStatusEntry(text, RUN), text);
  });
});
