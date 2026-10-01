import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { sidebarConfig } from "./sidebar.mjs";

const AGENT_ROWS =
  /\[\["state_icon", "workspace"\], \[\{ token = "\$wt_repo" \}, "agent"\], \[\{ token = "\$wt_state", dim = true, rules = /;

describe("sidebarConfig", () => {
  it("gives a config without sidebar rows both tables", () => {
    const rows = sidebarConfig("");
    assert.match(rows, /^\[ui\.sidebar\.spaces\]$/m);
    assert.match(rows, /^\[ui\.sidebar\.agents\]$/m);
    assert.match(rows, new RegExp(`^rows = ${AGENT_ROWS.source}`, "m"));
  });

  it("names no space twice on an agent card: the space above, the repo by the agent", () => {
    const rows = sidebarConfig("");
    const agentRows = rows.split("\n").find(line => line.includes("$wt_repo"));
    assert.equal(agentRows.split('"workspace"').length, 2);
    assert.doesNotMatch(agentRows, /"tab"|\$wt_head|\$wt_tree/);
  });

  it("takes the colours of [theme.custom], else its own", () => {
    const base = [
      "[theme.custom]",
      'blue = "#112233"',
      'red = "#AA0000"',
      "",
      "[ui]",
      'pink = "#000000"',
    ].join("\n");
    const rows = sidebarConfig(base);
    assert.match(rows, /\{ token = "\$orch1", fg = "#112233", bold = true \}/);
    assert.match(
      rows,
      /\{ starts_with = "asks you", fg = "#AA0000", bold = true \}/,
    );
    assert.match(rows, /\{ token = "\$wt_branch", fg = "#FF79C6" \}/);
    // Whom an agent waits for (#288): you before GitLab, GitLab blue.
    assert.match(
      rows,
      /"waits for you", fg = "#AA0000" \}.*"waits", fg = "#112233" \}.*"free", fg = "#94E2D5" \}/,
    );
  });

  it("leaves spaces rows of the config alone", () => {
    const rows = sidebarConfig('[ui.sidebar.spaces]\nrows = [["workspace"]]\n');
    assert.doesNotMatch(rows, /\[ui\.sidebar\.spaces\]/);
    assert.match(rows, /\[ui\.sidebar\.agents\]/);
  });

  it("keeps the agent rows of the config for other agents and gives claude its own", () => {
    const base = [
      "[ui.sidebar.agents]",
      'rows = [["state_icon", "tab"], ["workspace", "agent"]]',
      "",
      "[ui.sidebar.spaces]",
      "rows = []",
    ].join("\n");
    const rows = sidebarConfig(base);
    assert.doesNotMatch(rows, /^\[ui\.sidebar\.agents\]$/m);
    assert.match(rows, /^\[ui\.sidebar\.agents\.rows_by_agent\]$/m);
    assert.match(rows, new RegExp(`^claude = ${AGENT_ROWS.source}`, "m"));
  });

  it("is null when the config has all of it", () => {
    const base = [
      "[ui.sidebar.agents]",
      "rows = []",
      "[ui.sidebar.agents.rows_by_agent]",
      "claude = []",
      "[ui.sidebar.spaces]",
      "rows = []",
    ].join("\n");
    assert.equal(sidebarConfig(base), null);
  });
});
