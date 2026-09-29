#!/usr/bin/env node
// A url into the browser, in the tab group of the herdr space it belongs to.
//
//   pnpm browser    put the extension where the browser loads it from, say how to load it
//
// Chromium has no command line for tab groups, an extension does: the one in
// packages/breadboard/chromium (see its README). With Chromium and the extension there, the
// url opens through its page, which moves the tab into the group; otherwise
// the desktop's browser opens it as before, without a group.
//
// The browser loads the extension from a copy outside the repo (INSTALL_DIR),
// not from a checkout: a worktree goes away, and the main checkout only gets a
// change of the extension with the merge. A browser that finds the directory
// missing at its start complains in a dialog. Every use here brings the copy
// up to the extension of the checkout that runs it.
//
// No dependencies, like worktree.mjs.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Fixed by the `key` of packages/breadboard/chromium/manifest.json. */
const EXTENSION_ID = "aikcnbeljldocaafkokjbngkboecmoli";
const SOURCE = path.join(import.meta.dirname, "..", "chromium");
const INSTALL_DIR = path.join(
  process.env.XDG_DATA_HOME ?? path.join(os.homedir(), ".local", "share"),
  "monocircuit",
  "chromium-extension",
);
const BROWSERS = [
  "chromium",
  "chromium-browser",
  "google-chrome-stable",
  "google-chrome",
];
/** Where each browser keeps its profiles and its flags file (Arch reads `<name>-flags.conf`). */
const CONFIG = {
  chromium: "chromium",
  "chromium-browser": "chromium",
  "google-chrome-stable": "google-chrome",
  "google-chrome": "google-chrome",
};

const onPath = command =>
  spawnSync("which", [command], { stdio: "ignore" }).status === 0;
const configOf = browser =>
  path.join(
    process.env.XDG_CONFIG_HOME ?? path.join(os.homedir(), ".config"),
    CONFIG[browser],
  );

/**
 * INSTALL_DIR as packages/breadboard/chromium of this checkout, files that differ written
 * anew; the README stays behind. True when something changed: the browser
 * takes it at its next start.
 */
const syncExtension = () => {
  if (!fs.existsSync(SOURCE)) return false;
  fs.mkdirSync(INSTALL_DIR, { recursive: true });
  let isChanged = false;
  for (const file of fs
    .readdirSync(SOURCE)
    .filter(name => name !== "README.md")) {
    const from = fs.readFileSync(path.join(SOURCE, file));
    const to = path.join(INSTALL_DIR, file);
    if (fs.existsSync(to) && fs.readFileSync(to).equals(from)) continue;
    fs.writeFileSync(to, from);
    isChanged = true;
  }
  return isChanged;
};

/** Whether the flags file of `browser` loads INSTALL_DIR with `--load-extension`. */
const isInFlags = browser => {
  const flags = `${configOf(browser)}-flags.conf`;
  return (
    fs.existsSync(flags) &&
    fs.readFileSync(flags, "utf-8").includes(INSTALL_DIR)
  );
};

/**
 * Whether the browser knows the extension: loaded with `--load-extension` in
 * the flags file, or installed through "Load unpacked" (a profile's
 * preferences name its id).
 */
const hasExtension = browser => {
  if (isInFlags(browser)) return true;
  const config = configOf(browser);
  if (!fs.existsSync(config)) return false;
  return fs
    .readdirSync(config)
    .flatMap(profile =>
      ["Preferences", "Secure Preferences"].map(file =>
        path.join(config, profile, file),
      ),
    )
    .some(
      file =>
        fs.existsSync(file) &&
        fs.readFileSync(file, "utf-8").includes(EXTENSION_ID),
    );
};

/** The browser that can group tabs, null when there is none; looked up once, the copy synced with it. */
let found;
const groupingBrowser = () => {
  if (found !== undefined) return found;
  found =
    BROWSERS.find(browser => onPath(browser) && hasExtension(browser)) ?? null;
  if (found) syncExtension();
  return found;
};

const detached = (command, args) => {
  const child = spawn(command, args, { detached: true, stdio: "ignore" });
  child.on("error", () => {});
  child.unref();
};

/**
 * Open `url` in the tab group `group` (the label of a herdr space), without
 * waiting for the browser. True when something was opened.
 */
const openInGroup = (url, group) => {
  if (!url) return false;
  const browser = group && groupingBrowser();
  if (browser) {
    const page = `chrome-extension://${EXTENSION_ID}/open.html?${new URLSearchParams({ url, group })}`;
    detached(browser, [page]);
  } else {
    detached(process.platform === "darwin" ? "open" : "xdg-open", [url]);
  }
  return true;
};

const POPUP = { width: 960, height: 720 };
/** How long a new window of the browser is looked for; a cold start takes a moment. */
const POPUP_WAIT_MS = 8000;

const hyprctl = args => spawnSync("hyprctl", args, { encoding: "utf-8" });
const hyprClients = () => {
  try {
    return JSON.parse(hyprctl(["clients", "-j"]).stdout);
  } catch {
    return [];
  }
};

/**
 * Under Hyprland the new window would be tiled in like any other. The one this
 * opened — the new window of the app's class, `chrome-<host>__…` — floats
 * instead, at POPUP's size in the middle: a popup to drag around. Only that
 * window, from here; no rule goes into the machine's config. Elsewhere the
 * window manager places it, at the size `--window-size` asked for.
 */
const floatNewWindow = (host, before) => {
  if (!process.env.HYPRLAND_INSTANCE_SIGNATURE) return;
  const prefix = `chrome-${host}__`;
  const until = Date.now() + POPUP_WAIT_MS;
  const look = () => {
    const window = hyprClients().find(
      c => c.class.startsWith(prefix) && !before.has(c.address),
    );
    if (!window) {
      if (Date.now() < until) setTimeout(look, 150);
      return;
    }
    const target = `address:${window.address}`;
    // Hyprland ≥ 0.55 dispatches Lua (hl.dsp.*); an older one the classic dispatchers.
    const lua = [
      `hl.dsp.window.float({ action = "enable", window = "${target}" })`,
      `hl.dsp.window.resize({ x = ${POPUP.width}, y = ${POPUP.height}, window = "${target}" })`,
      `hl.dsp.window.center({ window = "${target}" })`,
      `hl.dsp.focus({ window = "${target}" })`,
    ];
    const isLua = hyprctl(["dispatch", lua[0]]).stdout.trim() === "ok";
    if (isLua) {
      for (const code of lua.slice(1)) hyprctl(["dispatch", code]);
      return;
    }
    hyprctl([
      "--batch",
      [
        `dispatch setfloating ${target}`,
        `dispatch resizewindowpixel exact ${POPUP.width} ${POPUP.height},${target}`,
        `dispatch focuswindow ${target}`,
        "dispatch centerwindow",
      ].join(" ; "),
    ]);
  };
  look();
};

/**
 * `url` in a small window of its own without tabs or address bar (`--app`),
 * floating under Hyprland, in the browser's own profile so a login there
 * counts: for a look at an issue or MR next to the terminal. Without Chromium
 * or Chrome it opens like any url.
 */
const openPopup = (url, group) => {
  if (!url) return false;
  const browser = BROWSERS.find(onPath);
  if (!browser) return openInGroup(url, group);
  const before = new Set(hyprClients().map(c => c.address));
  detached(browser, [
    `--app=${url}`,
    `--window-size=${POPUP.width},${POPUP.height}`,
  ]);
  floatNewWindow(new URL(url).host, before);
  return true;
};

/** Whether a url can go into a tab group, and so wait for an app that still starts. */
const canGroupTabs = () => Boolean(groupingBrowser());

export { canGroupTabs, openInGroup, openPopup };

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const isChanged = syncExtension();
  const browser = BROWSERS.find(onPath);
  console.log(
    `\n✓ the extension is in ${INSTALL_DIR}${isChanged ? " (updated: restart the browser)" : ""}`,
  );
  if (!browser) {
    console.log(
      "  no Chromium or Chrome found: urls open with xdg-open, without a group\n",
    );
  } else if (isInFlags(browser)) {
    console.log(`  ${browser} loads it (${configOf(browser)}-flags.conf)\n`);
  } else if (hasExtension(browser)) {
    console.log(`  ${browser} has it installed\n`);
  } else {
    console.log(`
  ${browser} doesn't load it yet. Either
    add ${INSTALL_DIR} to the --load-extension= list in ${configOf(browser)}-flags.conf and restart it, or
    chrome://extensions → Developer mode → Load unpacked → ${INSTALL_DIR}
`);
  }
}
