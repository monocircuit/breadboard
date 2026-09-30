// The colour of an agent's state as herdr's sidebar paints its dot: working
// yellow, idle and done teal, blocked red, no agent dim. The hues come from
// herdr's config (its theme sets `yellow`, `teal`, `red`), so the overview
// matches the sidebar with every theme; without them the terminal's own.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { cyan, dim, hex, red, yellow } from "../lib/tty.mjs";

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
/** Each name's colour when herdr's config has none: the terminal's. */
const FALLBACK = { yellow, teal: cyan, red };
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
export const tonesOf = (colours = {}) =>
  Object.fromEntries(
    Object.entries(STATE_COLOURS).map(([state, name]) => [
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
/**
 * The paint of herdr's state `status` (`working`, `idle`, `done`,
 * `blocked`); dim for none (no agent) or one herdr doesn't name.
 */
export const toneOf = status => {
  tones ??= tonesOf(readColours());
  return tones[status] ?? dim;
};
