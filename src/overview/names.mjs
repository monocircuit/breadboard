// A worktree's name in the overview is pink, the colour herdr's sidebar gives
// branches (#227): a worktree reads the same in both. The pink comes from the
// config herdr runs with, as the sidebar takes it.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { hex } from "../lib/tty.mjs";
import { branchColourOf, mainDir } from "../worktree.mjs";

/** The configs herdr may run with, the one it was started with first. */
const configs = () =>
  [
    process.env.HERDR_CONFIG_PATH,
    path.join(mainDir(), ".herdr", "config.toml"),
    path.join(os.homedir(), ".config", "herdr", "config.toml"),
  ].filter(Boolean);
let paint = null;
/** `text` in the sidebar's pink; the config is read once. */
export const pink = text => {
  if (!paint) {
    const file = configs().find(candidate => fs.existsSync(candidate));
    paint = hex(branchColourOf(file ? fs.readFileSync(file, "utf-8") : ""));
  }
  return paint(text);
};
