// Claude Code's IDE integration in a worktree's nvim (#258): the nvim tab runs
// coder/claudecode.nvim (nvim/ide.lua), which writes a lock file with the
// worktree as its workspace folder, and the worktree's claude starts with
// `--ide`: it connects to the one IDE whose folder holds its directory, so
// each worktree's claude finds its own nvim. An edit that asks first shows as
// a diff there; in auto mode claude asks nothing and opens none.
//
// The plugin is cloned once per machine into the main checkout's `.claude/`
// (git ignores it), pinned by tag and commit: nothing of the user's nvim or
// claude config changes. Off for the whole repo: `git config breadboard.ide false`.
import fs from "node:fs";
import path from "node:path";

import { git } from "../lib/proc.mjs";

/** The plugin, its tag and the commit the tag must point at. */
export const PLUGIN = {
  url: "https://github.com/coder/claudecode.nvim",
  tag: "v0.3.0",
  commit: "2e6ea6f2a63cdf4fd3c05e6a054151d46848d319",
};
/** What a worktree's nvim tab runs to start the plugin: nvim/ide.lua. */
export const IDE_LUA = path.join(
  import.meta.dirname,
  "..",
  "..",
  "nvim",
  "ide.lua",
);
/** Where the plugin lies: in `.claude/nvim/` of the main checkout `main`, one directory per tag. */
export const pluginDir = (main, plugin = PLUGIN) =>
  path.join(main, ".claude", "nvim", `claudecode.nvim-${plugin.tag}`);
/** Whether a value of `git config breadboard.ide` switches it off. */
export const isOff = value =>
  ["false", "off", "no", "0"].includes(value.trim().toLowerCase());
/** Whether the integration is on for the checkout `dir`: unless `git config breadboard.ide` says off. */
export const isIdeOn = dir =>
  !isOff(git(["config", "breadboard.ide"], dir).out);
/** ` --ide` for the claude of a worktree with the integration on, else nothing. */
export const ideFlag = dir => (isIdeOn(dir) ? " --ide" : "");
/**
 * The plugin's directory under the main checkout `main`, cloned first if it
 * is missing; null when the clone fails or its tag points at another commit
 * than the pinned one (then nothing of it stays). Comfort: nvim starts without
 * it rather than not at all.
 */
export const ensurePlugin = (main, plugin = PLUGIN) => {
  const dir = pluginDir(main, plugin);
  if (fs.existsSync(path.join(dir, "lua", "claudecode"))) return dir;
  fs.mkdirSync(path.dirname(dir), { recursive: true });
  const partial = `${dir}.partial`;
  fs.rmSync(partial, { recursive: true, force: true });
  const isCloned = git([
    "-c",
    "advice.detachedHead=false",
    "clone",
    "--quiet",
    "--depth",
    "1",
    "--branch",
    plugin.tag,
    plugin.url,
    partial,
  ]).ok;
  if (!isCloned || git(["rev-parse", "HEAD"], partial).out !== plugin.commit) {
    fs.rmSync(partial, { recursive: true, force: true });
    return null;
  }
  fs.renameSync(partial, dir);
  return dir;
};
/** The plugin for the nvim tab of the worktree `dir` (main checkout `main`): null when the integration is off or the plugin can't be had. */
export const ideFor = (dir, main) => (isIdeOn(dir) ? ensurePlugin(main) : null);
