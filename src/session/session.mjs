// The herdr session `platform` of pnpm herdr: the pinned version, where its
// binary and config live, and its CLI.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import { mainDir } from "../worktree.mjs";

/**
 * The herdr the plugin is tested with. A bump is a commit: new version, the
 * sha256 of each asset from the release (gh api repos/herdrdev/herdr/releases/tags/v<version>).
 */
export const VERSION = "0.9.1";
export const ASSETS = {
  "linux-x64": [
    "herdr-linux-x86_64",
    "2a02fed16beb651ef006e1d43f048f652ca4dc58ad053cd2d44450563d5c54b7",
  ],
  "linux-arm64": [
    "herdr-linux-aarch64",
    "f4ccf4de745f2cb9a39a983e9ba3703dad50ec2a58dea83026ceab721bbd8d9e",
  ],
  "darwin-arm64": [
    "herdr-macos-aarch64",
    "5fc7a7e7adfaca56fa80aa89dcb025693357268dab8285b9ce2d08a2313c89de",
  ],
  "darwin-x64": [
    "herdr-macos-x86_64",
    "053be0639935fe54ab5efbdb46651054e4f6a753a5b43153c88bd6912bce1e94",
  ],
};
export const SESSION = "platform";
export const PLUGIN = "monocircuit.platform";
// Tighter than worktree's steps: no empty line between them.
export const step = message => console.log(`▸ ${message}`);
export const main = mainDir();
export const home = path.join(main, ".herdr");
export const bin = path.join(home, "bin", "herdr");
export const configFile = path.join(home, "config.toml");
/** The plugin of the main checkout once it is merged, else the one of the checkout you run this in. */
export const pluginDir = [
  path.join(main, "packages", "breadboard", "herdr"),
  path.resolve(import.meta.dirname, "..", "..", "herdr"),
].find(dir => fs.existsSync(path.join(dir, "herdr-plugin.toml")));
/**
 * process.env without what a pane must not inherit: the variables of a herdr
 * this was started from, and those of a Claude Code session. `pnpm herdr` run
 * by an agent would otherwise hand CLAUDECODE and CLAUDE_CODE_CHILD_SESSION to
 * every pane, and a `claude` started there takes itself for a child session
 * and saves no transcript.
 */
export const cleanEnv = () =>
  Object.fromEntries(
    Object.entries(process.env).filter(
      ([key]) => !/^(HERDR_|CLAUDE)/.test(key),
    ),
  );
/**
 * The environment of the session's server, and so of every pane in it: the
 * pinned herdr first on PATH (wt:new and the plugin call `herdr`), the repo's
 * config, and nothing of a herdr or an agent this was started from.
 */
export const sessionEnv = () => {
  const env = cleanEnv();
  return {
    ...env,
    PATH: `${path.dirname(bin)}${path.delimiter}${env.PATH}`,
    HERDR_CONFIG_PATH: configFile,
  };
};
export const herdr = (args, { binary = bin, isQuiet = true } = {}) => {
  const result = spawnSync(
    binary,
    binary === bin ? ["--session", SESSION, ...args] : args,
    {
      encoding: "utf-8",
      // Your own herdr talks to your default session, also when this runs in a pane of `platform`.
      env: binary === bin ? sessionEnv() : cleanEnv(),
      stdio: isQuiet ? "pipe" : "inherit",
    },
  );
  return {
    isOk: result.status === 0,
    out: `${result.stdout ?? ""}${result.stderr ?? ""}`.trim(),
  };
};
export const isRunning = () =>
  /status: running/.test(herdr(["status", "server"]).out);
export const serverVersion = () =>
  herdr(["status", "server"]).out.match(/version: (\S+)/)?.[1] ?? null;
/** `herdr <args>` of the session → its `result`, or null. */
export const ask = (args, { binary = bin } = {}) => {
  const { isOk, out } = herdr(args, { binary });
  try {
    return isOk ? JSON.parse(out).result : null;
  } catch {
    return null;
  }
};
/** This process runs in a pane of the session `platform`. */
export const isInPlatform = () =>
  (process.env.HERDR_SOCKET_PATH ?? "").includes(
    `${path.sep}sessions${path.sep}${SESSION}${path.sep}`,
  );

/** The block pnpm herdr:install keeps in your config.toml; a line starting with the marker counts, whatever follows it. */
export const BLOCK_MARK = `# >>> ${PLUGIN}`;
export const BLOCK_BEGIN = `${BLOCK_MARK}: written by pnpm herdr:install on every run, changes here are overwritten`;
export const BLOCK_END = `# <<< ${PLUGIN}`;
/** A config text without the marked block: what is yours in it. */
export const withoutBlock = text => {
  const start = text.indexOf(BLOCK_MARK);
  const end = start === -1 ? -1 : text.indexOf(BLOCK_END, start);
  return start === -1 || end === -1
    ? text
    : `${text.slice(0, start)}${text.slice(end + BLOCK_END.length)}`;
};
