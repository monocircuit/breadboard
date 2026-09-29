// Your own herdr (pnpm herdr:install, e.g. Omarchy's): whether it can load the
// plugin, how to get one that can, and the marked block in its config.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline/promises";
import zlib from "node:zlib";

import { fail } from "../lib/proc.mjs";
import { SPACE, sidebarConfig } from "../worktree.mjs";
import {
  BLOCK_BEGIN,
  BLOCK_END,
  BLOCK_MARK,
  PLUGIN,
  bin,
  herdr,
  pluginDir,
  step,
  withoutBlock,
} from "./session.mjs";
import { linkPlugin, writeLinked } from "./setup.mjs";

/** The version the manifest asks for (`min_herdr_version`): older herdrs list the plugin but don't load it. */
const MIN_MINOR = 9;
const versionOf = text => text?.match(/(\d+)\.(\d+)\.(\d+)/)?.[0] ?? null;
const canLoadPlugin = version => {
  const [major, minor] = version.split(".").map(Number);
  return major > 0 || minor >= MIN_MINOR;
};
/** Your herdr: the first on PATH that is not the pinned one in .herdr/bin, which panes of `platform` see first. */
export const ownHerdr = () =>
  (process.env.PATH ?? "")
    .split(path.delimiter)
    .filter(dir => path.resolve(dir) !== path.dirname(bin))
    .map(dir => path.join(dir, "herdr"))
    .find(file => fs.existsSync(file));
/** Omarchy installs herdr as a pacman package of its own repo, updated by omarchy-update. */
const isOmarchyPackage = binary =>
  binary.startsWith("/usr/") &&
  fs.existsSync(path.join(os.homedir(), ".local", "share", "omarchy"));
/**
 * The herdr package of Omarchy's edge channel, read from its repo database,
 * when it can load the plugin; null otherwise. Its stable channel can lag
 * behind (0.8.2 when 0.9 was out), and `herdr update` is no way out there: it
 * doesn't know pacman and would overwrite a file pacman owns.
 */
const omarchyEdgePackage = async () => {
  const arch = { x64: "x86_64", arm64: "aarch64" }[process.arch];
  if (!arch || !zlib.zstdDecompressSync) return null;
  try {
    const response = await fetch(
      `https://pkgs.omarchy.org/edge/${arch}/omarchy.db`,
    );
    if (!response.ok) return null;
    const db = zlib
      .zstdDecompressSync(Buffer.from(await response.arrayBuffer()))
      .toString("latin1");
    const file = db.match(/%FILENAME%\n(herdr-\d[^\n]*)/)?.[1];
    return file && canLoadPlugin(versionOf(file))
      ? `https://pkgs.omarchy.org/edge/${arch}/${file}`
      : null;
  } catch {
    return null;
  }
};
/** What to do about a herdr too old for the plugin. */
const upgradeHint = async (binary, version) => {
  const intro = `herdr ${version} (${binary}) doesn't load the plugin, it needs 0.${MIN_MINOR} or newer.`;
  if (!isOmarchyPackage(binary)) {
    return `${intro} Update it (herdr update, or the package manager it came from), or use plain pnpm herdr`;
  }
  const edge = await omarchyEdgePackage();
  const install = edge
    ? `Omarchy's edge channel has ${versionOf(path.basename(edge))}. Install that package through pacman, so the next omarchy-update carries on from it:\n\n    sudo pacman -U ${edge}\n\n`
    : "Omarchy has no newer herdr package yet: omarchy-update brings it once it has. ";
  return `${intro} ${install}herdr update doesn't work here, pacman owns ${binary}. Then restart your herdr (herdr server stop, it ends its panes) and run pnpm herdr:install again`;
};
/**
 * What the session `platform` adds to your config, as one marked block at the
 * end of your config.toml: the plugin's keys, and the sidebar rows that mark
 * an orchestration's spaces unless your config has rows of its own. Replaced
 * on every run, so new keys arrive by themselves. The first time it asks;
 * with no terminal to ask in, or a no, it prints the block to paste instead.
 * `isRenewOnly` (--reload): only a block that is there, no question. Returns
 * whether the file changed.
 */
export const writeOwnBlock = async (file, { isRenewOnly = false } = {}) => {
  const text = fs.existsSync(file) ? fs.readFileSync(file, "utf-8") : "";
  const start = text.indexOf(BLOCK_MARK);
  const end = start === -1 ? -1 : text.indexOf(BLOCK_END, start);
  const isThere = start !== -1 && end !== -1;
  const rows = sidebarConfig(withoutBlock(text));
  const keys = fs
    .readFileSync(path.join(pluginDir, "keys.toml"), "utf-8")
    .trimEnd();
  const block = [
    BLOCK_BEGIN,
    keys,
    ...(rows ? ["", rows] : []),
    BLOCK_END,
  ].join("\n");
  const shown = file.replace(os.homedir(), "~");

  if (!isThere && isRenewOnly) return false;
  if (!isThere) {
    let isWanted = false;
    if (process.stdin.isTTY) {
      const prompt = readline.createInterface({
        input: process.stdin,
        output: process.stdout,
      });
      isWanted = !/^n/i.test(
        await prompt.question(
          `? add the plugin's keys and sidebar rows to ${shown} as one marked block [Y/n] `,
        ),
      );
      prompt.close();
    }
    if (!isWanted) {
      console.log(
        `\nFor ${shown} (then herdr server reload-config):\n\n${block}\n`,
      );
      return false;
    }
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(
      file,
      `${text.trimEnd()}${text.trim() ? "\n\n" : ""}${block}\n`,
    );
    step(`the plugin's block appended to ${shown}`);
    return true;
  }

  const next = `${text.slice(0, start)}${block}${text.slice(end + BLOCK_END.length)}`;
  if (next === text) return false;
  fs.writeFileSync(file, next);
  step(`the plugin's block in ${shown} renewed`);
  return true;
};
/**
 * The plugin in your own herdr instead of the session `platform`: e.g. the
 * one Omarchy ships. Your herdr and its running server have to load the
 * plugin (>= 0.9); then it is linked, its keys go into your config, and the
 * space of the repo is built once — from then on the plugin's restore keeps
 * it at every start, as the session has the space.
 */
export const installIntoOwn = async () => {
  const own = ownHerdr();
  if (!own)
    fail("no herdr of your own on PATH: install one, or use plain pnpm herdr");
  const version = versionOf(herdr(["--version"], { binary: own }).out);
  if (!version) fail(`${own} --version names no version`);
  if (!canLoadPlugin(version)) fail(await upgradeHint(own, version));

  const server = herdr(["status", "server"], { binary: own }).out;
  const isServing = /status: running/.test(server);
  const serving = versionOf(server.match(/version: (\S+)/)?.[1]);
  if (isServing && serving && !canLoadPlugin(serving)) {
    fail(
      `your herdr is ${version}, its server still runs ${serving} and doesn't load the plugin. herdr server stop (it ends its panes), start herdr again, then pnpm herdr:install`,
    );
  }

  if (linkPlugin(own)) step(`plugin ${PLUGIN} linked from ${pluginDir}`);
  else step(`plugin ${PLUGIN} is linked from ${pluginDir}`);
  writeLinked(own);
  const isChanged = await writeOwnBlock(
    path.join(os.homedir(), ".config", "herdr", "config.toml"),
  );

  if (!isServing) {
    console.log(
      `\n✓ ${PLUGIN} is ready. Your herdr doesn't run: start it (Omarchy: super+ctrl+enter), then pnpm herdr:install again for the space ${SPACE}`,
    );
    return;
  }
  if (isChanged && herdr(["server", "reload-config"], { binary: own }).isOk)
    step("config reloaded");
  const restored = herdr(["plugin", "action", "invoke", `${PLUGIN}.restore`], {
    binary: own,
  });
  if (!restored.isOk) fail(`the layout failed: ${restored.out}`);
  console.log(
    `\n✓ ${PLUGIN} runs in your herdr ${version}: the space ${SPACE}, a space per worktree, the plugin's keys`,
  );
};
