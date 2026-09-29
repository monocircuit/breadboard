// Setting the session `platform` up: the pinned binary, its config, the
// server, a window of its own, the plugin link.
import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { fail } from "../lib/proc.mjs";
import { sidebarConfig } from "../worktree.mjs";
import {
  ASSETS,
  PLUGIN,
  SESSION,
  VERSION,
  bin,
  configFile,
  herdr,
  home,
  isRunning,
  main,
  pluginDir,
  sessionEnv,
  step,
  withoutBlock,
} from "./session.mjs";

/** The pinned binary in .herdr/bin, downloaded and checked when it is missing or another version. */
export const ensureBinary = async () => {
  if (fs.existsSync(bin) && herdr(["--version"]).out.includes(VERSION)) return;
  const asset = ASSETS[`${process.platform}-${process.arch}`];
  if (!asset)
    fail(
      `no herdr release for ${process.platform}-${process.arch}; install it yourself and use pnpm herdr:install`,
    );
  const [name, sha] = asset;

  step(`download herdr ${VERSION} (${name})`);
  const response = await fetch(
    `https://github.com/herdrdev/herdr/releases/download/v${VERSION}/${name}`,
  );
  if (!response.ok) fail(`the download answered ${response.status}`);
  const data = Buffer.from(await response.arrayBuffer());
  const actual = crypto.createHash("sha256").update(data).digest("hex");
  if (actual !== sha)
    fail(`${name} has sha256 ${actual}, pinned is ${sha}: not installed`);

  fs.mkdirSync(path.dirname(bin), { recursive: true });
  const partial = `${bin}.download`;
  fs.writeFileSync(partial, data, { mode: 0o755 });
  fs.renameSync(partial, bin);
};
/**
 * .herdr/config.toml: a base, the plugin's keys and the sidebar rows that mark
 * an orchestration's spaces appended. The base is yours if you have one —
 * .herdr/config.local.toml, else ~/.config/herdr/config.toml (only read) — so
 * the session looks like your herdr; else the repo's. Rewritten on every run,
 * so new keys of the plugin arrive by themselves. A base with sidebar rows of
 * its own keeps them (`hasOwnRows`): the origin tokens are then yours to add.
 */
/**
 * A config with the plugin's status first in the tab bar (`⬢ 2 stacks · 1
 * needs you`, `node run.mjs status`): put in front of the base's own
 * `tab_bar_right`, else given one of its own with herdr's zoom. Unchanged
 * when it has the entry already, as the repo's config.toml does.
 */
export const withStatusEntry = (text, run) => {
  if (text.includes(`${run} status`)) return text;
  const entry = `{ type = "command", command = "node ${run} status", interval_seconds = 10, timeout_seconds = 3 }`;
  const ui = text.search(/^\[ui\]\s*$/m);
  if (ui === -1)
    return `${text}\n\n[ui]\ntab_bar_right = [${entry}, { type = "zoom" }]`;
  const next = text.slice(ui + 1).search(/^\[/m);
  const end = next === -1 ? text.length : ui + 1 + next;
  const section = text.slice(ui, end);
  const withEntry = /^tab_bar_right\s*=\s*\[/m.test(section)
    ? section.replace(/^(tab_bar_right\s*=\s*\[)/m, `$1${entry}, `)
    : section.replace(
        /^\[ui\]\s*$/m,
        `[ui]\ntab_bar_right = [${entry}, { type = "zoom" }]`,
      );
  return `${text.slice(0, ui)}${withEntry}${text.slice(end)}`;
};

export const writeConfig = () => {
  const run = path.join(pluginDir, "run.mjs");
  const fill = text => text.replaceAll("{{RUN}}", run);
  const bases = [
    path.join(home, "config.local.toml"),
    path.join(os.homedir(), ".config", "herdr", "config.toml"),
    path.join(pluginDir, "config.toml"),
  ];
  const base = bases.find(file => fs.existsSync(file));
  // Without the block pnpm herdr:install keeps in your config: its keys and rows follow anyway.
  const baseText = withStatusEntry(
    fill(withoutBlock(fs.readFileSync(base, "utf-8"))).trimEnd(),
    run,
  );
  const rows = sidebarConfig(baseText);
  const text = [
    `# Written by pnpm herdr from ${base} and packages/workflow/herdr/keys.toml. Changes here are`,
    "# overwritten: put your own into .herdr/config.local.toml.",
    "",
    baseText,
    "",
    fill(fs.readFileSync(path.join(pluginDir, "keys.toml"), "utf-8")).trimEnd(),
    "",
    ...(rows ? [rows, ""] : []),
  ].join("\n");
  const isChanged =
    !fs.existsSync(configFile) || fs.readFileSync(configFile, "utf-8") !== text;
  if (isChanged) fs.writeFileSync(configFile, text);
  return {
    base,
    isChanged,
    hasOwnRows: /^\[ui\.sidebar\.spaces\]/m.test(baseText),
  };
};
/**
 * A new window of the terminal this runs in, running `command`, with no
 * padding: the terminal's padding would sit as a frame around herdr's chrome,
 * and only a window's start can drop it. null when the terminal is unknown or
 * there is no display — then herdr attaches right here.
 */
export function terminalWindow(command) {
  const { env } = process;
  if (!env.WAYLAND_DISPLAY && !env.DISPLAY && process.platform !== "darwin")
    return null;
  if (env.KITTY_WINDOW_ID || env.KITTY_PID) {
    return [
      "kitty",
      ["-o", "window_padding_width=0", "--directory", main, ...command],
    ];
  }
  if (env.ALACRITTY_WINDOW_ID || env.ALACRITTY_SOCKET) {
    return [
      "alacritty",
      [
        "-o",
        "window.padding.x=0",
        "-o",
        "window.padding.y=0",
        "--working-directory",
        main,
        "-e",
        ...command,
      ],
    ];
  }
  if (env.TERM_PROGRAM === "ghostty" || env.GHOSTTY_RESOURCES_DIR) {
    return [
      "ghostty",
      [
        "--window-padding-x=0",
        "--window-padding-y=0",
        `--working-directory=${main}`,
        "-e",
        ...command,
      ],
    ];
  }
  return null;
}
/** The session's server, started headless and detached: closing this terminal keeps it. */
export const startServer = async () => {
  step(`start the herdr session ${SESSION}`);
  const child = spawn(bin, ["--session", SESSION, "server"], {
    env: sessionEnv(),
    detached: true,
    stdio: "ignore",
  });
  child.unref();
  for (let tries = 0; tries < 50 && !isRunning(); tries++)
    await new Promise(resolve => setTimeout(resolve, 100));
  if (!isRunning())
    fail(
      `the server of the session ${SESSION} did not come up: ${bin} --session ${SESSION} status`,
    );
};
/** Links the plugin unless this very directory is linked already. */
export const linkPlugin = binary => {
  const listed = herdr(["plugin", "list"], { binary }).out;
  if (listed.includes(PLUGIN) && listed.includes(pluginDir)) return false;
  if (listed.includes(PLUGIN)) herdr(["plugin", "unlink", PLUGIN], { binary });
  const linked = herdr(["plugin", "link", pluginDir], { binary });
  if (!linked.isOk) fail(`herdr refused the plugin:\n${linked.out}`);
  return true;
};
/**
 * Where the manifest a herdr has linked is written down, to relink only when
 * it changed: one file for the session `platform`, one for your own herdr.
 */
export const linkedFileOf = binary =>
  path.join(
    home,
    binary === bin ? "plugin-manifest.sha256" : "plugin-manifest.own.sha256",
  );
export const manifestHash = () =>
  crypto
    .createHash("sha256")
    .update(fs.readFileSync(path.join(pluginDir, "herdr-plugin.toml")))
    .digest("hex");
export const writeLinked = binary => {
  fs.mkdirSync(home, { recursive: true });
  fs.writeFileSync(linkedFileOf(binary), `${manifestHash()}\n`);
};
