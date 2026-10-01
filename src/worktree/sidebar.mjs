// The rows of herdr's sidebar the workflow adds to a config: the spaces show
// an orchestration's mark and the branch, an agent its space, its repo and
// what it waits for. Written by pnpm herdr into .herdr/config.toml and by pnpm
// herdr:install into the marked block of your own config.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { ORIGIN_SLOTS, originToken } from "./origins.mjs";
import { BRANCH_TOKEN } from "./tabs.mjs";
import { mainDir } from "./worktrees.mjs";

/** The token under a worktree's agent: what it waits for, `waits for you: merge · !192`, … (overview/agent-state.mjs). */
export const STATE_TOKEN = "wt_state";
/** The repo of an agent in a worktree's space: `Monocircuit/Platform` (overview/agent-repo.mjs). */
export const REPO_TOKEN = "wt_repo";

/** The colour of the branches in the sidebar: `pink` of the base config's `[theme.custom]`, else this. */
const BRANCH_COLOUR = { colour: "pink", fallback: "#FF79C6" };
/**
 * The colour of what a worktree's agent waits for, by whom (#288, as the
 * overview's WORKTREES paints it, overview/herdr-tones.mjs): you red, a
 * question the loudest; at work yellow; GitLab (the pipeline, another MR, a
 * handover after one) blue; free teal. The first rule that matches wins:
 * `waits for you` before `waits`.
 */
const RED = { colour: "red", fallback: "#F38BA8" };
const STATE_RULES = [
  { prefix: "asks you", ...RED, isBold: true },
  { prefix: "waits for you", ...RED },
  { prefix: "pipeline failed", ...RED },
  { prefix: "change requested", ...RED },
  { prefix: "not pushed", ...RED },
  { prefix: "at work", colour: "yellow", fallback: "#F9E2AF" },
  { prefix: "waits", colour: "blue", fallback: "#89B4FA" },
  { prefix: "→", colour: "blue", fallback: "#89B4FA" },
  { prefix: "free", colour: "teal", fallback: "#94E2D5" },
];

/** The section `[name]` of a TOML text, up to the next table, or null. */
const sectionOf = (text, name) =>
  text.split(/^(?=\[)/m).find(section => section.startsWith(`[${name}]`)) ??
  null;

/**
 * The sidebar rows for a config `base`, or null when it has all of them
 * already. Colours are the names of the base's `[theme.custom]`, else the
 * fallbacks.
 * - spaces: herdr's default rows with the orchestration tokens in front of
 *   the branch. Not when the base has `[ui.sidebar.spaces]`: TOML takes a
 *   table once, and the user's wins.
 * - agents (claude): the space by the state icon, the repo (`$wt_repo`) by
 *   the agent, what it waits for (`$wt_state`) below; a row whose tokens are
 *   all empty doesn't show, an empty token takes no room. A base without
 *   `[ui.sidebar.agents]` gets them as its rows; one with rows of its own
 *   keeps them for other agents and gets these for claude (`rows_by_agent`,
 *   as TOML can't add to a table defined above). Not when the base has
 *   `rows_by_agent` already.
 */
/** The hex of `colour` in the `[theme.custom]` of the config text `base`, else its fallback. */
export const colourIn = (base, { colour, fallback }) =>
  (sectionOf(base, "theme.custom") ?? "").match(
    new RegExp(`^${colour}\\s*=\\s*"(#[0-9A-Fa-f]{6})"`, "m"),
  )?.[1] ?? fallback;
/** The text of the config herdr runs with: the one it was started with, the repo's, your own; empty without one. */
export const herdrConfigText = () => {
  const file = [
    process.env.HERDR_CONFIG_PATH,
    path.join(mainDir(), ".herdr", "config.toml"),
    path.join(os.homedir(), ".config", "herdr", "config.toml"),
  ]
    .filter(Boolean)
    .find(candidate => fs.existsSync(candidate));
  return file ? fs.readFileSync(file, "utf-8") : "";
};
/** The pink herdr's sidebar gives branches, from the config text `base`: the overview names worktrees in it too. */
export const branchColourOf = base => colourIn(base, BRANCH_COLOUR);
export const sidebarConfig = base => {
  const colourOf = slot => colourIn(base, slot);
  const parts = [];

  if (!sectionOf(base, "ui.sidebar.spaces")) {
    const tokens = ORIGIN_SLOTS.map(
      (slot, i) =>
        `{ token = "$${originToken(i)}", fg = "${colourOf(slot)}", bold = true }`,
    );
    const pink = colourOf(BRANCH_COLOUR);
    const branches = [
      `{ token = "branch", fg = "${pink}" }`,
      `{ token = "$${BRANCH_TOKEN}", fg = "${pink}" }`,
    ];
    parts.push(
      "# The spaces of an orchestration (pnpm wt:orchestrate) show `⇶ <title>` (its name without one) in its colour, written by pnpm herdr.",
      "[ui.sidebar.spaces]",
      "# A worktree's branch is $wt_branch: herdr's branch stays empty for a linked worktree. Both in pink.",
      `rows = [["state_icon", "workspace"], [${[...tokens, ...branches].join(", ")}, "git_status"]]`,
    );
  }

  const agents = sectionOf(base, "ui.sidebar.agents");
  if (!sectionOf(base, "ui.sidebar.agents.rows_by_agent")) {
    const rules = STATE_RULES.map(
      rule =>
        `{ starts_with = "${rule.prefix}", fg = "${colourOf(rule)}"${rule.isBold ? ", bold = true" : ""} }`,
    );
    const state = `[{ token = "$${STATE_TOKEN}", dim = true, rules = [${rules.join(", ")}] }]`;
    const rows = `[["state_icon", "workspace"], [{ token = "$${REPO_TOKEN}" }, "agent"], ${state}]`;
    if (parts.length) parts.push("");
    parts.push(
      "# A claude agent: its space, then its repo (a worktree's) and the agent, then what it waits for:",
      "# asks you, waits for you: merge, waits for the pipeline, at work, free, … (the overview reports both).",
      ...(agents
        ? ["[ui.sidebar.agents.rows_by_agent]", `claude = ${rows}`]
        : ["[ui.sidebar.agents]", `rows = ${rows}`]),
    );
  }
  return parts.length ? parts.join("\n") : null;
};
