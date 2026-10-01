// The colour of an agent's state as herdr's sidebar paints its dot: working
// yellow, idle and done teal, blocked red, no agent dim. And whom an agent
// waits for (agent-state.mjs `agentStateOf`): at work yellow, GitLab blue,
// you red, free teal. The hues come from herdr's config (its theme sets
// `yellow`, `teal`, `red`, `blue`), so the overview matches the sidebar with
// every theme; without them the terminal's own.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { blue, cyan, dim, hex, red, yellow } from "../lib/tty.mjs";

/** herdr's config, where its theme writes the colours. */
const CONFIG = path.join(
  process.env.XDG_CONFIG_HOME ?? path.join(os.homedir(), ".config"),
  "herdr",
  "config.toml",
);
/** herdr state → the name of the colour its sidebar paints it with. */
export const STATE_COLOURS = {
  working: "yellow",
  idle: "teal",
  done: "teal",
  blocked: "red",
};
/** Whom an agent waits for (agentStateOf's `tone`) → the name of its colour; worktree/sidebar.mjs paints the same. */
export const WAIT_COLOURS = {
  work: "yellow",
  gitlab: "blue",
  you: "red",
  free: "teal",
};
/** Each name's colour when herdr's config has none: the terminal's. */
const FALLBACK = { yellow, teal: cyan, red, blue };
const NAMES = Object.keys(FALLBACK);
/** `name = "#rrggbb"` of each colour name in a herdr config's text; the last one wins, as in TOML sections read in order. */
export const coloursOf = text => {
  const colours = {};
  for (const [, name, colour] of text.matchAll(
    /^\s*(\w+)\s*=\s*"(#[\da-fA-F]{6})"/gm,
  ))
    if (NAMES.includes(name)) colours[name] = colour;
  return colours;
};
/** herdr state → paint, from `colours` (coloursOf); a missing one falls back to the terminal's. */
export const tonesOf = (colours = {}, names = STATE_COLOURS) =>
  Object.fromEntries(
    Object.entries(names).map(([state, name]) => [
      state,
      colours[name] ? hex(colours[name]) : FALLBACK[name],
    ]),
  );
const readColours = () => {
  try {
    return coloursOf(fs.readFileSync(CONFIG, "utf-8"));
  } catch {
    return {};
  }
};
let tones = null;
let waits = null;
/**
 * The paint of herdr's state `status` (`working`, `idle`, `done`,
 * `blocked`); dim for none (no agent) or one herdr doesn't name.
 */
export const toneOf = status => {
  tones ??= tonesOf(readColours());
  return tones[status] ?? dim;
};
/** The paint of whom an agent waits for (agentStateOf's `tone`: `work`, `gitlab`, `you`, `free`); dim for another. */
export const paintOf = tone => {
  waits ??= tonesOf(readColours(), WAIT_COLOURS);
  return waits[tone] ?? dim;
};
